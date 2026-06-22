//! GPUI-owned frame geometry.
//!
//! Solver per-node layout slots are private scratch. The retained
//! forest may read them only immediately after solving a legal retained root,
//! then copies the results into this current-frame store. GPUI-visible bounds
//! read from this store, not from the solver directly.

use super::super::{AvailableSpace, LayoutId, RetainedLayoutRootId};
use super::solver::{SolverLayout, SolverNodeId};
use crate::{Bounds, Pixels, Point, Size, size};
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

/// Layout geometry published by the legal retained-root solves for the current frame.
///
/// Solver layout slots are scratch. This output is captured immediately after a
/// legal solve and is the only place GPUI-visible bounds may be read from.
pub(super) struct FrameLayoutOutput {
    solved_roots: FxHashMap<RetainedLayoutRootId, SolvedRoot>,
    solved_root_nodes: FxHashSet<SolverNodeId>,
    current_layouts: FxHashMap<SolverNodeId, SolverLayout>,
    absolute_layout_bounds: FxHashMap<SolverNodeId, Bounds<Pixels>>,
    absolute_outer_origins: FxHashMap<SolverNodeId, Point<f32>>,
    bounds_by_layout_id: FxHashMap<LayoutId, Bounds<Pixels>>,
}

/// Transaction checkpoint for retained geometry state.
pub(super) struct FrameLayoutOutputCheckpoint {
    solved_roots: FxHashMap<RetainedLayoutRootId, SolvedRoot>,
    solved_root_nodes: FxHashSet<SolverNodeId>,
    current_layouts: FxHashMap<SolverNodeId, SolverLayout>,
    absolute_layout_bounds: FxHashMap<SolverNodeId, Bounds<Pixels>>,
    absolute_outer_origins: FxHashMap<SolverNodeId, Point<f32>>,
    bounds_by_layout_id: FxHashMap<LayoutId, Bounds<Pixels>>,
}

impl FrameLayoutOutput {
    pub(super) fn new() -> Self {
        Self {
            solved_roots: FxHashMap::default(),
            solved_root_nodes: FxHashSet::default(),
            current_layouts: FxHashMap::default(),
            absolute_layout_bounds: FxHashMap::default(),
            absolute_outer_origins: FxHashMap::default(),
            bounds_by_layout_id: FxHashMap::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.solved_roots.clear();
        self.solved_root_nodes.clear();
        self.current_layouts.clear();
        self.absolute_layout_bounds.clear();
        self.absolute_outer_origins.clear();
        self.bounds_by_layout_id.clear();
    }

    pub(super) fn checkpoint(&self) -> FrameLayoutOutputCheckpoint {
        FrameLayoutOutputCheckpoint {
            solved_roots: self.solved_roots.clone(),
            solved_root_nodes: self.solved_root_nodes.clone(),
            current_layouts: self.current_layouts.clone(),
            absolute_layout_bounds: self.absolute_layout_bounds.clone(),
            absolute_outer_origins: self.absolute_outer_origins.clone(),
            bounds_by_layout_id: self.bounds_by_layout_id.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: FrameLayoutOutputCheckpoint) {
        self.solved_roots = checkpoint.solved_roots;
        self.solved_root_nodes = checkpoint.solved_root_nodes;
        self.current_layouts = checkpoint.current_layouts;
        self.absolute_layout_bounds = checkpoint.absolute_layout_bounds;
        self.absolute_outer_origins = checkpoint.absolute_outer_origins;
        self.bounds_by_layout_id = checkpoint.bounds_by_layout_id;
    }

    pub(super) fn finish_frame(&mut self) {
        self.solved_roots.clear();
        self.solved_root_nodes.clear();
        self.current_layouts.clear();
        self.absolute_layout_bounds.clear();
        self.absolute_outer_origins.clear();
        self.bounds_by_layout_id.clear();
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
        layouts: impl IntoIterator<Item = (SolverNodeId, SolverLayout)>,
        mut parent: impl FnMut(SolverNodeId) -> Option<SolverNodeId>,
        mut layout_id_for_node: impl FnMut(SolverNodeId) -> Option<LayoutId>,
    ) {
        assert_eq!(
            self.solved_roots.get(&root_id),
            Some(&SolvedRoot::new(root_node, available_space, scale_factor)),
            "retained layout geometry should be captured only for the scheduled root solve"
        );

        let mut captured_root = false;
        let mut captured_node_ids = Vec::new();
        for (node_id, layout) in layouts {
            captured_root |= node_id == root_node;
            assert!(
                self.current_layouts.insert(node_id, layout).is_none(),
                "retained layout geometry snapshot should contain each solver node once"
            );
            captured_node_ids.push(node_id);
        }
        assert!(
            captured_root,
            "retained layout geometry snapshot should include the solved root"
        );

        for node_id in &captured_node_ids {
            self.capture_absolute_bounds_for_node(*node_id, scale_factor, &mut parent);
        }
        for node_id in captured_node_ids {
            let layout_id = layout_id_for_node(node_id)
                .expect("captured retained geometry should have a committed layout id");
            let bounds = self
                .absolute_layout_bounds
                .get(&node_id)
                .copied()
                .expect("captured retained geometry should have absolute bounds");
            assert!(
                self.bounds_by_layout_id.insert(layout_id, bounds).is_none(),
                "retained frame output should contain each layout id once"
            );
        }
    }

    pub(super) fn layout(&self, node_id: SolverNodeId) -> Option<SolverLayout> {
        self.current_layouts.get(&node_id).cloned()
    }

    pub(super) fn bounds(&self, id: LayoutId) -> Option<Bounds<Pixels>> {
        self.bounds_by_layout_id.get(&id).cloned()
    }

    fn capture_absolute_bounds_for_node(
        &mut self,
        node_id: SolverNodeId,
        scale_factor: f32,
        parent: &mut impl FnMut(SolverNodeId) -> Option<SolverNodeId>,
    ) {
        if self.absolute_layout_bounds.contains_key(&node_id) {
            return;
        }

        let mut path = vec![node_id];
        while let Some(parent_id) = parent(*path.last().expect("layout path is nonempty")) {
            if self.absolute_outer_origins.contains_key(&parent_id) {
                break;
            }
            assert!(
                self.current_layouts.contains_key(&parent_id),
                "retained frame geometry parent should be captured with child geometry"
            );
            path.push(parent_id);
        }

        for node_id in path.into_iter().rev() {
            if self.absolute_layout_bounds.contains_key(&node_id) {
                continue;
            }

            let node_layout = self
                .current_layouts
                .get(&node_id)
                .copied()
                .expect("retained frame geometry should contain requested node");
            let absolute_outer_origin = match parent(node_id) {
                Some(parent_id) => {
                    let parent_origin = *self
                        .absolute_outer_origins
                        .get(&parent_id)
                        .expect("parent absolute outer origin should be captured");
                    parent_origin + node_layout.location
                }
                None => node_layout.location,
            };
            self.absolute_outer_origins
                .insert(node_id, absolute_outer_origin);

            let absolute_far = absolute_outer_origin + Point::from(node_layout.size);
            let snapped_bounds = Bounds::from_corners(
                absolute_outer_origin.map(super::round_half_toward_zero),
                absolute_far.map(super::round_half_toward_zero),
            );
            self.absolute_layout_bounds
                .insert(node_id, (snapped_bounds / scale_factor).map(Pixels));
        }
    }
}
