//! Which scripts of the list are selected, by plain, Ctrl and Shift clicks.

use std::collections::BTreeSet;

#[derive(Debug, Default)]
pub struct Selection {
    /// The selected ids, or `None` while only the open script is.
    picked: Option<BTreeSet<String>>,
    /// Where a Shift+click's range starts. With none, at the open script.
    anchor: Option<String>,
}

impl Selection {
    /// Back to only the open script.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn contains(&self, id: &str, open: Option<&str>) -> bool {
        match &self.picked {
            Some(picked) => picked.contains(id),
            None => open == Some(id),
        }
    }

    /// What a drag or a right-click menu on `id`'s row acts on: every selected script if it
    /// is one of them, or else just that one.
    pub fn acted_on(&self, id: &str, open: Option<&str>) -> Vec<String> {
        match &self.picked {
            Some(picked) if self.contains(id, open) => picked.iter().cloned().collect(),
            _ => vec![id.to_owned()],
        }
    }

    /// Selects exactly `ids`, with a later range starting at `anchor`.
    pub fn select(&mut self, ids: &[String], anchor: &str) {
        self.picked = Some(ids.iter().cloned().collect());
        self.anchor = Some(anchor.to_owned());
    }

    /// A click on `id`'s row. `listed` is the rows in the order shown. True if the click
    /// opens the script, as a plain one does. Ctrl adds or takes out the one script. Shift
    /// selects from the anchor to it, and with Ctrl adds that range.
    pub fn click(&mut self, id: &str, ctrl: bool, shift: bool, listed: &[String], open: Option<&str>) -> bool {
        if !ctrl && !shift {
            self.picked = None;
            self.anchor = Some(id.to_owned());
            return true;
        }
        let mut picked = self.picked.take().unwrap_or_else(|| open.into_iter().map(str::to_owned).collect());
        if shift {
            let place = |id: &str| listed.iter().position(|other| other == id);
            let anchor = self.anchor.as_deref().or(open).filter(|anchor| place(anchor).is_some()).unwrap_or(id).to_owned();
            if let (Some(from), Some(to)) = (place(&anchor), place(id)) {
                if !ctrl {
                    picked.clear();
                }
                picked.extend(listed[from.min(to)..=from.max(to)].iter().cloned());
            }
            self.anchor = Some(anchor);
        } else {
            if !picked.remove(id) {
                picked.insert(id.to_owned());
            }
            self.anchor = Some(id.to_owned());
        }
        self.picked = (!picked.is_empty()).then_some(picked);
        false
    }

    /// Takes out scripts whose rows are not shown, so that nothing unseen is ever moved.
    pub fn keep_listed(&mut self, listed: &[String]) {
        if let Some(picked) = &mut self.picked {
            picked.retain(|id| listed.contains(id));
            if picked.is_empty() {
                self.picked = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed() -> Vec<String> {
        ["a", "b", "c", "d", "e"].map(str::to_owned).to_vec()
    }

    fn selected(selection: &Selection, open: Option<&str>) -> String {
        listed().into_iter().filter(|id| selection.contains(id, open)).collect()
    }

    /// A click with the open script "b".
    fn click(selection: &mut Selection, id: &str, ctrl: bool, shift: bool) -> bool {
        selection.click(id, ctrl, shift, &listed(), Some("b"))
    }

    #[test]
    fn a_plain_click_opens_and_selects_only_that_script() {
        let mut selection = Selection::default();
        assert_eq!(selected(&selection, Some("b")), "b");
        click(&mut selection, "d", true, false);
        assert!(click(&mut selection, "c", false, false));
        // The app opens it, and it alone is selected.
        assert_eq!(selected(&selection, Some("c")), "c");
    }

    #[test]
    fn ctrl_click_adds_and_takes_out_one_script_without_opening_it() {
        let mut selection = Selection::default();
        assert!(!click(&mut selection, "d", true, false));
        assert_eq!(selected(&selection, Some("b")), "bd");
        click(&mut selection, "a", true, false);
        click(&mut selection, "b", true, false);
        assert_eq!(selected(&selection, Some("b")), "ad", "the open script can be taken out");
        click(&mut selection, "a", true, false);
        click(&mut selection, "d", true, false);
        assert_eq!(selected(&selection, Some("b")), "b", "with none left, the open script");
    }

    #[test]
    fn shift_click_selects_from_the_anchor_either_way() {
        let mut selection = Selection::default();
        // With no click yet, from the open script.
        assert!(!click(&mut selection, "d", false, true));
        assert_eq!(selected(&selection, Some("b")), "bcd");
        click(&mut selection, "a", false, true);
        assert_eq!(selected(&selection, Some("b")), "ab", "the anchor stays");
        // A Ctrl+click moves the anchor, and Ctrl+Shift adds the range.
        click(&mut selection, "d", true, false);
        click(&mut selection, "e", true, true);
        assert_eq!(selected(&selection, Some("b")), "abde");
        click(&mut selection, "c", false, true);
        assert_eq!(selected(&selection, Some("b")), "cd");
    }

    #[test]
    fn a_shift_click_with_the_anchor_unlisted_starts_over_there() {
        let mut selection = Selection::default();
        selection.click("c", false, true, &listed(), Some("hidden"));
        assert_eq!(selected(&selection, Some("hidden")), "c");
    }

    #[test]
    fn a_row_in_the_selection_acts_for_all_of_it() {
        let mut selection = Selection::default();
        assert_eq!(selection.acted_on("b", Some("b")), ["b"]);
        assert_eq!(selection.acted_on("c", Some("b")), ["c"]);
        click(&mut selection, "d", true, false);
        assert_eq!(selection.acted_on("d", Some("b")), ["b", "d"]);
        assert_eq!(selection.acted_on("c", Some("b")), ["c"]);
        selection.select(&["c".to_owned()], "c");
        assert_eq!(selected(&selection, Some("b")), "c");
    }

    #[test]
    fn unlisted_scripts_leave_the_selection() {
        let mut selection = Selection::default();
        click(&mut selection, "d", true, false);
        selection.keep_listed(&["a".to_owned(), "d".to_owned()]);
        assert_eq!(selected(&selection, Some("b")), "d");
        selection.keep_listed(&["a".to_owned()]);
        assert_eq!(selected(&selection, Some("b")), "b");
    }
}
