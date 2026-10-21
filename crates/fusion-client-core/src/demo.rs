//! A fake market for building screens before real market data exists.
//!
//! Prices move in a random walk, in whole ticks. The random generator is
//! seeded, so the same seed always gives the same market: tests can rely on
//! it.

use fusion_domain::{Decimal, Price, Quantity};

use std::collections::BTreeSet;

use crate::market::{Book, FeedStatus, Level, MarketData, MarketFeed, Quote};

/// SplitMix64: a small, fast, seeded random generator. Fine for fake data;
/// never use it for anything secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoRng(u64);

impl DemoRng {
    /// A generator that starts from `seed`.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number from 0 up to, but not including, `n`. `n` must be above zero.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// One instrument in the demo market. Prices are kept in whole ticks.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DemoInstrument {
    symbol: &'static str,
    tick: Decimal,
    open_ticks: i64,
    bid_ticks: i64,
    spread_ticks: i64,
    last_ticks: i64,
}

impl DemoInstrument {
    fn price(&self, ticks: i64) -> Price {
        Price::new(self.tick * Decimal::from(ticks))
    }

    fn quote(&self) -> Quote {
        Quote {
            symbol: self.symbol.to_owned(),
            bid: self.price(self.bid_ticks),
            ask: self.price(self.bid_ticks + self.spread_ticks),
            last: self.price(self.last_ticks),
            open: self.price(self.open_ticks),
        }
    }
}

/// The instruments the demo market starts with: symbol, tick size as
/// (mantissa, scale), and the opening price in ticks.
const INSTRUMENTS: [(&str, (i64, u32), i64); 12] = [
    ("BTCUSDT", (1, 2), 6_400_000),
    ("ETHUSDT", (1, 2), 320_000),
    ("SOLUSDT", (1, 2), 15_000),
    ("BNBUSDT", (1, 2), 58_000),
    ("XRPUSDT", (1, 4), 5_200),
    ("ADAUSDT", (1, 4), 4_500),
    ("DOGEUSDT", (1, 5), 12_000),
    ("AVAXUSDT", (1, 2), 2_800),
    ("LINKUSDT", (1, 3), 14_000),
    ("DOTUSDT", (1, 3), 6_200),
    ("LTCUSDT", (1, 2), 7_200),
    ("TRXUSDT", (1, 5), 12_000),
];

/// A fake market of a dozen crypto pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoMarket {
    rng: DemoRng,
    seed: u64,
    step: u64,
    instruments: Vec<DemoInstrument>,
}

impl DemoMarket {
    /// A market at its opening prices.
    pub fn new(seed: u64) -> Self {
        let instruments = INSTRUMENTS
            .iter()
            .map(|&(symbol, (mantissa, scale), open)| DemoInstrument {
                symbol,
                tick: Decimal::new(mantissa, scale),
                open_ticks: open,
                bid_ticks: open,
                spread_ticks: 1,
                last_ticks: open,
            })
            .collect();
        Self {
            rng: DemoRng::new(seed),
            seed,
            step: 0,
            instruments,
        }
    }

    /// Moves a few instruments by up to two ticks. The bid stays above zero
    /// and below the ask, and the last trade is at the bid or the ask.
    pub fn step(&mut self) {
        self.step += 1;
        let count = self.instruments.len() as u64;
        for _ in 0..3 {
            let index = self.rng.below(count) as usize;
            let moves = self.rng.below(5) as i64 - 2;
            let spread = 1 + self.rng.below(3) as i64;
            let at_ask = self.rng.below(2) == 1;
            let instrument = &mut self.instruments[index];
            instrument.bid_ticks = (instrument.bid_ticks + moves).max(1);
            instrument.spread_ticks = spread;
            instrument.last_ticks = if at_ask {
                instrument.bid_ticks + spread
            } else {
                instrument.bid_ticks
            };
        }
    }

    /// How many steps the market has taken.
    pub fn steps(&self) -> u64 {
        self.step
    }

    /// Every instrument's quote, in a fixed order.
    pub fn quotes(&self) -> Vec<Quote> {
        self.instruments.iter().map(DemoInstrument::quote).collect()
    }

    /// The symbols, in the same order as [`DemoMarket::quotes`].
    pub fn symbols(&self) -> Vec<&'static str> {
        self.instruments.iter().map(|i| i.symbol).collect()
    }

    /// The seed for one instrument's book at the current step: the same
    /// within a step, different across steps and instruments.
    fn book_seed(&self, index: usize) -> u64 {
        self.seed ^ self.step.rotate_left(17) ^ index as u64
    }

    /// An order book around the current price, `depth` levels a side. The
    /// quantities change only when the market steps, so a book drawn every
    /// frame does not flicker. `None` for an unknown symbol.
    pub fn book(&self, symbol: &str, depth: usize) -> Option<Book> {
        let index = self.instruments.iter().position(|i| i.symbol == symbol)?;
        let instrument = &self.instruments[index];
        let mut rng = DemoRng::new(self.book_seed(index));
        let mut side = |start: i64, direction: i64| -> Vec<Level> {
            (0..depth as i64)
                .map(|n| Level {
                    price: instrument.price(start + direction * n),
                    quantity: Quantity::new(Decimal::new(1 + rng.below(5_000) as i64, 3)),
                })
                .filter(|level| level.price.is_positive())
                .collect()
        };
        let bids = side(instrument.bid_ticks, -1);
        let asks = side(instrument.bid_ticks + instrument.spread_ticks, 1);
        Some(Book { bids, asks })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_gives_the_same_market() {
        let mut a = DemoMarket::new(7);
        let mut b = DemoMarket::new(7);
        for _ in 0..500 {
            a.step();
            b.step();
        }
        assert_eq!(a, b);
        assert_eq!(a.quotes(), b.quotes());
    }

    #[test]
    fn different_seeds_give_different_markets() {
        let mut a = DemoMarket::new(1);
        let mut b = DemoMarket::new(2);
        for _ in 0..50 {
            a.step();
            b.step();
        }
        assert_ne!(a.quotes(), b.quotes());
    }

    #[test]
    fn prices_stay_sane_over_a_long_run() {
        let mut market = DemoMarket::new(42);
        for _ in 0..20_000 {
            market.step();
            for quote in market.quotes() {
                assert!(
                    quote.bid.is_positive(),
                    "{} bid {}",
                    quote.symbol,
                    quote.bid
                );
                assert!(quote.bid < quote.ask, "{} crossed", quote.symbol);
                assert!(
                    quote.last == quote.bid || quote.last == quote.ask,
                    "{} traded off the touch",
                    quote.symbol
                );
            }
        }
        assert_eq!(market.steps(), 20_000);
    }

    #[test]
    fn the_market_starts_at_the_open() {
        let market = DemoMarket::new(3);
        let btc = &market.quotes()[0];
        assert_eq!(btc.symbol, "BTCUSDT");
        assert_eq!(btc.last, btc.open);
        assert_eq!(btc.bid.to_string(), "64000.00");
        assert_eq!(btc.ask.to_string(), "64000.01");
        assert_eq!(market.symbols().len(), 12);
    }

    #[test]
    fn a_book_is_sorted_and_does_not_cross() {
        let mut market = DemoMarket::new(9);
        market.step();
        let book = market.book("ETHUSDT", 10).expect("known symbol");
        assert_eq!(book.bids.len(), 10);
        assert_eq!(book.asks.len(), 10);
        assert!(book.bids.windows(2).all(|w| w[0].price > w[1].price));
        assert!(book.asks.windows(2).all(|w| w[0].price < w[1].price));
        assert!(book.bids[0].price < book.asks[0].price);
        assert!(
            book.bids
                .iter()
                .chain(&book.asks)
                .all(|l| l.quantity.is_positive())
        );
    }

    #[test]
    fn a_book_holds_still_until_the_market_steps() {
        let mut market = DemoMarket::new(9);
        let first = market.book("SOLUSDT", 5);
        assert_eq!(market.book("SOLUSDT", 5), first);
        for _ in 0..20 {
            market.step();
        }
        assert_ne!(market.book("SOLUSDT", 5), first);
    }

    #[test]
    fn a_book_near_zero_drops_levels_at_or_below_zero() {
        let mut market = DemoMarket::new(5);
        let index = 4;
        market.instruments[index].bid_ticks = 2;
        let book = market.book("XRPUSDT", 5).expect("known symbol");
        assert_eq!(book.bids.len(), 2, "bids at 2 and 1 tick, not 0 or below");
    }

    #[test]
    fn an_unknown_symbol_has_no_book() {
        assert_eq!(DemoMarket::new(1).book("NOPE", 5), None);
    }

    #[test]
    fn the_rng_is_repeatable_and_bounded() {
        let mut a = DemoRng::new(11);
        let mut b = DemoRng::new(11);
        for _ in 0..1_000 {
            let x = a.below(6);
            assert_eq!(x, b.below(6));
            assert!(x < 6);
        }
        assert_ne!(DemoRng::new(1).next_u64(), DemoRng::new(2).next_u64());
    }

    #[test]
    fn the_feed_steps_once_per_step_time() {
        let mut feed = DemoFeed::new(1);
        feed.update(0.0);
        assert_eq!(feed.market().steps(), 1, "the first update moves at once");
        feed.update(0.05);
        assert_eq!(feed.market().steps(), 1, "too soon");
        feed.update(0.1);
        assert_eq!(feed.market().steps(), 2, "just in time");
        feed.update(9.0);
        assert_eq!(feed.market().steps(), 3, "one step after a long sleep");
        feed.want_books(&BTreeSet::from(["BTCUSDT".to_owned()]));
        assert_eq!(feed.status(), FeedStatus::Demo);
        assert_eq!(feed.status().to_string(), "Demo data");
    }

    #[test]
    fn the_feed_shows_its_market() {
        let feed = DemoFeed::new(1);
        assert_eq!(feed.quotes(), feed.market().quotes());
        assert_eq!(
            MarketData::symbols(&feed),
            MarketData::symbols(feed.market())
        );
        let symbol = MarketData::symbols(&feed).remove(0);
        assert_eq!(feed.book(&symbol, 5), feed.market().book(&symbol, 5));
        assert!(feed.book(&symbol, 5).is_some());
    }
}

impl MarketData for DemoMarket {
    fn quotes(&self) -> Vec<Quote> {
        DemoMarket::quotes(self)
    }

    fn symbols(&self) -> Vec<String> {
        DemoMarket::symbols(self)
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn book(&self, symbol: &str, depth: usize) -> Option<Book> {
        DemoMarket::book(self, symbol, depth)
    }
}

/// How often the demo market moves, in seconds.
pub const DEMO_STEP: f64 = 0.1;

/// The demo market as the app's feed: it moves every [`DEMO_STEP`].
#[derive(Debug, Clone)]
pub struct DemoFeed {
    market: DemoMarket,
    next_step_at: f64,
}

impl DemoFeed {
    /// A feed from a market at its opening prices.
    pub fn new(seed: u64) -> Self {
        Self {
            market: DemoMarket::new(seed),
            next_step_at: 0.0,
        }
    }

    /// The market behind the feed.
    pub fn market(&self) -> &DemoMarket {
        &self.market
    }
}

impl MarketData for DemoFeed {
    fn quotes(&self) -> Vec<Quote> {
        self.market.quotes()
    }

    fn symbols(&self) -> Vec<String> {
        MarketData::symbols(&self.market)
    }

    fn book(&self, symbol: &str, depth: usize) -> Option<Book> {
        self.market.book(symbol, depth)
    }
}

impl MarketFeed for DemoFeed {
    /// One step when it is time; never more than one, so a window that
    /// slept does not jump.
    fn update(&mut self, now: f64) {
        if now >= self.next_step_at {
            self.market.step();
            self.next_step_at = now + DEMO_STEP;
        }
    }

    /// Every book is made up when asked for, so there is nothing to do.
    fn want_books(&mut self, _symbols: &BTreeSet<String>) {}

    fn status(&self) -> FeedStatus {
        FeedStatus::Demo
    }
}

/// Golden tests: they pin the exact output, so any change to the generator
/// or the walk shows up. The demo market is meant to be exactly repeatable.
#[cfg(test)]
mod golden {
    use super::*;

    #[test]
    fn the_generator_is_splitmix64() {
        // The published SplitMix64 outputs for seed 0.
        let mut rng = DemoRng::new(0);
        assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(rng.next_u64(), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(rng.next_u64(), 0x06c4_5d18_8009_454f);
    }

    #[test]
    fn the_book_seed_mixes_seed_step_and_instrument() {
        let mut market = DemoMarket::new(7);
        for _ in 0..5 {
            market.step();
        }
        // 7 ^ (5 << 17) ^ 7: the seed and the instrument cancel out.
        assert_eq!(market.book_seed(7), 5 << 17);
        let mut market = DemoMarket::new(5 << 17);
        for _ in 0..5 {
            market.step();
        }
        // The seed and the rotated step cancel out.
        assert_eq!(market.book_seed(0), 0);
    }

    #[test]
    fn five_steps_from_seed_7() {
        let mut market = DemoMarket::new(7);
        for _ in 0..5 {
            market.step();
        }
        let shown: Vec<String> = market
            .quotes()
            .iter()
            .map(|q| format!("{} {} {} {}", q.symbol, q.bid, q.ask, q.last))
            .collect();
        assert_eq!(
            shown,
            [
                "BTCUSDT 64000.01 64000.02 64000.02",
                "ETHUSDT 3199.98 3200.00 3199.98",
                "SOLUSDT 150.00 150.02 150.02",
                "BNBUSDT 580.00 580.02 580.00",
                "XRPUSDT 0.5200 0.5201 0.5200",
                "ADAUSDT 0.4498 0.4500 0.4498",
                "DOGEUSDT 0.11999 0.12002 0.11999",
                "AVAXUSDT 28.01 28.04 28.04",
                "LINKUSDT 14.000 14.002 14.002",
                "DOTUSDT 6.200 6.201 6.200",
                "LTCUSDT 71.98 72.00 71.98",
                "TRXUSDT 0.12000 0.12001 0.12000",
            ]
        );
    }

    #[test]
    fn the_book_after_five_steps_from_seed_7() {
        let mut market = DemoMarket::new(7);
        for _ in 0..5 {
            market.step();
        }
        let book = market.book("BTCUSDT", 3).expect("known symbol");
        let shown: Vec<String> = book
            .bids
            .iter()
            .chain(&book.asks)
            .map(|l| format!("{} {}", l.price, l.quantity))
            .collect();
        assert_eq!(
            shown,
            [
                "64000.01 3.270",
                "64000.00 3.028",
                "63999.99 3.270",
                "64000.02 2.561",
                "64000.03 4.526",
                "64000.04 2.351",
            ]
        );
    }

    #[test]
    fn the_demo_market_serves_panels_like_the_live_one() {
        use crate::market::MarketData;
        let market = DemoMarket::new(1);
        let data: &dyn MarketData = &market;
        assert_eq!(data.quotes(), market.quotes());
        let symbols = data.symbols();
        assert_eq!(symbols.len(), market.symbols().len());
        assert_eq!(symbols[0], market.symbols()[0]);
        let book = data.book(&symbols[0], 3).expect("a book");
        assert_eq!(book.bids.len(), 3);
        assert_eq!(Some(book), market.book(&symbols[0], 3));
    }
}
