# hop – TODO

Planned, not yet implemented:

- **Undo key in the TUI.** The service already supports one-level undo
  (`RepoService::undo`, returns the action label and rolls back the last config
  mutation). Only the key binding is missing - bind `u` in `App::handle_list_key`
  (`src/tui/mod.rs`), report the returned label via the status line, and add it
  to the footer hints, help overlay and README.
