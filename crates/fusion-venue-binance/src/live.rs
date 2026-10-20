//! The live connection: streams from Binance, fetches what the [`Feed`]
//! asks for, and keeps going for days.
//!
//! - One stream connection carries every instrument. Binance closes a
//!   connection after 24 hours, so a second one opens before that
//!   ([`LiveConfig::rotate_after`]). For each instrument the new connection
//!   takes over once its data has caught up with the old one, so nothing
//!   is missed and no false gap appears. Then the old one closes.
//! - A connection that goes quiet for [`LiveConfig::stall_after`] is
//!   closed and opened again. A lost connection is opened again after a
//!   wait that doubles up to [`MAX_RETRY`].
//! - Snapshots and missed trades are fetched as the feed asks. A failed
//!   fetch is tried again after a wait that doubles up to [`MAX_RETRY`].

mod connection;
mod fetch;
mod overlap;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use fusion_domain::{BookSnapshot, InstrumentId, MarketEvent, Trade};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Utf8Bytes;

pub use fetch::{Fetch, HttpFetch, REQUEST_TIMEOUT};

use crate::feed::{Feed, Note, Output, Want};
use crate::symbols::{Endpoint, SymbolTable};
use crate::wire::{Message, parse_snapshot, parse_stream, parse_trades};
use overlap::{Admit, Overlap};

/// The longest wait before trying again.
pub const MAX_RETRY: Duration = Duration::from_secs(30);

/// How often the live loop looks at its connections.
const TICK: Duration = Duration::from_millis(100);

/// Installs ring as the TLS crypto. Installing twice is fine.
pub(crate) fn install_tls() {
    // Err only means it is installed already.
    let _installed = rustls::crypto::ring::default_provider().install_default();
}

/// The two base addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// WebSocket base, for example `wss://stream.testnet.binance.vision`.
    pub stream: String,
    /// REST base, for example `https://testnet.binance.vision`.
    pub rest: String,
}

impl From<Endpoint> for Endpoints {
    fn from(endpoint: Endpoint) -> Self {
        Self {
            stream: endpoint.stream_base().to_owned(),
            rest: endpoint.rest_base().to_owned(),
        }
    }
}

/// How the live connection behaves.
#[derive(Debug, Clone)]
pub struct LiveConfig {
    /// Where to connect.
    pub endpoints: Endpoints,
    /// What to follow.
    pub symbols: SymbolTable,
    /// When to open the next connection. Binance closes one at 24 hours.
    pub rotate_after: Duration,
    /// How long a connection may be silent before it is replaced.
    pub stall_after: Duration,
    /// How long two connections may overlap before the old one closes,
    /// caught up or not.
    pub overlap_limit: Duration,
    /// The first wait before trying again.
    pub first_retry: Duration,
    /// How often each live book's fingerprint is passed on.
    pub check_every: Duration,
}

impl LiveConfig {
    /// The settings for a long run: rotate after 23 hours, replace a
    /// connection silent for 90 seconds.
    pub fn new(endpoint: Endpoint, symbols: SymbolTable) -> Self {
        Self {
            endpoints: endpoint.into(),
            symbols,
            rotate_after: Duration::from_secs(23 * 3600),
            stall_after: Duration::from_secs(90),
            overlap_limit: Duration::from_secs(60),
            first_retry: Duration::from_secs(1),
            check_every: Duration::from_secs(10),
        }
    }
}

/// What the connection itself did, for the log and the gap report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkNote {
    /// A stream connection opened. Connections are numbered from 1.
    Opened(u64),
    /// A stream connection closed, and why.
    Closed(u64, String),
    /// The new connection took over every instrument from the old one.
    TookOver {
        /// The old connection.
        from: u64,
        /// The new connection.
        to: u64,
    },
    /// A fetch failed; it is tried again.
    FetchFailed {
        /// Why.
        error: String,
        /// How many times it failed before, from 0.
        attempt: u32,
    },
    /// A stream message that did not read.
    BadMessage(String),
}

/// What the live connection passes on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedItem {
    /// Market data, in order.
    Event(MarketEvent),
    /// What the feed noticed: gaps, missed trades.
    Note(Note),
    /// What the connection did.
    Link(LinkNote),
    /// A live book's fingerprint ([`fusion_domain::OrderBook::digest`]),
    /// so a replay can prove it rebuilt the same book.
    Check {
        /// The instrument.
        instrument: InstrumentId,
        /// The book's update number.
        update_id: u64,
        /// The fingerprint.
        digest: u64,
    },
}

/// Runs the live connection until the receiver of `out` is dropped.
pub async fn run<F: Fetch>(config: LiveConfig, fetch: F, out: mpsc::Sender<FeedItem>) {
    install_tls();
    let (tx, rx) = mpsc::channel(4096);
    let live = Live {
        url: config.symbols.stream_url(&config.endpoints.stream),
        feed: Feed::new(&config.symbols),
        retry: config.first_retry,
        config,
        fetch: Arc::new(fetch),
        tx,
        out,
        conns: BTreeMap::new(),
        primary: None,
        overlap: None,
        next_conn: 1,
        reconnect_at: None,
        overlap_wanted: false,
        overlap_retry_at: None,
        next_check: Instant::now(),
    };
    live.run(rx).await;
}

pub(crate) enum Incoming {
    Opened(u64),
    Text(u64, Utf8Bytes),
    Closed(u64, String),
    Snapshot(BookSnapshot),
    Trades(InstrumentId, Vec<Trade>),
    Failed(Want, String, u32),
}

struct Conn {
    opened: Instant,
    last_message: Instant,
    task: JoinHandle<()>,
}

struct Live<F> {
    config: LiveConfig,
    url: String,
    fetch: Arc<F>,
    feed: Feed,
    tx: mpsc::Sender<Incoming>,
    out: mpsc::Sender<FeedItem>,
    conns: BTreeMap<u64, Conn>,
    primary: Option<u64>,
    overlap: Option<Overlap>,
    next_conn: u64,
    retry: Duration,
    reconnect_at: Option<Instant>,
    /// A new connection is due: Binance said it will close this one, or
    /// the 24-hour limit nears.
    overlap_wanted: bool,
    /// When to try a new connection again after one failed.
    overlap_retry_at: Option<Instant>,
    next_check: Instant,
}

impl<F: Fetch> Live<F> {
    async fn run(mut self, mut rx: mpsc::Receiver<Incoming>) {
        self.primary = Some(self.open());
        let out = self.out.clone();
        let mut tick = tokio::time::interval(TICK);
        loop {
            tokio::select! {
                () = out.closed() => break,
                incoming = rx.recv() => match incoming {
                    Some(incoming) => self.on_incoming(incoming).await,
                    None => break,
                },
                _ = tick.tick() => self.housekeeping().await,
            }
        }
        for conn in self.conns.values() {
            conn.task.abort();
        }
    }

    fn open(&mut self) -> u64 {
        let conn = self.next_conn;
        self.next_conn += 1;
        let task = tokio::spawn(connection::run(conn, self.url.clone(), self.tx.clone()));
        let now = Instant::now();
        self.conns.insert(
            conn,
            Conn {
                opened: now,
                last_message: now,
                task,
            },
        );
        conn
    }

    fn close(&mut self, conn: u64) {
        if let Some(c) = self.conns.remove(&conn) {
            c.task.abort();
        }
    }

    async fn emit(&self, item: FeedItem) {
        // A closed receiver ends the loop at its next turn.
        let _sent = self.out.send(item).await;
    }

    async fn on_incoming(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Opened(conn) => self.emit(FeedItem::Link(LinkNote::Opened(conn))).await,
            Incoming::Text(conn, text) => self.on_text(conn, &text).await,
            Incoming::Closed(conn, reason) => self.on_closed(conn, reason).await,
            Incoming::Snapshot(snapshot) => {
                let mut out = Output::default();
                self.feed.on_snapshot(snapshot, &mut out);
                self.pass(out).await;
            }
            Incoming::Trades(id, trades) => {
                let mut out = Output::default();
                self.feed.on_trades(id, trades, &mut out);
                self.pass(out).await;
            }
            Incoming::Failed(want, error, attempt) => {
                self.emit(FeedItem::Link(LinkNote::FetchFailed { error, attempt }))
                    .await;
                let wait = backoff(self.config.first_retry, attempt);
                self.request(want, wait, attempt + 1);
            }
        }
    }

    async fn on_text(&mut self, conn: u64, text: &str) {
        // Messages still on their way from a connection closed on purpose.
        let Some(c) = self.conns.get_mut(&conn) else {
            return;
        };
        c.last_message = Instant::now();
        self.retry = self.config.first_retry;
        match parse_stream(text, &self.config.symbols) {
            Ok(Message::Event(event)) => self.on_event(conn, event).await,
            // This connection closes soon: open the next one.
            Ok(Message::Shutdown) => self.overlap_wanted = true,
            Err(err) => {
                self.emit(FeedItem::Link(LinkNote::BadMessage(err.to_string())))
                    .await;
            }
        }
    }

    async fn on_event(&mut self, conn: u64, event: MarketEvent) {
        let mut out = Output::default();
        match self.overlap.as_mut() {
            // Without an overlap the only open connection is the primary.
            None => self.feed.on_event(event, &mut out),
            Some(overlap) => match overlap.admit(conn, &event, &self.feed) {
                Admit::Pass => {
                    self.feed.on_event(event, &mut out);
                    while let Some(parked) = overlap.unpark(&self.feed) {
                        self.feed.on_event(parked, &mut out);
                    }
                }
                Admit::Park => {
                    if let Some(oldest) = overlap.park(event) {
                        self.feed.on_event(oldest, &mut out);
                    }
                }
                Admit::Drop => {}
            },
        }
        self.pass(out).await;
        let done = self
            .overlap
            .as_ref()
            .is_some_and(|o| o.complete(&self.config.symbols, &self.feed));
        if done {
            self.finish_overlap().await;
        }
    }

    async fn pass(&mut self, out: Output) {
        for want in out.wants {
            self.request(want, Duration::ZERO, 0);
        }
        for note in out.notes {
            self.emit(FeedItem::Note(note)).await;
        }
        for event in out.events {
            self.emit(FeedItem::Event(event)).await;
        }
    }

    async fn on_closed(&mut self, conn: u64, reason: String) {
        // A connection closed on purpose is gone already; nothing to do.
        let Some(_closed) = self.conns.remove(&conn) else {
            return;
        };
        self.emit(FeedItem::Link(LinkNote::Closed(conn, reason)))
            .await;
        let retry_at = Instant::now() + self.retry;
        self.retry = (self.retry * 2).min(MAX_RETRY);
        match self.overlap.as_ref().map(|o| o.old) {
            Some(old) if old == conn => self.finish_overlap().await,
            // The new connection failed: try another one a little later.
            Some(_) => {
                self.overlap = None;
                self.overlap_retry_at = Some(retry_at);
            }
            // Without an overlap the only connection is the primary.
            None => {
                self.primary = None;
                self.overlap_wanted = false;
                self.reconnect_at = Some(retry_at);
            }
        }
    }

    fn start_overlap(&mut self) {
        let Some(old) = self.primary else {
            return;
        };
        if self.overlap.is_some() {
            return;
        }
        let new = self.open();
        self.overlap = Some(Overlap::new(old, new, Instant::now()));
    }

    async fn finish_overlap(&mut self) {
        let Some(mut overlap) = self.overlap.take() else {
            return;
        };
        // What still waits goes to the feed, which finds any real gap.
        let mut out = Output::default();
        for event in overlap.drain() {
            self.feed.on_event(event, &mut out);
        }
        self.pass(out).await;
        self.overlap_wanted = false;
        self.overlap_retry_at = None;
        self.close(overlap.old);
        self.primary = Some(overlap.new);
        self.emit(FeedItem::Link(LinkNote::TookOver {
            from: overlap.old,
            to: overlap.new,
        }))
        .await;
    }

    async fn housekeeping(&mut self) {
        let now = Instant::now();
        if now >= self.next_check {
            self.next_check = now + self.config.check_every;
            self.check_books().await;
        }
        if self
            .overlap
            .as_ref()
            .is_some_and(|o| now - o.started >= self.config.overlap_limit)
        {
            self.finish_overlap().await;
        }
        if let Some(primary) = self.primary.and_then(|p| self.conns.get(&p))
            && now - primary.opened >= self.config.rotate_after
        {
            self.overlap_wanted = true;
        }
        if self.overlap_wanted && self.overlap_retry_at.is_none_or(|at| now >= at) {
            self.overlap_retry_at = None;
            self.start_overlap();
        }
        self.replace_if_stalled(now).await;
        if self.primary.is_none() && self.reconnect_at.is_none_or(|at| now >= at) {
            self.reconnect_at = None;
            self.primary = Some(self.open());
        }
    }

    /// Passes on the fingerprint of every book in sync.
    async fn check_books(&self) {
        for id in self.config.symbols.ids() {
            if let Some(book) = self.feed.book(id) {
                self.emit(FeedItem::Check {
                    instrument: id,
                    update_id: book.update_id(),
                    digest: book.digest(),
                })
                .await;
            }
        }
    }

    async fn replace_if_stalled(&mut self, now: Instant) {
        if self.overlap.is_some() {
            return;
        }
        let Some(primary) = self.primary else {
            return;
        };
        let silent = self
            .conns
            .get(&primary)
            .is_some_and(|c| now - c.last_message >= self.config.stall_after);
        if silent {
            self.close(primary);
            self.primary = None;
            self.overlap_wanted = false;
            self.emit(FeedItem::Link(LinkNote::Closed(
                primary,
                "silent for too long".to_owned(),
            )))
            .await;
        }
    }

    /// Fetches what the feed wants, after `wait`, on its own task.
    fn request(&self, want: Want, wait: Duration, attempt: u32) {
        let symbols = &self.config.symbols;
        let rest = &self.config.endpoints.rest;
        let url = match &want {
            Want::Snapshot(id) => symbols.snapshot_url(rest, *id),
            Want::Trades { instrument, from } => symbols.trades_url(rest, *instrument, *from),
        };
        let Some(url) = url else {
            return;
        };
        let fetch = Arc::clone(&self.fetch);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let incoming = match fetch.get(url).await {
                Ok(body) => read_reply(want, &body, attempt),
                Err(error) => Incoming::Failed(want, error, attempt),
            };
            // The loop may have stopped; then nobody needs it.
            let _sent = tx.send(incoming).await;
        });
    }
}

fn read_reply(want: Want, body: &str, attempt: u32) -> Incoming {
    let read = match &want {
        Want::Snapshot(id) => parse_snapshot(body, *id).map(Incoming::Snapshot),
        Want::Trades { instrument, .. } => {
            parse_trades(body, *instrument).map(|t| Incoming::Trades(*instrument, t))
        }
    };
    read.unwrap_or_else(|err| Incoming::Failed(want, err.to_string(), attempt))
}

/// `first`, doubled for each attempt, up to [`MAX_RETRY`].
fn backoff(first: Duration, attempt: u32) -> Duration {
    first
        .saturating_mul(2u32.saturating_pow(attempt.min(16)))
        .min(MAX_RETRY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_double_up_to_the_limit() {
        let first = Duration::from_secs(1);
        assert_eq!(backoff(first, 0), Duration::from_secs(1));
        assert_eq!(backoff(first, 1), Duration::from_secs(2));
        assert_eq!(backoff(first, 3), Duration::from_secs(8));
        assert_eq!(backoff(first, 5), MAX_RETRY);
        assert_eq!(backoff(first, u32::MAX), MAX_RETRY);
    }

    #[test]
    fn the_defaults_suit_a_long_run() {
        let config = LiveConfig::new(Endpoint::Testnet, SymbolTable::new(["BTCUSDT"]));
        assert_eq!(config.rotate_after, Duration::from_secs(23 * 3600));
        assert!(config.rotate_after < Duration::from_secs(24 * 3600));
        assert_eq!(config.endpoints, Endpoints::from(Endpoint::Testnet));
        assert_eq!(config.endpoints.rest, "https://testnet.binance.vision");
    }
}
