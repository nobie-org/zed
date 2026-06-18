//! Root identity registry for the retained layout forest.
//!
//! Retained roots are GPUI site identities, not Taffy concepts. This module
//! owns the keying and duplicate-occurrence accounting needed to keep anonymous
//! roots from aliasing a retained mirror node.

use crate::{ElementId, GlobalElementId};
use collections::FxHashMap;
use std::sync::Arc;

use super::super::RetainedLayoutRootId;

/// Allocates stable retained ids for root compute sites.
pub(super) struct RootRegistry {
    root_ids: FxHashMap<RetainedLayoutRootKey, RetainedLayoutRootId>,
    root_occurrences: FxHashMap<Arc<[ElementId]>, u64>,
    next_root_id: u64,
}

/// Transaction checkpoint for root identity state.
pub(super) struct RootRegistryCheckpoint {
    root_ids: FxHashMap<RetainedLayoutRootKey, RetainedLayoutRootId>,
    root_occurrences: FxHashMap<Arc<[ElementId]>, u64>,
    next_root_id: u64,
}

/// Private matching key for retained root reuse.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum RetainedLayoutRootKey {
    Global(GlobalElementId),
    Anonymous {
        element_id_stack: Arc<[ElementId]>,
        occurrence: u64,
    },
}

impl RootRegistry {
    pub(super) fn new() -> Self {
        Self {
            root_ids: FxHashMap::default(),
            root_occurrences: FxHashMap::default(),
            next_root_id: 0,
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.root_occurrences.clear();
    }

    pub(super) fn checkpoint(&self) -> RootRegistryCheckpoint {
        RootRegistryCheckpoint {
            root_ids: self.root_ids.clone(),
            root_occurrences: self.root_occurrences.clone(),
            next_root_id: self.next_root_id,
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: RootRegistryCheckpoint) {
        self.root_ids = checkpoint.root_ids;
        self.root_occurrences = checkpoint.root_occurrences;
        self.next_root_id = checkpoint.next_root_id;
    }

    pub(super) fn retained_root_id(
        &mut self,
        global_id: Option<&GlobalElementId>,
        element_id_stack: &[ElementId],
    ) -> RetainedLayoutRootId {
        let key = if let Some(global_id) = global_id {
            RetainedLayoutRootKey::Global(global_id.clone())
        } else {
            let element_id_stack: Arc<[ElementId]> = Arc::from(element_id_stack);
            let occurrence = self
                .root_occurrences
                .entry(element_id_stack.clone())
                .or_default();
            let key = RetainedLayoutRootKey::Anonymous {
                element_id_stack,
                occurrence: *occurrence,
            };
            *occurrence += 1;
            key
        };

        if let Some(root_id) = self.root_ids.get(&key) {
            return *root_id;
        }

        let root_id = RetainedLayoutRootId::new(self.next_root_id);
        self.next_root_id = self.next_root_id.saturating_add(1);
        self.root_ids.insert(key, root_id);
        root_id
    }
}
