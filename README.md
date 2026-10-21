# Fusion

Fusion is a runtime-configurable trading platform written in Rust. It has a
view server, market and reference data, order routing over FIX, a rules
scanner and algorithmic trading, with one UI that runs both as a desktop app
and in the browser.

> **Status: milestone 0 is done** (domain types, event journal, FIX order
> state machine). Milestone 1, the dock shell, is next. Nothing here can
> trade yet. Do not point it at an account with real money.

## The idea

Trading screens get slow when the client does the work. In Fusion the server
does it: it keeps every table, filters, sorts and groups it, and sends each
screen only the rows it can see, merged to a fixed rate. The UI only draws.

- **View server:** tables, viewports, conflation, group trees and actions.
- **Order core:** one single-writer thread for orders, risk checks and algos,
  so the same input events always give the same result. Replay and backtest
  run the same code as live trading.
- **Client:** one process per window, a local hub for linking windows, a
  layout format you own, and a UI that builds for desktop and for the browser.

See [docs/architecture.md](docs/architecture.md) for the full design and the
build order.

## Build

You need [rustup](https://rustup.rs). The toolchain version is pinned in
[rust-toolchain.toml](rust-toolchain.toml), and rustup installs it on the first
build. On Windows you also need the Visual Studio Build Tools with the
"Desktop development with C++" workload.

```sh
cargo test --workspace                                         # all tests
cargo clippy --workspace --all-targets                         # lints
cargo fmt --all                                                # format
cargo build --target wasm32-unknown-unknown -p fusion-domain   # browser build check
cargo build -p fusion-window -p fusion-launcher && target/debug/fusion-launcher   # the desktop app, one process per window
cargo run -p fusion-window                                     # the desktop app in one window, without the launcher
cd apps/fusion-web && trunk serve --release                   # the same app in the browser (needs Trunk)
cargo run -p fusion-recorder -- record --for-minutes 10            # record Binance testnet market data to ./recordings
cargo run -p fusion-recorder -- check recordings                   # replay a recording: gaps, and books rebuilt to the same fingerprints
cargo run -p fusion-server                                     # the view server on ws://127.0.0.1:7070, fed by the Binance testnet
cargo run -p fusion-server --example watch -- ws://127.0.0.1:7070 prices 10   # watch a table on a running server
```

The apps show made-up demo data unless they are given a view server. Start
`fusion-server` first, then:

```sh
target/debug/fusion-launcher --server ws://127.0.0.1:7070       # every window on live data
cargo run -p fusion-window -- --server ws://127.0.0.1:7070       # one window on live data
# in the browser: http://127.0.0.1:8080/?server=ws://127.0.0.1:7070
```

The right end of the menu bar says where the data comes from: `Demo data`,
`Live`, or why it is not connected. A lost connection is tried again by
itself.

With a server, the layout is kept there too, so the desktop app and the
browser show the same one. The server's copy wins at the start; the file
(desktop) or the browser's storage stays as a local copy. A change made
before the server's copy arrives wins over it.

CI runs the same checks on Linux, Windows and macOS. It also blocks a merge
on: cognitive complexity limits in clippy, cyclomatic complexity above 15
([lizard](https://github.com/terryyin/lizard)), line coverage below 95%
([cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)), surviving
mutants in changed code ([cargo-mutants](https://github.com/sourcefrog/cargo-mutants)),
doc warnings, unused dependencies, typos, and licence or security problems
([cargo-deny](https://github.com/EmbarkStudios/cargo-deny)). A full mutation
run happens every week.

### A database

The server keeps layouts in PostgreSQL 18 when it is given one. Without
`--database` it keeps them in memory. A local database in Docker:

```sh
docker run -d --name fusion-postgres -e POSTGRES_USER=fusion -e POSTGRES_PASSWORD=fusion   -e POSTGRES_DB=fusion -p 127.0.0.1:55432:5432 postgres:18-alpine
cargo run -p fusion-server -- --database postgres://fusion:fusion@127.0.0.1:55432/fusion
FUSION_TEST_DATABASE_URL=postgres://fusion:fusion@127.0.0.1:55432/fusion cargo test -p fusion-persistence-pg
```

The tables come from the migrations in `crates/fusion-persistence-pg/migrations`,
which run when the server connects. Without `FUSION_TEST_DATABASE_URL` the
database tests pass without running; CI runs them on Linux, and there
`FUSION_REQUIRE_DATABASE` makes a missing database an error.

## Layout

| Path | What it is |
| --- | --- |
| `crates/fusion-domain` | Core types: time, instruments, orders, events, and the repository traits (ports) such as `LayoutStore`. No I/O; builds for wasm32. |
| `crates/fusion-persistence-pg` | PostgreSQL storage (sqlx): the layouts. Implements the repository traits of `fusion-domain`. |
| `crates/fusion-serve` | The view server over WebSocket: sessions polled at their rate, slow clients, layout requests. |
| `crates/fusion-viewserver` | The view server engine: tables, and viewports that send only the rows in range and only what changed. No I/O. |
| `crates/fusion-protocol` | The wire protocol between the view server and its clients: Protobuf messages (`proto/`), checked by `buf` in CI. No I/O; builds for wasm32. |
| `crates/fusion-layout` | The layout model: windows, dock trees, panels, as versioned JSON. |
| `crates/fusion-theme` | Colour tokens, dark and light, with a colour-blind option. |
| `crates/fusion-client-core` | Presentation models, the demo market, and the live market from the view server. No UI library, no I/O. |
| `crates/fusion-client-link` | The client's line to the view server: a WebSocket that reconnects by itself. A thread of its own on desktop; the browser's WebSocket in the browser. |
| `crates/fusion-hub` | The hub between windows: it owns the layout and the links, as messages in and out. No I/O. |
| `crates/fusion-pipe` | The hub messages over a local socket between the window processes and the launcher. Desktop only. |
| `crates/fusion-launch` | The launcher's logic: the hub over pipes, one process per window, restarts, the layout file. Desktop only. |
| `crates/fusion-venue-binance` | Binance market data: the live connection, its messages read into market events, order books kept in sync, missed trades fetched. |
| `crates/fusion-recording` | Market data recordings in Parquet, and a replay that checks the books it rebuilds. |
| `crates/fusion-egui-theme` | Maps the colour tokens onto egui. |
| `crates/fusion-egui-dock` | Draws a layout with egui_tiles and reads the user's edits back. |
| `crates/fusion-ui-egui` | The panels, the panel registry and the app. |
| `apps/fusion-launcher` | The desktop app: starts one `fusion-window` per window and keeps the layout. |
| `apps/fusion-window` | One desktop window. On its own, the whole app in one window. |
| `apps/fusion-web` | The same app in the browser, built with [Trunk](https://trunk-rs.github.io/trunk/). Pop out opens a browser window. |
| `scripts/browser-smoke.mjs` | The browser smoke test CI runs in a headless Chrome. |
| `apps/fusion-recorder` | Records market data to Parquet, and checks a recording. |
| `apps/fusion-server` | The server: Binance market data into the view server, served over WebSocket. |
| `docs/` | Architecture and design notes. |

More crates arrive with each milestone: protocol, view server, venue
adapters, persistence and rules.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT licence](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 licence,
shall be dual licensed as above, without any additional terms or conditions.
