//! Root identity registry for the retained layout forest.
//!
//! Retained roots are GPUI site identities, not solver concepts. This module
//! owns the explicit keying needed to reuse a root across frames. Anonymous
//! roots are intentionally scratch roots: without a semantic id, there is no
//! cross-frame identity to retain.

use crate::GlobalElementId;
use collections::FxHashMap;

use super::super::{RetainedLayoutRootId, RetainedLayoutRootSite};

/// Allocates stable retained ids for root compute sites.
pub(super) struct RootRegistry {
    root_ids: FxHashMap<RetainedLayoutRootKey, RetainedLayoutRootId>,
    next_root_id: u64,
}

/// Transaction checkpoint for root identity state.
pub(super) struct RootRegistryCheckpoint {
    root_ids: FxHashMap<RetainedLayoutRootKey, RetainedLayoutRootId>,
    next_root_id: u64,
}

/// Private matching key for retained root reuse.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum RetainedLayoutRootKey {
    Global {
        root_site: RetainedLayoutRootSite,
        global_id: GlobalElementId,
    },
}

impl RootRegistry {
    pub(super) fn new() -> Self {
        Self {
            root_ids: FxHashMap::default(),
            next_root_id: 0,
        }
    }

    pub(super) fn checkpoint(&self) -> RootRegistryCheckpoint {
        RootRegistryCheckpoint {
            root_ids: self.root_ids.clone(),
            next_root_id: self.next_root_id,
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: RootRegistryCheckpoint) {
        self.root_ids = checkpoint.root_ids;
        self.next_root_id = checkpoint.next_root_id;
    }

    pub(super) fn retained_root_id(
        &mut self,
        root_site: RetainedLayoutRootSite,
        global_id: Option<&GlobalElementId>,
    ) -> RetainedLayoutRootId {
        let Some(global_id) = global_id else {
            return self.allocate_root_id();
        };

        let key = RetainedLayoutRootKey::Global {
            root_site,
            global_id: global_id.clone(),
        };
        if let Some(root_id) = self.root_ids.get(&key) {
            return *root_id;
        }

        let root_id = self.allocate_root_id();
        self.root_ids.insert(key, root_id);
        root_id
    }

    fn allocate_root_id(&mut self) -> RetainedLayoutRootId {
        let root_id = RetainedLayoutRootId::new(self.next_root_id);
        self.next_root_id = self.next_root_id.saturating_add(1);
        root_id
    }
}
