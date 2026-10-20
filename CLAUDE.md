# Fusion

A runtime-configurable trading platform in Rust: view server, market data,
FIX order routing, rules scanner and algos, with a UI for desktop and browser.
Public repo: https://github.com/Ninfendo/Fusion. Design and build order:
[docs/architecture.md](docs/architecture.md). Read it before any design work.

## Commands

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked     # CI adds RUSTFLAGS=-D warnings
cargo fmt --all
cargo build --locked --target wasm32-unknown-unknown -p fusion-domain
```

No local Rust? Run the same checks in Docker:
`docker run --rm -v "<repo>:/work" -w /work rust:latest sh -c "rustup toolchain install && cargo test --workspace --locked"`.

## Performance first

Speed is a core goal. When the owner asks for something that will limit
performance, say so plainly **before** building it: what it costs, roughly
how much, and a faster alternative. Do not stay silent, and do not quietly
work around it. The hot paths are tick-to-screen, tick-to-trade and the view
server's fan-out. See "Performance rules" in docs/architecture.md.

## Hard rules

- **Determinism.** The engine reads time only through `fusion_domain::Clock`.
  No `HashMap` or `HashSet` in core crates (each core crate has a
  `clippy.toml` that bans them; copy it into every new core crate).
- **Money.** Prices and quantities use `rust_decimal`, never `f32` or `f64`.
- **Time.** UTC nanoseconds in an `i64` (`fusion_domain::Timestamp`).
- **Browser.** `fusion-domain` and `client-core` must build for
  `wasm32-unknown-unknown`. Add every crate the browser uses to the CI `wasm` job.
- **Client layering.** `client-core` has no UI library and no async runtime.
  UI crates only draw. Each panel's presentation model has an
  `update(action) -> Vec<Effect>` that does no I/O.
- **Wiring.** No DI container crate. Each binary's `main.rs` is the
  composition root. Generics in the hot path, `Arc<dyn Trait>` at the edges.
- **New crates** go in `crates/` (libraries) or `apps/` (binaries), take the
  workspace package fields (`version.workspace = true`, and so on) and
  `[lints] workspace = true`.
- **Dependencies** must pass `deny.toml`: permissive licences only. A copyleft
  crate (GPL, AGPL, LGPL) needs the owner's decision first.
- **Toolchain** is pinned in `rust-toolchain.toml`. Upgrade it on purpose, in
  its own commit.

## Way of working

- One feature or fix per branch and pull request. CI must be green before merge.
- Each milestone ends with its gate test from docs/architecture.md.
