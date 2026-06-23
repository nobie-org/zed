//! Retained root slots and deferred subtree removals.
//!
//! Root slots are the cross-frame ownership layer above individual retained nodes:
//! previous roots are consumed during commit, current roots are promoted at
//! frame finish, and detached subtrees are removed only by the forest's mirror
//! cleanup path.

use super::super::RetainedLayoutRootId;
use super::geometry::RootSolveInput;
use super::node::RetainedLayoutNode;
use collections::FxHashMap;
use std::mem;

/// Owns previous/current root slots and detached subtree removals.
pub(super) struct RootSlots {
    retained_roots: FxHashMap<RetainedLayoutRootId, RetainedLayoutNode>,
    current_roots: FxHashMap<RetainedLayoutRootId, RetainedLayoutNode>,
    detached_subtree_removals: Vec<RetainedLayoutNode>,
    last_solve_inputs: FxHashMap<RetainedLayoutRootId, RootSolveInput>,
}

/// Transaction checkpoint for root slot state.
pub(super) struct RootSlotsCheckpoint {
    retained_roots: FxHashMap<RetainedLayoutRootId, RetainedLayoutNode>,
    current_roots: FxHashMap<RetainedLayoutRootId, RetainedLayoutNode>,
    detached_subtree_removals: Vec<RetainedLayoutNode>,
    last_solve_inputs: FxHashMap<RetainedLayoutRootId, RootSolveInput>,
}

impl RootSlots {
    pub(super) fn new() -> Self {
        Self {
            retained_roots: FxHashMap::default(),
            current_roots: FxHashMap::default(),
            detached_subtree_removals: Vec::new(),
            last_solve_inputs: FxHashMap::default(),
        }
    }

    pub(super) fn checkpoint(&self) -> RootSlotsCheckpoint {
        RootSlotsCheckpoint {
            retained_roots: self.retained_roots.clone(),
            current_roots: self.current_roots.clone(),
            detached_subtree_removals: self.detached_subtree_removals.clone(),
            last_solve_inputs: self.last_solve_inputs.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: RootSlotsCheckpoint) {
        self.retained_roots = checkpoint.retained_roots;
        self.current_roots = checkpoint.current_roots;
        self.detached_subtree_removals = checkpoint.detached_subtree_removals;
        self.last_solve_inputs = checkpoint.last_solve_inputs;
    }

    pub(super) fn take_retained_roots(&mut self) -> Vec<RetainedLayoutNode> {
        mem::take(&mut self.retained_roots).into_values().collect()
    }

    pub(super) fn promote_current_roots(&mut self) {
        let current_roots = mem::take(&mut self.current_roots);
        self.last_solve_inputs
            .retain(|root_id, _| current_roots.contains_key(root_id));
        self.retained_roots = current_roots;
    }

    pub(super) fn has_current_root(&self, root_id: RetainedLayoutRootId) -> bool {
        self.current_roots.contains_key(&root_id)
    }

    pub(super) fn take_retained_root(
        &mut self,
        root_id: RetainedLayoutRootId,
    ) -> Option<RetainedLayoutNode> {
        self.retained_roots.remove(&root_id)
    }

    pub(super) fn insert_current_root(
        &mut self,
        root_id: RetainedLayoutRootId,
        root: RetainedLayoutNode,
    ) {
        let previous = self.current_roots.insert(root_id, root);
        assert!(
            previous.is_none(),
            "retained layout root should be committed at most once per frame"
        );
    }

    pub(super) fn solve_input_changed(
        &self,
        root_id: RetainedLayoutRootId,
        input: RootSolveInput,
    ) -> bool {
        self.last_solve_inputs
            .get(&root_id)
            .is_some_and(|previous| *previous != input)
    }

    pub(super) fn record_solve_input(
        &mut self,
        root_id: RetainedLayoutRootId,
        input: RootSolveInput,
    ) {
        assert!(
            self.current_roots.contains_key(&root_id),
            "retained root solve input should be recorded only for a current root"
        );
        self.last_solve_inputs.insert(root_id, input);
    }

    pub(super) fn current_root_count(&self) -> usize {
        self.current_roots.len()
    }

    pub(super) fn detach_subtree(&mut self, root: RetainedLayoutNode) {
        self.detached_subtree_removals.push(root);
    }

    pub(super) fn take_detached_subtree_removals(&mut self) -> Vec<RetainedLayoutNode> {
        mem::take(&mut self.detached_subtree_removals)
    }
}
