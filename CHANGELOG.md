# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Inline git tool overlay**: `l` opens the configured git tool (lazygit) for the selected git repo as an overlay – the TUI suspends the terminal, runs the tool, then returns to the list and refreshes only that repo's status. `Enter` is unchanged (launches the tool and exits).

### Changed

- **The error list (`!`) and other select modals now show a scrollbar** when their contents overflow the modal height.

### Fixed

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
