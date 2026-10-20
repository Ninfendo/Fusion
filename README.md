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
cargo run -p fusion-server                                     # the server (a stub for now)
```

CI runs the same checks on Linux, Windows and macOS. It also blocks a merge
on: cognitive complexity limits in clippy, cyclomatic complexity above 15
([lizard](https://github.com/terryyin/lizard)), line coverage below 95%
([cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov)), surviving
mutants in changed code ([cargo-mutants](https://github.com/sourcefrog/cargo-mutants)),
doc warnings, unused dependencies, typos, and licence or security problems
([cargo-deny](https://github.com/EmbarkStudios/cargo-deny)). A full mutation
run happens every week.

## Layout

| Path | What it is |
| --- | --- |
| `crates/fusion-domain` | Core types: time, instruments, orders, events. No I/O; builds for wasm32. |
| `crates/fusion-viewserver` | The view server engine: tables, and viewports that send only the rows in range and only what changed. No I/O. |
| `crates/fusion-protocol` | The wire protocol between the view server and its clients: Protobuf messages (`proto/`), checked by `buf` in CI. No I/O; builds for wasm32. |
| `crates/fusion-layout` | The layout model: windows, dock trees, panels, as versioned JSON. |
| `crates/fusion-theme` | Colour tokens, dark and light, with a colour-blind option. |
| `crates/fusion-client-core` | Presentation models and the demo market. No UI library. |
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
| `apps/fusion-server` | The server binary. |
| `docs/` | Architecture and design notes. |

More crates arrive with each milestone: protocol, view server, venue
adapters, persistence and rules.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT licence](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 licence,
shall be dual licensed as above, without any additional terms or conditions.
