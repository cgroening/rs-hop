# Contributing to hop

Thanks for your interest in improving hop. This guide covers the local
workflow and the conventions the project follows.

## Prerequisites

- Rust **1.88 or newer** (edition 2024). The toolchain is pinned in
  `rust-toolchain.toml`.
- A real terminal for trying the TUI (`cargo run`).

## Build and verify

Every change must keep the full gate green before it is proposed:

```
cargo build
cargo test                                   # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo doc --no-deps --all-features
```

The TUI itself is verified manually in a real terminal; the pure layers
(`domain`, `service`, `storage`, `config`, `util`) carry the automated tests.
To exercise the app against the bundled example data:

```
HOP_CONFIG=examples/config.toml cargo run -- list   # plain list
HOP_CONFIG=examples/config.toml cargo run            # TUI (needs a real TTY)
```

## Coding rules

- Rust edition 2024; `rustfmt.toml` sets `max_width = 80`. Imports go
  std → external crate → `crate`, one per line.
- Public items carry a `///` doc comment; the crate builds under
  `#![deny(missing_docs)]`, so an undocumented public item fails the build.
- Errors flow through `Result` and `?`; `unwrap()` is forbidden and `expect()`
  is only for provably-infallible points, with a reason.
- The architecture is layered, with dependencies pointing inward
  (`domain` ← `storage` ← `service` ← `cli`/`tui`). See
  [`CLAUDE.md`](CLAUDE.md) and [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) for
  the full architecture and the "why" behind the design decisions.

## Changelog

For any user-facing change, add a bullet under the `## [Unreleased]` section of
[`CHANGELOG.md`](CHANGELOG.md) in the right group (Added / Changed / Deprecated
/ Removed / Fixed / Security), following the
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format. Purely
internal changes (tests, refactors, docs) need no entry.

## Commits and pull requests

- Write commit titles in English, imperative mood, following
  [Conventional Commits](https://www.conventionalcommits.org/) (e.g.
  `feat(tui): add a sort picker`).
- Keep refactoring separate from behaviour changes.
- Update call sites, tests and docs alongside the code they concern.
