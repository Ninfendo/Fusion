//! Write a recording, read it back, replay it: the books must come back the
//! same, and gaps must show.

use std::path::PathBuf;
use std::time::Duration;

use fusion_domain::{
    BookDelta, BookSnapshot, InstrumentId, Level, MarketEvent, OrderBook, Side, Timestamp,
    TopOfBook, Trade,
};
use fusion_recording::{
    BookCheck, Record, RecordingError, RecordingWriter, Replay, Row, SymbolMap, WriterConfig,
    read_file, recording_files,
};

const BTC: InstrumentId = InstrumentId(1);
const ETH: InstrumentId = InstrumentId(2);

/// A fresh folder for one test.
fn folder(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fusion-recording-{}-{test}", std::process::id()));
    // A leftover from an earlier run of this test; ours to remove.
    let _removed = std::fs::remove_dir_all(&dir);
    dir
}

fn level(price: &str, quantity: &str) -> Level {
    Level::new(
        price.parse().expect("a price"),
        quantity.parse().expect("a quantity"),
    )
}

fn at_ms(ms: i64) -> Timestamp {
    Timestamp::from_unix_millis(1_792_514_598_000 + ms).expect("a time")
}

fn trade(instrument: InstrumentId, id: u64) -> Record {
    Record::Event(MarketEvent::Trade(Trade {
        instrument,
        id,
        price: "82731.35".parse().expect("a price"),
        quantity: "0.00151".parse().expect("a quantity"),
        taker: if id.is_multiple_of(2) {
            Side::Buy
        } else {
            Side::Sell
        },
        time: at_ms(i64::try_from(id).expect("small")),
    }))
}

/// A session: ETH trades, then a BTC book that starts, changes, is checked,
/// and starts again; best prices; a missing trade filled in later.
fn session() -> Vec<Record> {
    let snapshot = BookSnapshot {
        instrument: BTC,
        update_id: 10,
        bids: vec![level("100.5", "1"), level("100", "2")],
        asks: vec![level("101", "3")],
    };
    let mut book = OrderBook::from_snapshot(&snapshot);
    let mut records = vec![
        trade(ETH, 500),
        Record::Event(MarketEvent::Snapshot(snapshot)),
    ];
    for n in 11..=20u64 {
        let delta = BookDelta {
            instrument: BTC,
            first_update: n,
            last_update: n,
            time: at_ms(i64::try_from(n).expect("small")),
            bids: vec![level(&format!("100.{n}"), "0.5")],
            asks: vec![level("101", if n % 2 == 0 { "0" } else { "4" })],
        };
        book.apply(&delta).expect("fits");
        records.push(Record::Event(MarketEvent::Delta(delta)));
    }
    records.push(Record::Check(BookCheck {
        instrument: BTC,
        update_id: book.update_id(),
        digest: book.digest(),
    }));
    records.push(Record::Event(MarketEvent::Top(TopOfBook {
        instrument: BTC,
        update_id: 20,
        bid: level("100.20", "0.5"),
        ask: level("101.5", "1"),
    })));
    // 503 is missed and fetched later, so it arrives after 504.
    for id in [501, 502, 504, 503, 505] {
        records.push(trade(ETH, id));
    }
    // The book starts again from a new snapshot.
    records.push(Record::Event(MarketEvent::Snapshot(BookSnapshot {
        instrument: BTC,
        update_id: 30,
        bids: vec![],
        asks: vec![level("102", "1")],
    })));
    records
}

/// Writes `records` one 100 ms apart; files rotate every second.
fn write(dir: &PathBuf, records: &[Record]) -> Vec<PathBuf> {
    let config = WriterConfig {
        rotate_every: Duration::from_secs(1),
        batch_rows: 3,
        ..WriterConfig::new(dir, "test")
    };
    let mut writer = RecordingWriter::new(config, vec!["BTCUSDT".into(), "ETHUSDT".into()]);
    for (n, record) in records.iter().enumerate() {
        let ms = i64::try_from(n).expect("small") * 100;
        writer.write(at_ms(ms), record).expect("written");
    }
    assert_eq!(writer.rows(), records.len() as u64);
    writer.close().expect("closed");
    recording_files(dir).expect("listed")
}

fn read_all(files: &[PathBuf]) -> (Vec<Row>, SymbolMap) {
    let mut symbols = SymbolMap::default();
    let rows = files
        .iter()
        .flat_map(|f| read_file(f, &mut symbols).expect("read"))
        .collect();
    (rows, symbols)
}

#[test]
fn a_recording_reads_back_row_for_row() {
    let dir = folder("rows");
    let records = session();
    let files = write(&dir, &records);
    assert_eq!(
        files.len(),
        2,
        "20 rows 100 ms apart, a new file every second"
    );
    assert!(
        files[0]
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("test-20261020T164318Z")),
        "{files:?}"
    );
    let (rows, symbols) = read_all(&files);
    // The reader numbers symbols as it meets them: ETH first.
    assert_eq!(symbols.symbols(), ["ETHUSDT", "BTCUSDT"]);
    assert_eq!(symbols.symbol(InstrumentId(1)), Some("ETHUSDT"));
    assert_eq!(symbols.symbol(InstrumentId(3)), None);
    let swap = |id: InstrumentId| if id == BTC { ETH } else { BTC };
    let read: Vec<Record> = rows.iter().map(|r| renumber(&r.record, swap)).collect();
    assert_eq!(read, records);
    let seqs: Vec<u64> = rows.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, (0..records.len() as u64).collect::<Vec<_>>());
    assert_eq!(rows[1].received, at_ms(100));
}

/// The same record with its instrument mapped.
fn renumber(record: &Record, map: impl Fn(InstrumentId) -> InstrumentId) -> Record {
    let mut record = record.clone();
    match &mut record {
        Record::Event(MarketEvent::Trade(t)) => t.instrument = map(t.instrument),
        Record::Event(MarketEvent::Top(t)) => t.instrument = map(t.instrument),
        Record::Event(MarketEvent::Snapshot(s)) => s.instrument = map(s.instrument),
        Record::Event(MarketEvent::Delta(d)) => d.instrument = map(d.instrument),
        Record::Check(c) => c.instrument = map(c.instrument),
    }
    record
}

#[test]
fn a_replay_rebuilds_the_same_books_and_finds_no_gaps() {
    let dir = folder("replay");
    let (rows, symbols) = read_all(&write(&dir, &session()));
    let mut replay = Replay::default();
    for row in &rows {
        replay.apply(row);
    }
    let btc = symbols
        .symbols()
        .iter()
        .position(|s| s == "BTCUSDT")
        .expect("BTC")
        + 1;
    let btc = InstrumentId(u32::try_from(btc).expect("small"));
    assert_eq!(replay.book(btc).map(OrderBook::update_id), Some(30));
    let report = replay.finish();
    assert!(report.is_clean(), "{report:?}");
    assert_eq!(report.rows, rows.len() as u64);
    assert_eq!(
        report.span,
        Some((at_ms(0), at_ms(100 * (rows.len() as i64 - 1))))
    );
    let book = &report.instruments[&btc];
    assert_eq!((book.snapshots, book.deltas, book.tops), (2, 10, 1));
    assert_eq!((book.checks_passed, book.checks_failed.len()), (1, 0));
    let eth = report
        .instruments
        .iter()
        .find(|(id, _)| **id != btc)
        .map(|(_, r)| r)
        .expect("ETH");
    assert_eq!(eth.trades, 6);
    assert!(eth.trade_gaps.is_empty(), "503 arrived late, not lost");
}

#[test]
fn a_replay_shows_holes_and_wrong_books() {
    let mut records = session();
    // Lose trade 503 and the change to update 15, and break the check.
    records.retain(|r| match r {
        Record::Event(MarketEvent::Trade(t)) => t.id != 503,
        Record::Event(MarketEvent::Delta(d)) => d.last_update != 15,
        _ => true,
    });
    let mut replay = Replay::default();
    for (n, record) in records.iter().enumerate() {
        replay.apply(&Row {
            seq: n as u64,
            received: at_ms(0),
            record: record.clone(),
        });
    }
    // A change before any snapshot, and a check on a book that is not there.
    let stray = BookDelta {
        instrument: InstrumentId(3),
        first_update: 1,
        last_update: 1,
        time: at_ms(0),
        bids: vec![],
        asks: vec![],
    };
    for record in [
        Record::Event(MarketEvent::Delta(stray)),
        Record::Check(BookCheck {
            instrument: InstrumentId(3),
            update_id: 1,
            digest: 0,
        }),
    ] {
        replay.apply(&Row {
            seq: 99,
            received: at_ms(0),
            record,
        });
    }
    let report = replay.finish();
    assert!(!report.is_clean());
    let btc = &report.instruments[&BTC];
    assert_eq!(
        btc.book_gaps, 5,
        "16 to 20 no longer fit after 15 went missing"
    );
    assert_eq!(btc.checks_failed, [20]);
    let gaps = &report.instruments[&ETH].trade_gaps;
    assert_eq!((gaps.len(), gaps.first()), (1, Some(&(503..504))));
    let stray = &report.instruments[&InstrumentId(3)];
    assert_eq!((stray.book_gaps, stray.checks_failed.len()), (1, 1));
}

#[test]
fn files_being_written_are_left_out() {
    let dir = folder("partial");
    std::fs::create_dir_all(&dir).expect("a folder");
    std::fs::write(dir.join("test-x.parquet.partial"), b"half").expect("written");
    std::fs::write(dir.join("notes.txt"), b"x").expect("written");
    assert!(recording_files(&dir).expect("listed").is_empty());
    assert!(recording_files(&dir.join("missing")).is_err());
}

#[test]
fn two_files_starting_in_the_same_second_get_different_names() {
    let dir = folder("names");
    for _ in 0..3 {
        let mut writer =
            RecordingWriter::new(WriterConfig::new(&dir, "same"), vec!["BTCUSDT".into()]);
        writer.write(at_ms(0), &trade(BTC, 1)).expect("written");
        writer.close().expect("closed");
    }
    let names: Vec<String> = recording_files(&dir)
        .expect("listed")
        .iter()
        .filter_map(|p| p.file_name()?.to_str().map(str::to_owned))
        .collect();
    assert_eq!(
        names,
        [
            "same-20261020T164318Z-2.parquet",
            "same-20261020T164318Z-3.parquet",
            "same-20261020T164318Z.parquet"
        ]
    );
}

#[test]
fn a_writer_refuses_what_it_cannot_record() {
    let dir = folder("refuse");
    let mut writer = RecordingWriter::new(WriterConfig::new(&dir, "x"), vec!["BTCUSDT".into()]);
    assert!(matches!(
        writer.write(at_ms(0), &trade(InstrumentId(2), 1)),
        Err(RecordingError::UnknownInstrument(2))
    ));
    assert!(matches!(
        writer.write(at_ms(0), &trade(InstrumentId(0), 1)),
        Err(RecordingError::UnknownInstrument(0))
    ));
    let too_fine = Record::Event(MarketEvent::Top(TopOfBook {
        instrument: BTC,
        update_id: 1,
        bid: level("0.0000000000001", "1"),
        ask: level("1", "1"),
    }));
    let err = writer.write(at_ms(0), &too_fine).expect_err("refused");
    assert!(err.to_string().contains("more than 12 decimals"), "{err}");
    assert_eq!(writer.rows(), 0, "nothing was written");
    assert_eq!(
        writer.close().expect("closed"),
        Some(dir.join("x-20261020T164318Z.parquet"))
    );
    assert_eq!(
        writer.close().expect("closed"),
        None,
        "nothing open any more"
    );
    writer.flush().expect("nothing to flush");
}

#[test]
fn a_file_that_is_not_a_recording_is_refused() {
    let dir = folder("foreign");
    std::fs::create_dir_all(&dir).expect("a folder");
    let path = dir.join("other.parquet");
    write_foreign_parquet(&path);
    let err = read_file(&path, &mut SymbolMap::default()).expect_err("refused");
    assert!(matches!(err, RecordingError::NotARecording(_)), "{err}");
    std::fs::write(dir.join("broken.parquet"), b"not parquet").expect("written");
    assert!(read_file(&dir.join("broken.parquet"), &mut SymbolMap::default()).is_err());
    assert!(read_file(&dir.join("missing.parquet"), &mut SymbolMap::default()).is_err());
}

/// A valid Parquet file with some other schema.
fn write_foreign_parquet(path: &PathBuf) {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int32Array, RecordBatch};
    let column: ArrayRef = Arc::new(Int32Array::from(vec![1, 2]));
    let batch = RecordBatch::try_from_iter([("n", column)]).expect("a batch");
    let file = std::fs::File::create(path).expect("created");
    let mut writer =
        parquet::arrow::ArrowWriter::try_new(file, batch.schema(), None).expect("a writer");
    writer.write(&batch).expect("written");
    writer.close().expect("closed");
}

#[test]
fn rows_go_to_the_file_in_batches() {
    use parquet::file::reader::{FileReader as _, SerializedFileReader};

    let dir = folder("batches");
    let config = WriterConfig {
        batch_rows: 3,
        ..WriterConfig::new(&dir, "batch")
    };
    let mut writer = RecordingWriter::new(config, vec!["BTCUSDT".into()]);
    for id in 1..=7 {
        writer.write(at_ms(0), &trade(BTC, id)).expect("written");
    }
    let path = writer.close().expect("closed").expect("a file");
    let file = std::fs::File::open(&path).expect("opens");
    let groups: Vec<i64> = SerializedFileReader::new(file)
        .expect("a parquet file")
        .metadata()
        .row_groups()
        .iter()
        .map(|g| g.num_rows())
        .collect();
    assert_eq!(groups, [3, 3, 1], "batches of 3, then the rest at close");
}
