//! Slug generation and validation for the `hop <slug>` fast jump.
//!
//! A slug is a short, URL-safe shortcut (`[a-z0-9-]+`). It must not collide
//! with a reserved subcommand name; uniqueness against other entries is checked
//! by the service layer, which owns the full list.

use crate::domain::error::{Error, Result};

/// Subcommand names a slug must never shadow (`hop <reserved>` is a command).
/// Kept in sync with the clap subcommands; a CLI test cross-checks it.
pub const RESERVED: &[&str] =
    &["add", "scan", "doctor", "list", "config-path", "help"];

/// Maximum slug length, keeping shortcuts terse.
const MAX_LEN: usize = 40;

/// Turns arbitrary text into a slug: lowercased, German umlauts transliterated,
/// every run of non-alphanumeric characters collapsed to a single hyphen, and
/// trimmed of leading/trailing hyphens. Capped at `MAX_LEN`.
pub fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut last_was_hyphen = true; // suppress a leading hyphen
    for ch in input.trim().chars() {
        for mapped in transliterate(ch).chars() {
            if mapped.is_ascii_alphanumeric() {
                out.push(mapped.to_ascii_lowercase());
                last_was_hyphen = false;
            } else if !last_was_hyphen {
                out.push('-');
                last_was_hyphen = true;
            }
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out.truncate(MAX_LEN);
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Expands German umlauts and the sharp s; other characters pass through.
fn transliterate(ch: char) -> String {
    match ch {
        'ä' | 'Ä' => "ae".to_string(),
        'ö' | 'Ö' => "oe".to_string(),
        'ü' | 'Ü' => "ue".to_string(),
        'ß' => "ss".to_string(),
        other => other.to_string(),
    }
}

/// Validates a slug's format: non-empty, only `[a-z0-9-]`, not all hyphens, and
/// not a reserved subcommand name. Uniqueness is enforced by the caller.
///
/// # Errors
/// Returns [`Error::Slug`] describing the first rule the slug breaks.
pub fn validate_format(slug: &str) -> Result<()> {
    if slug.is_empty() {
        return Err(Error::Slug("slug must not be empty".to_string()));
    }
    if slug.len() > MAX_LEN {
        return Err(Error::Slug(format!(
            "slug must be at most {MAX_LEN} characters"
        )));
    }
    let well_formed = slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !well_formed {
        return Err(Error::Slug(
            "slug may only contain a-z, 0-9 and hyphens".to_string(),
        ));
    }
    if slug.chars().all(|c| c == '-') {
        return Err(Error::Slug(
            "slug must contain a letter or digit".to_string(),
        ));
    }
    if RESERVED.contains(&slug) {
        return Err(Error::Slug(format!(
            "'{slug}' is a reserved command name"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_hyphenates() {
        assert_eq!(slugify("(rs) My Repo!"), "rs-my-repo");
    }

    #[test]
    fn slugify_transliterates_umlauts() {
        assert_eq!(slugify("Übungs-Ärger"), "uebungs-aerger");
    }

    #[test]
    fn slugify_trims_and_collapses_separators() {
        assert_eq!(slugify("  a___b  "), "a-b");
        assert_eq!(slugify("---"), "");
    }

    #[test]
    fn validate_rejects_reserved_and_malformed() {
        assert!(validate_format("list").is_err());
        assert!(validate_format("add").is_err());
        assert!(validate_format("").is_err());
        assert!(validate_format("Has Space").is_err());
        assert!(validate_format("UPPER").is_err());
        assert!(validate_format("-").is_err());
    }

    #[test]
    fn validate_accepts_well_formed() {
        assert!(validate_format("hop").is_ok());
        assert!(validate_format("rs-mdtask").is_ok());
        assert!(validate_format("py-due2").is_ok());
    }
}
