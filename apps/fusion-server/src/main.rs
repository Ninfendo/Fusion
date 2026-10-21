//! The Fusion server.
//!
//! ```text
//! fusion-server [--venue testnet|public|synthetic] [--symbols BTCUSDT,ETHUSDT]
//!               [--instruments 500] [--rate 10000] [--listen 127.0.0.1:7070]
//!               [--database postgres://USER:PASSWORD@HOST:PORT/DB]
//! ```
//!
//! Streams Binance market data into the view server's tables (`prices`,
//! and `book/<SYMBOL>` per instrument) and serves them over WebSocket at
//! `ws://<listen>`. `--venue synthetic` serves a made-up market instead:
//! `--instruments` of them (`SYN0001` and on), with `--rate` changes a
//! second in all, for load tests and for working offline. Layouts are kept in PostgreSQL with `--database`, and
//! in memory without it. Gaps, reconnects and failed fetches are printed.
//! Stops on Ctrl+C.

use std::error::Error;
use std::future::Future;
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use fusion_domain::{MarketEvent, Timestamp};
use fusion_sim::SyntheticMarket;

use fusion_persistence_pg::PgLayouts;
use fusion_serve::{MemoryLayouts, ServeConfig, serve};
use fusion_venue_binance::live::{self, FeedItem, HttpFetch, LiveConfig};
use fusion_venue_binance::{Endpoint, SymbolTable};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// The watchlist's symbols.
const DEFAULT_SYMBOLS: &str = "BTCUSDT,ETHUSDT,SOLUSDT,BNBUSDT,XRPUSDT,ADAUSDT,DOGEUSDT,AVAXUSDT,LINKUSDT,DOTUSDT,LTCUSDT,TRXUSDT";

/// Where the market data comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Source {
    Binance(Endpoint),
    Synthetic,
}

#[derive(Debug, PartialEq)]
struct Options {
    source: Source,
    symbols: Vec<String>,
    /// How many made-up instruments.
    instruments: usize,
    /// Made-up changes a second, in all.
    rate: u32,
    listen: String,
    /// Where layouts are kept; in memory without one.
    database: Option<String>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        source: Source::Binance(Endpoint::Testnet),
        symbols: split(DEFAULT_SYMBOLS),
        instruments: 500,
        rate: 10_000,
        listen: "127.0.0.1:7070".to_owned(),
        database: None,
    };
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--venue" => options.source = source(value)?,
            "--instruments" => {
                options.instruments = value.parse().map_err(|_| "--instruments needs a number")?;
            }
            "--rate" => options.rate = value.parse().map_err(|_| "--rate needs a number")?,
            "--symbols" => options.symbols = split(value),
            "--listen" => options.listen.clone_from(value),
            "--database" => options.database = Some(value.clone()),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(options)
}

fn source(name: &str) -> Result<Source, String> {
    if name == "synthetic" {
        return Ok(Source::Synthetic);
    }
    let venue = Endpoint::from_name(name).ok_or("--venue is testnet, public or synthetic")?;
    Ok(Source::Binance(venue))
}

fn split(symbols: &str) -> Vec<String> {
    symbols
        .split(',')
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse(&args) {
        Ok(options) => options,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(options)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

async fn run(options: Options) -> Result<(), Box<dyn Error>> {
    // The database first: a wrong address stops the server at once.
    let database = match &options.database {
        Some(url) => Some(fusion_persistence_pg::connect(url).await?),
        None => None,
    };
    let listener = TcpListener::bind(&options.listen).await?;
    let (market_tx, market) = mpsc::channel(8192);
    let symbols = match options.source {
        Source::Binance(venue) => {
            binance(venue, &options.symbols, market_tx)?;
            options.symbols.clone()
        }
        Source::Synthetic => {
            let sim = SyntheticMarket::new(options.instruments, 1);
            let symbols = sim.symbols();
            tokio::spawn(synthetic(sim, options.rate, market_tx));
            symbols
        }
    };
    eprintln!(
        "serving {} symbols at ws://{}",
        symbols.len(),
        listener.local_addr()?
    );
    let config = ServeConfig::default();
    match database {
        Some(pool) => {
            eprintln!("layouts are kept in the database");
            let layouts = PgLayouts::new(pool);
            until_stopped(serve(listener, &symbols, market, layouts, config)).await;
        }
        None => {
            eprintln!("layouts are kept in memory until the server stops");
            let layouts = MemoryLayouts::default();
            until_stopped(serve(listener, &symbols, market, layouts, config)).await;
        }
    }
    Ok(())
}

/// Streams Binance market data to `market`; notes and link changes are
/// printed.
fn binance(
    venue: Endpoint,
    symbols: &[String],
    market: mpsc::Sender<MarketEvent>,
) -> Result<(), Box<dyn Error>> {
    let config = LiveConfig::new(venue, SymbolTable::new(symbols));
    let (feed_tx, mut feed) = mpsc::channel(8192);
    tokio::spawn(live::run(config, HttpFetch::new()?, feed_tx));
    tokio::spawn(async move {
        while let Some(item) = feed.recv().await {
            match item {
                FeedItem::Event(event) => {
                    if market.send(event).await.is_err() {
                        return;
                    }
                }
                FeedItem::Note(note) => eprintln!("{note:?}"),
                FeedItem::Link(link) => eprintln!("{link:?}"),
                FeedItem::Check { .. } => {}
            }
        }
    });
    Ok(())
}

/// How often the made-up market sends a batch.
const BATCH: Duration = Duration::from_millis(10);
/// How often the made-up market says how fast it really went.
const REPORT: Duration = Duration::from_secs(10);

/// The wall clock, as a market timestamp.
fn wall_clock() -> Timestamp {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_nanos()).ok())
        .map_or(Timestamp::UNIX_EPOCH, Timestamp::from_unix_nanos)
}

/// Streams the made-up market to `market`: every book first, then `rate`
/// changes a second. The changes due are counted from the time gone by, so
/// a coarse timer (about 15.6 ms on Windows) does not slow it down; a batch
/// catches up at most a tenth of a second. Every 10 s it prints how many
/// changes went out, which is lower than asked when the server cannot keep
/// up.
async fn synthetic(mut sim: SyntheticMarket, rate: u32, market: mpsc::Sender<MarketEvent>) {
    for event in sim.snapshots() {
        if market.send(event).await.is_err() {
            return;
        }
    }
    let catch_up = u64::from(rate / 10).max(1);
    let mut batches = tokio::time::interval(BATCH);
    batches.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let start = tokio::time::Instant::now();
    let (mut sent, mut reported, mut since) = (0u64, 0u64, start);
    loop {
        batches.tick().await;
        let micros = start.elapsed().as_micros() * u128::from(rate) / 1_000_000;
        let due = u64::try_from(micros)
            .unwrap_or(u64::MAX)
            .min(sent + catch_up);
        let now = wall_clock();
        for _ in sent..due {
            for event in sim.step(now) {
                if market.send(event).await.is_err() {
                    return;
                }
            }
        }
        sent = sent.max(due);
        if since.elapsed() >= REPORT {
            let seconds = since.elapsed().as_secs_f64();
            eprintln!("synthetic: {} changes in {seconds:.1} s", sent - reported);
            (reported, since) = (sent, tokio::time::Instant::now());
        }
    }
}

/// Runs `server` until it ends or Ctrl+C is pressed.
async fn until_stopped(server: impl Future<Output = ()>) {
    tokio::select! {
        () = server => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn the_command_line_reads() {
        let defaults = parse(&[]).expect("defaults");
        assert_eq!(defaults.source, Source::Binance(Endpoint::Testnet));
        assert_eq!((defaults.instruments, defaults.rate), (500, 10_000));
        assert_eq!(defaults.symbols.len(), 12);
        assert_eq!(defaults.listen, "127.0.0.1:7070");
        let set = parse(&args(
            "--venue public --symbols btcusdt,,ethusdt --listen 0.0.0.0:9000",
        ))
        .expect("options");
        assert_eq!(set.source, Source::Binance(Endpoint::Public));
        assert_eq!(set.symbols, ["BTCUSDT", "ETHUSDT"]);
        assert_eq!(set.listen, "0.0.0.0:9000");
        assert_eq!(set.database, None);
        let with_database =
            parse(&args("--database postgres://u:p@h:5432/fusion")).expect("options");
        assert_eq!(
            with_database.database.as_deref(),
            Some("postgres://u:p@h:5432/fusion")
        );
        assert!(parse(&args("--venue live")).is_err());
        assert!(parse(&args("--listen")).is_err());
        assert!(parse(&args("--fast yes")).is_err());
    }

    #[test]
    fn the_made_up_market_has_its_own_options() {
        let sim = parse(&args("--venue synthetic --instruments 50 --rate 200")).expect("options");
        assert_eq!(
            (sim.source, sim.instruments, sim.rate),
            (Source::Synthetic, 50, 200)
        );
        assert!(parse(&args("--instruments many")).is_err());
        assert!(parse(&args("--rate fast")).is_err());
        assert_eq!(source("synthetic"), Ok(Source::Synthetic));
    }
}
