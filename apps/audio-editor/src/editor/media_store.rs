//! editor::media_store: conformed audio for bin items, kept for the whole session.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::io::import::Loaded;

/// Loaded audio for every bin item seen this session.
///
/// Entries are never removed while the project is open: undo and redo can
/// re-introduce bin items, so their audio must stay available.
#[derive(Default)]
pub struct MediaStore {
    /// Loaded audio per media id.
    entries: HashMap<Uuid, Loaded>,
    /// Media ids whose file is missing from media/.
    offline: HashSet<Uuid>,
}

impl MediaStore {
    /// Creates an empty store.
    pub fn new() -> MediaStore {
        MediaStore {
            entries: HashMap::new(),
            offline: HashSet::new(),
        }
    }

    /// Loaded audio for the media id, if present.
    pub fn get(&self, id: Uuid) -> Option<&Loaded> {
        self.entries.get(&id)
    }

    /// Stores loaded audio for the media id, clearing any offline mark.
    pub fn insert(&mut self, id: Uuid, loaded: Loaded) {
        self.offline.remove(&id);
        self.entries.insert(id, loaded);
    }

    /// Marks a media id offline (its file is missing from media/).
    pub fn set_offline(&mut self, id: Uuid) {
        self.offline.insert(id);
    }

    /// True when the media id is marked offline.
    pub fn is_offline(&self, id: Uuid) -> bool {
        self.offline.contains(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::media::Peaks;

    #[test]
    fn insert_get_and_offline_round_trip() {
        let mut store = MediaStore::new();
        let id = Uuid::new_v4();
        assert!(store.get(id).is_none());
        assert!(!store.is_offline(id));

        store.set_offline(id);
        assert!(store.is_offline(id));

        store.insert(
            id,
            Loaded {
                samples: vec![0.0; 4].into(),
                peaks: Peaks {
                    channels: 2,
                    frames: 0,
                    levels: Vec::new(),
                }
                .into(),
            },
        );
        assert!(!store.is_offline(id), "insert clears the offline mark");
        let loaded = store.get(id).expect("entry exists after insert");
        assert_eq!(loaded.samples.len(), 4);
    }
}
