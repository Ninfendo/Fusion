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
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo llvm-cov --workspace --exclude fusion-server --summary-only --show-missing-lines
cargo mutants --in-diff <(git diff origin/main...HEAD)   # mutation testing of your change
cargo machete && typos                                    # unused dependencies, spelling
```

## Quality gates (all block a merge)

- Clippy with warnings as errors, including complexity limits from the root
  `clippy.toml`: cognitive complexity 15, 80 lines per function, 6
  arguments, nesting depth 5. Split a function rather than raise a limit.
- Line coverage of library crates at least 95%. The floor only goes up.
- Mutation testing on the lines a pull request changes: a missed mutant
  means a test is missing. Add the test; do not skip the mutant without a
  written reason.
- Public items need docs (`missing_docs`), and `cargo doc` must be clean.
- No unused dependencies (cargo-machete), no typos, cargo-deny clean.

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
  `HashMap` and `HashSet` are banned workspace-wide by the root
  `clippy.toml`. A crate that needs one, and never iterates it on a replayed
  path, allows it at its crate root with the reason. Never add a
  `clippy.toml` inside a crate: it would replace the root one.
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
