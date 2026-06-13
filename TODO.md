# hop – TODO

Planned, not yet implemented (roughly in priority order):

- **Repo scan / discovery.** A `hop scan <dir>` command that walks `<dir>`
  recursively, finds git work trees (a `.git` entry) and offers a bulk import
  into `config.toml` (skipping entries that already exist). The biggest
  time-saver for building the list initially. Reuse `RepoService::add` and the
  comment-preserving `config::writer`; keep the walk bounded and skip
  `node_modules`/hidden dirs.

- **Shell completion for slugs.** `hop completions {zsh,bash,fish,powershell}`
  that prints a completion script so `hop <Tab>` completes known slugs. Slugs
  come from `RepoService`/the repository; clap can generate the static scaffold
  (`clap_complete`) with a dynamic slug list.

- **`hop doctor` (CLI).** Reports problems without the TUI: missing paths,
  duplicate or invalid slugs, and entries marked `git` that are not
  repositories. Reuse `Repo::entry_error` and the existence-check logic; print a
  grouped, scriptable summary and exit non-zero when issues are found.

- **Per-entry open override.** Two optional per-entry fields that override the
  global open heuristic (`domain::repo::classify_path`), applied before it when
  set; **both** should be implemented:
  - `open_with` — a command/template run with the path, e.g. `open_with = "code"`
    or `open_with = "open -a Preview"`. Lets you force a specific app/editor for
    one entry regardless of its extension.
  - `open_as` — a forced mode `editor | app | cd` that overrides the auto
    folder/text/other detection (e.g. always open a `.png` in the editor, or
    always `cd` into a path instead of opening a file in it).
  Add the fields to `Repo` + the TOML (de)serialise (`storage::toml_repo_repository`
  / `config::writer`), the add/edit form, and the open paths
  (`tui::open_selected` / `cli::perform_jump`).

Done: undo key (`u`), per-tab cursor memory, paging, tab cycling, frecency sort,
`hop add`, open-on-GitHub (`b`), fuzzy-match highlighting, header lenses,
detail/preview panel (`v`), Windows support (paths, clipboard, opener).
