//! The Scripts panel: search, sort and the scripts grouped by folder.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use prompter_core::store::{Library, ScriptMeta};
use ui_kit::tokens::{color, space, typography};

use crate::workspace::Workspace;

/// The group for scripts in no folder.
const UNFILED: &str = "Unfiled";
/// A list row's inset at each end and the least room between its title and time.
const ROW_INSET: f32 = space::SCALE_4;
const ROW_GAP: f32 = space::SCALE_8;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Name,
    LastEdited,
}

/// What a click in the panel asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Open(String),
    Rename(String),
    Duplicate(String),
    Move(String, Option<String>),
    Delete(String),
    RenameFolder(String),
    DeleteFolder(String),
}

#[derive(Default)]
pub struct ScriptsPanel {
    search: String,
    sort: Sort,
    /// Lowercased text for the search, by script id, with the edit time it was read at.
    bodies: HashMap<String, (DateTime<Utc>, String)>,
}

impl ScriptsPanel {
    /// Draws the panel under its header. `focus_search` gives the search field the keyboard.
    pub fn show(&mut self, ui: &mut egui::Ui, workspace: &Workspace, focus_search: bool) -> Option<Action> {
        ui.add_space(space::SCALE_8);
        let search = ui_kit::search_field(ui, &mut self.search, "Search scripts");
        if focus_search {
            search.request_focus();
        }
        ui.add_space(space::SCALE_8);
        let mut sort = usize::from(self.sort == Sort::LastEdited);
        ui_kit::segmented_filled(ui, "script-sort", &mut sort, &["Name", "Last edited"], &[]);
        self.sort = if sort == 0 { Sort::Name } else { Sort::LastEdited };
        ui.add_space(space::SCALE_8);

        let query = self.search.trim().to_lowercase();
        let library = workspace.store().library();
        let found: Vec<&ScriptMeta> = library.scripts.iter().filter(|script| query.is_empty() || self.is_match(workspace, script, &query)).collect();
        let searching = !query.is_empty();
        let groups = groups(&library.folders, found, self.sort, searching);
        if groups.is_empty() {
            crate::app::empty_state(ui, if searching { "No matching scripts" } else { "No scripts yet" });
            return None;
        }

        let open = workspace.open_script().map(|open| open.id.as_str());
        let now = Local::now();
        let mut action = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for group in &groups {
                let name = group.folder.unwrap_or(UNFILED);
                let heading = egui::RichText::new(name).font(ui_kit::fonts::semibold(typography::CAPTION_SIZE)).color(color::TEXT_SECONDARY);
                // A search has its own open and closed state, so it shows every match without
                // opening the groups the user had closed.
                let header = egui::CollapsingHeader::new(heading).id_salt(("script-group", group.folder, searching)).default_open(true);
                let shown = header.show_unindented(ui, |ui| {
                    for script in &group.scripts {
                        let selected = open == Some(script.id.as_str());
                        ui.push_id(&script.id, |ui| row(ui, library, script, selected, &now, &mut action));
                    }
                });
                if let Some(folder) = group.folder {
                    let empty = !library.scripts.iter().any(|script| script.folder.as_deref() == Some(folder));
                    shown.header_response.context_menu(|ui| {
                        if ui.button("Rename").clicked() {
                            action = Some(Action::RenameFolder(folder.to_owned()));
                            ui.close();
                        }
                        if ui.add_enabled(empty, egui::Button::new("Delete")).on_disabled_hover_text("Only an empty folder can be deleted").clicked() {
                            action = Some(Action::DeleteFolder(folder.to_owned()));
                            ui.close();
                        }
                    });
                }
            }
        });
        action
    }

    /// True if the title or text holds `query`, which is already lowercase.
    fn is_match(&mut self, workspace: &Workspace, script: &ScriptMeta, query: &str) -> bool {
        // The open script may have text that is not saved yet.
        if let Some(open) = workspace.open_script().filter(|open| open.id == script.id) {
            return matches(&script.title, &open.text.to_lowercase(), query);
        }
        if self.bodies.get(&script.id).is_none_or(|(edited, _)| *edited != script.edited) {
            match workspace.store().load(&script.id) {
                Ok(text) => {
                    self.bodies.insert(script.id.clone(), (script.edited, text.to_lowercase()));
                }
                // The title can still match. Opening the script reports the error.
                Err(_) => {
                    self.bodies.remove(&script.id);
                }
            }
        }
        let body = self.bodies.get(&script.id).map_or("", |(_, body)| body.as_str());
        matches(&script.title, body, query)
    }
}

/// One script's row and its right-click menu.
fn row(ui: &mut egui::Ui, library: &Library, script: &ScriptMeta, selected: bool, now: &DateTime<Local>, action: &mut Option<Action>) {
    let time = edited_label(&script.edited.with_timezone(&Local), now);
    let time_width = ui.painter().layout_no_wrap(time.clone(), ui_kit::fonts::mono(typography::VALUE_SIZE), color::TEXT_TERTIARY).size().x;
    let room = ui.available_width() - time_width - 2.0 * ROW_INSET - ROW_GAP;
    let title = elided(ui, &script.title, room);
    let cut = title != script.title;
    let response = ui_kit::list_row(ui, selected, None, &title, Some(&time));
    let response = if cut { response.on_hover_text(&script.title) } else { response };
    if response.clicked() {
        *action = Some(Action::Open(script.id.clone()));
    }
    response.context_menu(|ui| {
        let mut pick = |ui: &mut egui::Ui, picked| {
            *action = Some(picked);
            ui.close();
        };
        if ui.button("Rename").clicked() {
            pick(ui, Action::Rename(script.id.clone()));
        }
        if ui.button("Duplicate").clicked() {
            pick(ui, Action::Duplicate(script.id.clone()));
        }
        ui.menu_button("Move to folder", |ui| {
            let targets = std::iter::once(None).chain(library.folders.iter().map(|folder| Some(folder.as_str())));
            for target in targets {
                let current = script.folder.as_deref() == target;
                if ui.add_enabled(!current, egui::Button::new(target.unwrap_or(UNFILED))).clicked() {
                    pick(ui, Action::Move(script.id.clone(), target.map(str::to_owned)));
                }
            }
        });
        if ui.button("Delete").clicked() {
            pick(ui, Action::Delete(script.id.clone()));
        }
    });
}

/// `title`, cut with an ellipsis if it is wider than `room` in a list row.
fn elided(ui: &egui::Ui, title: &str, room: f32) -> String {
    let font = ui_kit::fonts::medium(typography::BODY_SIZE);
    let mut job = egui::text::LayoutJob::simple_singleline(title.to_owned(), font.clone(), color::TEXT_PRIMARY);
    job.wrap = egui::text::TextWrapping::truncate_at_width(room.max(0.0));
    let galley = ui.painter().layout_job(job);
    if !galley.elided {
        return title.to_owned();
    }
    galley.rows.iter().flat_map(|row| row.glyphs.iter().map(|glyph| glyph.chr)).collect()
}

fn matches(title: &str, body_lower: &str, query_lower: &str) -> bool {
    title.to_lowercase().contains(query_lower) || body_lower.contains(query_lower)
}

/// A folder and the scripts to list under it. `None` is the Unfiled group.
struct Group<'a> {
    folder: Option<&'a str>,
    scripts: Vec<&'a ScriptMeta>,
}

/// The folders by name, then Unfiled. Unfiled is left out when it is empty, and so is every
/// empty folder during a search.
fn groups<'a>(folders: &'a [String], mut scripts: Vec<&'a ScriptMeta>, sort: Sort, searching: bool) -> Vec<Group<'a>> {
    match sort {
        Sort::Name => scripts.sort_by_cached_key(|script| script.title.to_lowercase()),
        Sort::LastEdited => scripts.sort_by_key(|script| std::cmp::Reverse(script.edited)),
    }
    let mut names: Vec<&str> = folders.iter().map(String::as_str).collect();
    names.sort_by_cached_key(|name| name.to_lowercase());
    let mut groups: Vec<Group<'a>> = names.into_iter().map(|name| Group { folder: Some(name), scripts: Vec::new() }).collect();
    let mut unfiled = Group { folder: None, scripts: Vec::new() };
    for script in scripts {
        // A hand-edited library may name a folder it does not list. Its script is unfiled.
        match groups.iter_mut().find(|group| group.folder == script.folder.as_deref()) {
            Some(group) => group.scripts.push(script),
            None => unfiled.scripts.push(script),
        }
    }
    if searching {
        groups.retain(|group| !group.scripts.is_empty());
    }
    if !unfiled.scripts.is_empty() {
        groups.push(unfiled);
    }
    groups
}

/// When a script was edited: the time today, the day this year, else the date.
fn edited_label<Z: TimeZone>(edited: &DateTime<Z>, now: &DateTime<Z>) -> String
where
    Z::Offset: std::fmt::Display,
{
    let format = if edited.date_naive() == now.date_naive() {
        "%H:%M"
    } else if edited.year() == now.year() {
        "%b %-d"
    } else {
        "%Y-%m-%d"
    };
    edited.format(format).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, hour, 5, 0).single().unwrap()
    }

    fn script(title: &str, folder: Option<&str>, day: u32) -> ScriptMeta {
        ScriptMeta {
            id: title.to_owned(),
            title: title.to_owned(),
            folder: folder.map(str::to_owned),
            created: at(day, 0),
            edited: at(day, 0),
            favorite: false,
            last_position: None,
            extra: Default::default(),
        }
    }

    fn listed<'a>(groups: &[Group<'a>]) -> Vec<(Option<&'a str>, Vec<&'a str>)> {
        groups.iter().map(|group| (group.folder, group.scripts.iter().map(|script| script.title.as_str()).collect())).collect()
    }

    #[test]
    fn the_search_reads_titles_and_text_without_case() {
        assert!(matches("Intro Take", "", "take"));
        assert!(matches("Intro", "welcome back", "come ba"));
        assert!(!matches("Intro", "welcome back", "outro"));
    }

    #[test]
    fn scripts_are_grouped_by_folder_and_sorted() {
        let folders = ["shorts".to_owned(), "Ads".to_owned()];
        let scripts = [script("beta", None, 1), script("Alpha", None, 2), script("c", Some("shorts"), 3), script("lost", Some("Gone"), 4)];
        let by_name = groups(&folders, scripts.iter().collect(), Sort::Name, false);
        assert_eq!(listed(&by_name), [(Some("Ads"), vec![]), (Some("shorts"), vec!["c"]), (None, vec!["Alpha", "beta", "lost"])]);
        let by_edit = groups(&folders, scripts.iter().collect(), Sort::LastEdited, true);
        assert_eq!(listed(&by_edit), [(Some("shorts"), vec!["c"]), (None, vec!["lost", "Alpha", "beta"])]);
    }

    #[test]
    fn a_long_title_is_cut_with_an_ellipsis() {
        let ctx = egui::Context::default();
        ctx.set_fonts(ui_kit::fonts::definitions());
        let long = "A title far too long to share a row with its edited time";
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(elided(ui, "Short", 200.0), "Short");
            let cut = elided(ui, long, 100.0);
            assert!(cut.ends_with('\u{2026}'), "{cut}");
            assert!(long.starts_with(cut.trim_end_matches('\u{2026}')), "{cut}");
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_edited_label_gets_longer_with_age() {
        let now = at(25, 18);
        assert_eq!(edited_label(&at(25, 9), &now), "09:05");
        assert_eq!(edited_label(&at(3, 9), &now), "Nov 3");
        let last_year = Utc.with_ymd_and_hms(2025, 11, 25, 9, 0, 0).single().unwrap();
        assert_eq!(edited_label(&last_year, &now), "2025-11-25");
    }
}
