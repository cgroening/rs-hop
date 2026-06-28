# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **ZIP backups**: `z` zips the selected/cursor git repo and `Z` zips all git repos into the configured `zip_backup_folder`, excluding build artefacts (the new `zip_exclude_dirs` setting, matched as a name prefix so `target` also covers `target.nosync`) but keeping `.git`. The archive is named after the entry (the slugified name, e.g. `(rs) mdtask` → `rs-mdtask.zip`); two repos that would share a name each get a short, stable path-hash suffix so no backup is overwritten, and `hop doctor` reports the name clash. A repo is only (re)written when its content changed since the last backup (name + size + CRC32 comparison), leaving unchanged archives untouched so cloud sync does not re-upload them (a local fingerprint cache makes that check avoid re-downloading an iCloud-evicted archive). Archives are written atomically (temp file + rename) so a failed run never corrupts the previous backup, and Unix permissions and symlinks are preserved. Progress shows in the header bar (same place as the status refresh). The Git Repos and Archive tabs gain a `ZIP Backup` column showing each repo's last-backup date.
- **Inline git tool overlay**: `l` opens the configured git tool (lazygit) for the selected git repo as an overlay – the TUI suspends the terminal, runs the tool, then returns to the list and refreshes only that repo's status. `L` launches the tool and exits.

### Changed

- **Favourite moved from `z` to `*`** to free `z`/`Z` for the new ZIP backups.
- **`Enter` now only jumps (like `o`)** instead of launching the git tool / opening the file: it `cd`s into a folder, writes the parent of a file, and for a git repo writes the path and exits without launching the tool. The previous `Enter` behaviour (git → tool · folder → cd · text file → editor · other file → default app) moved to `L`.
- **The error list (`!`) and other select modals now show a scrollbar** when their contents overflow the modal height.
- **The preview `git log` now loads in the background** instead of blocking on each cursor move, so navigating with the preview open (`v`) stays smooth. The log for an entry is fetched only after the cursor rests on it briefly (debounced), cached, and shown with a `loading…` placeholder until it arrives. A full reload (`r`) reloads the current tab's logs and a per-entry refresh (`x`) reloads that entry's log – both only while the preview is visible.

### Fixed

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
