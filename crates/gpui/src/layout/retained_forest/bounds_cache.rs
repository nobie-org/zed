//! GPUI absolute-bounds cache for committed layout nodes.
//!
//! Taffy stores node-local layout results. GPUI needs snapped absolute bounds
//! and unrounded absolute origins for paint, hit testing, and descendant bounds
//! queries. This cache owns those derived facts for the current frame.

use crate::{Bounds, Pixels, Point, Size};
use collections::{FxHashMap, FxHashSet};
use taffy::tree::{Layout, NodeId};

/// Derived absolute bounds for the current committed layout graph.
pub(super) struct BoundsCache {
    absolute_layout_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    absolute_outer_origins: FxHashMap<NodeId, Point<f32>>,
    computed_layouts: FxHashSet<NodeId>,
    scratch_space: Vec<NodeId>,
}

/// Transaction checkpoint for derived bounds state.
pub(super) struct BoundsCacheCheckpoint {
    absolute_layout_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    absolute_outer_origins: FxHashMap<NodeId, Point<f32>>,
    computed_layouts: FxHashSet<NodeId>,
    scratch_space: Vec<NodeId>,
}

impl BoundsCache {
    pub(super) fn new() -> Self {
        Self {
            absolute_layout_bounds: FxHashMap::default(),
            absolute_outer_origins: FxHashMap::default(),
            computed_layouts: FxHashSet::default(),
            scratch_space: Vec::new(),
        }
    }

    pub(super) fn checkpoint(&self) -> BoundsCacheCheckpoint {
        BoundsCacheCheckpoint {
            absolute_layout_bounds: self.absolute_layout_bounds.clone(),
            absolute_outer_origins: self.absolute_outer_origins.clone(),
            computed_layouts: self.computed_layouts.clone(),
            scratch_space: self.scratch_space.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: BoundsCacheCheckpoint) {
        self.absolute_layout_bounds = checkpoint.absolute_layout_bounds;
        self.absolute_outer_origins = checkpoint.absolute_outer_origins;
        self.computed_layouts = checkpoint.computed_layouts;
        self.scratch_space = checkpoint.scratch_space;
    }

    pub(super) fn clear(&mut self) {
        self.absolute_layout_bounds.clear();
        self.absolute_outer_origins.clear();
        self.computed_layouts.clear();
    }

    pub(super) fn mark_computed(&mut self, node_id: NodeId) -> bool {
        self.computed_layouts.insert(node_id)
    }

    pub(super) fn layout_bounds_for_node(
        &mut self,
        node_id: NodeId,
        scale_factor: f32,
        mut layout: impl FnMut(NodeId) -> Layout,
        mut parent: impl FnMut(NodeId) -> Option<NodeId>,
    ) -> Bounds<Pixels> {
        if let Some(bounds) = self.absolute_layout_bounds.get(&node_id).cloned() {
            return bounds;
        }

        let mut path = vec![node_id];
        while let Some(parent_id) = parent(*path.last().expect("layout path is nonempty")) {
            if self.absolute_outer_origins.contains_key(&parent_id) {
                break;
            }
            path.push(parent_id);
        }

        for node_id in path.into_iter().rev() {
            if self.absolute_layout_bounds.contains_key(&node_id) {
                continue;
            }

            let node_layout = layout(node_id);
            let absolute_outer_origin = match parent(node_id) {
                Some(parent_id) => {
                    let parent_origin = *self
                        .absolute_outer_origins
                        .get(&parent_id)
                        .expect("parent absolute outer origin should be cached");
                    parent_origin + Point::from(node_layout.location)
                }
                None => Point::from(node_layout.location),
            };
            self.absolute_outer_origins
                .insert(node_id, absolute_outer_origin);

            let absolute_far = absolute_outer_origin + Point::from(Size::from(node_layout.size));
            let snapped_bounds = Bounds::from_corners(
                absolute_outer_origin.map(super::round_half_toward_zero),
                absolute_far.map(super::round_half_toward_zero),
            );

            let bounds = (snapped_bounds / scale_factor).map(Pixels);
            self.absolute_layout_bounds.insert(node_id, bounds);
        }

        self.absolute_layout_bounds
            .get(&node_id)
            .cloned()
            .expect("requested node bounds should be cached")
    }
}
