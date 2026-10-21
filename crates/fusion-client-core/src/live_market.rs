//! Market data from the view server, for the panels.
//!
//! [`LiveMarket`] keeps one viewport on the `prices` table, and one on
//! `book/<SYMBOL>` for each instrument a panel shows. It does no I/O: it
//! says what to send, and takes what comes back.

use std::collections::{BTreeMap, BTreeSet};

use fusion_domain::{Decimal, Price, Quantity};
use fusion_protocol::Cell;
use fusion_protocol::v1::{
    ClientMessage, CreateViewport, RemoveViewport, ServerMessage, ViewportSpec, client_message,
    server_message,
};

use crate::market::{Book, Level, MarketData, Quote};
use crate::viewport::ViewportCache;

/// The viewport on `prices`.
const PRICES: u32 = 1;

fn decimal(cell: Option<&Cell>) -> Option<Decimal> {
    match cell {
        Some(Cell::Decimal(d)) => Some(*d),
        _ => None,
    }
}

/// Market data from the view server.
#[derive(Debug)]
pub struct LiveMarket {
    prices: ViewportCache,
    /// Book viewports by symbol: their number and their rows.
    books: BTreeMap<String, (u32, ViewportCache)>,
    next_viewport: u32,
    next_request: u64,
}

impl Default for LiveMarket {
    fn default() -> Self {
        Self {
            prices: ViewportCache::default(),
            books: BTreeMap::new(),
            next_viewport: PRICES + 1,
            next_request: 1,
        }
    }
}

impl LiveMarket {
    fn request(&mut self, body: client_message::Body) -> ClientMessage {
        let request = self.next_request;
        self.next_request += 1;
        ClientMessage {
            request,
            body: Some(body),
        }
    }

    fn create(&mut self, viewport: u32, table: String) -> ClientMessage {
        self.request(client_message::Body::CreateViewport(CreateViewport {
            viewport,
            spec: Some(ViewportSpec {
                table,
                ..ViewportSpec::default()
            }),
        }))
    }

    /// What to send on a new connection: the prices viewport and every
    /// book viewport wanted. What was received before is forgotten; the
    /// server sends fresh snapshots.
    pub fn on_connected(&mut self) -> Vec<ClientMessage> {
        self.prices = ViewportCache::default();
        let mut out = vec![self.create(PRICES, "prices".to_owned())];
        let books: Vec<(String, u32)> = self
            .books
            .iter()
            .map(|(s, (id, _))| (s.clone(), *id))
            .collect();
        for (symbol, id) in books {
            self.books
                .insert(symbol.clone(), (id, ViewportCache::default()));
            out.push(self.create(id, format!("book/{symbol}")));
        }
        out
    }

    /// Opens a book viewport for each symbol in `wanted` that has none,
    /// and closes the rest. Returns what to send.
    pub fn want_books(&mut self, wanted: &BTreeSet<String>) -> Vec<ClientMessage> {
        let mut out = Vec::new();
        let unwanted: Vec<String> = self
            .books
            .keys()
            .filter(|s| !wanted.contains(*s))
            .cloned()
            .collect();
        for symbol in unwanted {
            if let Some((viewport, _)) = self.books.remove(&symbol) {
                out.push(
                    self.request(client_message::Body::RemoveViewport(RemoveViewport {
                        viewport,
                    })),
                );
            }
        }
        for symbol in wanted {
            if !self.books.contains_key(symbol) {
                let id = self.next_viewport;
                self.next_viewport += 1;
                self.books
                    .insert(symbol.clone(), (id, ViewportCache::default()));
                out.push(self.create(id, format!("book/{symbol}")));
            }
        }
        out
    }

    /// Takes a message from the server. Returns what to send back: a request
    /// for a fresh snapshot if a delta was missed.
    pub fn on_message(&mut self, message: ServerMessage) -> Vec<ClientMessage> {
        let (viewport, missed) = match message.body {
            Some(server_message::Body::Snapshot(s)) => {
                let id = s.viewport;
                if let Some(cache) = self.cache(id) {
                    cache.apply_snapshot(s);
                }
                (id, false)
            }
            Some(server_message::Body::Delta(d)) => {
                let id = d.viewport;
                let missed = self
                    .cache(id)
                    .is_some_and(|cache| cache.apply_delta(d).is_err());
                (id, missed)
            }
            _ => return Vec::new(),
        };
        if !missed {
            return Vec::new();
        }
        // Ask again: a changed viewport starts with a snapshot.
        let table = if viewport == PRICES {
            "prices".to_owned()
        } else {
            let symbol = self
                .books
                .iter()
                .find(|(_, (id, _))| *id == viewport)
                .map(|(s, _)| s.clone());
            format!("book/{}", symbol.unwrap_or_default())
        };
        let change = client_message::Body::ChangeViewport(fusion_protocol::v1::ChangeViewport {
            viewport,
            spec: Some(ViewportSpec {
                table,
                ..ViewportSpec::default()
            }),
        });
        vec![self.request(change)]
    }

    fn cache(&mut self, viewport: u32) -> Option<&mut ViewportCache> {
        if viewport == PRICES {
            return Some(&mut self.prices);
        }
        self.books
            .values_mut()
            .find(|(id, _)| *id == viewport)
            .map(|(_, cache)| cache)
    }

    fn quote(&self, symbol: &str, cells: &[Cell]) -> Option<Quote> {
        let get = |name: &str| decimal(self.prices.column(name).and_then(|i| cells.get(i)));
        let bid = get("bid")?;
        let ask = get("ask")?;
        let last = get("last").unwrap_or(bid);
        let open = get("open").unwrap_or(last);
        Some(Quote {
            symbol: symbol.to_owned(),
            bid: Price::new(bid),
            ask: Price::new(ask),
            last: Price::new(last),
            open: Price::new(open),
        })
    }
}

impl MarketData for LiveMarket {
    /// Instruments with a bid and an ask. One with no trade yet shows the
    /// bid as its last price.
    fn quotes(&self) -> Vec<Quote> {
        self.prices
            .rows()
            .filter_map(|(symbol, cells)| self.quote(symbol, cells))
            .collect()
    }

    fn symbols(&self) -> Vec<String> {
        self.prices
            .rows()
            .map(|(symbol, _)| symbol.to_owned())
            .collect()
    }

    fn book(&self, symbol: &str, depth: usize) -> Option<Book> {
        let (_, cache) = self.books.get(symbol)?;
        let (price, quantity) = (cache.column("price")?, cache.column("quantity")?);
        let side = |letter: char| -> Vec<Level> {
            (0..depth)
                .map_while(|n| {
                    let cells = cache.row(&format!("{letter}{n:02}"))?;
                    Some(Level {
                        price: Price::new(decimal(cells.get(price))?),
                        quantity: Quantity::new(decimal(cells.get(quantity))?),
                    })
                })
                .collect()
        };
        Some(Book {
            bids: side('b'),
            asks: side('a'),
        })
    }
}

#[cfg(test)]
mod tests;
