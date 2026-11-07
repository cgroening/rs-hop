use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratada::form::YES;

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn edit_form() -> RepoForm {
    let mut repo = Repo::new("/code/hop".into());
    repo.name = Some("hop".to_string());
    repo.include_in_backup = true;
    RepoForm::for_edit(&repo)
}

fn focus(form: &mut RepoForm, field: Field) {
    form.state.focus_on(field.key());
}

#[test]
fn letters_are_commands_until_e_starts_typing() {
    let mut form = edit_form();
    focus(&mut form, Field::Name);
    form.handle_key(key(KeyCode::Char('x')));
    assert_eq!(form.state.value(NAME), "hop");
    form.handle_key(key(KeyCode::Char('e')));
    form.handle_key(key(KeyCode::Char('x')));
    form.handle_key(key(KeyCode::Esc));
    assert_eq!(form.state.value(NAME), "hopx");
    assert!(!form.state.is_editing());
}

#[test]
fn enter_on_the_path_opens_the_picker_and_e_types_it() {
    let mut form = edit_form();
    assert!(matches!(
        form.handle_key(key(KeyCode::Enter)),
        FormResult::PickPath
    ));
    form.handle_key(key(KeyCode::Char('e')));
    assert!(form.state.is_editing());
}

#[test]
fn enter_on_the_section_opens_the_section_picker() {
    let mut form = edit_form();
    focus(&mut form, Field::Section);
    assert!(matches!(
        form.handle_key(key(KeyCode::Enter)),
        FormResult::PickSection
    ));
}

#[test]
fn a_save_keeps_the_form_and_a_second_press_closes_it() {
    let mut form = edit_form();
    assert!(matches!(
        form.handle_key(key(KeyCode::Char('s'))),
        FormResult::Save(_)
    ));
    form.mark_saved();
    assert!(matches!(
        form.handle_key(key(KeyCode::Char('s'))),
        FormResult::Close
    ));
}

#[test]
fn esc_on_a_changed_form_asks_and_each_answer_does_its_part() {
    let mut form = edit_form();
    focus(&mut form, Field::Fav);
    form.handle_key(key(KeyCode::Char(' ')));
    assert!(form.is_dirty());
    assert!(matches!(
        form.handle_key(key(KeyCode::Esc)),
        FormResult::Pending
    ));
    form.handle_key(key(KeyCode::Char('b')));
    form.handle_key(key(KeyCode::Esc));
    assert!(matches!(
        form.handle_key(key(KeyCode::Char('y'))),
        FormResult::Save(_)
    ));
    assert!(form.closes_after_save());

    let mut form = edit_form();
    focus(&mut form, Field::Fav);
    form.handle_key(key(KeyCode::Char(' ')));
    form.handle_key(key(KeyCode::Esc));
    assert!(matches!(
        form.handle_key(key(KeyCode::Char('n'))),
        FormResult::Cancel
    ));
}

#[test]
fn ctrl_o_puts_the_focused_field_back() {
    let mut form = edit_form();
    focus(&mut form, Field::Kind);
    form.handle_key(key(KeyCode::Right));
    assert_eq!(form.kind(), RepoKind::Path);
    form.handle_key(ctrl('o'));
    assert_eq!(form.kind(), RepoKind::Git);
    assert!(!form.is_dirty());
}

#[test]
fn an_unchanged_form_closes_on_esc() {
    let mut form = edit_form();
    assert!(matches!(
        form.handle_key(key(KeyCode::Esc)),
        FormResult::Cancel
    ));
}

#[test]
fn a_field_problem_takes_the_focus() {
    let mut form = edit_form();
    form.set_error(Field::Slug, "taken");
    assert_eq!(form.state.focus_key(), SLUG);
}

/// Draws `form` on an 80x24 screen; trailing blanks trimmed per row.
fn drawn(form: &RepoForm) -> String {
    use ratatui::{Terminal, backend::TestBackend};
    let skin = crate::config::Config::default().skin();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| form.render(frame, frame.area(), &skin))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..24)
        .map(|y| {
            let row: String =
                (0..80).map(|x| buffer[(x, y)].symbol()).collect();
            row.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The whole form: frame, label column, picker fields, stepper, the
/// change marker and the two hint rows.
#[test]
fn the_form_renders_as_a_whole() {
    let mut form = edit_form();
    focus(&mut form, Field::Fav);
    form.handle_key(key(KeyCode::Char(' ')));
    insta::assert_snapshot!(drawn(&form));
}

/// A field problem stands in red under its field.
#[test]
fn a_field_problem_renders_under_its_field() {
    let mut form = edit_form();
    form.set_error(Field::Path, "path must not be empty");
    let text = drawn(&form);
    let rows: Vec<&str> = text.lines().collect();
    let path = rows.iter().position(|row| row.contains("Path")).unwrap();
    assert!(rows[path + 1].contains("path must not be empty"), "{text}");
}

#[test]
fn basename_takes_final_component() {
    assert_eq!(basename("/code/hop").as_deref(), Some("hop"));
    assert_eq!(basename("/code/hop/").as_deref(), Some("hop"));
    assert_eq!(basename("  ").as_deref(), None);
}

#[test]
fn the_name_fills_from_the_path_on_arrival() {
    let mut form = RepoForm::for_add("/code/hop", RepoKind::Git);
    form.handle_key(key(KeyCode::Tab));
    assert_eq!(form.state.value(NAME), "hop");
}

#[test]
fn a_picked_path_fills_a_blank_name() {
    let mut form = RepoForm::for_add("", RepoKind::Git);
    form.set_path("/code/hop");
    assert_eq!(form.path_value(), "/code/hop");
    assert_eq!(form.state.value(NAME), "hop");
}

#[test]
fn the_single_form_keeps_the_field_order() {
    let form = RepoForm::for_add("/p", RepoKind::Path);
    let keys: Vec<&str> = form
        .state
        .fields()
        .iter()
        .map(|field| field.key.as_str())
        .collect();
    assert_eq!(keys, [PATH, NAME, SLUG, SECTION, FAV, BACKUP, KIND]);
}

#[test]
fn the_kind_steps_and_keeps_the_section() {
    let mut repo = Repo::new("/p".into());
    repo.section = Some("Backend".to_string());
    let mut form = RepoForm::for_edit(&repo);
    focus(&mut form, Field::Kind);
    form.handle_key(key(KeyCode::Right));
    assert_eq!(form.kind(), RepoKind::Path);
    assert_eq!(form.section().as_deref(), Some("Backend"));
}

#[test]
fn for_add_seeds_backup_toggle_per_kind() {
    let git = RepoForm::for_add("/p", RepoKind::Git);
    assert!(git.draft().include_in_backup);
    let folder = RepoForm::for_add("/p", RepoKind::Path);
    assert!(!folder.draft().include_in_backup);
}

#[test]
fn the_draft_slugifies_and_drops_blanks() {
    let mut form = RepoForm::for_add("/p", RepoKind::Git);
    form.state.set_value(SLUG, "My Repo");
    let draft = form.draft();
    assert_eq!(draft.slug.as_deref(), Some("my-repo"));
    assert_eq!(draft.name, None);
    assert_eq!(draft.section, None);
}

#[test]
fn a_paste_lands_in_the_focused_text_field() {
    let mut form = RepoForm::for_add("", RepoKind::Git);
    form.paste("/code/hop");
    form.handle_key(key(KeyCode::Enter));
    assert_eq!(form.path_value(), "/code/hop");
}

#[test]
fn a_paste_on_a_non_text_field_changes_nothing() {
    let mut form = RepoForm::for_add("/p", RepoKind::Git);
    focus(&mut form, Field::Fav);
    form.paste("/code/hop");
    assert_eq!(form.path_value(), "/p");
    assert!(!form.is_dirty());
}

#[test]
fn set_section_sets_and_focuses_the_field() {
    let mut form = RepoForm::for_add("/p", RepoKind::Git);
    form.set_section(Some("Work".to_string()));
    assert_eq!(form.section().as_deref(), Some("Work"));
    assert_eq!(form.state.focus_key(), SECTION);
}

#[test]
fn bulk_form_shows_only_the_shared_fields() {
    let form =
        RepoForm::for_bulk(3, RepoKind::Git, Some(None), Some(true), None);
    let keys: Vec<&str> = form
        .state
        .fields()
        .iter()
        .map(|field| field.key.as_str())
        .collect();
    assert_eq!(keys, [SECTION, FAV, BACKUP, KIND]);
}

#[test]
fn bulk_draft_only_carries_changed_fields() {
    let mut form = RepoForm::for_bulk(
        2,
        RepoKind::Git,
        Some(Some("A".to_string())),
        Some(false),
        None,
    );
    let draft = form.bulk_draft();
    assert!(draft.section.is_none());
    assert!(draft.fav.is_none());
    assert!(draft.include_in_backup.is_none());
    assert!(draft.kind.is_none());

    focus(&mut form, Field::Fav);
    form.handle_key(key(KeyCode::Char(' ')));
    form.set_section(Some("B".to_string()));
    let draft = form.bulk_draft();
    assert_eq!(draft.fav, Some(true));
    assert_eq!(draft.section, Some(Some("B".to_string())));
    assert!(draft.include_in_backup.is_none());
    assert!(draft.kind.is_none());
}

#[test]
fn a_mixed_bulk_checkbox_toggles_to_on_first() {
    let mut form = RepoForm::for_bulk(2, RepoKind::Git, None, None, None);
    assert!(drawn(&form).contains("[-]"));
    focus(&mut form, Field::Fav);
    form.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(form.state.value(FAV), YES);
}

#[test]
fn a_mixed_section_picked_as_ungrouped_is_still_applied() {
    let mut form = RepoForm::for_bulk(2, RepoKind::Git, None, Some(true), None);
    assert!(drawn(&form).contains("(mixed)"));
    form.set_section(None);
    assert!(drawn(&form).contains(UNGROUPED));
    assert_eq!(form.bulk_draft().section, Some(None));
}
