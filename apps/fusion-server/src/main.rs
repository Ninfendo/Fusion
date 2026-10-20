//! The Fusion server.
//!
//! ```text
//! fusion-server [--venue testnet|public] [--symbols BTCUSDT,ETHUSDT] [--listen 127.0.0.1:7070]
//! ```
//!
//! Streams Binance market data into the view server's tables (`prices`,
//! and `book/<SYMBOL>` per instrument) and serves them over WebSocket at
//! `ws://<listen>`. Layouts are kept in memory for now. Gaps, reconnects
//! and failed fetches are printed. Stops on Ctrl+C.

use std::error::Error;
use std::process::ExitCode;

use fusion_serve::{MemoryLayouts, ServeConfig, serve};
use fusion_venue_binance::live::{self, FeedItem, HttpFetch, LiveConfig};
use fusion_venue_binance::{Endpoint, SymbolTable};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// The watchlist's symbols.
const DEFAULT_SYMBOLS: &str = "BTCUSDT,ETHUSDT,SOLUSDT,BNBUSDT,XRPUSDT,ADAUSDT,DOGEUSDT,AVAXUSDT,LINKUSDT,DOTUSDT,LTCUSDT,TRXUSDT";

#[derive(Debug, PartialEq)]
struct Options {
    venue: Endpoint,
    symbols: Vec<String>,
    listen: String,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        venue: Endpoint::Testnet,
        symbols: split(DEFAULT_SYMBOLS),
        listen: "127.0.0.1:7070".to_owned(),
    };
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--venue" => {
                options.venue = Endpoint::from_name(value).ok_or("--venue is testnet or public")?;
            }
            "--symbols" => options.symbols = split(value),
            "--listen" => options.listen.clone_from(value),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(options)
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
    let listener = TcpListener::bind(&options.listen).await?;
    eprintln!(
        "serving {} symbols at ws://{}",
        options.symbols.len(),
        listener.local_addr()?
    );
    let config = LiveConfig::new(options.venue, SymbolTable::new(&options.symbols));
    let (feed_tx, mut feed) = mpsc::channel(8192);
    tokio::spawn(live::run(config, HttpFetch::new()?, feed_tx));
    let (market_tx, market) = mpsc::channel(8192);
    // Market events go to the view server; the rest is printed.
    tokio::spawn(async move {
        while let Some(item) = feed.recv().await {
            match item {
                FeedItem::Event(event) => {
                    if market_tx.send(event).await.is_err() {
                        return;
                    }
                }
                FeedItem::Note(note) => eprintln!("{note:?}"),
                FeedItem::Link(link) => eprintln!("{link:?}"),
                FeedItem::Check { .. } => {}
            }
        }
    });
    tokio::select! {
        () = serve(listener, &options.symbols, market, MemoryLayouts::default(), ServeConfig::default()) => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    Ok(())
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
        assert_eq!(defaults.venue, Endpoint::Testnet);
        assert_eq!(defaults.symbols.len(), 12);
        assert_eq!(defaults.listen, "127.0.0.1:7070");
        let set = parse(&args(
            "--venue public --symbols btcusdt,,ethusdt --listen 0.0.0.0:9000",
        ))
        .expect("options");
        assert_eq!(set.venue, Endpoint::Public);
        assert_eq!(set.symbols, ["BTCUSDT", "ETHUSDT"]);
        assert_eq!(set.listen, "0.0.0.0:9000");
        assert!(parse(&args("--venue live")).is_err());
        assert!(parse(&args("--listen")).is_err());
        assert!(parse(&args("--fast yes")).is_err());
    }
}
