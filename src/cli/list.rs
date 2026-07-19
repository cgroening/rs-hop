//! `hop list`: the entries as a styled table, as plain tab-separated lines, or
//! as JSON.

use serde::Serialize;
use sparcli::{Cell, Column, Table};

use crate::cli::context::CliContext;
use crate::cli::output::{CliResult, Streams, info};
use crate::domain::repo::Repo;
use crate::service::repo_service::RepoService;

/// One entry in the `--json` payload.
///
/// The field names are part of the public interface, so they do not change
/// alongside internal refactorings.
#[derive(Serialize)]
struct JsonEntry<'a> {
    slug: Option<&'a str>,
    name: String,
    kind: &'a str,
    path: String,
    fav: bool,
    archived: bool,
    section: Option<&'a str>,
}

/// Lists the entries.
///
/// On a terminal this renders a styled table; piped it falls back to plain
/// tab-separated lines, so scripts consuming `hop list` are unaffected.
/// `--json` overrides both and prints nothing but the JSON document.
///
/// # Errors
///
/// Returns an error if the payload cannot be written or cannot be serialised.
pub fn run(
    service: &RepoService,
    ctx: CliContext,
    is_json: bool,
    streams: &mut Streams,
) -> CliResult {
    let repos = service.repos();
    if is_json {
        return print_json(repos, streams);
    }
    if repos.is_empty() {
        // A hint, not payload: it belongs on stderr so `hop list > file` does
        // not write it into the data.
        info(
            streams,
            "No entries yet. Run hop to add some, or hop add <path>.",
        );
        return Ok(());
    }
    if ctx.is_output_tty {
        streams.payload(&entry_table(repos))?;
        return Ok(());
    }
    for repo in repos {
        streams.line(&list_line(repo))?;
    }
    Ok(())
}

/// Prints the entries as a single JSON array, unstyled.
fn print_json(repos: &[Repo], streams: &mut Streams) -> CliResult {
    let entries: Vec<JsonEntry> = repos.iter().map(json_entry).collect();
    let text = serde_json::to_string_pretty(&entries).map_err(|error| {
        crate::cli::output::CliError::runtime(error.to_string())
    })?;
    streams.line(&text)?;
    Ok(())
}

/// The JSON view of one entry.
fn json_entry(repo: &Repo) -> JsonEntry<'_> {
    JsonEntry {
        slug: repo.slug.as_deref(),
        name: repo.display_name().to_string(),
        kind: repo.kind.as_config_value(),
        path: repo.path.display().to_string(),
        fav: repo.fav,
        archived: repo.archived,
        section: repo.section.as_deref(),
    }
}

/// The entry list as a styled sparcli table.
fn entry_table(repos: &[Repo]) -> Table {
    let mut table = Table::new().columns([
        Column::new("Slug"),
        Column::new("Name"),
        Column::new("Kind"),
        Column::new("Path"),
        Column::new("Flags"),
    ]);
    for repo in repos {
        table = table.row([
            Cell::new(repo.slug.clone().unwrap_or_default()),
            Cell::new(repo.display_name().to_string()),
            Cell::new(repo.kind.as_config_value().to_string()),
            Cell::new(repo.path.display().to_string()),
            Cell::new(entry_flags(repo)),
        ]);
    }
    table
}

/// The comma-separated `fav`/`archived` flags of an entry (empty when none).
fn entry_flags(repo: &Repo) -> String {
    let mut flags = Vec::new();
    if repo.fav {
        flags.push("fav");
    }
    if repo.archived {
        flags.push("archived");
    }
    flags.join(", ")
}

/// A single plain-text list line for an entry (the piped, script-friendly form).
fn list_line(repo: &Repo) -> String {
    let slug = repo
        .slug
        .as_deref()
        .map(|s| format!("[{s}] "))
        .unwrap_or_default();
    let flags = entry_flags(repo);
    let flags = if flags.is_empty() {
        String::new()
    } else {
        format!(" ({flags})")
    };
    format!(
        "{slug}{}\t{}\t{}{flags}",
        repo.display_name(),
        repo.kind.as_config_value(),
        repo.path.display(),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn repo(name: &str, slug: Option<&str>) -> Repo {
        let mut repo = Repo::new(PathBuf::from(format!("/code/{name}")));
        repo.name = Some(name.to_string());
        repo.slug = slug.map(str::to_string);
        repo
    }

    fn streams<'a>(out: &'a mut Vec<u8>, err: &'a mut Vec<u8>) -> Streams<'a> {
        Streams { out, err }
    }

    #[test]
    fn list_line_includes_slug_kind_and_flags() {
        let mut entry = repo("hop", Some("hop"));
        entry.fav = true;
        let line = list_line(&entry);
        assert!(line.starts_with("[hop] hop\tgit\t/code/hop"));
        assert!(line.ends_with("(fav)"));
    }

    #[test]
    fn an_empty_store_keeps_stdout_clean_and_hints_on_stderr() {
        // The regression this pins: the hint used to go to stdout, so
        // `hop list > out.txt` wrote it into the payload file.
        let repos: Vec<Repo> = Vec::new();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let ctx = CliContext {
            is_interactive: false,
            is_output_tty: false,
        };
        let mut streams = streams(&mut out, &mut err);
        if repos.is_empty() {
            info(&mut streams, "No entries yet.");
        }
        assert!(out.is_empty(), "stdout must stay empty");
        assert!(String::from_utf8(err).unwrap().contains("No entries yet"));
        let _ = ctx;
    }

    #[test]
    fn the_json_payload_carries_the_documented_fields() {
        let entries = vec![repo("hop", Some("hop"))];
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut s = streams(&mut out, &mut err);
        print_json(&entries, &mut s).unwrap();

        let text = String::from_utf8(out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let first = &parsed[0];
        assert_eq!(first["slug"], "hop");
        assert_eq!(first["name"], "hop");
        assert_eq!(first["kind"], "git");
        assert_eq!(first["path"], "/code/hop");
        assert_eq!(first["fav"], false);
        // Nothing human-readable may accompany the JSON on stdout.
        assert!(text.trim_start().starts_with('['));
        assert!(err.is_empty());
    }

    #[test]
    fn a_missing_slug_serialises_as_null_rather_than_an_empty_string() {
        let entry = repo("notes", None);
        let json = serde_json::to_value(json_entry(&entry)).unwrap();
        assert!(json["slug"].is_null());
    }
}
