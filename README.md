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
- **In-app management**: add, edit, delete, favourite, archive/restore, set a
  slug - the config is written back preserving its comments.
- **Slugs**: `hop <slug>` jumps straight to an entry from the shell.
- **Sort modes**: favourites first, most recently used, or by name.
- **Missing-path marker** (a red `!`) with a picker that opens at the closest
  existing ancestor to repair the path.
- **Three icon tiers** (Nerd Font / Unicode / ASCII), selectable in the config.

## Install

```
cargo build --release
# the binary is at target.nosync/release/hop
```

## Shell integration (zsh)

hop writes the chosen path to `$XDG_STATE_HOME/hop/selected-repo.txt` (default
`~/.local/state/hop/selected-repo.txt`). Wrap the binary so your shell `cd`s
there after hop exits:

```zsh
hop() {
    command hop "$@"
    local f="${XDG_STATE_HOME:-$HOME/.local/state}/hop/selected-repo.txt"
    [[ -f "$f" ]] && { local d="$(cat "$f")"; [[ -d "$d" ]] && cd "$d"; }
}
```

Now `hop` opens the TUI and drops you into the selected directory, and
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
# editor = "nvim"              # for opening files; else $VISUAL / $EDITOR

[icons]
variant = "nerdfont"           # nerdfont | unicode | ascii

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
kind = "git"                   # git | folder | file
slug = "hop"                   # optional; enables `hop hop`
fav = true                     # favourites sort first
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
--fetch / --cached  fetch before / use cached git status
```

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `1` / `2` / `3` | switch tab (Git Repos / Files / Archiv) |
| `↑` / `↓` | move cursor (wraps) |
| `Enter` | open: write path + launch git tool, then exit |
| `o` | jump only: write path and exit (cd, no tool) |
| `F` | live fuzzy filter (`Esc` clears) |
| `s` | cycle sort (favourites / recent / name) |
| `a` | add an entry (pick a path, then fill the form) |
| `e` | edit the selected entry |
| `d` | delete the selected entry (confirm) |
| `f` | toggle favourite |
| `A` | archive / restore the selected entry |
| `S` | set or change the slug |
| `p` | repair a missing path |
| `r` | reload git status   ·   `R`: `git fetch` + reload |
| `?` | toggle the help overlay |
| `Ctrl+Q` | quit |

## Development

```
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

See [`CLAUDE.md`](CLAUDE.md) for the architecture and coding rules.
