//! Current-frame layout facts storage.
//!
//! Frame facts are pure inputs for this render pass. They are not retained
//! identity and are truncated on transaction rollback.

use super::super::LayoutId;
use super::facts::CurrentLayoutNodeFacts;

/// Owns the current-frame layout facts log and measured producer slots.
pub(super) struct CurrentLayoutFactsLog {
    facts: Vec<CurrentLayoutNodeFacts>,
}

/// Transaction checkpoint for frame-local facts storage.
pub(super) struct CurrentLayoutFactsLogCheckpoint {
    facts_len: usize,
}

impl CurrentLayoutFactsLog {
    pub(super) fn new() -> Self {
        Self { facts: Vec::new() }
    }

    pub(super) fn clear(&mut self) {
        self.facts.clear();
    }

    pub(super) fn checkpoint(&self) -> CurrentLayoutFactsLogCheckpoint {
        CurrentLayoutFactsLogCheckpoint {
            facts_len: self.facts.len(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: CurrentLayoutFactsLogCheckpoint) {
        self.facts.truncate(checkpoint.facts_len);
    }

    pub(super) fn push_facts(&mut self, facts: CurrentLayoutNodeFacts) -> LayoutId {
        let id = LayoutId(self.facts.len());
        self.facts.push(facts);
        id
    }

    pub(super) fn facts(&self, id: LayoutId) -> &CurrentLayoutNodeFacts {
        self.facts
            .get(id.0)
            .expect("layout facts id should come from the current frame")
    }
}
