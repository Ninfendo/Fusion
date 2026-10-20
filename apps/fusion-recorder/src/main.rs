//! The market data recorder.
//!
//! ```text
//! fusion-recorder record [--venue testnet|public] [--symbols BTCUSDT,ETHUSDT] [--out DIR]
//!                        [--rotate-minutes 5] [--for-minutes N]
//! fusion-recorder check DIR
//! ```
//!
//! `record` streams Binance market data into Parquet files in `DIR`, with
//! every live book's fingerprint every 10 seconds, until Ctrl+C or for
//! `--for-minutes`; both close the last file cleanly. Gaps, reconnects and
//! fetches go to `DIR/recorder.log`.
//!
//! `check` replays a recording and prints, per symbol, what is missing and
//! whether the rebuilt books match. It exits with 1 unless the recording
//! is clean: no missing trade, no hole in a book, no failed check, no book
//! that had to start again, and no silence longer than a minute.

use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fusion_domain::Timestamp;
use fusion_recording::{
    BookCheck, Record, RecordingWriter, Replay, ReplayReport, SymbolMap, WriterConfig, read_file,
    recording_files, utc_stamp,
};
use fusion_venue_binance::live::{self, FeedItem, HttpFetch, LinkNote, LiveConfig};
use fusion_venue_binance::{Endpoint, Note, SymbolTable};
use tokio::sync::mpsc;

/// The watchlist's symbols.
const DEFAULT_SYMBOLS: &str = "BTCUSDT,ETHUSDT,SOLUSDT,BNBUSDT,XRPUSDT,ADAUSDT,DOGEUSDT,AVAXUSDT,LINKUSDT,DOTUSDT,LTCUSDT,TRXUSDT";

/// The longest silence a clean recording may have.
const MAX_SILENCE: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq)]
enum Command {
    Record {
        venue: Endpoint,
        venue_name: String,
        symbols: Vec<String>,
        out: PathBuf,
        rotate: Duration,
        stop_after: Option<Duration>,
    },
    Check(PathBuf),
}

fn parse(args: &[String]) -> Result<Command, String> {
    match args.split_first() {
        Some((cmd, rest)) if cmd == "record" => parse_record(rest),
        Some((cmd, [dir])) if cmd == "check" => Ok(Command::Check(PathBuf::from(dir))),
        _ => Err("usage: fusion-recorder record [options] | fusion-recorder check DIR".to_owned()),
    }
}

fn parse_record(args: &[String]) -> Result<Command, String> {
    let mut venue_name = "testnet".to_owned();
    let mut symbols = DEFAULT_SYMBOLS.to_owned();
    let mut out = PathBuf::from("recordings");
    let mut minutes = 5u64;
    let mut stop_after = None;
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--venue" => venue_name.clone_from(value),
            "--symbols" => symbols.clone_from(value),
            "--out" => out = PathBuf::from(value),
            "--rotate-minutes" => {
                minutes = value
                    .parse()
                    .map_err(|_| "--rotate-minutes needs a number")?;
            }
            "--for-minutes" => {
                let n: u64 = value.parse().map_err(|_| "--for-minutes needs a number")?;
                stop_after = Some(Duration::from_secs(n * 60));
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    let venue = Endpoint::from_name(&venue_name).ok_or("--venue is testnet or public")?;
    Ok(Command::Record {
        venue,
        venue_name,
        symbols: symbols
            .split(',')
            .map(|s| s.trim().to_ascii_uppercase())
            .collect(),
        out,
        rotate: Duration::from_secs(minutes.max(1) * 60),
        stop_after,
    })
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match parse(&args) {
        Ok(Command::Check(dir)) => check(&dir),
        Ok(record) => run_record(record).map(|()| true),
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

fn now() -> Timestamp {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX));
    Timestamp::from_unix_nanos(nanos)
}

fn run_record(command: Command) -> Result<(), Box<dyn Error>> {
    let Command::Record {
        venue,
        venue_name,
        symbols,
        out,
        rotate,
        stop_after,
    } = command
    else {
        return Ok(());
    };
    std::fs::create_dir_all(&out)?;
    let config = WriterConfig {
        rotate_every: rotate,
        ..WriterConfig::new(&out, format!("binance-{venue_name}"))
    };
    let writer = RecordingWriter::new(config, symbols.clone());
    let table = SymbolTable::new(&symbols);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(record(
        LiveConfig::new(venue, table),
        writer,
        &out,
        stop_after,
    ))
}

async fn record(
    config: LiveConfig,
    mut writer: RecordingWriter,
    out: &Path,
    stop_after: Option<Duration>,
) -> Result<(), Box<dyn Error>> {
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("recorder.log"))?;
    let symbols = config.symbols.clone();
    let (tx, mut rx) = mpsc::channel(8192);
    tokio::spawn(live::run(config, HttpFetch::new()?, tx));
    writeln!(
        log,
        "{} started: {}",
        utc_stamp(now()),
        symbols_line(&symbols)
    )?;
    let mut flush = tokio::time::interval(Duration::from_secs(10));
    // Far enough away to mean never.
    let never = Duration::from_secs(100 * 365 * 86_400);
    let stop = tokio::time::sleep(stop_after.unwrap_or(never));
    tokio::pin!(stop);
    loop {
        tokio::select! {
            item = rx.recv() => match item {
                Some(item) => take(item, &mut writer, &mut log, &symbols)?,
                None => break,
            },
            _ = flush.tick() => writer.flush()?,
            _ = tokio::signal::ctrl_c() => break,
            () = &mut stop => break,
        }
    }
    let rows = writer.rows();
    writer.close()?;
    writeln!(log, "{} stopped after {rows} rows", utc_stamp(now()))?;
    Ok(())
}

fn symbols_line(symbols: &SymbolTable) -> String {
    symbols
        .ids()
        .filter_map(|id| symbols.symbol(id))
        .collect::<Vec<_>>()
        .join(",")
}

fn take(
    item: FeedItem,
    writer: &mut RecordingWriter,
    log: &mut std::fs::File,
    symbols: &SymbolTable,
) -> Result<(), Box<dyn Error>> {
    let at = now();
    match item {
        FeedItem::Event(event) => writer.write(at, &Record::Event(event))?,
        FeedItem::Check {
            instrument,
            update_id,
            digest,
        } => writer.write(
            at,
            &Record::Check(BookCheck {
                instrument,
                update_id,
                digest,
            }),
        )?,
        FeedItem::Note(note) => {
            writeln!(log, "{} {}", utc_stamp(at), describe_note(&note, symbols))?
        }
        FeedItem::Link(link) => writeln!(log, "{} {}", utc_stamp(at), describe_link(&link))?,
    }
    Ok(())
}

fn describe_note(note: &Note, symbols: &SymbolTable) -> String {
    let name = |id| symbols.symbol(id).unwrap_or("?");
    match note {
        Note::BookGap(id, gap) => format!(
            "{}: book gap, expected update {} got {}; starting again",
            name(*id),
            gap.expected,
            gap.got
        ),
        Note::SnapshotTooOld(id) => format!("{}: snapshot too old; asking again", name(*id)),
        Note::BufferFull(id) => format!("{}: buffer full; starting again", name(*id)),
        Note::MissedTrades(id, range) => format!(
            "{}: missed trades {}..{}; fetching",
            name(*id),
            range.start,
            range.end
        ),
        Note::TradesLost(id, ranges) => format!("{}: trades lost {ranges:?}", name(*id)),
    }
}

fn describe_link(link: &LinkNote) -> String {
    match link {
        LinkNote::Opened(n) => format!("connection {n} open"),
        LinkNote::Closed(n, why) => format!("connection {n} closed: {why}"),
        LinkNote::TookOver { from, to } => format!("connection {to} took over from {from}"),
        LinkNote::FetchFailed { error, attempt } => {
            format!("fetch failed (attempt {}): {error}", attempt + 1)
        }
        LinkNote::BadMessage(why) => format!("bad message: {why}"),
    }
}

/// Replays the recording in `dir` and prints the report. True if clean.
fn check(dir: &Path) -> Result<bool, Box<dyn Error>> {
    let files = recording_files(dir)?;
    let mut symbols = SymbolMap::default();
    let mut replay = Replay::default();
    for file in &files {
        for row in read_file(file, &mut symbols)? {
            replay.apply(&row);
        }
    }
    let report = replay.finish();
    print_report(&report, &symbols, files.len());
    Ok(verdict(&report))
}

fn verdict(report: &ReplayReport) -> bool {
    let quiet_enough = report
        .longest_silence
        .is_none_or(|(_, n)| u128::try_from(n).unwrap_or(0) <= MAX_SILENCE.as_nanos());
    let no_restarts = report.instruments.values().all(|s| s.snapshots <= 1);
    report.rows > 0 && report.is_clean() && quiet_enough && no_restarts
}

fn print_report(report: &ReplayReport, symbols: &SymbolMap, files: usize) {
    let hours = |(a, b): (Timestamp, Timestamp)| (b.unix_nanos() - a.unix_nanos()) as f64 / 3.6e12;
    println!("files {files}, rows {}", report.rows);
    if let Some(span) = report.span {
        println!(
            "from {} to {} ({:.2} hours)",
            utc_stamp(span.0),
            utc_stamp(span.1),
            hours(span)
        );
    }
    if let Some((at, nanos)) = report.longest_silence {
        println!(
            "longest silence {:.1} s, from {}",
            nanos as f64 / 1e9,
            utc_stamp(at)
        );
    }
    println!("symbol      trades  missing  snapshots  deltas  holes  checks ok  failed");
    for (id, s) in &report.instruments {
        let missing: u64 = s.trade_gaps.iter().map(|r| r.end - r.start).sum();
        println!(
            "{:<10} {:>7} {:>8} {:>10} {:>7} {:>6} {:>10} {:>7}",
            symbols.symbol(*id).unwrap_or("?"),
            s.trades,
            missing,
            s.snapshots,
            s.deltas,
            s.book_gaps,
            s.checks_passed,
            s.checks_failed.len()
        );
    }
    println!(
        "{}",
        if verdict(report) {
            "CLEAN"
        } else {
            "NOT CLEAN"
        }
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn the_command_line_reads() {
        assert_eq!(
            parse(&args("check out/dir")),
            Ok(Command::Check(PathBuf::from("out/dir")))
        );
        let Ok(Command::Record {
            venue,
            symbols,
            rotate,
            stop_after,
            ..
        }) = parse(&args(
            "record --venue public --symbols btcusdt,ethusdt --rotate-minutes 1 --for-minutes 2",
        ))
        else {
            panic!("a record command");
        };
        assert_eq!(venue, Endpoint::Public);
        assert_eq!(symbols, ["BTCUSDT", "ETHUSDT"]);
        assert_eq!(rotate, Duration::from_secs(60));
        assert_eq!(stop_after, Some(Duration::from_secs(120)));
        assert!(parse(&args("record --for-minutes soon")).is_err());
        let Ok(Command::Record { symbols, out, .. }) = parse(&args("record")) else {
            panic!("defaults");
        };
        assert_eq!(symbols.len(), 12);
        assert_eq!(out, PathBuf::from("recordings"));
        assert!(parse(&args("")).is_err());
        assert!(parse(&args("record --venue live")).is_err());
        assert!(parse(&args("record --out")).is_err());
        assert!(parse(&args("record --rotate-minutes x")).is_err());
        assert!(parse(&args("record --fast yes")).is_err());
        assert!(parse(&args("check a b")).is_err());
    }
}
