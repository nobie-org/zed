//! Current-frame committed layout-node ownership.
//!
//! A retained mirror node may appear at only one current-frame position. This
//! module owns that uniqueness proof and the `LayoutId` to mirror-node mapping
//! used for bounds reads and debug verification.

use super::super::LayoutId;
use super::solver::SolverNodeId;
use collections::{FxHashMap, FxHashSet};

/// Current-frame map from GPUI layout ids to private mirror nodes.
pub(super) struct CommittedLayoutState {
    layout_nodes: FxHashMap<LayoutId, SolverNodeId>,
    node_layouts: FxHashMap<SolverNodeId, LayoutId>,
    dirty_solver_nodes: FxHashSet<SolverNodeId>,
}

/// Transaction checkpoint for committed-node state.
pub(super) struct CommittedLayoutCheckpoint {
    layout_nodes: FxHashMap<LayoutId, SolverNodeId>,
    node_layouts: FxHashMap<SolverNodeId, LayoutId>,
    dirty_solver_nodes: FxHashSet<SolverNodeId>,
}

impl CommittedLayoutState {
    pub(super) fn new() -> Self {
        Self {
            layout_nodes: FxHashMap::default(),
            node_layouts: FxHashMap::default(),
            dirty_solver_nodes: FxHashSet::default(),
        }
    }

    pub(super) fn checkpoint(&self) -> CommittedLayoutCheckpoint {
        CommittedLayoutCheckpoint {
            layout_nodes: self.layout_nodes.clone(),
            node_layouts: self.node_layouts.clone(),
            dirty_solver_nodes: self.dirty_solver_nodes.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: CommittedLayoutCheckpoint) {
        self.layout_nodes = checkpoint.layout_nodes;
        self.node_layouts = checkpoint.node_layouts;
        self.dirty_solver_nodes = checkpoint.dirty_solver_nodes;
    }

    pub(super) fn clear(&mut self) {
        self.layout_nodes.clear();
        self.node_layouts.clear();
        self.dirty_solver_nodes.clear();
    }

    pub(super) fn node(&self, id: LayoutId) -> SolverNodeId {
        *self
            .layout_nodes
            .get(&id)
            .expect("layout bounds should only be requested after layout is committed")
    }

    pub(super) fn try_node(&self, id: LayoutId) -> Option<SolverNodeId> {
        self.layout_nodes.get(&id).copied()
    }

    pub(super) fn contains_layout(&self, id: LayoutId) -> bool {
        self.layout_nodes.contains_key(&id)
    }

    pub(super) fn layout_id_for_node(&self, node_id: SolverNodeId) -> Option<LayoutId> {
        self.node_layouts.get(&node_id).copied()
    }

    pub(super) fn node_layout_ids_for_trace(&self) -> Vec<(SolverNodeId, LayoutId)> {
        self.node_layouts
            .iter()
            .map(|(node_id, layout_id)| (*node_id, *layout_id))
            .collect()
    }

    pub(super) fn insert(&mut self, id: LayoutId, node_id: SolverNodeId) {
        assert!(
            !self.layout_nodes.contains_key(&id),
            "layout facts should appear only once in a committed layout tree"
        );
        assert!(
            !self.node_layouts.contains_key(&node_id),
            "committed solver node should map to only one current layout id"
        );
        self.layout_nodes.insert(id, node_id);
        self.node_layouts.insert(node_id, id);
    }

    pub(super) fn mark_solver_node_dirty(&mut self, node_id: SolverNodeId) -> bool {
        self.dirty_solver_nodes.insert(node_id)
    }
}
