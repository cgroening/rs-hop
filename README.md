# hop

A fast, fuzzy-finder TUI to jump between your git repositories and folders -
a Rust port of [git-repo-jumper](../../Python/cli/git-repo-jumper), built on
[ratatui](https://ratatui.rs).

Pick an entry and hop writes its path to a handoff file your shell reads to
`cd` into it; for git repos it also launches your git tool (lazygit by
default). On top of the original it adds in-app management, three tabs, slugs
for a one-word jump, sort modes and repair for paths that have moved.

## Features

- **Fuzzy finder** over all visible columns (name, branch, status, GitHub
  name, path).
- **Live git status** (branch, uncommitted changes, ahead/behind, GitHub repo
  name), gathered in the background and cached so the list shows instantly.
- **Three tabs**: Git Repos, Files and Folders, Archiv.
- **Sections** on the Files and Folders tab: group folders/files under named,
  reorderable section headers; `s` jumps to a section and `M` manages them.
- **In-app management**: add, edit, delete, favourite, archive/restore, set a
  slug - the config is written back preserving its comments.
- **Content-aware open**: a file/folder entry auto-detects its target — a
  folder `cd`s, a text file (by extension; configurable via `editor_extensions`)
  opens in the editor, and any other file (image, PDF, …) opens with the
  system's default app.
- **Slugs**: `hop <slug>` jumps straight to an entry from the shell.
- **Sort modes**: by name, most recently used, or a custom drag order;
  favourites are pinned to the top (except in the recent view).
- **Missing-path marker** (a red `!`) with a picker that opens at the closest
  existing ancestor to repair the path, plus an error list (`!`) to repair /
  edit / delete all broken entries.
- **Status bar**: a local status line (entry count, sort, last status time) and
  a remote line showing the last `git fetch` (warns when over a day old); a
  progress bar replaces it while a refresh runs.
- **Two icon tiers** (Unicode by default, or ASCII), selectable in the config.

## Install

Install the binary globally from the project directory (puts `hop` in
`~/.cargo/bin`, which is on your `PATH`):

```
cargo install --path .
```

Re-run the same command to update after code changes; `cargo uninstall hop`
removes it. (For a local build without installing, use `cargo build --release`;
the binary is then at `target.nosync/release/hop`.)

## Shell integration (zsh)

The binary alone writes the chosen path and exits - it cannot change your
shell's directory. Wrap it in a shell function so your shell `cd`s into the
selected entry after hop exits. hop writes that path to
`$XDG_STATE_HOME/hop/selected-repo.txt` (default
`~/.local/state/hop/selected-repo.txt`).

Add this to a sourced zsh file (e.g. `~/.zshrc`, or a functions file it
sources):

```zsh
# Run hop, then cd into the selected entry after it exits.
hop() {
    command hop "$@"
    local f="${XDG_STATE_HOME:-$HOME/.local/state}/hop/selected-repo.txt"
    [[ -f "$f" ]] && { local d="$(cat "$f")"; [[ -d "$d" ]] && cd "$d"; }
}

# Optional short alias.
alias hp='hop'
```

The function is named `hop` and shadows the binary; it runs the real binary via
`command hop`, so there is no conflict. Reload with `source ~/.zshrc` (or open a
new shell).

Now `hop` (or `hp`) opens the TUI and drops you into the selected directory, and
`hop <slug>` jumps directly.

## Configuration

hop reads `$XDG_CONFIG_HOME/hop/config.toml` (default
`~/.config/hop/config.toml`). On first start, if no config exists but a
git-repo-jumper `~/.config/hop/config.yaml` does, it is imported automatically;
you can also run `hop import --from <path>` to convert any git-repo-jumper
`config.yaml`. A hidden `show: false` entry becomes `archived = true`.

```toml
git_program = "lazygit"        # tool launched for git repos; omit to disable
github_username = "you"        # stripped from displayed remote names
example_mode = false           # show example_git_info instead of real status
fetch_on_start = false         # git fetch in the background when hop starts
# editor = "nvim"              # for opening text files; else $VISUAL / $EDITOR
# editor_extensions = ["rs", "md", "txt"]  # override the built-in text list

[icons]
variant = "unicode"            # unicode | ascii

[column_widths]
name = 30
status = 6
github_repo_name = 20
[column_widths.current_branch_name]
min = 10
max = 14

[[repos]]
name = "(rs) hop"              # optional; defaults to the path's basename
path = "/Users/you/Code/hop"
kind = "git"                   # git | path (path = file or folder, auto-detected)
slug = "hop"                   # optional; enables `hop hop`
fav = true                     # favourites sort first
# section = "Work"             # groups path entries on the Files tab
# archived = false             # archived entries live in the Archiv tab
```

See [`examples/config.toml`](examples/config.toml) for a fuller sample. Try it
without touching your real config:

```
HOP_CONFIG=examples/config.toml cargo run
```

## Commands

```
hop                 open the TUI
hop <slug>          jump to a slug: write the path + launch the git tool
hop <slug> -s       jump only: write the path, no tool (cd)
hop list            list entries as plain text
hop import [--from PATH]  import a git-repo-jumper config.yaml
hop config-path     print the resolved config file path
-C / --config PATH  use a specific config file (also via HOP_CONFIG)
--fetch             git fetch first (TUI: on start; hop <slug>: before launching)
--cached            TUI: show only cached status, run no git
```

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `1` / `2` / `3` | switch tab (Git Repos / Files / Archiv) |
| `↑` / `↓` | move cursor (wraps) |
| `Space` | toggle selection · `Shift+↑/↓`: extend range · `Esc`: clear |
| `Enter` | open: git → tool · folder → cd · text file → editor · other file → default app |
| `o` | jump only: write path and exit (folder → cd, file → its parent) |
| `O` | force open with the default app (regardless of kind) |
| `f` | live fuzzy filter (`Esc` clears) |
| `s` | git tabs: cycle sort (name / recent / custom) · Files: jump to a section |
| `M` | Files tab: manage sections (add / rename / delete / reorder) |
| `Alt+↑/↓` | reorder the entry (custom sort, or within a Files section; favourites stay on top) |
| `Ctrl+↑/↓` | Files tab: jump to the previous / next section |
| `n` | add an entry (fill the form; `^O` opens the path picker) |
| `e` | edit the selected entry |
| `d` / `Del` / `Backspace` | delete (acts on the selection, else the cursor; confirm) |
| `z` | toggle favourite (selection or cursor) |
| `A` | archive / restore (selection or cursor) |
| `S` | set or change the slug |
| `y` | copy the selected entry's path to the clipboard |
| `p` | repair a missing path |
| `!` | list entries with path errors, then repair / edit / delete |
| `r` | git tabs: reload status (`R`: + `git fetch`) · Files tab: check that paths still exist |
| `x` | refresh selection/cursor   ·   `X`: with `git fetch` |
| `?` | toggle the help overlay |
| `q` / `Ctrl+Q` | quit |

## Development

```
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

See [`CLAUDE.md`](CLAUDE.md) for the architecture and coding rules.
