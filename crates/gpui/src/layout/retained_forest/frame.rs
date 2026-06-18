//! Current-frame layout intent storage.
//!
//! Frame intents are pure inputs for this render pass. They are not retained
//! identity and are truncated on transaction rollback.

use super::super::LayoutId;
use super::LayoutIntent;

/// Owns the current-frame layout intent log and measured producer slots.
pub(super) struct FrameIntents {
    intents: Vec<LayoutIntent>,
}

/// Transaction checkpoint for frame-local intent storage.
pub(super) struct FrameIntentsCheckpoint {
    intents_len: usize,
}

impl FrameIntents {
    pub(super) fn new() -> Self {
        Self {
            intents: Vec::new(),
        }
    }

    pub(super) fn clear(&mut self) {
        self.intents.clear();
    }

    pub(super) fn checkpoint(&self) -> FrameIntentsCheckpoint {
        FrameIntentsCheckpoint {
            intents_len: self.intents.len(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: FrameIntentsCheckpoint) {
        self.intents.truncate(checkpoint.intents_len);
    }

    pub(super) fn push_intent(&mut self, intent: LayoutIntent) -> LayoutId {
        let id = LayoutId(self.intents.len());
        self.intents.push(intent);
        id
    }

    pub(super) fn intent(&self, id: LayoutId) -> &LayoutIntent {
        self.intents
            .get(id.0)
            .expect("layout intent id should come from the current frame")
    }
}
