//! Integration tests driving the public API: the TOML entry backend round
//! trips on disk and the service persists through it.

use std::fs;
use std::path::PathBuf;

use hop::config::loader::load_config;
use hop::domain::repo::RepoKind;
use hop::service::repo_service::RepoService;
use hop::storage::repository::RepoRepository;
use hop::storage::toml_repo_repository::TomlRepoRepository;

/// A unique temporary directory for one test.
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hop-it-{tag}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

const SAMPLE_CONFIG: &str = r#"# settings with a comment
git_program = "lazygit"
github_username = "cgroening"

[[repos]]
name = "hop"
path = "/code/hop"
slug = "hop"
fav = true

[[repos]]
path = "/notes"
kind = "folder"
archived = true
include_in_backup = true
"#;

#[test]
fn toml_backend_round_trips_and_preserves_settings() {
    let dir = temp_dir("roundtrip");
    let config_path = dir.join("config.toml");
    fs::write(&config_path, SAMPLE_CONFIG).unwrap();

    let repo = TomlRepoRepository::new(config_path.clone());
    let mut repos = repo.find_all().unwrap();
    assert_eq!(repos.len(), 2);
    assert_eq!(repos[0].slug.as_deref(), Some("hop"));
    assert_eq!(repos[1].kind, RepoKind::Path);
    assert!(repos[1].archived);
    // Git default is included; the folder opts in explicitly.
    assert!(repos[0].include_in_backup);
    assert!(repos[1].include_in_backup);

    // Mutate and persist, then reload through a fresh backend.
    repos[0].fav = false;
    repo.save_all(&repos).unwrap();
    let reloaded = TomlRepoRepository::new(config_path.clone())
        .find_all()
        .unwrap();
    assert!(!reloaded[0].fav);
    // The folder's opt-in survives the rewrite (deviates from path default).
    assert!(reloaded[1].include_in_backup);

    // Settings (and the comment) survive the rewrite.
    let config = load_config(&config_path).unwrap();
    assert_eq!(config.git_program.as_deref(), Some("lazygit"));
    let text = fs::read_to_string(&config_path).unwrap();
    assert!(text.contains("# settings with a comment"));

    fs::remove_dir_all(&dir).ok();
}

const SECTIONED_CONFIG: &str = r#"
sections = ["Notes"]
git_sections = ["Backend", "Frontend"]

[[repos]]
name = "api"
path = "/code/api"
kind = "git"
section = "Backend"

[[repos]]
name = "old-api"
path = "/code/old-api"
kind = "git"
archived = true
section = "Backend"

[[repos]]
name = "diary"
path = "/notes/diary"
kind = "folder"
section = "Notes"
"#;

#[test]
fn per_kind_sections_stay_separate_and_survive_archiving() {
    let dir = temp_dir("sections");
    let config_path = dir.join("config.toml");
    fs::write(&config_path, SECTIONED_CONFIG).unwrap();

    let backend = TomlRepoRepository::new(config_path.clone());
    // Each kind's section order is read from its own key.
    assert_eq!(
        backend.find_sections(RepoKind::Git).unwrap(),
        vec!["Backend".to_string(), "Frontend".to_string()]
    );
    assert_eq!(
        backend.find_sections(RepoKind::Path).unwrap(),
        vec!["Notes".to_string()]
    );

    // The archived git repo keeps its section (grouping survives archiving).
    let repos = backend.find_all().unwrap();
    let archived = repos.iter().find(|r| r.archived).unwrap();
    assert_eq!(archived.section.as_deref(), Some("Backend"));

    // Rewriting the git order leaves the files order untouched, and both keys
    // are present after the write.
    backend
        .save_sections(RepoKind::Git, &["Frontend".to_string()])
        .unwrap();
    let reloaded = TomlRepoRepository::new(config_path.clone());
    assert_eq!(
        reloaded.find_sections(RepoKind::Git).unwrap(),
        vec!["Frontend".to_string()]
    );
    assert_eq!(
        reloaded.find_sections(RepoKind::Path).unwrap(),
        vec!["Notes".to_string()]
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn service_persists_changes_to_disk() {
    let dir = temp_dir("service");
    let config_path = dir.join("config.toml");
    fs::write(&config_path, SAMPLE_CONFIG).unwrap();

    {
        let mut service = RepoService::new(
            Box::new(TomlRepoRepository::new(config_path.clone())),
            dir.join("usage.toml"),
            dir.join("selected.txt"),
        )
        .unwrap();
        service.set_slug(1, Some("notes".to_string())).unwrap();
        service.set_archived(1, false).unwrap();
    }

    // A brand new service reads the persisted change back.
    let service = RepoService::new(
        Box::new(TomlRepoRepository::new(config_path.clone())),
        dir.join("usage.toml"),
        dir.join("selected.txt"),
    )
    .unwrap();
    assert_eq!(service.index_by_slug("notes"), Some(1));
    assert!(!service.get(1).unwrap().archived);

    fs::remove_dir_all(&dir).ok();
}
