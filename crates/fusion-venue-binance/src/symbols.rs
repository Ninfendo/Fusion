//! Which instruments to follow, and where.

use fusion_domain::InstrumentId;

/// Where the market data comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    /// The Spot Testnet, where Fusion's test orders go.
    Testnet,
    /// Binance's public market data: the real market, read only, no key.
    Public,
}

impl Endpoint {
    /// The WebSocket base address.
    pub fn stream_base(self) -> &'static str {
        match self {
            Self::Testnet => "wss://stream.testnet.binance.vision",
            Self::Public => "wss://data-stream.binance.vision",
        }
    }

    /// The REST base address.
    pub fn rest_base(self) -> &'static str {
        match self {
            Self::Testnet => "https://testnet.binance.vision",
            Self::Public => "https://data-api.binance.vision",
        }
    }

    /// The endpoint for a name: `testnet` or `public`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "testnet" => Some(Self::Testnet),
            "public" => Some(Self::Public),
            _ => None,
        }
    }
}

/// The streams of each instrument: trades, book changes every 100 ms, and
/// the best prices.
const STREAMS: [&str; 3] = ["aggTrade", "depth@100ms", "bookTicker"];

/// The instruments to follow. Instrument ids are positions in the list,
/// starting at 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolTable {
    symbols: Vec<String>,
}

impl SymbolTable {
    /// A table of symbols such as `BTCUSDT`, in any case.
    pub fn new<S: AsRef<str>>(symbols: impl IntoIterator<Item = S>) -> Self {
        Self {
            symbols: symbols
                .into_iter()
                .map(|s| s.as_ref().to_ascii_uppercase())
                .collect(),
        }
    }

    /// The number of instruments.
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// True if there are none.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// The id of a symbol, in any case.
    pub fn id(&self, symbol: &str) -> Option<InstrumentId> {
        self.symbols
            .iter()
            .position(|s| s.eq_ignore_ascii_case(symbol))
            .and_then(|i| u32::try_from(i + 1).ok())
            .map(InstrumentId)
    }

    /// The symbol of an id, upper case.
    pub fn symbol(&self, id: InstrumentId) -> Option<&str> {
        let index = usize::try_from(id.0).ok()?.checked_sub(1)?;
        self.symbols.get(index).map(String::as_str)
    }

    /// Every id, in order.
    pub fn ids(&self) -> impl Iterator<Item = InstrumentId> + '_ {
        (1..=self.symbols.len()).filter_map(|i| u32::try_from(i).ok().map(InstrumentId))
    }

    /// The address of one combined stream with every instrument's streams.
    pub fn stream_url(&self, endpoint: Endpoint) -> String {
        let streams: Vec<String> = self
            .symbols
            .iter()
            .flat_map(|s| {
                let lower = s.to_ascii_lowercase();
                STREAMS.iter().map(move |kind| format!("{lower}@{kind}"))
            })
            .collect();
        format!(
            "{}/stream?streams={}",
            endpoint.stream_base(),
            streams.join("/")
        )
    }

    /// The REST address of a book snapshot with up to 1000 levels a side.
    pub fn snapshot_url(&self, endpoint: Endpoint, id: InstrumentId) -> Option<String> {
        let symbol = self.symbol(id)?;
        Some(format!(
            "{}/api/v3/depth?symbol={symbol}&limit=1000",
            endpoint.rest_base()
        ))
    }

    /// The REST address of up to 1000 trades from trade number `from` on.
    pub fn trades_url(&self, endpoint: Endpoint, id: InstrumentId, from: u64) -> Option<String> {
        let symbol = self.symbol(id)?;
        Some(format!(
            "{}/api/v3/aggTrades?symbol={symbol}&fromId={from}&limit=1000",
            endpoint.rest_base()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_get_ids_from_one() {
        let table = SymbolTable::new(["btcusdt", "ETHUSDT"]);
        assert_eq!(table.len(), 2);
        assert!(!table.is_empty());
        assert_eq!(table.id("BTCUSDT"), Some(InstrumentId(1)));
        assert_eq!(table.id("ethusdt"), Some(InstrumentId(2)));
        assert_eq!(table.id("SOLUSDT"), None);
        assert_eq!(table.symbol(InstrumentId(2)), Some("ETHUSDT"));
        assert_eq!(table.symbol(InstrumentId(0)), None);
        assert_eq!(table.symbol(InstrumentId(3)), None);
        assert_eq!(
            table.ids().collect::<Vec<_>>(),
            [InstrumentId(1), InstrumentId(2)]
        );
        assert!(SymbolTable::new(Vec::<String>::new()).is_empty());
    }

    #[test]
    fn addresses_follow_the_endpoint() {
        let table = SymbolTable::new(["BTCUSDT", "ETHUSDT"]);
        assert_eq!(
            table.stream_url(Endpoint::Testnet),
            "wss://stream.testnet.binance.vision/stream?streams=\
             btcusdt@aggTrade/btcusdt@depth@100ms/btcusdt@bookTicker/\
             ethusdt@aggTrade/ethusdt@depth@100ms/ethusdt@bookTicker"
        );
        assert_eq!(
            table
                .snapshot_url(Endpoint::Public, InstrumentId(1))
                .as_deref(),
            Some("https://data-api.binance.vision/api/v3/depth?symbol=BTCUSDT&limit=1000")
        );
        assert_eq!(
            table
                .trades_url(Endpoint::Testnet, InstrumentId(2), 7)
                .as_deref(),
            Some(
                "https://testnet.binance.vision/api/v3/aggTrades?symbol=ETHUSDT&fromId=7&limit=1000"
            )
        );
        assert_eq!(table.snapshot_url(Endpoint::Testnet, InstrumentId(9)), None);
        assert_eq!(
            table.trades_url(Endpoint::Testnet, InstrumentId(9), 1),
            None
        );
        assert!(
            Endpoint::Public
                .stream_base()
                .starts_with("wss://data-stream")
        );
        assert!(Endpoint::Testnet.rest_base().starts_with("https://testnet"));
    }

    #[test]
    fn endpoints_have_names() {
        assert_eq!(Endpoint::from_name("testnet"), Some(Endpoint::Testnet));
        assert_eq!(Endpoint::from_name("public"), Some(Endpoint::Public));
        assert_eq!(Endpoint::from_name("live"), None);
    }
}
