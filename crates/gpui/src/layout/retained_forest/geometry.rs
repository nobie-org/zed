//! GPUI-owned frame geometry.
//!
//! Taffy's per-node layout slots are private solver scratch. The retained
//! forest may read them only immediately after solving a legal retained root,
//! then copies the results into this current-frame store. GPUI-visible bounds
//! read from this store, not from Taffy directly.

use super::super::{AvailableSpace, RetainedLayoutRootId};
use crate::{Size, size};
use collections::FxHashMap;
use taffy::tree::{Layout, NodeId};

/// Current-frame proof that one retained root was solved exactly once.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SolvedRoot {
    root_node: NodeId,
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
    fn new(root_node: NodeId, available_space: Size<AvailableSpace>, scale_factor: f32) -> Self {
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
    current_layouts: FxHashMap<NodeId, Layout>,
}

/// Transaction checkpoint for retained geometry state.
pub(super) struct GeometryStoreCheckpoint {
    solved_roots: FxHashMap<RetainedLayoutRootId, SolvedRoot>,
    current_layouts: FxHashMap<NodeId, Layout>,
}

impl GeometryStore {
    pub(super) fn new() -> Self {
        Self {
            solved_roots: FxHashMap::default(),
            current_layouts: FxHashMap::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.solved_roots.clear();
        self.current_layouts.clear();
    }

    pub(super) fn checkpoint(&self) -> GeometryStoreCheckpoint {
        GeometryStoreCheckpoint {
            solved_roots: self.solved_roots.clone(),
            current_layouts: self.current_layouts.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: GeometryStoreCheckpoint) {
        self.solved_roots = checkpoint.solved_roots;
        self.current_layouts = checkpoint.current_layouts;
    }

    pub(super) fn finish_frame(&mut self) {
        self.solved_roots.clear();
        self.current_layouts.clear();
    }

    pub(super) fn has_solved_root(&self, root_id: RetainedLayoutRootId) -> bool {
        self.solved_roots.contains_key(&root_id)
    }

    pub(super) fn capture_from_solver(
        &mut self,
        root_id: RetainedLayoutRootId,
        root_node: NodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        mut layout: impl FnMut(NodeId) -> Layout,
        mut children: impl FnMut(NodeId) -> Vec<NodeId>,
    ) {
        let solved_root = SolvedRoot::new(root_node, available_space, scale_factor);
        if let Some(previous) = self.solved_roots.insert(root_id, solved_root) {
            assert_eq!(
                previous, solved_root,
                "retained layout root should be solved at most once per frame"
            );
            return;
        }

        let mut stack = vec![root_node];

        while let Some(node_id) = stack.pop() {
            self.current_layouts.insert(node_id, layout(node_id));
            stack.extend(children(node_id));
        }
    }

    pub(super) fn layout(&self, node_id: NodeId) -> Option<Layout> {
        self.current_layouts.get(&node_id).cloned()
    }
}
