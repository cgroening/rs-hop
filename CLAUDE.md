# CLAUDE.md – hop (Rust)

Context for Claude Code sessions in this directory. It carries what you need to keep developing the project **and** the binding coding rules (so the external style guide need not be opened each time).

## 1. What this is

`hop` is a fuzzy-finder TUI in **Rust (edition 2024)** to jump between git repositories and folders. It is a port of the Python `git-repo-jumper`: pick an entry, its path is written to a handoff file the shell wrapper reads to `cd`, and (for git repos) the configured git tool (lazygit) is launched. On top of the original it adds in-app management, three tabs, slugs for a `hop <slug>` fast jump, sort modes, and a marker plus picker for paths that no longer exist.

Main screen: **ratatui** + **crossterm** TUI with three tabs (Git Repos / Files and Folders / Archiv), a live fuzzy filter, and modal overlays (confirm, slug prompt, add/edit form, path picker, help).

## 2. Invariants and key behaviours

- **XDG paths, replicated by hand** in `src/util/paths.rs` (the `directories` crate resolves differently on macOS): config `$XDG_CONFIG_HOME/hop/config.toml`, state `$XDG_STATE_HOME/hop/` for the git-info cache (`git-info-cache.toml`), usage state (`usage.toml`), the selected-repo handoff (`selected-repo.txt`) and the log (`hop.log`). `~` is expanded in config paths. Home prefers `$HOME`, then `%USERPROFILE%`.
- **Config = settings, repository = entries, same file.** `config::loader` reads only the settings (`git_program`, `github_username`, `example_mode`, `editor`, `[icons]`, `[column_widths]`) and ignores `[[repos]]`; `storage::toml_repo_repository` reads/writes the `[[repos]]` array. Both point at the resolved config path. Writing entries back goes through `config::writer` (`toml_edit`), so the settings block and its comments survive; the `[[repos]]` array is rebuilt (per-entry comments are not preserved). Runtime fields (live git info, usage counters) are never persisted.
- **`show` became `archived`, inverted.** The git-repo-jumper YAML import (`config::migrate`) maps a hidden `show: false` entry to `archived = true`; everything else to `archived = false`. The archive tab shows `archived = true` entries.
- **Auto-naming**: an entry's `name` is optional; `Repo::display_name` falls back to the path's basename.
- **Runtime hydration**: usage counters are loaded onto entries in `RepoService::new`; git status is loaded from the cache at TUI start and refreshed in the background (`service::status_service::spawn_refresh`, one worker thread streaming `StatusUpdate`s over an mpsc channel). The run loop polls with a 150 ms timeout and drains updates, so the list shows immediately and fills in. `example_mode` shows each entry's `example_git_info` and skips all git calls.
- **Fetch / startup status**: `tui::StartupStatus` (`Cached` | `Refresh { fetch }`) decides what `App::new` does on start; `cli::startup_status` derives it from `--cached` (cache only, no git), `--fetch` and the `fetch_on_start` config key (`fetch = --fetch || fetch_on_start`). In the TUI `R` = `git fetch` + reload all, `r` = reload all only (`reload_status`); `x`/`X` = refresh only the selected entry without/with fetch (`refresh_one`, which leaves the global `fetched_at` untouched). `hop <slug> --fetch` fetches the repo before launching the tool. The cache is rewritten from the full entry state on every refresh finish, so a single-entry refresh never drops the others. `GitClient::fetch` runs `git fetch --quiet` and must capture output / skip missing paths (otherwise git's stderr corrupts the alternate screen).
- **Header box**: a rounded bordered box (`render_header`) with the `hop` brand + `[n] Label` tabs (active tab in `colors::tab_active()`, the accent's hue complement) on line 1, and one combined info line (`render_info`) on line 2: error count, entry count, sort, local status time and remote fetch time, each behind an `IconSet` glyph, in `colors::MUTED`. While `loading` the info line becomes the centred progress bar (`render_progress`). The remote segment warns (amber `colors::CHANGES`) when over 24h old or never fetched. `fetched_at` is persisted in the git-info cache (`cache::save(.., fetched_at)`), updated only when a refresh actually fetched (`App::refresh_fetched`). During a refresh, rows not yet updated this pass show an animated spinner in the status column (`TableView::spinner`, from `App::refreshing`).
- **Entry errors**: `Repo::entry_error` (pure) reports a missing path or an invalid git entry; the table marker shows `!` for any errored entry. `!` opens the error list (`Overlay::Errors`), and selecting one opens an action menu (`Overlay::ErrorAction`: repair path / edit / delete) that reuses the index-parameterised `repair_picker_for`/`edit_form_for`/`delete_confirm_for`.
- **Opening** (`tui::open_selected` -> `RunOutcome`, acted on by `cli` after the terminal is restored): Enter writes the handoff path and launches the git tool (git), `cd`s (folder) or opens `$EDITOR` (file); `o` writes the handoff path only (a file `cd`s to its parent). `hop <slug>` mirrors Enter; `--save-only`/`-s` mirrors `o`.
- **Slugs** (`domain::slug`) are `[a-z0-9-]+`, validated against reserved command names (`list`, `import`, `config-path`, `help`) and for uniqueness by the service.
- **Multi-select / bulk**: `App::selected` (service indices) + `anchor`; `Space` toggles, `Shift+↑/↓` ranges (`extend_selection`), `Esc`/tab-switch clears. `App::targets()` returns the selection or the cursor entry; `d`/`A`/`z`/`x` act on it via the service bulk methods (`delete_many`/`set_archived_many`/`set_fav_many`, each one undo frame) and clear the selection. Selected rows get the `colors::MULTI_SELECT_BG` tint (`TableView::selected`).
- **Custom order / reorder**: `SortMode::Custom` keeps the stored `[[repos]]` order (favourites floated on top, stable). `Alt+↑/↓` (`move_entry`, only in custom sort) swaps the cursor entry with its neighbour within the same favourite group via `RepoService::swap_entries`, which persists the new `[[repos]]` order. Footer hints are tab-specific (`hints(tab)`); the Archive tab shows `A: restore`, the Git tab shows the refresh keys. The chosen sort mode **and the active tab** are persisted across runs in `$XDG_STATE_HOME/hop/ui-state.toml` (`storage::ui_state` holds `UiState { sort, tab }`; loaded in `App::new`, written together via `save_ui_state` on each `s` and on every tab switch — so neither field clobbers the other). `Tab::as_key`/`Tab::from_key` map the tab to/from its stored string.
- **Missing paths**: the table shows a red `!` in the leftmost column; `p` opens the path picker at the nearest existing ancestor (`domain::path_repair::nearest_existing`).
- **One-level undo** is in the service (`RepoService` records the pre-change entry list per config mutation and rolls back if a write fails). The TUI does not yet bind an undo key.

## 3. Architecture (layered; dependencies point inward)

`domain` ← `storage` ← `service` ← (`cli`, `tui`); `config` and `util` are leaf utilities. `main.rs` is the composition root; a **library crate** (`src/lib.rs`) holds all logic so the integration tests in `tests/` drive the public API. DIP via the `RepoRepository` and `GitClient` traits, injected as `Box<dyn …>` / `Arc<dyn …>`.

```
src/
  lib.rs            crate root: pub mod cli/config/domain/service/storage/tui/util
  main.rs           thin binary: parse Cli, init file logging, dispatch
  domain/
    repo.rs         Repo, RepoKind, GitInfo, display_name (pure)
    sort.rs         SortMode + sort_repos / sort_indices (pure)
    slug.rs         slugify + validate_format (reserved names) (pure)
    filter.rs       Tab, belongs_to_tab, searchable_text, fuzzy_indices
    path_repair.rs  nearest_existing ancestor (pure, predicate-injected)
    error.rs        Error (thiserror) + Result alias
  storage/
    repository.rs           RepoRepository trait (port)
    toml_repo_repository.rs read [[repos]] (serde), write via config::writer
    git_client.rs           GitClient trait + parse_github_name (pure)
    subprocess_git_client.rs git via std::process::Command (.arg, no shell)
    cache.rs                git-info cache (TOML, load/save, timestamp)
    usage_state.rs          last_used/open_count (TOML) + selected-repo writer
    in_memory_repository.rs second backend / test fake
  service/
    repo_service.rs   CRUD, fav/archive, slug, path repair, usage, undo
    status_service.rs collect_all (sync) + spawn_refresh (background)
  config/
    mod.rs    Config, IconVariant, ColumnWidth(s)
    loader.rs defaults < config.toml < HOP_ env (settings only)
    writer.rs toml_edit rewrite of [[repos]] (comment-preserving)
    migrate.rs git-repo-jumper YAML -> TOML (show -> archived inverted)
  cli/
    mod.rs    clap Cli/Command; bare -> TUI; external-subcommand slug jump;
              list / import / config-path; composition root for commands
    output.rs error reporting at the binary edge
  tui/
    mod.rs        App + run loop (poll + drain status) + key handling + render
    terminal.rs   Tui RAII guard (raw mode + alt screen)
    colors.rs     named colour constants + header/selection styles
    presentation.rs IconSet (Unicode/ASCII glyphs), status_text, truncate, footer,
                  scrollbar, empty hint
    text_input.rs single-line input with a block caret (the one edit primitive)
    navigation.rs cyclic cursor helper
    table.rs      repo table rendering + column widths (TableView context)
    widgets.rs    confirm / text prompt / select modals + centered_rect
    path_picker.rs filesystem picker (repair / add), starts near a path
    form.rs       add/edit form (name/path/slug/kind/fav)
    help.rs       help overlay (?)
  util/
    paths.rs      XDG resolution + ~ expansion
    opener.rs     launch git tool / editor (terminal handed over)
    logging.rs    file-only log backend
    app_info.rs   APP_NAME / APP_VERSION / APP_ABOUT
```

### Dependencies (kept minimal; agree before adding more)
`ratatui` + `crossterm` (TUI), `nucleo-matcher` (fuzzy filter), `clap` (CLI/slug), `serde` + `toml` (config + cache + usage), `toml_edit` (comment-preserving config rewrites), `serde_yaml` (one-time YAML import), `chrono` (timestamps), `thiserror` (domain errors), `anyhow` (binary edge if needed), `log` (facade; file backend in `util/logging.rs`), `unicode-width` (column alignment).

## 4. Where to make common changes

- **New list key / behaviour**: `App::handle_list_key` in `src/tui/mod.rs`; keep the footer `hints`, the `help::SHORTCUTS` list and `README.md` in sync.
- **New overlay**: add an `Overlay` variant + its state struct (model the existing widgets), handle it in `handle_overlay_key` and `render_overlay`.
- **New entry field**: extend `domain::repo::Repo`, then the (de)serialize in `storage::toml_repo_repository` / `config::writer` (keep them in sync), the form in `tui::form`, and the table in `tui::table`.
- **New config setting**: `config/mod.rs` (field + default) + `config/loader.rs` (resolve, honouring any `HOP_` env override).
- **New CLI command**: add a `Command` variant + handler in `src/cli/mod.rs`.
- **Git status detail**: `storage::subprocess_git_client` gathers it; `tui::presentation::status_text` formats it from the structured counts.

## 5. Conventions specific to this repo

- Rust edition **2024**; `rustfmt.toml` sets `max_width = 80` (plus nightly-only import grouping). Write imports std → external crate → `crate`, one per line.
- Build artifacts go to `target.nosync/` (set in `.cargo/config.toml`) to keep them out of iCloud.
- **Never commit.** At the end of a change, propose one imperative English commit-message title only.
- TUI is verified manually in a real terminal; the pure layers carry the tests.
- **Icons: two tiers only - `unicode` (default) and `ascii`** (`config.icons.variant`, `IconVariant`/`IconSet`). No Nerd Font glyphs and no colourful emoji: a Nerd Font icon sits in the Private Use Area where `unicode-width` reports one cell but many terminals render two, which clipped the column and made markers render at half width. Every glyph in `IconSet` must be a single-cell symbol.

## 6. Verify after every change (all must pass)

```
cargo build
cargo test                                   # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
HOP_CONFIG=examples/config.toml cargo run -- list   # plain list
HOP_CONFIG=examples/config.toml cargo run            # TUI (needs a real TTY)
```

---

# Coding rules (binding)

These reproduce the project style guide (general + Rust). On conflict the more specific (language) rule wins; these documented rules take precedence over rustfmt/clippy defaults.

## G. General (all languages)

- **KISS / YAGNI**; **DRY** (code) and **SSOT** (data); **consistency**; no magic numbers/strings (named constants).
- **SLAP** – one level of abstraction per function. **Max two nesting levels** (early returns / guard clauses). **No flag arguments** – split into two well-named functions. Command–Query separation; prefer pure functions.
- Polymorphism over type switching; composition over inheritance; Tell-Don't-Ask; **SOLID** (DIP via injection); high cohesion, loose coupling.
- **Naming**: booleans as yes/no questions (`is_`/`has_`/`can_`/`should_`, never negative); methods = verbs, types = nouns, no `Manager`/`Data`/`Helper`; `snake_case` vars/functions, `UPPER_CASE` consts, `PascalCase` types; files `snake_case`. Acronyms as words (`UserId`).
- **Errors**: fail fast; don't signal "nothing" with `null`/`None` where an empty collection / error fits; errors carry context (what/where/why). Validate at trust boundaries; fail loudly over silent-wrong.
- **Logging** via the `log` facade, never `println!` for diagnostics (visible CLI/TUI output is not logging).
- **Formatting**: 4-space indent, **max 80 columns** (`.rs`); operators at line start on wraps; trailing commas in multi-line; spaces only; one final newline; UTF-8/LF. Never `—` (em dash) in code. Markdown prose: one line per paragraph/item, no hard wraps. Straight quotes only.
- **Comments** explain the **why**, moderately; everything in **English**.
- **Types**: `bool` not int flags; enums over magic strings; structs over loose tuples; newtypes where useful; immutable where sensible; don't mutate inputs needlessly.
- **Functions** small (SRP); **few parameters (≤3, group related into a struct)**; pass named/explicit; helpers below callers; public before private.
- **Architecture**: layer by responsibility; DI through constructors/parameters; depend on abstractions; composition root in `main`. Prefer stdlib; **agree before adding a dependency**.
- **Tests** always ship (FIRST; one concept each; fakes over mocks; names describe behaviour). **Run all tests after every change.** Measure before optimising.
- **Maintenance**: respect local style; minimal focused changes; keep refactoring separate from behaviour changes; update call sites, tests and docs; remove dead code; fix root causes.

## R. Rust-specific

- **Edition 2024**; rustfmt with import grouping; **clippy `-D warnings` must pass**; `log` for diagnostics.
- Submodules via `mod`; abstract behaviour with **traits**, inject via generics or `dyn`.
- **Errors**: `Result<T, E>` + `?`; **`thiserror`** for the domain error enum (one per crate/domain, `#[from]` for foreign errors); **`anyhow`** only at the binary edge with `.context()`; **`unwrap()` forbidden, `expect()` only at provably-infallible points** with a reason; **no `panic!` in normal flow**; avoid `unsafe`.
- **Docs**: `///` on every public item (one-sentence first line, prose, no `# Arguments` lists); `# Errors`/`# Panics`/`# Safety` where they apply; `//!` module summary; private items get a short `///`.
- **Idioms**: enums for states; derive sensible traits; `new()` (+ `Default` where parameterless); prefer borrows, avoid needless `clone`; `Option<T>` for absence; `match`/`if let` with early returns; iterators for simple map/filter; keep visibility minimal, re-export via `pub use`.
- **Concurrency**: sync by default; `async` only for real I/O concurrency; prefer ownership/channels over shared locks.
- **Tests**: unit in-file under `#[cfg(test)] mod tests`; integration in `tests/`; doctests must run.
- **Security**: choose `checked_*`/`saturating_*` for external integer values; `Command` with `.arg()`, never a shell string; validate external paths; no secrets in code/logs; bound untrusted input sizes.

## Git

- **Do not create commits yourself.** At the end of a change, propose a commit-message **title only**, in English imperative, Conventional-Commits style.
