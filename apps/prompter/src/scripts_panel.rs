//! The Scripts panel: search, sort, favorites and the scripts grouped by folder. Scripts drag
//! onto folders, and folders drag into a new order.

use std::collections::HashMap;

use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use prompter_core::store::{self, Library, ScriptMeta, Sort};
use ui_kit::tokens::{color, radius, size, space, typography};

use crate::drag::{self, Drag, GroupArea, Payload};
use crate::selection::Selection;
use crate::workspace::Workspace;

/// The group for scripts in no folder.
const UNFILED: &str = "Unfiled";
/// A list row's inset at each end and the least room between its title and time.
const ROW_INSET: f32 = space::SCALE_4;
const ROW_GAP: f32 = space::SCALE_8;
/// The room before a title for the star: the 8px dot of `ui_kit::list_row` and the gap after
/// it.
const STAR_ROOM: f32 = 8.0 + ROW_INSET;
/// Where the dragged row's ghost sits from the pointer: clear of what is under the pointer.
const GHOST_OFFSET: egui::Vec2 = egui::vec2(space::SCALE_12, space::SCALE_8);
/// The widest title in the ghost.
const GHOST_TITLE: f32 = 180.0;
const GHOST_OPACITY: f32 = 0.92;
/// The width of the line showing where a dragged folder would go.
const INSERTION_STROKE: f32 = 2.0;

/// What a click in the panel asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Open(String),
    Rename(String),
    Duplicate(String),
    /// Moves scripts into a folder, or out of all folders with `None`.
    Move(Vec<String>, Option<String>),
    /// Puts a folder at an index of the folder list, counted without it.
    MoveFolder(String, usize),
    SetFavorite(String, bool),
    SetSort(Sort),
    SetCollapsed(String, bool),
    Delete(String),
    RenameFolder(String),
    DeleteFolder(String),
}

#[derive(Default)]
pub struct ScriptsPanel {
    search: String,
    /// The Favorites filter chip in the panel's header. Not remembered across launches.
    pub favorites_only: bool,
    /// Lowercased text for the search, by script id, with the edit time it was read at.
    bodies: HashMap<String, (DateTime<Utc>, String)>,
    /// Where the last frame showed each row that files can be dropped on to import them into
    /// a folder: the folders' own rows and their scripts'.
    folder_rows: Vec<(egui::Rect, String)>,
    selection: Selection,
    /// The open script as of the last frame. Opening another one ends a multiple selection.
    open_seen: Option<String>,
    drag: Drag,
    /// The cursor a drag asks for, as of the last frame.
    drag_cursor: Option<egui::CursorIcon>,
    /// The search field, once drawn.
    #[cfg(test)]
    pub search_id: Option<egui::Id>,
}

impl ScriptsPanel {
    /// The folder that files dropped at `at` go into, if the row there is a folder's or one
    /// of its scripts', with where that row is. As of the last [`ScriptsPanel::show`].
    pub fn folder_at(&self, at: egui::Pos2) -> Option<(egui::Rect, &str)> {
        self.folder_rows.iter().find(|(row, _)| row.contains(at)).map(|(row, folder)| (*row, folder.as_str()))
    }

    /// The cursor to show while dragging. Set after all widgets, so that none overrides it.
    pub fn drag_cursor(&self) -> Option<egui::CursorIcon> {
        self.drag_cursor
    }

    /// Calls off a drag, such as for a dialog that opens during it.
    pub fn cancel_drag(&mut self) {
        self.drag.cancel();
    }

    /// Draws the panel under its header. `focus_search` gives the search field the keyboard
    /// and selects its text.
    pub fn show(&mut self, ui: &mut egui::Ui, workspace: &Workspace, focus_search: bool) -> Option<Action> {
        self.folder_rows.clear();
        self.drag_cursor = None;
        let open = workspace.open_script().map(|open| open.id.as_str());
        if self.open_seen.as_deref() != open {
            self.open_seen = open.map(str::to_owned);
            self.selection.clear();
        }
        // Before the rows, which must know whether a release is a click.
        self.track_drag(ui);
        ui.add_space(space::SCALE_8);
        let search = ui_kit::search_field(ui, &mut self.search, "Search scripts");
        if focus_search {
            crate::find::focus_and_select(ui, search.id, &self.search);
        }
        #[cfg(test)]
        {
            self.search_id = Some(search.id);
        }
        ui.add_space(space::SCALE_8);
        let library = workspace.store().library();
        let mut action = None;
        let mut picked = usize::from(library.sort == Sort::Edited);
        ui_kit::segmented_filled(ui, "script-sort", &mut picked, &["Name", "Last edited"], &[]);
        let sort = if picked == 0 { Sort::Name } else { Sort::Edited };
        if sort != library.sort {
            action = Some(Action::SetSort(sort));
        }
        ui.add_space(space::SCALE_8);

        let query = self.search.trim().to_lowercase();
        let favorites_only = self.favorites_only;
        let wanted = library.scripts.iter().filter(|script| !favorites_only || script.favorite);
        let found: Vec<&ScriptMeta> = wanted.filter(|script| query.is_empty() || self.is_match(workspace, script, &query)).collect();
        let searching = !query.is_empty();
        let filtering = searching || favorites_only;
        let groups = groups(&library.folders, found, sort, filtering);
        if groups.is_empty() {
            let caption = if searching {
                "No matching scripts"
            } else if favorites_only {
                "No favorites yet"
            } else {
                "No scripts yet"
            };
            crate::app::empty_state(ui, caption);
            self.selection.clear();
            self.drag = Drag::Idle;
            return action;
        }

        let now = Local::now();
        let mut listed = Listed::default();
        let shown_list = egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for group in &groups {
                let name = group.folder.unwrap_or(UNFILED);
                let heading = egui::RichText::new(name).font(ui_kit::fonts::semibold(typography::CAPTION_SIZE)).color(color::TEXT_SECONDARY);
                // A search or filter has its own open and closed state, so it shows every
                // match without opening the groups the user had closed.
                let mut header = egui::CollapsingHeader::new(heading).id_salt(("script-group", group.folder, filtering)).default_open(true);
                // The library remembers which folders are closed. Unfiled is not a folder,
                // so egui keeps its state until the app exits.
                let remembered = group.folder.filter(|_| !filtering);
                let closed = remembered.is_some_and(|folder| library.collapsed_folders.iter().any(|other| other == folder));
                if remembered.is_some() {
                    header = header.open(Some(!closed));
                }
                let shown = header.show_unindented(ui, |ui| {
                    for script in &group.scripts {
                        let selected = self.selection.contains(&script.id, open);
                        let chosen = self.selection.acted_on(&script.id, open);
                        let (response, menu_asked) = ui.push_id(&script.id, |ui| row(ui, library, script, selected, &chosen, &now, &mut action)).inner;
                        listed.ids.push(script.id.clone());
                        if let Some(origin) = press_origin(ui, &response) {
                            let payload = Payload::Scripts { ids: chosen.clone(), title: script.title.clone() };
                            self.drag.press(response.id, payload, origin);
                        }
                        if self.is_clicked(ui, &response) {
                            listed.clicked = Some((script.id.clone(), ui.input(|i| i.modifiers)));
                        }
                        // The menu of a row outside the selection is for that row alone.
                        if menu_asked && !selected {
                            self.selection.select(&chosen, &script.id);
                        }
                        if let Some(folder) = group.folder {
                            self.add_drop_row(ui, response.rect, folder);
                        }
                    }
                });
                let heading = &shown.header_response;
                let body_bottom = shown.body_response.as_ref().map_or(heading.rect.bottom(), |body| body.rect.bottom());
                listed.areas.push(GroupArea {
                    folder: group.folder.map(str::to_owned),
                    header: egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), heading.rect.y_range()),
                    bottom: body_bottom.max(heading.rect.bottom()),
                });
                // Only folders drag, and only in the plain list: a search or filter hides some.
                if let Some(folder) = remembered {
                    if let Some(origin) = press_origin(ui, heading) {
                        self.drag.press(heading.id, Payload::Folder(folder.to_owned()), origin);
                    }
                    if self.is_clicked(ui, heading) {
                        action = Some(Action::SetCollapsed(folder.to_owned(), !closed));
                    }
                }
                if let Some(folder) = group.folder {
                    self.add_drop_row(ui, heading.rect, folder);
                    folder_menu(heading, library, folder, filtering, &mut action);
                }
            }
        });

        if let Some((id, modifiers)) = listed.clicked {
            if self.selection.click(&id, modifiers.command, modifiers.shift, &listed.ids, open) {
                action = Some(Action::Open(id));
            }
            // The rows drawn before this one show the old selection.
            ui.ctx().request_repaint();
        }
        self.selection.keep_listed(&listed.ids);
        self.finish_drag(ui, library, &listed.areas, shown_list.inner_rect).or(action)
    }

    /// Takes in this frame's pointer and Esc, ahead of the rows.
    fn track_drag(&mut self, ui: &egui::Ui) {
        let (pointer, left) = ui.input(|i| (i.pointer.latest_pos(), i.raw.events.contains(&egui::Event::WindowFocused(false))));
        if left {
            self.drag.cancel();
        }
        if self.drag.track(pointer)
            && let Some(Payload::Scripts { ids, .. }) = self.drag.dragged()
            && let [only] = ids.as_slice()
        {
            // Dragging a row selects it.
            self.selection.select(ids, only);
        }
        if self.drag.dragged().is_some() {
            // Taken, so that Esc does nothing else. Ctrl or Shift may still be held.
            let escaped = ui.input_mut(|i| {
                let held = i.modifiers;
                i.consume_key(held, egui::Key::Escape)
            });
            if escaped {
                self.drag.cancel();
            }
        }
    }

    /// True if a row or header was clicked, by the pointer or the keys.
    fn is_clicked(&self, ui: &egui::Ui, response: &egui::Response) -> bool {
        let released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
        !self.drag.takes_the_click() && (response.clicked() || self.drag.is_click(response.id, released))
    }

    /// Shows where a drag would land, and the ghost of what it carries. Returns what a drop
    /// this frame does. `list` is the part of the list in view.
    fn finish_drag(&mut self, ui: &egui::Ui, library: &Library, areas: &[GroupArea], list: egui::Rect) -> Option<Action> {
        let (pointer, down, released) =
            ui.input(|i| (i.pointer.latest_pos(), i.pointer.primary_down(), i.pointer.button_released(egui::PointerButton::Primary)));
        let mut on_drop = None;
        if let Some(payload) = self.drag.dragged() {
            let over_target = match payload {
                Payload::Scripts { ids, .. } => {
                    let area = pointer.and_then(|at| drag::script_target(areas, list, at));
                    let moving = area.map(|area| (area, not_in(library, ids, area.folder.as_deref())));
                    if let Some((area, moving)) = moving.filter(|(_, moving)| !moving.is_empty()) {
                        crate::app::mark_drop_target(ui, area.header.intersect(list));
                        on_drop = Some(Action::Move(moving, area.folder.clone()));
                    }
                    on_drop.is_some()
                }
                Payload::Folder(folder) => {
                    let from = library.folders.iter().position(|other| other == folder);
                    let gap = pointer.and_then(|at| drag::folder_gap(areas, list, at));
                    if let (Some(from), Some(gap)) = (from, gap) {
                        if let Some(y) = drag::gap_line(areas, gap, ui.spacing().item_spacing.y) {
                            insertion_line(ui, list, y);
                        }
                        // Its own place is a fine target, with nothing to do.
                        on_drop = drag::moved_to(from, gap).map(|to| Action::MoveFolder(folder.clone(), to));
                    }
                    from.is_some() && gap.is_some()
                }
            };
            self.drag_cursor = Some(if over_target { egui::CursorIcon::Grabbing } else { egui::CursorIcon::NotAllowed });
            if let Some(at) = pointer {
                ghost(ui, library, payload, at);
            }
        }
        let dropped = self.drag.end(down, released).is_some();
        if dropped {
            self.drag_cursor = None;
            // What is already drawn shows the drag.
            ui.ctx().request_repaint();
        }
        on_drop.filter(|_| dropped)
    }

    fn add_drop_row(&mut self, ui: &egui::Ui, shown: egui::Rect, folder: &str) {
        // A heading is only as wide as its text; the row is as wide as the list.
        let row = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), shown.y_range());
        self.folder_rows.push((row.intersect(ui.clip_rect()), folder.to_owned()));
    }

    /// True if the title or text holds `query`, which is already lowercase.
    fn is_match(&mut self, workspace: &Workspace, script: &ScriptMeta, query: &str) -> bool {
        // An open script may have text that is not saved yet.
        if let Some(open) = workspace.tab(&script.id) {
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

/// What the star and the row's menu call the change a click makes.
fn favorite_label(favorite: bool) -> &'static str {
    if favorite { "Remove from favorites" } else { "Add to favorites" }
}

/// What the list showed this frame.
#[derive(Default)]
struct Listed {
    /// The scripts with rows, in the order shown.
    ids: Vec<String>,
    /// The script row clicked, and the keys held.
    clicked: Option<(String, egui::Modifiers)>,
    areas: Vec<GroupArea>,
}

/// Where the primary button went down, if it did so on this widget in this frame.
fn press_origin(ui: &egui::Ui, response: &egui::Response) -> Option<egui::Pos2> {
    if !response.is_pointer_button_down_on() {
        return None;
    }
    ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary).then(|| i.pointer.press_origin()).flatten())
}

/// A right-click menu on a row or header, which Shift+F10 also opens for the one with the
/// keyboard. `contents` is told to give its first item the keyboard, as it is on the frame
/// the keys open the menu.
fn row_menu(response: &egui::Response, contents: impl FnOnce(&mut egui::Ui, bool)) {
    let ctx = &response.ctx;
    let id = egui::Popup::default_response_id(response);
    let by_keys = response.has_focus() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F10));
    if by_keys {
        egui::Popup::open_id(ctx, id);
    }
    let mut popup = egui::Popup::context_menu(response);
    // A menu the keys opened has no pointer place, so it goes under the row.
    if by_keys || (egui::Popup::is_id_open(ctx, id) && egui::Popup::position_of_id(ctx, id).is_none()) {
        popup = popup.at_position(response.rect.left_bottom());
    }
    popup.show(|ui| contents(ui, by_keys));
}

/// A folder header's menu. Moving is off during a search or filter, as dragging is.
fn folder_menu(heading: &egui::Response, library: &Library, folder: &str, filtering: bool, action: &mut Option<Action>) {
    let empty = !library.scripts.iter().any(|script| script.folder.as_deref() == Some(folder));
    let at = library.folders.iter().position(|other| other == folder);
    row_menu(heading, |ui, focus_first| {
        let mut pick = |ui: &mut egui::Ui, picked| {
            *action = Some(picked);
            ui.close();
        };
        let rename = ui.button("Rename");
        if focus_first {
            rename.request_focus();
        }
        if rename.clicked() {
            pick(ui, Action::RenameFolder(folder.to_owned()));
        }
        let up = at.and_then(|at| at.checked_sub(1));
        let down = at.map(|at| at + 1).filter(|&to| to < library.folders.len());
        for (label, to) in [("Move up", up), ("Move down", down)] {
            let to = to.filter(|_| !filtering);
            let mut button = ui.add_enabled(to.is_some(), egui::Button::new(label));
            if filtering {
                button = button.on_disabled_hover_text("Clear the search and the Favorites filter to reorder folders");
            }
            if button.clicked()
                && let Some(to) = to
            {
                pick(ui, Action::MoveFolder(folder.to_owned(), to));
            }
        }
        if ui.add_enabled(empty, egui::Button::new("Delete")).on_disabled_hover_text("Only an empty folder can be deleted").clicked() {
            pick(ui, Action::DeleteFolder(folder.to_owned()));
        }
    });
}

/// The scripts among `ids` that are not in `folder`.
fn not_in(library: &Library, ids: &[String], folder: Option<&str>) -> Vec<String> {
    let elsewhere = |id: &&String| library.scripts.iter().any(|script| script.id == **id && script.folder.as_deref() != folder);
    ids.iter().filter(elsewhere).cloned().collect()
}

/// The targets of "Move to folder". One that already holds all of `ids` is off. When the keys
/// open the submenu, the first target that is on gets the keyboard.
fn move_targets(ui: &mut egui::Ui, library: &Library, ids: &[String]) -> Option<Action> {
    let by_keys = ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space));
    let mut picked = None;
    let mut first = None;
    let mut has_keyboard = false;
    let targets = std::iter::once(None).chain(library.folders.iter().map(|folder| Some(folder.as_str())));
    for target in targets {
        let moving = not_in(library, ids, target);
        let on = !moving.is_empty();
        let button = ui.add_enabled(on, egui::Button::new(target.unwrap_or(UNFILED)));
        has_keyboard |= button.has_focus();
        if on {
            first = first.or(Some(button.id));
        }
        if button.clicked() {
            picked = Some(Action::Move(moving, target.map(str::to_owned)));
        }
    }
    // Only once all have seen this frame's keys, so that the one with the keyboard keeps its
    // press.
    if by_keys
        && !has_keyboard
        && picked.is_none()
        && let Some(first) = first
    {
        ui.memory_mut(|memory| memory.request_focus(first));
    }
    picked
}

/// The accent line showing where a dragged folder would go.
fn insertion_line(ui: &egui::Ui, list: egui::Rect, y: f32) {
    let accent = ui_kit::Theme::get(ui.ctx()).accent.accent;
    // The line ahead of the top group or after the bottom one may sit just past the list.
    let painter = ui.painter().with_clip_rect(list.expand(space::SCALE_4));
    painter.hline(list.x_range(), y, egui::Stroke::new(INSERTION_STROKE, accent));
}

/// A copy of the dragged row or header, beside the pointer. With several scripts, it says how
/// many.
fn ghost(ui: &egui::Ui, library: &Library, payload: &Payload, pointer: egui::Pos2) {
    let (title, count) = match payload {
        Payload::Scripts { ids, title } => (title, (ids.len() > 1).then(|| format!("{} scripts", ids.len()))),
        // The library may have lost the folder since the press.
        Payload::Folder(folder) if library.folders.contains(folder) => (folder, None),
        Payload::Folder(_) => return,
    };
    let mut painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("scripts-drag-ghost")));
    painter.set_opacity(GHOST_OPACITY);
    let title = painter.layout_no_wrap(elided(ui, title, GHOST_TITLE), ui_kit::fonts::medium(typography::BODY_SIZE), color::TEXT_PRIMARY);
    let count = count.map(|count| painter.layout_no_wrap(count, ui_kit::fonts::mono(typography::VALUE_SIZE), color::TEXT_TERTIARY));
    let count_width = count.as_ref().map_or(0.0, |count| ROW_GAP + count.size().x);
    let wanted = egui::vec2(title.size().x + count_width + 4.0 * ROW_INSET, size::LIST_ROW_HEIGHT);
    let rect = egui::Rect::from_min_size(pointer + GHOST_OFFSET, wanted);
    // Solid under the row's own fill.
    painter.rect_filled(rect, radius::SEGMENT, color::BG_GROUND);
    let outline = egui::Stroke::new(1.0, color::STROKE_HAIRLINE);
    painter.rect(rect, radius::SEGMENT, color::BG_ROW_SELECTED, outline, egui::StrokeKind::Inside);
    let mut x = rect.left() + 2.0 * ROW_INSET;
    for (galley, tint) in std::iter::once((title, color::TEXT_PRIMARY)).chain(count.map(|count| (count, color::TEXT_TERTIARY))) {
        let width = galley.size().x;
        painter.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, tint);
        x += width + ROW_GAP;
    }
}

/// One script's row, with its star and right-click menu. `chosen` is what the menu's move
/// acts on. Returns the row, whose clicks leave out the star's, and whether the pointer asked
/// for the menu.
fn row(
    ui: &mut egui::Ui,
    library: &Library,
    script: &ScriptMeta,
    selected: bool,
    chosen: &[String],
    now: &DateTime<Local>,
    action: &mut Option<Action>,
) -> (egui::Response, bool) {
    let time = edited_label(&script.edited.with_timezone(&Local), now);
    // The time is a caption. `ui_kit::list_row` sets its trailing text in monospace, so the
    // row draws the time itself.
    let time = ui.painter().layout_no_wrap(time, time_font(), color::TEXT_TERTIARY);
    let room = ui.available_width() - STAR_ROOM - time.size().x - 2.0 * ROW_INSET - ROW_GAP;
    let title = elided(ui, &script.title, room);
    let cut = title != script.title;
    // A place under the row, for its hover fill while the pointer is on the star.
    let under = ui.painter().add(egui::Shape::Noop);
    // A clear dot makes the room for the star.
    let response = ui_kit::list_row(ui, selected, Some(egui::Color32::TRANSPARENT), &title, None);
    let rect = response.rect;
    let at = egui::pos2(rect.right() - ROW_INSET - time.size().x, rect.center().y - time.size().y / 2.0);
    ui.painter().galley(at, time, color::TEXT_TERTIARY);
    let response = if cut { response.on_hover_text(&script.title) } else { response };

    let mut area = rect;
    area.set_width(ROW_INSET + STAR_ROOM);
    let label = favorite_label(script.favorite);
    let star = ui.interact(area, ui.id().with("favorite"), egui::Sense::click()).on_hover_text(label);
    star.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if star.hovered() && !selected {
        ui.painter().set(under, egui::Shape::rect_filled(rect, radius::SEGMENT, color::BG_GROUP));
    }
    let (font, tint) = if script.favorite {
        (ui_kit::fonts::icon_fill(typography::CAPTION_SIZE), ui_kit::Theme::get(ui.ctx()).accent.accent)
    } else {
        let tint = if star.hovered() { color::TEXT_PRIMARY } else { color::TEXT_TERTIARY };
        (ui_kit::fonts::icon_regular(typography::CAPTION_SIZE), tint)
    };
    let center = egui::pos2(rect.left() + ROW_INSET + 4.0, rect.center().y);
    ui.painter().text(center, egui::Align2::CENTER_CENTER, ui_kit::icons::regular::STAR, font, tint);
    if star.clicked() {
        *action = Some(Action::SetFavorite(script.id.clone(), !script.favorite));
    }

    let both = response.union(star);
    row_menu(&both, |ui, focus_first| {
        let mut pick = |ui: &mut egui::Ui, picked| {
            *action = Some(picked);
            ui.close();
        };
        let favorite = ui.button(favorite_label(script.favorite));
        if focus_first {
            favorite.request_focus();
        }
        if favorite.clicked() {
            pick(ui, Action::SetFavorite(script.id.clone(), !script.favorite));
        }
        if ui.button("Rename").clicked() {
            pick(ui, Action::Rename(script.id.clone()));
        }
        if ui.button("Duplicate").clicked() {
            pick(ui, Action::Duplicate(script.id.clone()));
        }
        let move_label = if chosen.len() > 1 { format!("Move {} scripts to folder", chosen.len()) } else { "Move to folder".to_owned() };
        ui.menu_button(move_label, |ui| {
            if let Some(moved) = move_targets(ui, library, chosen) {
                pick(ui, moved);
            }
        });
        if ui.button("Delete").clicked() {
            pick(ui, Action::Delete(script.id.clone()));
        }
    });
    let menu_asked = both.secondary_clicked();
    (response, menu_asked)
}

/// The ui-kit caption style (`type.caption`), in Inter Regular.
fn time_font() -> egui::FontId {
    egui::FontId::new(typography::CAPTION_SIZE, egui::FontFamily::Proportional)
}

/// `title`, cut with an ellipsis if it is wider than `room` in a list row or a tab.
pub(crate) fn elided(ui: &egui::Ui, title: &str, room: f32) -> String {
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

/// Unfiled, then the folders in the library's order, with favorites first in each. A search
/// or filter leaves out every empty group, and so does a library with no folders. Otherwise
/// all are listed, so that each can be dropped on.
fn groups<'a>(folders: &'a [String], mut scripts: Vec<&'a ScriptMeta>, sort: Sort, filtering: bool) -> Vec<Group<'a>> {
    store::sort_scripts(&mut scripts, sort);
    let names = std::iter::once(None).chain(folders.iter().map(|folder| Some(folder.as_str())));
    let mut groups: Vec<Group<'a>> = names.map(|folder| Group { folder, scripts: Vec::new() }).collect();
    for script in scripts {
        // A hand-edited library may name a folder it does not list. Its script is unfiled,
        // in the first group.
        let at = groups.iter().position(|group| group.folder == script.folder.as_deref()).unwrap_or(0);
        groups[at].scripts.push(script);
    }
    if filtering || folders.is_empty() {
        groups.retain(|group| !group.scripts.is_empty());
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
        // Unfiled comes first, and the folders keep the library's order.
        assert_eq!(listed(&by_name), [(None, vec!["Alpha", "beta", "lost"]), (Some("shorts"), vec!["c"]), (Some("Ads"), vec![])]);
        let by_edit = groups(&folders, scripts.iter().collect(), Sort::Edited, true);
        assert_eq!(listed(&by_edit), [(None, vec!["lost", "Alpha", "beta"]), (Some("shorts"), vec!["c"])]);
    }

    #[test]
    fn an_empty_unfiled_is_listed_only_beside_folders_in_the_plain_list() {
        let folders = ["Ads".to_owned()];
        let filed = [script("one", Some("Ads"), 1)];
        assert_eq!(listed(&groups(&folders, filed.iter().collect(), Sort::Name, false)), [(None, vec![]), (Some("Ads"), vec!["one"])]);
        assert_eq!(listed(&groups(&folders, filed.iter().collect(), Sort::Name, true)), [(Some("Ads"), vec!["one"])]);
        assert!(groups(&[], Vec::new(), Sort::Name, false).is_empty());
    }

    #[test]
    fn favorites_come_first_in_their_group_in_both_sorts() {
        let folders = ["Ads".to_owned()];
        let mut scripts = [
            script("alpha", None, 4),
            script("beta", None, 1),
            script("gamma", None, 3),
            script("delta", None, 2),
            script("one", Some("Ads"), 5),
            script("two", Some("Ads"), 1),
        ];
        for at in [1, 3, 5] {
            scripts[at].favorite = true;
        }
        let by_name = groups(&folders, scripts.iter().collect(), Sort::Name, false);
        assert_eq!(listed(&by_name), [(None, vec!["beta", "delta", "alpha", "gamma"]), (Some("Ads"), vec!["two", "one"])]);
        let by_edit = groups(&folders, scripts.iter().collect(), Sort::Edited, false);
        assert_eq!(listed(&by_edit), [(None, vec!["delta", "beta", "alpha", "gamma"]), (Some("Ads"), vec!["two", "one"])]);
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
