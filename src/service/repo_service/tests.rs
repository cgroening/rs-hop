use super::*;
use crate::domain::repo::RepoKind;
use crate::storage::in_memory_repository::InMemoryRepoRepository;

fn service(initial: Vec<Repo>) -> RepoService {
    let dir = std::env::temp_dir().join(format!(
        "hop-svc-{}-{}",
        std::process::id(),
        initial.len()
    ));
    RepoService::new(
        Box::new(InMemoryRepoRepository::new(initial)),
        dir.join("usage.toml"),
        dir.join("selected.txt"),
    )
    .unwrap()
}

fn repo(name: &str) -> Repo {
    let mut repo = Repo::new(PathBuf::from(format!("/code/{name}")));
    repo.name = Some(name.to_string());
    repo
}

#[test]
fn add_rejects_duplicate_slug() {
    let mut a = repo("a");
    a.slug = Some("x".to_string());
    let mut svc = service(vec![a]);
    let mut b = repo("b");
    b.slug = Some("x".to_string());
    assert!(matches!(svc.add(b), Err(Error::Slug(_))));
    assert_eq!(svc.repos().len(), 1);
}

#[test]
fn delete_many_removes_all_and_undoes_as_one() {
    let mut svc = service(vec![repo("a"), repo("b"), repo("c")]);
    svc.delete_many(&[0, 2]).unwrap();
    let names: Vec<_> = svc.repos().iter().map(Repo::display_name).collect();
    assert_eq!(names, vec!["b"]);
    svc.undo().unwrap();
    assert_eq!(svc.repos().len(), 3);
}

#[test]
fn set_archived_and_fav_many() {
    let mut svc = service(vec![repo("a"), repo("b"), repo("c")]);
    svc.set_archived_many(&[0, 1], true).unwrap();
    assert!(svc.get(0).unwrap().archived);
    assert!(svc.get(1).unwrap().archived);
    assert!(!svc.get(2).unwrap().archived);
    svc.set_fav_many(&[1, 2], true).unwrap();
    assert!(svc.get(1).unwrap().fav);
    assert!(svc.get(2).unwrap().fav);
}

#[test]
fn swap_entries_reorders() {
    let mut svc = service(vec![repo("a"), repo("b"), repo("c")]);
    svc.swap_entries(0, 2).unwrap();
    let names: Vec<_> = svc.repos().iter().map(Repo::display_name).collect();
    assert_eq!(names, vec!["c", "b", "a"]);
}

#[test]
fn add_rejects_reserved_slug() {
    let mut svc = service(vec![]);
    let mut a = repo("a");
    a.slug = Some("list".to_string());
    assert!(matches!(svc.add(a), Err(Error::Slug(_))));
}

#[test]
fn archive_and_restore_round_trip() {
    let mut svc = service(vec![repo("a")]);
    svc.set_archived(0, true).unwrap();
    assert!(svc.get(0).unwrap().archived);
    svc.set_archived(0, false).unwrap();
    assert!(!svc.get(0).unwrap().archived);
}

#[test]
fn toggle_fav_flips_flag() {
    let mut svc = service(vec![repo("a")]);
    svc.toggle_fav(0).unwrap();
    assert!(svc.get(0).unwrap().fav);
}

#[test]
fn set_slug_then_lookup_by_slug() {
    let mut svc = service(vec![repo("a"), repo("b")]);
    svc.set_slug(1, Some("bee".to_string())).unwrap();
    assert_eq!(svc.index_by_slug("bee"), Some(1));
}

#[test]
fn undo_reverts_last_change() {
    let mut svc = service(vec![repo("a")]);
    svc.add(repo("b")).unwrap();
    assert_eq!(svc.repos().len(), 2);
    let label = svc.undo().unwrap();
    assert_eq!(label.as_deref(), Some("add entry"));
    assert_eq!(svc.repos().len(), 1);
}

fn sectioned(name: &str, section: Option<&str>) -> Repo {
    let mut repo = repo(name);
    repo.section = section.map(str::to_string);
    repo
}

#[test]
fn update_many_applies_to_all_in_one_undo_frame() {
    let mut svc = service(vec![repo("a"), repo("b"), repo("c")]);
    svc.update_many(&[0, 2], |repo| {
        repo.section = Some("Work".to_string());
    })
    .unwrap();
    assert_eq!(svc.get(0).unwrap().section.as_deref(), Some("Work"));
    assert_eq!(svc.get(1).unwrap().section, None);
    assert_eq!(svc.get(2).unwrap().section.as_deref(), Some("Work"));
    // One undo frame restores every touched entry.
    svc.undo().unwrap();
    assert_eq!(svc.get(0).unwrap().section, None);
    assert_eq!(svc.get(2).unwrap().section, None);
}

const GIT: RepoKind = RepoKind::Git;
const PATH: RepoKind = RepoKind::Path;

#[test]
fn add_section_rejects_empty_reserved_and_duplicate() {
    let mut svc = service(vec![]);
    svc.add_section(GIT, "Work").unwrap();
    assert_eq!(svc.sections(GIT), ["Work"]);
    assert!(matches!(svc.add_section(GIT, "  "), Err(Error::Invalid(_))));
    assert!(matches!(
        svc.add_section(GIT, "Ungrouped"),
        Err(Error::Invalid(_))
    ));
    // Duplicate is case-insensitive.
    assert!(matches!(
        svc.add_section(GIT, "work"),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn ensure_section_is_idempotent() {
    let mut svc = service(vec![]);
    svc.ensure_section(GIT, "Work").unwrap();
    svc.ensure_section(GIT, "work").unwrap();
    svc.ensure_section(GIT, "  ").unwrap();
    assert_eq!(svc.sections(GIT), ["Work"]);
}

#[test]
fn rename_section_updates_entries() {
    let mut svc =
        service(vec![sectioned("a", Some("Work")), sectioned("b", None)]);
    svc.add_section(GIT, "Work").unwrap();
    svc.rename_section(GIT, "Work", "Job").unwrap();
    assert_eq!(svc.sections(GIT), ["Job"]);
    assert_eq!(svc.get(0).unwrap().section.as_deref(), Some("Job"));
    assert_eq!(svc.get(1).unwrap().section, None);
}

#[test]
fn delete_section_ungroups_entries() {
    let mut svc = service(vec![sectioned("a", Some("Work"))]);
    svc.add_section(GIT, "Work").unwrap();
    svc.delete_section(GIT, "Work").unwrap();
    assert!(svc.sections(GIT).is_empty());
    assert_eq!(svc.get(0).unwrap().section, None);
}

#[test]
fn move_section_reorders() {
    let mut svc = service(vec![]);
    svc.add_section(GIT, "Work").unwrap();
    svc.add_section(GIT, "Personal").unwrap();
    svc.add_section(GIT, "Misc").unwrap();
    svc.move_section(GIT, 2, 0).unwrap();
    assert_eq!(svc.sections(GIT), ["Misc", "Work", "Personal"]);
}

#[test]
fn git_and_path_namespaces_are_independent() {
    // Same section name in both namespaces is allowed and separate.
    let mut svc = service(vec![]);
    svc.add_section(GIT, "Work").unwrap();
    svc.add_section(PATH, "Work").unwrap();
    assert_eq!(svc.sections(GIT), ["Work"]);
    assert_eq!(svc.sections(PATH), ["Work"]);
}

#[test]
fn rename_only_touches_the_matching_kind() {
    // A git entry and a (path) folder both sit in a "Work" section.
    let git_entry = sectioned("g", Some("Work"));
    let mut folder = sectioned("f", Some("Work"));
    folder.kind = PATH;
    let mut svc = service(vec![git_entry, folder]);
    svc.add_section(GIT, "Work").unwrap();
    svc.rename_section(GIT, "Work", "Job").unwrap();
    // Only the git entry is rewritten; the folder keeps its own "Work".
    assert_eq!(svc.get(0).unwrap().section.as_deref(), Some("Job"));
    assert_eq!(svc.get(1).unwrap().section.as_deref(), Some("Work"));
}

#[test]
fn delete_removes_entry() {
    let mut svc = service(vec![repo("a"), repo("b")]);
    svc.delete(0).unwrap();
    assert_eq!(svc.repos().len(), 1);
    assert_eq!(svc.get(0).unwrap().display_name(), "b");
}

#[test]
fn add_many_appends_all_in_one_action() {
    let mut svc = service(vec![repo("a")]);
    svc.add_many(vec![repo("b"), repo("c")]).unwrap();
    let names: Vec<_> = svc.repos().iter().map(Repo::display_name).collect();
    assert_eq!(names, vec!["a", "b", "c"]);
    // One undo frame reverts the whole batch.
    svc.undo().unwrap();
    assert_eq!(svc.repos().len(), 1);
    // An empty batch is a no-op.
    svc.add_many(vec![]).unwrap();
    assert_eq!(svc.repos().len(), 1);
}

#[test]
fn update_changes_kind() {
    let mut svc = service(vec![repo("a")]);
    let mut edited = repo("a");
    edited.kind = RepoKind::Path;
    svc.update(0, edited).unwrap();
    assert_eq!(svc.get(0).unwrap().kind, RepoKind::Path);
}
