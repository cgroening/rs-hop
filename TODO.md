# hop – TODO

Planned, not yet implemented (roughly in priority order):

- [ ] **Shell completion for slugs.** `hop completions {zsh,bash,fish,powershell}` that prints a completion script so `hop <Tab>` completes known slugs. Slugs come from `RepoService`/the repository; clap can generate the static scaffold (`clap_complete`) with a dynamic slug list.

- [ ] **Per-entry open override.** Two optional per-entry fields that override the
  global open heuristic (`domain::repo::classify_path`), applied before it when
  set; **both** should be implemented:
  - `open_with` — a command/template run with the path, e.g. `open_with = "code"` or `open_with = "open -a Preview"`. Lets you force a specific app/editor for one entry regardless of its extension.
  - `open_as` — a forced mode `editor | app | cd` that overrides the auto folder/text/other detection (e.g. always open a `.png` in the editor, or always `cd` into a path instead of opening a file in it).
  Add the fields to `Repo` + the TOML (de)serialise (`storage::toml_repo_repository` / `config::writer`), the add/edit form, and the open paths (`tui::open_selected` / `cli::perform_jump`).

Done: undo key, per-tab cursor, paging, tab cycling, frecency sort, `hop add`,
`hop scan` (discover + multi-select import), `hop doctor`, open-on-GitHub,
fuzzy-match highlighting, header lenses, detail/preview panel, Windows support.
