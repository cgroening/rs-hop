# Development

Developer notes for working on `hop`. For what the app does and how to use it see [`README.md`](../README.md); for the coding conventions and the detailed architecture/invariants see [`CLAUDE.md`](../CLAUDE.md). This file adds the "why" behind the less obvious choices and the day-to-day workflow.

## Project layout

```text
src/
  main.rs                 thin binary: parse Cli, init file logging, dispatch
  lib.rs                  library crate root; re-exports ratada::theme as crate::theme
  keymap.rs               Action SSOT: config name, description, default keys, Keymap
  domain/                 pure core (no I/O, no UI framework)
    repo.rs               Repo, RepoKind{Git,Path}, GitInfo, classify_path
    error.rs              Error (thiserror) + Result alias
    filter.rs             Tab, belongs_to_tab, searchable_text, fuzzy_indices
    slug.rs               slugify + validate_format (reserved names)
    sections.rs           group / flatten / section_starts / jump_target
    sort.rs               SortMode + sort_indices (name/recent/frecency/custom)
    backup.rs             backup_filename: unique slugified ZIP name per repo
    doctor.rs             diagnose(repos, exists, is_git) -> Vec<Issue>
    path_repair.rs        nearest_existing ancestor (predicate-injected)
  storage/                ports + adapters
    repository.rs         RepoRepository trait (port)
    toml_repo_repository.rs  read [[repos]] (serde DTO), write via config::writer
    in_memory_repository.rs  test fake / --demo backend
    git_client.rs         GitClient trait (port) + parse_github_name
    subprocess_git_client.rs  git via std::process::Command
    cache.rs              git-info cache (TOML)
    usage_state.rs        last_used/open_count + selected-repo handoff writer
    ui_state.rs           persisted sort / tab / show_slugs / preview
    zip_cache.rs          ZIP-backup fingerprint cache (skip iCloud re-download)
  service/                use cases over the ports
    repo_service.rs       CRUD, fav/archive, slug, path repair, usage, undo
    status_service.rs     collect_all (sync) + spawn_refresh (background)
    preview_service.rs    spawn_logs: background git log for the preview panel
    zip_service.rs        spawn_zip: ZIP backups in the background (ZipUpdate)
  config/
    mod.rs                Config + palette()/skin()/theme_registry()/keymap()
    appearance.rs         [appearance]: theme name, colour overrides, glyphs
    loader.rs             defaults -> config.toml -> HOP_* env
    writer.rs             comment-preserving toml_edit rewrite of [[repos]]/sections
  cli/
    mod.rs                clap Cli/Command + dispatch + apply_sparcli_theme
    output.rs             error/status reporting via sparcli alerts
    demo.rs               sample entries for --demo
  tui/
    mod.rs                App + run loop (poll + drain background) + key handling + render
    appframe.rs           the panel app-frame (bands + hints + dim backdrop)
    skin.rs               Colors::from_palette: hop colour roles from the palette
    colors.rs             original rose constants (content renderers, = default theme)
    terminal.rs           re-export of ratada::{Tui, TuiEvent}
    presentation.rs       IconSet (glyphs), status_text, truncate, scrollbar, footer
    table.rs              repo table rendering (git tabs)
    sections_view.rs      Files-tab sectioned list (headers + entry rows)
    preview.rs            detail/preview panel + cached git log
    form.rs               add/edit form (RepoForm)
    widgets.rs            confirm / text prompt / single-select modals
    path_picker.rs        filesystem picker (repair / form path)
    scan_picker.rs        standalone multi-select picker for `hop scan`
    sections_modal.rs     manage-sections overlay
    help.rs               help overlay (?)
    navigation.rs         cyclic cursor helper
    text_input.rs         single-line input with a block caret
  util/
    paths.rs              XDG (Unix) / %APPDATA% (Windows) + ~ expansion
    logging.rs            file-only log backend (never stderr)
    clipboard.rs          copy via pbcopy / clip / wl-copy|xclip|xsel
    opener.rs             launch git tool / editor / default app / url
    scan.rs               recursive git-repo discovery for `hop scan`
    archive.rs            collect_files (excl. build dirs) + write_zip
    app_info.rs           APP_NAME / APP_VERSION / APP_ABOUT
tests/                    integration tests driving the public library API
```

## Layered architecture

Dependencies point inward to `domain`: `domain` ← `storage` ← `service` ← (`cli`, `tui`); `config` and `util` are leaf utilities. `main.rs` is a thin binary; the real composition root is `cli::run_with_service`, which resolves the config path, wires a `TomlRepoRepository` into a `RepoService`, constructs the `SubprocessGitClient`, and dispatches to a CLI handler or the TUI. DIP is via the `RepoRepository` and `GitClient` traits, injected as `Box<dyn …>` / `Arc<dyn …>`; the `InMemoryRepoRepository` is the test/`--demo` backend. Storage errors are surfaced through the single domain `Error` enum (`domain/error.rs`) at the service boundary – the domain never names I/O or TOML types. A library crate (`src/lib.rs`) holds all logic so the integration tests in `tests/` drive the public API.

## The panel app-frame (`tui/appframe.rs`)

The whole TUI renders through one helper, `appframe::render_frame`, so every view shares the same clibase-style chrome. It draws, top to bottom: a **tinted, borderless header band** (brand + tab bar via `ratada::tabs`), a 1-row content padding, the **content surface** (one step lighter than the bands), an optional single-row **progress bar** (shown only while a refresh/backup runs), a **tinted status band** (the info line plus the live-filter input or a transient message), and a blank separator followed by the **backgroundless, full-width key-hint band** (`ratada::shortcut_hints`, the flat list of the view's shortcuts). It returns `FrameAreas { content, progress }` – the caller paints the entry list/preview into `content` and, when present, the progress bar into `progress`. No border lines are used on the app frame itself; only individual widgets (modals, form fields, boxes) get rounded borders. Modals dim the live view rather than a black screen: at the end of a frame `snapshot_frame` stores the buffer, and each overlay calls `dim_backdrop` (blit the snapshot + `ratada::overlay::dim`) before drawing its box. Band and surface colours come from the resolved `Skin` (`config.skin()`); the resolved role colours for content cells come from `tui/skin.rs` `Colors::from_palette`.

## The `ratada` toolkit (external crate)

The reusable ratatui toolkit and the framework-agnostic theming layer live in the external `ratada` crate, consumed as a path dependency (`ratada = { path = "../../libs/ratada", version = "0.2" }`). hop uses it for: the terminal guard and event model (`Tui`, `TuiEvent`, re-exported by `tui/terminal.rs`), theming (`ratada::theme::{Color, Palette, Skin, Glyphs, GlyphVariant, ThemeColors, ThemeRegistry}`, re-exported as `crate::theme` in `src/lib.rs`), the panel-frame building blocks (`tabs`, `shortcut_hints`, `overlay::dim`, `style`, `chrome::{modal_block, border_title}`) and the fuzzy-highlight helper. It is the single source of truth for the look; build new screens on it rather than reimplementing navigation, scrollbars or theming. `nucleo-matcher` stays a **direct** dependency (not routed through `ratada`) because the pure `domain::filter` must not depend on a UI toolkit.

Note: hop deliberately does **not** use `ratada`'s blocking dialog widgets (`modal::*`, `form::Form`, `path_picker`). Its overlays stay non-blocking and are only restyled with `ratada::chrome` – see [Architecture decisions](#architecture-decisions).

## The `sparcli` toolkit (external crate)

The CLI presentation layer uses the external `sparcli` crate (path dependency, feature `fuzzy`) the way `ratada` serves the TUI: styled output (`Table`, `Alert`) themed from the same config palette. `cli::apply_sparcli_theme` builds a `sparcli::Theme` from `config.palette()` (accent, semantic colours, unicode/ascii) and installs it globally with `sparcli::set_theme`, so CLI and TUI share one look from one source. sparcli detects `NO_COLOR` and non-terminal output itself. To keep `hop list` script-safe, its output is **TTY-gated**: on a terminal it renders a styled `sparcli::Table`, but when piped it falls back to the original plain, tab-separated lines (unchanged for scripts). Status and error messages go through `sparcli::Alert` (`cli/output.rs` prints error alerts to stderr). The interactive `hop scan` picker is still hop's own `scan_picker` (its own `ratada::Tui` loop); off a terminal it fails early instead of hanging.

## Keys, theming and glyphs

`keymap.rs` is a top-level layer (not under `tui`) so it can be the single source of truth for shortcuts: `Action` carries each shortcut's config name, description and default keys, and `Keymap::from_overrides(&config.keys)` applies the `[keys]` config overrides and reports conflicts. **Caveat:** the keymap is built and tested but **not yet wired into the TUI dispatch** – `App::handle_list_key` still matches keys inline, and the footer hints come from the `hints(tab)` function, so `[keys]` overrides do not take effect yet. Wiring `keymap` into dispatch + the footer is a planned follow-up (see [Known follow-ups](#known-follow-ups)).

Theming flows from one config: `[appearance]` (`config/appearance.rs`) sets the active theme, per-colour overrides and the glyph variant; `[themes.<name>]` tables add custom themes over the built-in rose `default` and `monochrome`. `Config` exposes `palette()`, `skin()`, `theme_registry()`, `color_overrides()` and `keymap()` as the bridge to `ratada` and `sparcli`. The glyph variant is `ratada::theme::GlyphVariant`; hop keeps its own richer `IconSet` (`tui/presentation.rs`) keyed off it. The env prefix is `HOP_` (`HOP_GIT_PROGRAM`, `HOP_EDITOR`, `HOP_THEME`, `HOP_GLYPHS`, `HOP_CONFIG`). The legacy `[icons]` block still loads (folded into `[appearance].glyphs`).

## Background work

hop keeps the list responsive by doing git and archive work off the main thread. `service::status_service::spawn_refresh` streams `StatusUpdate`s over an mpsc channel (a `Started { path }` before each repo and a `Done { path, info }` after); `zip_service::spawn_zip` streams `ZipUpdate`s; `preview_service::spawn_logs` fetches the preview `git log`. The run loop (`tui::run`) polls events on a short timeout and, each iteration, drains all three channels (`drain_status`/`drain_zip`/`drain_preview`) – so the list shows immediately and fills in, and the single-row progress bar animates. This continuous draining is the reason the overlays must stay non-blocking (next section).

## Architecture decisions

### Overlays stay non-blocking – hop keeps its own modals and path picker instead of `ratada`'s blocking widgets

**Status:** accepted (during the `clibase`/`ratada`/`sparcli` migration).

**Context.** hop's overlays (confirm, slug prompt, add/edit form, path picker, section manager, help, error list) are a **non-blocking state machine**: the run loop draws a frame, polls one event, mutates the active `Overlay` variant, and repeats. The overlay is redrawn every frame by `App::render_overlay` and driven by `App::handle_overlay_key`. Because the loop keeps spinning while an overlay is open, the background workers keep draining – the git-status refresh, the ZIP-backup progress and the preview-`git log` fetch all continue behind the dialog. The `ratada` dialog widgets are the opposite: `ratada::modal::{confirm, select, input, …}`, `ratada::form::Form` and `ratada::path_picker::path_picker` are **blocking** – each takes `&mut Tui`, runs its own event loop until the user answers, and returns the result. While such a widget is open, hop's main loop is not running, so nothing drains the background channels. The two models do not compose: `render_overlay` runs with `&self` at draw time and has no `&mut Tui` to hand a blocking widget.

**Decision.** Keep the non-blocking overlay state machine; do not swap the overlays for `ratada`'s blocking widgets. Instead, restyle hop's own overlay widgets to look clibase-native: every overlay draws over the **dimmed live view** (`appframe::dim_backdrop`), and each box uses `ratada::chrome::modal_block` so the title reads as an inset border label (`╭─ Title ─`, bold accent). The same applies to the add/edit form, which keeps its own `RepoForm` (path-picker hookup, kind/section cycles, inline slug validation), restyled rather than rebuilt on `ratada::form::Form`. So the **path picker is hop's own `PathPicker`** (`tui/path_picker.rs`), not `ratada::path_picker`.

**Consequences.** Background refresh/zip/preview stay responsive while a dialog is open – the main behavioural reason hop wants this model. In return hop keeps a small amount of its own modal/picker/form code that a pure clibase app would delegate to `ratada`; this is a deliberate trade-off, and the migration rule "use `ratada` widgets, don't reimplement" is knowingly relaxed here. The look is still unified (dimmed backdrop + `chrome::modal_block`), so the deviation is under the hood, not on screen.

**How to adopt the blocking `ratada` widgets later, if wanted.** The precedent exists: the inline git tool (`l`) suspends the TUI with `tui.suspend(…)`, runs an external program, and resumes (`App::run_git_inline`). A blocking `ratada` widget fits the same shape: (1) the key handler returns an outcome instead of setting `self.overlay`; (2) the run loop (which holds `&mut Tui`) calls the blocking widget – e.g. `ratada::path_picker::path_picker(tui, …)` – and applies the result; (3) accept that the background workers pause while the widget is open. This could be done for the path picker alone (the `p` repair flow and the form's `^O` path pick) or for all dialogs at once; either is a conscious behaviour change and belongs in its own step.

## Common commands

```bash
cargo build
cargo run                                     # launch the TUI (needs a real TTY)
cargo run -- --demo                           # TUI with built-in demo data
cargo run -- list                             # a CLI command
HOP_CONFIG=examples/config.toml cargo run     # try the TUI with the sample config
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                                    # unit + integration + doctests
```

When changing widget or theming behaviour that lives in `ratada`/`sparcli`, work in those repos and re-run their checks too; hop picks up the path dependency automatically.

## Logging, configuration and tests

**Logging** (`util/logging.rs`). hop logs to a **file only** (`<state_dir>/hop/hop.log`) and never to stderr, because the TUI owns the terminal and stray stderr would corrupt the alternate screen. Diagnostics use the `log` facade (`log::{info, debug, warn, error}`) at the layer where the event happens – never `println!`, which is reserved for user-facing CLI output.

**Configuration.** Settings resolve as defaults → `config.toml` → `HOP_*` env vars (`config/loader.rs`). The same `config.toml` holds both the settings and the `[[repos]]` entries, read by two layers (the loader for settings, `storage::toml_repo_repository` for entries) and written back comment-preservingly through `config::writer` (`toml_edit`) so the settings block and its comments survive. On-disk formats stay backward compatible: new fields are `#[serde(default)]`, and old files (including the legacy `[icons]` block) load unchanged – there are loader tests for this. `examples/config.toml` documents every setting, including the new `[appearance]`/`[themes.*]`/`[keys]` sections.

**Tests.** Unit tests sit beside their code under `#[cfg(test)]`; integration tests live in `tests/` and drive the public library API. The pure layers (domain, config, storage, service) carry the automated coverage; the TUI is verified with headless `ratatui::TestBackend` render checks (assert on the rendered buffer) for structure, and **manually in a real terminal** for the visual result – there are no snapshot fixtures. After any change, run the full check list above and keep it green; a pure refactor must not change behaviour.

## Known follow-ups

These are documented, deliberate gaps from the clibase migration, not bugs:

- **`keymap.rs` is not yet wired into TUI dispatch.** `App::handle_list_key` still matches keys inline and the footer uses `hints(tab)`, so `[keys]` config overrides have no effect yet. Wiring `Keymap::action_for` into dispatch and sourcing the footer from `keymap.hints(...)` is the intended end state.
- **Content-cell colours still come from `tui/colors.rs`** (the original rose constants), which equal the `default` theme. So re-theming currently recolours the frame and modals but not the table/sections/preview content cells. Threading `Colors` (`tui/skin.rs`) through the content renderers would let content follow the active theme and let `colors.rs` be removed.
- **`scan_picker` keeps the pre-migration bordered style** (it is a standalone CLI picker with its own loop); it could be restyled or moved to a sparcli/`ratada` picker.
- **Config writes are not atomic** (`config::writer` uses `toml_edit` + `fs::write`); a `util/fs::write_atomic` (temp file + rename) would harden against a crash mid-write.
