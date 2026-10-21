//! The live feed: a [`LiveMarket`] kept up to date through a [`Link`].

use std::collections::BTreeSet;

use fusion_client_core::{Book, FeedStatus, LiveMarket, MarketData, MarketFeed, Quote};
use fusion_protocol::v1::ClientMessage;

use crate::{Link, LinkEvent, Wake};

/// Market data from the view server.
pub struct LiveFeed {
    market: LiveMarket,
    link: Link,
    status: FeedStatus,
}

impl LiveFeed {
    /// Starts connecting to the view server at `url`, for example
    /// `ws://127.0.0.1:7070`. `wake` runs when something comes in.
    pub fn connect(url: &str, wake: impl Wake) -> std::io::Result<Self> {
        Ok(Self::over(Link::connect(url, wake)?))
    }

    fn over(link: Link) -> Self {
        Self {
            market: LiveMarket::default(),
            link,
            status: FeedStatus::Connecting,
        }
    }

    fn send_all(&mut self, messages: &[ClientMessage]) {
        for message in messages {
            self.link.send(message);
        }
    }
}

impl MarketData for LiveFeed {
    fn quotes(&self) -> Vec<Quote> {
        self.market.quotes()
    }

    fn symbols(&self) -> Vec<String> {
        self.market.symbols()
    }

    fn book(&self, symbol: &str, depth: usize) -> Option<Book> {
        self.market.book(symbol, depth)
    }
}

impl MarketFeed for LiveFeed {
    /// Takes in what the link brought, and answers it.
    fn update(&mut self, _now: f64) {
        for event in self.link.poll() {
            let replies = match event {
                LinkEvent::Connected => {
                    self.status = FeedStatus::Live;
                    self.market.on_connected()
                }
                LinkEvent::Message(message) => self.market.on_message(message),
                LinkEvent::Disconnected(why) => {
                    self.status = FeedStatus::Away(why);
                    Vec::new()
                }
            };
            self.send_all(&replies);
        }
    }

    /// Opens a viewport for each new book and closes the others.
    fn want_books(&mut self, symbols: &BTreeSet<String>) {
        let messages = self.market.want_books(symbols);
        self.send_all(&messages);
    }

    fn status(&self) -> FeedStatus {
        self.status.clone()
    }
}
