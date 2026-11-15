//! editor::ops::marker: add, move and delete markers.

use uuid::Uuid;

use crate::editor::ops::OpError;
use crate::model::marker::Marker;
use crate::model::project::Project;

/// Returns `true` when a marker already sits at the given position.
fn occupied(project: &Project, pos: u64) -> bool {
    project.markers.iter().any(|marker| marker.position == pos)
}

/// Sorts the markers by position, keeping the sorted-by-position invariant.
fn sort_markers(project: &mut Project) {
    project.markers.sort_by_key(|marker| marker.position);
}

/// Adds a new marker at `pos` (error when one already sits there).
///
/// The marker list is kept sorted by position.
pub fn add_marker(project: &mut Project, pos: u64) -> Result<(), OpError> {
    if occupied(project, pos) {
        return Err(OpError::Invalid(format!("marker already at position {pos}")));
    }
    project.markers.push(Marker { id: Uuid::new_v4(), position: pos });
    sort_markers(project);
    Ok(())
}

/// Moves the marker with the given id to `pos` (error when one already sits there).
///
/// The marker list is re-sorted by position after the move.
pub fn move_marker(project: &mut Project, id: Uuid, pos: u64) -> Result<(), OpError> {
    if occupied(project, pos) {
        return Err(OpError::Invalid(format!("marker already at position {pos}")));
    }
    let marker = project.markers.iter_mut().find(|marker| marker.id == id).ok_or(OpError::NotFound(id))?;
    marker.position = pos;
    sort_markers(project);
    Ok(())
}

/// Deletes the listed markers. Unknown ids return [`OpError::NotFound`]
/// and nothing is changed.
pub fn delete_markers(project: &mut Project, ids: &[Uuid]) -> Result<(), OpError> {
    for id in ids {
        if !project.markers.iter().any(|marker| marker.id == *id) {
            return Err(OpError::NotFound(*id));
        }
    }
    project.markers.retain(|marker| !ids.contains(&marker.id));
    Ok(())
}

/// Position of the last marker strictly before `pos`, if any.
pub fn prev_marker(project: &Project, pos: u64) -> Option<u64> {
    project.markers.iter().map(|marker| marker.position).filter(|&marker_pos| marker_pos < pos).max()
}

/// Position of the first marker strictly after `pos`, if any.
pub fn next_marker(project: &Project, pos: u64) -> Option<u64> {
    project.markers.iter().map(|marker| marker.position).filter(|&marker_pos| marker_pos > pos).min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::project_with_clips;

    #[test]
    fn add_three_out_of_order_comes_out_sorted() {
        let mut project = project_with_clips(&[&[]]);
        add_marker(&mut project, 500).unwrap();
        add_marker(&mut project, 100).unwrap();
        add_marker(&mut project, 300).unwrap();
        let positions: Vec<u64> = project.markers.iter().map(|m| m.position).collect();
        assert_eq!(positions, [100, 300, 500]);
    }

    #[test]
    fn add_at_existing_position_errors_and_changes_nothing() {
        let mut project = project_with_clips(&[&[]]);
        add_marker(&mut project, 250).unwrap();
        let before = project.clone();
        assert!(matches!(add_marker(&mut project, 250), Err(OpError::Invalid(_))));
        assert_eq!(project, before);
    }

    #[test]
    fn move_marker_re_sorts() {
        let mut project = project_with_clips(&[&[]]);
        add_marker(&mut project, 100).unwrap();
        add_marker(&mut project, 300).unwrap();
        let id = project.markers[1].id;
        move_marker(&mut project, id, 50).unwrap();
        let positions: Vec<u64> = project.markers.iter().map(|m| m.position).collect();
        assert_eq!(positions, [50, 100]);
    }

    #[test]
    fn move_marker_onto_existing_position_errors_and_changes_nothing() {
        let mut project = project_with_clips(&[&[]]);
        add_marker(&mut project, 100).unwrap();
        add_marker(&mut project, 300).unwrap();
        let before = project.clone();
        let id = project.markers[1].id;
        assert!(matches!(move_marker(&mut project, id, 100), Err(OpError::Invalid(_))));
        assert_eq!(project, before);
    }

    #[test]
    fn move_unknown_marker_errors_and_changes_nothing() {
        let mut project = project_with_clips(&[&[]]);
        add_marker(&mut project, 100).unwrap();
        let before = project.clone();
        assert!(matches!(move_marker(&mut project, Uuid::new_v4(), 200), Err(OpError::NotFound(_))));
        assert_eq!(project, before);
    }

    #[test]
    fn delete_markers_removes_two_at_once() {
        let mut project = project_with_clips(&[&[]]);
        add_marker(&mut project, 100).unwrap();
        add_marker(&mut project, 200).unwrap();
        add_marker(&mut project, 300).unwrap();
        let ids = [project.markers[0].id, project.markers[2].id];
        delete_markers(&mut project, &ids).unwrap();
        let positions: Vec<u64> = project.markers.iter().map(|m| m.position).collect();
        assert_eq!(positions, [200]);
    }

    #[test]
    fn delete_markers_unknown_id_errors_and_changes_nothing() {
        let mut project = project_with_clips(&[&[]]);
        add_marker(&mut project, 100).unwrap();
        let before = project.clone();
        assert!(matches!(delete_markers(&mut project, &[Uuid::new_v4()]), Err(OpError::NotFound(_))));
        assert_eq!(project, before);
    }

    #[test]
    fn prev_and_next_cover_every_case() {
        let mut project = project_with_clips(&[&[]]);
        add_marker(&mut project, 100).unwrap();
        add_marker(&mut project, 300).unwrap();
        add_marker(&mut project, 500).unwrap();

        // Strictly before/after: a marker position sees neither itself.
        assert_eq!(prev_marker(&project, 100), None);
        assert_eq!(next_marker(&project, 100), Some(300));
        assert_eq!(prev_marker(&project, 300), Some(100));
        assert_eq!(next_marker(&project, 300), Some(500));

        // Between two markers.
        assert_eq!(prev_marker(&project, 200), Some(100));
        assert_eq!(next_marker(&project, 200), Some(300));

        // Before the first and after the last.
        assert_eq!(prev_marker(&project, 50), None);
        assert_eq!(next_marker(&project, 50), Some(100));
        assert_eq!(prev_marker(&project, 999), Some(500));
        assert_eq!(next_marker(&project, 999), None);

        // Empty project.
        let empty = project_with_clips(&[&[]]);
        assert_eq!(prev_marker(&empty, 100), None);
        assert_eq!(next_marker(&empty, 100), None);
    }
}
