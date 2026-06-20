//! GPUI-owned frame geometry.
//!
//! Solver per-node layout slots are private scratch. The retained
//! forest may read them only immediately after solving a legal retained root,
//! then copies the results into this current-frame store. GPUI-visible bounds
//! read from this store, not from the solver directly.

use super::super::{AvailableSpace, RetainedLayoutRootId};
use super::solver::{SolverLayout, SolverNodeId};
use crate::{Size, size};
use collections::{FxHashMap, FxHashSet};

/// Current-frame proof that one retained root was solved exactly once.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SolvedRoot {
    root_node: SolverNodeId,
    available_space: Size<AvailableSpaceKey>,
    scale_factor_bits: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum AvailableSpaceKey {
    Definite(u32),
    #[default]
    MinContent,
    MaxContent,
}

impl SolvedRoot {
    fn new(
        root_node: SolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
    ) -> Self {
        Self {
            root_node,
            available_space: size(
                AvailableSpaceKey::from(available_space.width),
                AvailableSpaceKey::from(available_space.height),
            ),
            scale_factor_bits: scale_factor.to_bits(),
        }
    }
}

impl From<AvailableSpace> for AvailableSpaceKey {
    fn from(value: AvailableSpace) -> Self {
        match value {
            AvailableSpace::Definite(pixels) => Self::Definite(pixels.0.to_bits()),
            AvailableSpace::MinContent => Self::MinContent,
            AvailableSpace::MaxContent => Self::MaxContent,
        }
    }
}

/// Geometry authority for the current frame.
///
/// `current_layouts` is the only source used by GPUI-visible bounds. A retained
/// root may be captured once per frame; a second capture with different inputs
/// is a lifecycle bug and fails loudly.
pub(super) struct GeometryStore {
    solved_roots: FxHashMap<RetainedLayoutRootId, SolvedRoot>,
    solved_root_nodes: FxHashSet<SolverNodeId>,
    current_layouts: FxHashMap<SolverNodeId, SolverLayout>,
}

/// Transaction checkpoint for retained geometry state.
pub(super) struct GeometryStoreCheckpoint {
    solved_roots: FxHashMap<RetainedLayoutRootId, SolvedRoot>,
    solved_root_nodes: FxHashSet<SolverNodeId>,
    current_layouts: FxHashMap<SolverNodeId, SolverLayout>,
}

impl GeometryStore {
    pub(super) fn new() -> Self {
        Self {
            solved_roots: FxHashMap::default(),
            solved_root_nodes: FxHashSet::default(),
            current_layouts: FxHashMap::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.solved_roots.clear();
        self.solved_root_nodes.clear();
        self.current_layouts.clear();
    }

    pub(super) fn checkpoint(&self) -> GeometryStoreCheckpoint {
        GeometryStoreCheckpoint {
            solved_roots: self.solved_roots.clone(),
            solved_root_nodes: self.solved_root_nodes.clone(),
            current_layouts: self.current_layouts.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: GeometryStoreCheckpoint) {
        self.solved_roots = checkpoint.solved_roots;
        self.solved_root_nodes = checkpoint.solved_root_nodes;
        self.current_layouts = checkpoint.current_layouts;
    }

    pub(super) fn finish_frame(&mut self) {
        self.solved_roots.clear();
        self.solved_root_nodes.clear();
        self.current_layouts.clear();
    }

    pub(super) fn begin_solve(
        &mut self,
        root_id: RetainedLayoutRootId,
        root_node: SolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
    ) {
        assert!(
            self.solved_roots
                .insert(
                    root_id,
                    SolvedRoot::new(root_node, available_space, scale_factor)
                )
                .is_none(),
            "retained layout root should be solved at most once per frame"
        );
        assert!(
            self.solved_root_nodes.insert(root_node),
            "retained layout node should not be computed through multiple roots in one frame"
        );
    }

    pub(super) fn capture_from_solver(
        &mut self,
        root_id: RetainedLayoutRootId,
        root_node: SolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        mut layout: impl FnMut(SolverNodeId) -> SolverLayout,
        mut children: impl FnMut(SolverNodeId) -> Vec<SolverNodeId>,
    ) {
        assert_eq!(
            self.solved_roots.get(&root_id),
            Some(&SolvedRoot::new(root_node, available_space, scale_factor)),
            "retained layout geometry should be captured only for the scheduled root solve"
        );

        let mut stack = vec![root_node];

        while let Some(node_id) = stack.pop() {
            self.current_layouts.insert(node_id, layout(node_id));
            stack.extend(children(node_id));
        }
    }

    pub(super) fn layout(&self, node_id: SolverNodeId) -> Option<SolverLayout> {
        self.current_layouts.get(&node_id).cloned()
    }
}
