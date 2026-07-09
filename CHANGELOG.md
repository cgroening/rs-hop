# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **Migrated to the `clibase` architecture and the `ratada`/`sparcli` toolkits.** The TUI now renders as a panel layout (tinted, borderless header/content/status bands plus a backgroundless, full-width key-hint band) via the shared `ratada` toolkit instead of the previous rounded header box and fixed footer; the header carries only the brand and tab bar, and everything status-like (counts, sort, filter input, transient messages) moved into the lower status band. Modals (confirm, slug prompt, add/edit form, path picker, section manager, help, error list) now overlay the **dimmed** live view with rounded, accent-titled `ratada` chrome instead of a plain box. Theming, glyphs and key bindings are now config-driven from one palette: a new `[appearance]` section (theme/colours/glyphs), user `[themes.<name>]` tables layered over the built-in rose `default` and `monochrome` themes, and a `[keys]` section for rebinding actions (see `src/keymap.rs`). `ratatui`/`crossterm` were upgraded to 0.30/0.29. The CLI now renders through `sparcli`: `hop list` prints a styled table on a terminal (and the unchanged plain, tab-separated lines when piped), and status/error messages use styled alerts, all themed from the same config palette. The old `[icons]` config block still loads (folded into `[appearance].glyphs`), and existing `config.toml`/state files continue to load unchanged.

### Added

- **ZIP backups for files/folders, with a per-entry exclude**: `z` and `Z` now back up folder entries on the Files tab, not just git repos. A new `Backup` toggle in the add/edit form decides whether the "backup all" (`Z`) run includes an entry – git repos are included by default, file/folder entries excluded by default (persisted as `include_in_backup` in `[[repos]]`, written only when it deviates from the per-kind default). `z` (the explicit selection/cursor backup) ignores the toggle and always backs up its target. Excluded entries show a `⊘` marker in the `ZIP Backup` column, which is now also shown on the Files tab. The progress bar names the entry currently being zipped. The unique-name check (`hop doctor` included) now spans folders as well as git repos.
- **ZIP backups**: `z` zips the selected/cursor git repo and `Z` zips all git repos into the configured `zip_backup_folder`, excluding build artefacts (the new `zip_exclude_dirs` setting, matched as a name prefix so `target` also covers `target.nosync`) but keeping `.git`. The archive is named after the entry (the slugified name, e.g. `(rs) mdtask` → `rs-mdtask.zip`); two repos that would share a name each get a short, stable path-hash suffix so no backup is overwritten, and `hop doctor` reports the name clash. A repo is only (re)written when its content changed since the last backup (name + size + CRC32 comparison), leaving unchanged archives untouched so cloud sync does not re-upload them (a local fingerprint cache makes that check avoid re-downloading an iCloud-evicted archive). Archives are written atomically (temp file + rename) so a failed run never corrupts the previous backup, and Unix permissions and symlinks are preserved. Progress shows in the header bar (same place as the status refresh). The Git Repos and Archive tabs gain a `ZIP Backup` column showing each repo's last-backup date.
- **Inline git tool overlay**: `l` opens the configured git tool (lazygit) for the selected git repo as an overlay – the TUI suspends the terminal, runs the tool, then returns to the list and refreshes only that repo's status. `L` launches the tool and exits.

### Changed

- **The header progress bar now shows a percentage and the current entry's name.** A refresh (`r`/`R`/startup) shows `XX % - <name>` and names the repo it is currently refreshing (not just the one it finished); a ZIP backup (`z`/`Z`) shows `XX % (<file>/<files>) - <name>`. The text block is sized for the widest name and pinned, so the `XX %` column stays fixed for the whole run instead of jittering as names change length.
- **Favourite moved from `z` to `*`** to free `z`/`Z` for the new ZIP backups.
- **`hop <slug>` now only `cd`s** (writes the handoff path) instead of launching the git tool or opening files, mirroring the TUI's `Enter`. `--save-only`/`-s` is now a deprecated no-op (the default already only `cd`s).
- **`Enter` now only jumps (like `o`)** instead of launching the git tool / opening the file: it `cd`s into a folder, writes the parent of a file, and for a git repo writes the path and exits without launching the tool. The previous `Enter` behaviour (git → tool · folder → cd · text file → editor · other file → default app) moved to `L`.
- **The error list (`!`) and other select modals now show a scrollbar** when their contents overflow the modal height.
- **The preview `git log` now loads in the background** instead of blocking on each cursor move, so navigating with the preview open (`v`) stays smooth. The log for an entry is fetched only after the cursor rests on it briefly (debounced), cached, and shown with a `loading…` placeholder until it arrives. A full reload (`r`) reloads the current tab's logs and a per-entry refresh (`x`) reloads that entry's log – both only while the preview is visible.

### Fixed

- **The repo table no longer scrolls up the moment you move the cursor up from the bottom.** On the Git Repos, Archive and filtered Files tabs the list was rebuilt each frame with a fresh (bottom-anchored) scroll offset, so the cursor stayed pinned to the last visible row and moving it up scrolled the whole list. The offset is now carried across frames, so the cursor moves freely within the viewport and the list only scrolls once the cursor reaches an edge.
- **The Name column is no longer squeezed by a wide GitHub column** on the Git Repos and Archive tabs. The GitHub column now yields width to the (higher-priority) Name column when space is tight: it keeps its full content width when there is room and shrinks (truncating long remote names) otherwise, so entry names stay readable.
- **Cursor movement no longer lags on tabs with slow-to-reach paths** (e.g. archived repos on disconnected, network or cloud drives). A git entry's missing/invalid marker and status column were recomputed with a live `path.exists()` stat on every frame and every visible row, so scrolling blocked on those stats. The marker now derives from the gathered git info (populated by the background refresh) instead, doing no filesystem I/O during rendering.
- **Repairing a path now clears its error immediately**, even when the entry lives on a different tab than the one in view. The error list spans all tabs, but the post-repair refresh only covered the current tab, so a repaired entry could keep showing the `✗` marker until a manual refresh. Repair now re-checks file/folder existence and refreshes the repaired entry's git status directly.

## [0.1.0] - 2026-06-14

First public release. A fuzzy-finder TUI to jump between git repositories and folders, a Rust port of the Python `git-repo-jumper` with in-app management on top.

### Added

- **TUI** (ratatui + crossterm) with three tabs (Git Repos / Files and Folders / Archive), a live fuzzy filter, a rounded header box with brand, tabs and a combined info line, and modal overlays (confirm, slug prompt, add/edit form, path picker, help).
- **Jump handoff**: the selected entry's path is written to a handoff file the shell wrapper reads to `cd`; git repos launch the configured git tool (lazygit). `o` / `--save-only` write the path only.
- **Content-aware opening**: a single `Path` kind auto-classified at open time into folder / text file (`$EDITOR`) / other file (OS default app).
- **Slugs** for a `hop <slug>` fast jump, validated for format and uniqueness and checked against reserved subcommand names.
- **CLI commands**: `hop add`, `hop scan` (recursive git-repo discovery with a multi-select import), `hop doctor` (reports problems, exits non-zero), `hop list`, `hop config-path`, plus `--cached` / `--fetch` / `--demo` flags.
- **Sections** on the Files tab: grouped rendering, jump-to-section picker, section-to-section navigation, in-section reorder, and a manage-sections overlay.
- **Sort modes**: name, recent, frecency, and custom order with `Alt+↑/↓` reorder; favourites floated on top.
- **Background git status refresh** streamed over a channel, with a cache, per-entry refresh (`x` / `X`), full reload (`r`), and fetch (`R`); animated spinner while refreshing and a remote-staleness warning.
- **Multi-select / bulk actions** (delete, archive, favourite) with range selection.
- **Preview panel** (`v`) showing entry details and a cached `git log`.
- **Entry errors**: missing-path and invalid-git markers, an error list and an action menu (repair / edit / delete), plus a filesystem path picker for repair.
- **GitHub open** (`b`), inline-slug toggle (`i`), and persisted UI state (sort, active tab, inline slugs, preview mode) across runs.
- **One-level undo** in the service for config mutations.
- **Cross-platform paths** (XDG on Unix, `%APPDATA%`/`%LOCALAPPDATA%` on Windows) and openers/clipboard.
- Unicode and ASCII icon tiers (`config.icons.variant`).

[Unreleased]: https://github.com/cgroening/rs-hop/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/cgroening/rs-hop/releases/tag/v0.1.0
