//! Every colour comes from a theme: no colour literal or colour constant may
//! appear in `src/` outside the files listed in `ALLOWED`, each with its
//! reason. Test modules, comments and doc examples are not scanned.

use std::{fs, path::Path};

/// Files that may hold colour values, and why.
const ALLOWED: &[(&str, &str)] =
    &[("src/config/app_colors.rs", "hop's built-in theme")];

/// Text that marks a colour literal.
const PATTERNS: &[&str] = &[
    "Color::hex(",
    "Color::Rgb(0",
    "Color::Rgb(1",
    "Color::Rgb(2",
    "Color::Rgb(3",
    "Color::Rgb(4",
    "Color::Rgb(5",
    "Color::Rgb(6",
    "Color::Rgb(7",
    "Color::Rgb(8",
    "Color::Rgb(9",
    "Color::Black",
    "Color::Red",
    "Color::Green",
    "Color::Yellow",
    "Color::Blue",
    "Color::Magenta",
    "Color::Cyan",
    "Color::Gray",
    "Color::DarkGray",
    "Color::White",
    "Color::Light",
];

#[test]
fn colours_come_only_from_the_theme() {
    let mut found = Vec::new();
    scan(Path::new("src"), &mut found);
    assert!(
        found.is_empty(),
        "colour literals outside the theme:\n{}",
        found.join("\n")
    );
}

/// Collects every offending line under `dir`.
fn scan(dir: &Path, found: &mut Vec<String>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .expect("readable source directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            scan(&path, found);
            continue;
        }
        let name = path.to_string_lossy().replace('\\', "/");
        let is_allowed = ALLOWED.iter().any(|(file, _)| name.starts_with(file));
        let is_rust = path.extension().is_some_and(|ext| ext == "rs");
        if !is_rust || is_allowed || name.ends_with("tests.rs") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("readable source file");
        scan_file(&name, &text, found);
    }
}

/// Collects the offending lines of one file, stopping at its test module.
fn scan_file(name: &str, text: &str, found: &mut Vec<String>) {
    for (index, line) in text.lines().enumerate() {
        let code = line.trim_start();
        if code.starts_with("#[cfg(test)]") {
            return;
        }
        if code.starts_with("//") {
            continue;
        }
        if PATTERNS.iter().any(|pattern| line.contains(pattern))
            || has_hex_string(line)
        {
            found.push(format!("{name}:{}: {}", index + 1, code));
        }
    }
}

/// Whether `line` holds a quoted `"#rgb"` or `"#rrggbb"` color.
fn has_hex_string(line: &str) -> bool {
    line.match_indices("\"#").any(|(start, _)| {
        let rest = &line[start + 2..];
        let digits = rest.chars().take_while(char::is_ascii_hexdigit).count();
        (digits == 3 || digits == 6) && rest[digits..].starts_with('"')
    })
}
