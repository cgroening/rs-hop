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

    // Mutate and persist, then reload through a fresh backend.
    repos[0].fav = false;
    repo.save_all(&repos).unwrap();
    let reloaded = TomlRepoRepository::new(config_path.clone())
        .find_all()
        .unwrap();
    assert!(!reloaded[0].fav);

    // Settings (and the comment) survive the rewrite.
    let config = load_config(&config_path).unwrap();
    assert_eq!(config.git_program.as_deref(), Some("lazygit"));
    let text = fs::read_to_string(&config_path).unwrap();
    assert!(text.contains("# settings with a comment"));

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
