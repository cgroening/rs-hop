use super::*;
use crate::domain::slug::RESERVED;

#[test]
fn the_parser_definition_is_internally_consistent() {
    // Catches conflicting flags, duplicate shorts and bad defaults at test
    // time rather than on the user's first run.
    Cli::command().debug_assert();
}

#[test]
fn every_subcommand_name_is_a_reserved_slug() {
    // A slug that shadows a command would make that command unreachable.
    for command in Cli::command().get_subcommands() {
        let name = command.get_name();
        if name == "help" {
            continue;
        }
        assert!(
            RESERVED.contains(&name),
            "subcommand '{name}' is missing from domain::slug::RESERVED"
        );
    }
}

#[test]
fn verbosity_flags_pick_the_log_level() {
    let quiet = Cli::parse_from(["hop", "--quiet"]);
    assert_eq!(quiet.log_level(), log::LevelFilter::Error);
    let plain = Cli::parse_from(["hop"]);
    assert_eq!(plain.log_level(), log::LevelFilter::Info);
    let verbose = Cli::parse_from(["hop", "-v"]);
    assert_eq!(verbose.log_level(), log::LevelFilter::Debug);
    let louder = Cli::parse_from(["hop", "-vv"]);
    assert_eq!(louder.log_level(), log::LevelFilter::Trace);
}

#[test]
fn the_command_line_overrides_fetch_on_start_in_both_directions() {
    // The precedence rule: an option always beats the config file.
    let mut config = Config {
        fetch_on_start: true,
        ..Config::default()
    };
    let off = Cli::parse_from(["hop", "--no-fetch"]);
    assert!(matches!(
        startup_status(&off, &config),
        StartupStatus::Refresh { fetch: false }
    ));

    config.fetch_on_start = false;
    let on = Cli::parse_from(["hop", "--fetch"]);
    assert!(matches!(
        startup_status(&on, &config),
        StartupStatus::Refresh { fetch: true }
    ));
}

#[test]
fn the_legacy_config_path_name_still_resolves() {
    // Renaming a published command must not break existing scripts.
    let cli = Cli::parse_from(["hop", "config-path"]);
    assert!(matches!(cli.command, Some(Command::ShowConfig)));
}

#[test]
fn an_unknown_kind_is_rejected_rather_than_silently_becoming_a_path() {
    assert!(Cli::try_parse_from(["hop", "add", "--kind", "gti", "."]).is_err());
    let ok = Cli::parse_from(["hop", "add", "--kind", "git", "."]);
    assert!(matches!(
        ok.command,
        Some(Command::Add {
            kind: Some(KindArg::Git),
            ..
        })
    ));
}
