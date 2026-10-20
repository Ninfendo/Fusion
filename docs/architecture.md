# Fusion architecture

Fusion is built in Rust end to end. The server does the work and the UI only
draws: the view server keeps every table, filters, sorts and groups it, and
sends each screen only the rows it can see. The first user is the author's own
trading, on one crypto exchange's test environment, with orders over FIX. A
product for others comes later.

## The parts

```mermaid
flowchart TB
  subgraph client["Client: desktop or browser"]
    L["Launcher: local hub for links and layout"]
    W1["Window process 1"]
    W2["Window process 2"]
    L --- W1
    L --- W2
  end
  subgraph server["fusion-server: one process, one crate per part"]
    VS["View server"]
    R["Rules scanner"]
    C["Order core: single writer"]
    P["Persistence"]
    V["Venue adapter"]
    R -->|alerts| VS
    C <-->|events, actions| VS
    C -->|write-behind| P
    V -->|market data| R
    V <-->|fills, orders| C
    V -->|prices| VS
  end
  W1 <-->|"WebSocket + Protobuf: deltas down, actions up"| VS
  W2 <--> VS
  V <-->|"WebSocket + FIX 4.4"| X[("Exchange test environment")]
  P --> DB[("PostgreSQL")]
  V --> PQ[("Parquet recordings")]
```

In a backtest a simulated venue replays the recordings in place of the
exchange, and nothing else changes.

## Decisions

| Layer | Pick | Why |
| --- | --- | --- |
| UI | egui, for desktop and browser from one code base | Mature on the web, fast with ticking data, simple runtime layouts. gpui-kit has better desktop components but its web build is "showcase only". Chosen on 2026-10-20; the dock is drawn with `egui_tiles`. |
| View server | Custom Rust, using the FINOS Vuu model | No open-source Rust view server exists. This is the core of the product. |
| Wire protocol | WebSocket with Protobuf (prost), checked by `buf breaking` | Independent of the UI library. |
| Expressions | CEL (`cel` crate) | One safe language for filters, computed columns and rules. |
| Venue | Binance Spot Testnet, FIX 4.4 | The only self-service crypto venue with FIX orders, market data and drop copy on a free testnet. |
| FIX engine | HotFIX with a small Logon fork, behind our own trait | Pure Rust. Plan B is quickfix-rs. |
| Order core | One single-writer thread for orders, risk and algos | Deterministic: replay and backtest equal live. |
| Rules | Declared features plus CEL conditions; alerts are a table | Features are shared across rules; alerts use normal viewports. |
| Algos | Native Rust house algos (TWAP first); user strategies as WASM later | Fast and trusted at home, sandboxed for user code. |
| Database | PostgreSQL 18 with sqlx, behind repository traits | Pluggable at the domain boundary, not at the driver. |
| Tick data | Parquet files | PostgreSQL is not for ticks. |
| Reference design | NautilusTrader, as a reference only | LGPL-3.0 and v2 is still a release candidate. |

## View server

- **Tables** keyed by primary key: instruments, prices, books, orders,
  executions, positions, alerts.
- **Viewport:** one client's window onto one table: visible row range,
  columns, sort, CEL filter and group-by. Only rows in range are sent.
- **Conflation:** each session keeps a set of changed rows and sends them at a
  fixed rate (default 20 per second, set per viewport).
- **Slow clients:** a bounded send queue per session. When it fills, drop the
  queued changes and send a fresh snapshot. Never block the core.
- **Actions:** new order, cancel, kill switch and save layout use the same
  connection, and get an ack or an error.
- **Protocol:** CreateViewport, ChangeViewport, Snapshot, Delta, RemoveRows,
  Action, Ack, Error. Each message carries a sequence number per viewport.
- **Order books:** the server subscribes in advance to the books of every
  instrument in any watchlist, so switching an L2 panel is fast.

## Client

The client is built from these blocks from the start, not later.

- **`client-core` crate:** protocol client, viewport cache, layout model, link
  routing, order-ticket logic and checks, hotkey map, price formatting and
  theme tokens. It has no UI library and no async runtime in it, so a second
  UI library can reuse it unchanged.
- **Desktop and browser:** the same UI crate builds natively and for wasm32.
  The browser build runs in CI from milestone 1.
- **One process per window.** A launcher starts one process per window and
  restarts a window that crashes. In the browser each tab is already a
  separate process, so both work the same way.
- **Local hub.** Windows send each other only small messages: context
  (FDC3-style, for example `fdc3.instrument`), layout changes and window
  commands. On desktop the launcher relays them over local pipes (named pipes
  on Windows, Unix sockets elsewhere); in the browser tabs use
  BroadcastChannel, with Web Locks to pick the layout owner. Data never goes
  window to window: each window gets it from the view server.
- **The hub owns the layout.** A window never changes the layout itself. It
  sends a message (`ToHub`) through its `Bus` and takes the layout the hub
  sends back (`FromHub`). The hub (`fusion-hub`) does no I/O, so the same
  code runs in the launcher and, for a single window or a browser tab, inside
  the window (`LocalBus`). A window that starts again says hello and gets
  its panels and their last contexts back.
- **Configurable links.** A link is a rule in the layout: from a panel, a
  context type, to one or more panels, with a mode (`all`, `round-robin`,
  `last-focused`) and an optional gesture (for example Shift+click). Colour
  groups are a shortcut that creates links. Only a user action sends context,
  so links cannot loop. The hub keeps the last context per target panel.
- **Pop out and pop in.** Buttons, not drag tear-off. The layout remembers
  where a panel came from and where its window was. Closing a popped-out
  window pops it back in.
- **Own layout format:** windows, each window's dock tree, the panels and the
  links, versioned and saved in PostgreSQL. It does not depend on any UI
  library's format.
- **Theme as tokens:** background, panel, text, accent, buy, sell, up tick,
  down tick, warning, grid line, selected row. A colour-blind option for buy
  and sell, and a monospace font for prices.

## Orders and risk

- The order state machine is keyed on FIX OrdStatus (39) and ExecType (150),
  chains ClOrdID (11) and OrigClOrdID (41), and includes the pending states,
  unsolicited cancels, rejects and busts.
- Every change is an event in the journal, written to PostgreSQL behind the
  core. On restart: rebuild from events, then reconcile with the venue.
- Pre-trade checks before every order: instrument enabled with tick, lot and
  minimum notional respected; maximum value and quantity; a price collar;
  a message-rate limit; a position limit; duplicate detection.
- A kill switch (button and hotkey) cancels everything and blocks new orders.
- Prices and quantities use `rust_decimal`, never floats.

## Venue and FIX

Binance Spot Testnet for milestones 1 to 6: FIX 4.4 order entry, market data
and drop copy, self-service with a GitHub login.

- Logon needs an Ed25519 signature over tags 35, 49, 56, 34 and 52, so it is
  signed after MsgSeqNum and SendingTime are set. Also 553, 141=Y and 25035.
- Binance has no ResendRequest and no documented cancel-on-disconnect.
  Deribit's testnet has both, so Deribit is the second venue.
- Binance cut back its EU service from 1 July 2026. Choose the real-money
  venue (Deribit allows Sweden) before the last milestone.

## Rules and algos

- A scanner rule is a universe, declared features, a CEL condition and an
  action. Windowed features use time buckets in ring buffers per instrument.
- Only instruments with changed inputs are checked. Alerts fire on the edge,
  with hysteresis and a cooldown.
- House algos in native Rust: TWAP, then POV and iceberg, then VWAP, as
  parent and child orders. Every child order passes the risk checks.
- Rules and algos are versioned, never edited in place.

## Crate map

One workspace, many crates: a crate is the Rust equivalent of a .NET project,
and the workspace is the solution. Cargo refuses circular dependencies, so the
layers below are checked by the compiler. A crate is created when its
milestone needs it, not before.

| Layer | Crates | May depend on |
| --- | --- | --- |
| Domain (no I/O, builds for wasm32) | `fusion-domain` (types, orders, journal, port traits such as `Clock` and `ExecutionClient`), `fusion-protocol` (wire messages) | Nothing of ours |
| Server core | `fusion-engine` (orders, risk, algos), `fusion-viewserver`, `fusion-rules`, `fusion-sim` | Domain, protocol |
| Adapters | `fusion-venue-binance`, `fusion-venue-deribit`, `fusion-fix`, `fusion-persistence-pg` | Domain only, never the engine |
| Client models (no UI library, build for wasm32) | `fusion-client-core` (presentation models, protocol client, links), `fusion-hub` (the hub between windows and its messages, no I/O), `fusion-layout` (windows, dock tree, panels, links, versioned save format), `fusion-theme` (tokens), `fusion-chart-model` (candles, studies, scales) | Domain, protocol, never server crates |
| Client drawing (egui) | `fusion-egui-dock`, `fusion-egui-theme`, `fusion-egui-controls` (price input, grid, L2 ladder, ticket parts), `fusion-egui-charts`, `fusion-ui-egui` (panels and the panel registry) | Client models, egui |
| Apps (composition roots) | `fusion-server`, `fusion-launcher`, `fusion-window`, `fusion-web` | What they wire |

- Adapters implement traits from `fusion-domain` (ports and adapters), so an
  adapter never depends on the engine.
- The client and the server share only the domain and the protocol.
- Every UI area is split into a **model** crate with no UI library and a
  **drawing** crate for egui. A change of UI library rewrites only the
  drawing crates. All egui crates use the one egui version pinned in the
  workspace.
- Code with its own life lives in its own repo: the HotFIX fork is
  `Ninfendo/hotfix`, used as a git dependency until upstream takes the change.
- Release builds use fat LTO and one codegen unit, so the crate boundaries
  cost nothing at runtime.

## Code patterns

### Presentation model and one-way data flow

This is Fusion's equivalent of MVVM with a Presentation Model.

- Each panel has a **presentation model** in `client-core`: a plain struct that
  holds all view state (field text, selection, errors, what is enabled). It
  has no UI types in it.
- The **view** reads the model and draws. It never changes the model. It
  reports what the user did as an `Action` value.
- `update(&mut self, action) -> Vec<Effect>` applies an action. It does no
  I/O. Effects (send an order, publish a context, save a layout) are carried
  out by the code around it. So every panel is testable without a UI, and
  replayable.
- Server events reach the UI thread through a channel and are applied to the
  models at the start of each frame.

| WPF | Fusion |
| --- | --- |
| ViewModel / Presentation Model | A struct in `client-core` |
| Binding and INotifyPropertyChanged | Not needed: egui re-reads the model every frame |
| ICommand / CanExecute | An `Action` enum / a method such as `can_submit()` |
| Converters | Formatting functions in `client-core` |
| DataTemplate per type | The panel registry |
| Dispatcher.BeginInvoke | A channel drained at the start of each frame |
| Messenger / EventAggregator | The `fusion-hub` messages, sent through a `Bus` |

### Wiring

This is Fusion's equivalent of an IoC container. There is no container crate.

- One **composition root** per binary (`main.rs` of the server, the launcher
  and the window process) builds every service and passes it in.
- **Constructor injection.** Generics in the hot path (the order core), for
  zero runtime cost. `Arc<dyn Trait>` at the edges (venues, storage), for
  runtime choice.
- **Lifetimes are explicit:** a singleton is one shared `Arc`; a per-session
  object is owned by the session.
- **Swap an implementation** with a `match` on the config at startup, or with
  a Cargo feature at compile time.
- **Registries** for what the layout names at runtime: panel types and venues.
- **Cross-cutting concerns** (logging, metrics, retry) are decorators that wrap
  the real implementation and implement the same trait, plus `tracing`.
- **Tests** wire the same types with fakes, such as `ManualClock`.

## Performance rules

- Nothing blocks the order core. I/O happens at the edges, on other threads.
- Conflate before sending: the view server sends at a fixed rate per
  viewport, never one message per tick.
- Compile expressions (CEL filters, rule conditions) once, and evaluate them
  only for rows whose inputs changed.
- Avoid an allocation per message in hot loops: reuse buffers.
- Measure before optimising, with latency histograms (`hdrhistogram`).

## Determinism rules

- The engine reads time only through the `Clock` trait in `fusion-domain`.
- No iteration over `std::collections::HashMap` or `HashSet` in core crates.
  Clippy enforces this per crate through `clippy.toml`.
- Timestamps are UTC nanoseconds in an `i64`.
- WASM user code runs with fuel in backtests and NaN canonicalisation on.

## Build order

Each milestone ends with a test you can run. Start the next one only when the
gate of the one before it passes.

| Milestone | Builds | Gate: done when |
| --- | --- | --- |
| M0 Skeleton (done) | Workspace, CI, domain types, Clock, event journal, order state machine | State machine tests pass for every FIX status change |
| M1 Dock shell | `fusion-layout`, `fusion-theme`, `fusion-egui-theme`, `fusion-egui-dock`, demo panels on fake data, the desktop app and the browser app | Drag, split, resize and close tabs; the layout is saved and restored; light and dark theme; the same app runs on the desktop and in Chrome |
| M2 Client building blocks | Launcher, one process per window, local hub, configurable links, pop out and pop in | A watchlist click moves only the linked L2 panel in another window; a killed window comes back with its panels |
| M3 Market data | Binance testnet WebSocket in, normalised trades, prices and books, Parquet recording | 24 hours with no gaps; replay rebuilds the same books |
| M4 View server and live screens | Viewports, conflation, the Protobuf protocol; the panels switch from fake data to the view server; layouts saved in PostgreSQL | 500 instruments at full tick rate; the UI keeps 60 frames a second |
| M5 Orders over FIX | FIX session to the testnet, order ticket, blotter, risk checks, kill switch, order events in PostgreSQL | Place, replace and cancel work; a restart mid-order recovers state |
| M6 Rules scanner | Features, CEL conditions, alerts table and panel | A volume-spike rule fires correctly on recorded data |
| M7 Algos and backtest | TWAP, POV, iceberg; simulated venue on recordings | Backtest and testnet make the same decisions on the same events |
| M8 Small real money | Live with small limits, on a venue that serves Sweden | Two weeks live with no manual fixes |

The client comes first (M1 and M2) because the dock, the windows and the
links are the base every later screen stands on, and they need no market
data: demo panels on fake data are enough to build and test them.

## Later, for a product

Login and SSO, entitlements (Cedar fits), market-data licensing, MiFID II
algo controls, high availability, signed desktop builds. Cheap things done now
so these need no rewrite: a user id on every session, row filtering through
one function, UTC nanosecond timestamps on every event, a stable id on every
rule and algo version, and an append-only audit table.
