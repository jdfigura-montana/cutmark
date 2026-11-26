//! Dragging in the Scripts panel: when a press becomes a drag, and where a drop lands. No
//! drawing happens here.

use egui::{Pos2, Rect};

/// How far the pointer must move from the press to start a drag. Short of it, the press is a
/// click.
pub const DRAG_DISTANCE: f32 = 4.0;

/// What is being dragged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// One or more scripts, by id. `title` is the one under the pointer.
    Scripts { ids: Vec<String>, title: String },
    /// A folder's header.
    Folder(String),
}

/// One press of the primary button on a row or header, from the press to the release.
#[derive(Debug, Default)]
pub enum Drag {
    #[default]
    Idle,
    /// Pressed, and not yet moved far enough to be a drag.
    Pressed {
        widget: egui::Id,
        payload: Payload,
        origin: Pos2,
    },
    Dragging(Payload),
    /// Called off with the button still down. Its release does nothing.
    Cancelled,
}

impl Drag {
    pub fn press(&mut self, widget: egui::Id, payload: Payload, origin: Pos2) {
        *self = Self::Pressed { widget, payload, origin };
    }

    /// Takes in where the pointer is. True if this starts a drag.
    pub fn track(&mut self, pointer: Option<Pos2>) -> bool {
        let far = match (&*self, pointer) {
            (Self::Pressed { origin, .. }, Some(pointer)) => origin.distance(pointer) >= DRAG_DISTANCE,
            _ => false,
        };
        if far && let Self::Pressed { payload, .. } = std::mem::take(self) {
            *self = Self::Dragging(payload);
        }
        far
    }

    /// What is being dragged.
    pub fn dragged(&self) -> Option<&Payload> {
        match self {
            Self::Dragging(payload) => Some(payload),
            _ => None,
        }
    }

    /// Calls off a press or a drag.
    pub fn cancel(&mut self) {
        if !matches!(self, Self::Idle) {
            *self = Self::Cancelled;
        }
    }

    /// True if the button's release must not count as a click.
    pub fn takes_the_click(&self) -> bool {
        matches!(self, Self::Dragging(_) | Self::Cancelled)
    }

    /// True if `released` ends a press on `widget` that never became a drag. egui calls a
    /// long press no click, so this is asked as well as egui.
    pub fn is_click(&self, widget: egui::Id, released: bool) -> bool {
        released && matches!(self, Self::Pressed { widget: pressed, .. } if *pressed == widget)
    }

    /// Ends the press once the button is up. Returns what a drag dropped. A button that went
    /// up with no release, such as one let go over another program, drops nothing.
    pub fn end(&mut self, down: bool, released: bool) -> Option<Payload> {
        if !released && down {
            return None;
        }
        match std::mem::take(self) {
            Self::Dragging(payload) if released => Some(payload),
            _ => None,
        }
    }
}

/// Where the last frame showed one group of the list: a folder, or Unfiled with `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupArea {
    pub folder: Option<String>,
    /// The header, as wide as the list.
    pub header: Rect,
    /// The bottom of the group's last row, or of its header while closed.
    pub bottom: f32,
}

/// The group whose header dragged scripts are over. `list` is the part of the list in view.
pub fn script_target(areas: &[GroupArea], list: Rect, pointer: Pos2) -> Option<&GroupArea> {
    if !list.contains(pointer) {
        return None;
    }
    areas.iter().find(|area| area.header.y_range().contains(pointer.y))
}

/// The gap a dragged folder header is over, counted in folders above it: 0 is ahead of the
/// first folder. Each folder's header and rows put a drop ahead of it in their upper half and
/// after it in their lower. `None` outside the list and over Unfiled, which stays first.
pub fn folder_gap(areas: &[GroupArea], list: Rect, pointer: Pos2) -> Option<usize> {
    if !list.contains(pointer) || areas.iter().any(|area| area.folder.is_none() && pointer.y <= area.bottom) {
        return None;
    }
    let folders = areas.iter().filter(|area| area.folder.is_some());
    Some(folders.filter(|area| (area.header.top() + area.bottom) / 2.0 < pointer.y).count())
}

/// The height of the insertion line for a gap, with `spacing` between groups.
pub fn gap_line(areas: &[GroupArea], gap: usize, spacing: f32) -> Option<f32> {
    let mut folders = areas.iter().filter(|area| area.folder.is_some());
    match folders.clone().nth(gap) {
        Some(below) => Some(below.header.top() - spacing / 2.0),
        None => folders.next_back().map(|last| last.bottom + spacing / 2.0),
    }
}

/// The index the store's `move_folder` takes to put the folder at `from` into `gap`, which is
/// counted with the folder still in the list. `None` if that is where it already is.
pub fn moved_to(from: usize, gap: usize) -> Option<usize> {
    let to = if gap > from { gap - 1 } else { gap };
    (to != from).then_some(to)
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::*;

    fn scripts() -> Payload {
        Payload::Scripts { ids: vec!["a".to_owned()], title: "A".to_owned() }
    }

    fn pressed() -> Drag {
        let mut drag = Drag::default();
        drag.press(egui::Id::new("row"), scripts(), pos2(10.0, 10.0));
        drag
    }

    #[test]
    fn a_press_becomes_a_drag_at_four_px() {
        let mut drag = pressed();
        assert!(!drag.track(Some(pos2(13.9, 10.0))));
        assert!(!drag.track(Some(pos2(12.0, 13.0))), "3.6 px on the diagonal");
        assert!(!drag.takes_the_click());
        assert!(drag.is_click(egui::Id::new("row"), true));
        assert!(!drag.is_click(egui::Id::new("row"), false));
        assert!(!drag.is_click(egui::Id::new("other"), true));

        assert!(drag.track(Some(pos2(10.0, 14.0))));
        assert_eq!(drag.dragged(), Some(&scripts()));
        assert!(drag.takes_the_click() && !drag.is_click(egui::Id::new("row"), true));
        // Back at the press, it is still a drag, and it started only once.
        assert!(!drag.track(Some(pos2(10.0, 10.0))));
        assert_eq!(drag.end(true, false), None);
        assert_eq!(drag.end(false, true), Some(scripts()));
        assert!(matches!(drag, Drag::Idle));
    }

    #[test]
    fn a_cancelled_drag_drops_nothing_and_takes_the_click() {
        let mut drag = pressed();
        drag.track(Some(pos2(30.0, 10.0)));
        drag.cancel();
        assert_eq!(drag.dragged(), None);
        // Moving on with the button down does not start it again.
        assert!(!drag.track(Some(pos2(90.0, 10.0))));
        assert_eq!(drag.end(true, false), None);
        assert!(drag.takes_the_click());
        assert_eq!(drag.end(false, true), None);
        assert!(matches!(drag, Drag::Idle) && !drag.takes_the_click());
        drag.cancel();
        assert!(matches!(drag, Drag::Idle));
    }

    #[test]
    fn a_button_that_goes_up_unseen_drops_nothing() {
        let mut drag = pressed();
        drag.track(Some(pos2(30.0, 10.0)));
        assert_eq!(drag.end(false, false), None);
        assert!(matches!(drag, Drag::Idle));
    }

    /// Unfiled with two rows, then folders A, B and C. A is open and B and C are closed.
    fn areas() -> Vec<GroupArea> {
        let area = |folder: Option<&str>, top: f32, bottom: f32| GroupArea {
            folder: folder.map(str::to_owned),
            header: Rect::from_x_y_ranges(0.0..=200.0, top..=top + 20.0),
            bottom,
        };
        vec![area(None, 100.0, 180.0), area(Some("A"), 184.0, 264.0), area(Some("B"), 268.0, 288.0), area(Some("C"), 292.0, 312.0)]
    }

    fn list() -> Rect {
        Rect::from_x_y_ranges(0.0..=200.0, 100.0..=400.0)
    }

    #[test]
    fn scripts_drop_only_on_a_header_in_view() {
        let areas = areas();
        let at = |x: f32, y: f32| script_target(&areas, list(), pos2(x, y)).map(|area| area.folder.as_deref());
        assert_eq!(at(10.0, 110.0), Some(None));
        assert_eq!(at(199.0, 190.0), Some(Some("A")));
        assert_eq!(at(10.0, 280.0), Some(Some("B")));
        // A script's row, the space between groups, past the list's side and below it.
        assert_eq!(at(10.0, 230.0), None);
        assert_eq!(at(10.0, 266.0), None);
        assert_eq!(at(201.0, 190.0), None);
        assert_eq!(at(10.0, 350.0), None);
        // A header scrolled out of view takes nothing.
        let scrolled = Rect::from_x_y_ranges(0.0..=200.0, 200.0..=400.0);
        assert_eq!(script_target(&areas, scrolled, pos2(10.0, 190.0)), None);
    }

    #[test]
    fn a_folder_drops_into_the_gap_nearest_the_pointer_but_never_ahead_of_unfiled() {
        let areas = areas();
        let at = |y: f32| folder_gap(&areas, list(), pos2(10.0, y));
        // Unfiled's header and rows.
        assert_eq!((at(110.0), at(180.0)), (None, None));
        // A is 184 to 264, rows and all.
        assert_eq!((at(182.0), at(223.0), at(225.0)), (Some(0), Some(0), Some(1)));
        assert_eq!((at(277.0), at(279.0), at(303.0), at(390.0)), (Some(1), Some(2), Some(3), Some(3)));
        assert_eq!(folder_gap(&areas, list(), pos2(250.0, 250.0)), None);

        assert_eq!(gap_line(&areas, 0, 4.0), Some(182.0));
        assert_eq!(gap_line(&areas, 2, 4.0), Some(290.0));
        assert_eq!(gap_line(&areas, 3, 4.0), Some(314.0));
        assert_eq!(gap_line(&areas[..1], 0, 4.0), None);
    }

    #[test]
    fn the_reorder_index_counts_without_the_moved_folder() {
        let moved = |from: usize, gap: usize| {
            let mut folders = vec!["A", "B", "C", "D"];
            if let Some(to) = moved_to(from, gap) {
                // What the store does.
                let folder = folders.remove(from);
                folders.insert(to, folder);
            }
            folders.concat()
        };
        assert_eq!([0, 1, 2, 3, 4].map(|gap| moved(1, gap)), ["BACD", "ABCD", "ABCD", "ACBD", "ACDB"]);
        assert_eq!((moved(0, 4), moved(3, 0)), ("BCDA".to_owned(), "DABC".to_owned()));
        assert_eq!((moved_to(1, 1), moved_to(1, 2)), (None, None));
    }
}
