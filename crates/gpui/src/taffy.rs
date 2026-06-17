use crate::{
    AbsoluteLength, App, Bounds, DefiniteLength, Edges, GridTemplate, Length, Pixels, Point, Size,
    Style, Window, size,
    util::{
        ceil_to_device_pixel, round_half_toward_zero, round_stroke_to_device_pixel,
        round_to_device_pixel,
    },
};
use collections::{FxHashMap, FxHashSet};
use stacksafe::{StackSafe, stacksafe};
use std::{fmt::Debug, ops::Range, time::Duration};
use taffy::{
    TaffyTree, TraversePartialTree as _,
    geometry::{Point as TaffyPoint, Rect as TaffyRect, Size as TaffySize},
    prelude::{max_content, min_content},
    style::AvailableSpace as TaffyAvailableSpace,
    tree::NodeId,
};

fn retained_layout_trace_enabled() -> bool {
    std::env::var_os("GPUI_RETAINED_LAYOUT_TRACE").is_some()
}

macro_rules! trace_retained_layout {
    ($($arg:tt)*) => {
        if retained_layout_trace_enabled() {
            eprintln!($($arg)*);
        }
    };
}

type NodeMeasureFn = StackSafe<
    Box<
        dyn FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &mut App,
        ) -> Size<Pixels>,
    >,
>;

struct NodeContext {
    measure: NodeMeasureFn,
}

/// Layout work performed by GPUI for one completed window draw.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LayoutWorkSample {
    /// Monotonic window-local draw index that produced this sample.
    pub draw_index: u64,
    /// Non-measured nodes requested through [`Window::request_layout`](crate::Window::request_layout).
    pub layout_node_requests: u64,
    /// Measured nodes requested through [`Window::request_measured_layout`](crate::Window::request_measured_layout).
    pub measured_layout_node_requests: u64,
    /// Parent-to-child layout edges passed to Taffy.
    pub child_edges: u64,
    /// Root layout computations requested for the draw.
    pub compute_layout_calls: u64,
    /// Measured layout callbacks invoked by Taffy.
    pub measured_layout_calls: u64,
    /// Wall time spent computing root layouts.
    pub compute_layout_duration: Duration,
    /// Wall time spent inside measured layout callbacks.
    pub measured_layout_duration: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetainedLayoutNodeKind {
    Unmeasured,
    Measured,
}

#[derive(Clone, Copy, Debug)]
struct ActiveRetainedLayoutScope {
    id: RetainedLayoutScopeId,
    next_node_index: usize,
}

pub struct TaffyLayoutEngine {
    taffy: TaffyTree<NodeContext>,
    absolute_layout_bounds: FxHashMap<LayoutId, Bounds<Pixels>>,
    /// Unrounded absolute border-box top-left per-node coordinate in device pixels.
    absolute_outer_origins: FxHashMap<LayoutId, Point<f32>>,
    computed_layout_roots: FxHashSet<LayoutId>,
    layout_bounds_scratch_space: Vec<LayoutId>,
    layout_work: LayoutWorkSample,
    scratch_nodes: Vec<LayoutId>,
    retained_layout_scope_stack: Vec<ActiveRetainedLayoutScope>,
    retained_layout_nodes_by_scope: FxHashMap<RetainedLayoutScopeId, Vec<LayoutId>>,
    retained_layout_node_kinds: FxHashMap<LayoutId, RetainedLayoutNodeKind>,
    retained_layout_root_by_scope: FxHashMap<RetainedLayoutScopeId, LayoutId>,
    #[cfg(any(test, feature = "test-support"))]
    retained_layout_node_creates: usize,
}

const EXPECT_MESSAGE: &str = "we should avoid taffy layout errors by construction if possible";

impl TaffyLayoutEngine {
    pub fn new() -> Self {
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        TaffyLayoutEngine {
            taffy,
            absolute_layout_bounds: FxHashMap::default(),
            absolute_outer_origins: FxHashMap::default(),
            computed_layout_roots: FxHashSet::default(),
            layout_bounds_scratch_space: Vec::new(),
            layout_work: LayoutWorkSample::default(),
            scratch_nodes: Vec::new(),
            retained_layout_scope_stack: Vec::new(),
            retained_layout_nodes_by_scope: FxHashMap::default(),
            retained_layout_node_kinds: FxHashMap::default(),
            retained_layout_root_by_scope: FxHashMap::default(),
            #[cfg(any(test, feature = "test-support"))]
            retained_layout_node_creates: 0,
        }
    }

    pub fn finish_frame(&mut self) -> LayoutWorkSample {
        let layout_work = self.layout_work;
        for node in std::mem::take(&mut self.scratch_nodes).into_iter().rev() {
            self.remove_node(node);
        }
        self.absolute_layout_bounds.clear();
        self.absolute_outer_origins.clear();
        self.computed_layout_roots.clear();
        self.layout_work = LayoutWorkSample::default();
        layout_work
    }

    pub fn begin_frame(&mut self) {
        self.layout_work = LayoutWorkSample::default();
    }

    #[cfg(test)]
    fn layout_work_sample(&self) -> LayoutWorkSample {
        self.layout_work
    }

    pub(crate) fn begin_retained_layout_scope(&mut self, scope: RetainedLayoutScopeId) {
        self.retained_layout_nodes_by_scope
            .entry(scope)
            .or_default();
        self.retained_layout_scope_stack
            .push(ActiveRetainedLayoutScope {
                id: scope,
                next_node_index: 0,
            });
    }

    pub(crate) fn finish_retained_layout_scope(
        &mut self,
        scope: RetainedLayoutScopeId,
        root: LayoutId,
    ) -> bool {
        let Some(active_scope) = self.retained_layout_scope_stack.pop() else {
            panic!("retained layout scopes must be finished in stack order");
        };
        assert_eq!(
            active_scope.id, scope,
            "retained layout scopes must be finished in stack order"
        );

        let root_is_owned = self
            .retained_layout_nodes_by_scope
            .get(&scope)
            .is_some_and(|nodes| {
                nodes
                    .get(..active_scope.next_node_index)
                    .is_some_and(|active_nodes| active_nodes.contains(&root))
            });

        if root_is_owned {
            if let Some(nodes) = self.retained_layout_nodes_by_scope.get_mut(&scope) {
                for node in nodes
                    .split_off(active_scope.next_node_index)
                    .into_iter()
                    .rev()
                {
                    self.remove_node(node);
                }
            }
            self.retained_layout_root_by_scope.insert(scope, root);
            true
        } else {
            if let Some(nodes) = self.retained_layout_nodes_by_scope.remove(&scope) {
                for node in nodes.into_iter().rev() {
                    self.remove_node(node);
                }
            }
            self.retained_layout_root_by_scope.remove(&scope);
            false
        }
    }

    pub(crate) fn discard_retained_layout_scope(&mut self, scope: RetainedLayoutScopeId) {
        let Some(active_scope) = self.retained_layout_scope_stack.pop() else {
            panic!("retained layout scopes must be discarded in stack order");
        };
        assert_eq!(
            active_scope.id, scope,
            "retained layout scopes must be discarded in stack order"
        );
        if let Some(nodes) = self.retained_layout_nodes_by_scope.remove(&scope) {
            self.scratch_nodes.extend(nodes);
        }
        self.retained_layout_root_by_scope.remove(&scope);
    }

    pub(crate) fn remove_retained_layout_scope(&mut self, scope: RetainedLayoutScopeId) {
        if let Some(nodes) = self.retained_layout_nodes_by_scope.remove(&scope) {
            for node in nodes.into_iter().rev() {
                self.remove_node(node);
            }
        }
        self.retained_layout_root_by_scope.remove(&scope);
    }

    pub(crate) fn retained_layout_root(&self, scope: RetainedLayoutScopeId) -> Option<LayoutId> {
        self.retained_layout_root_by_scope.get(&scope).copied()
    }

    #[cfg(debug_assertions)]
    pub(crate) fn debug_assert_retained_layout_invariants(&self) {
        debug_assert!(
            self.retained_layout_scope_stack.is_empty(),
            "retained layout scopes must not cross frame boundaries"
        );

        let mut tracked_nodes = FxHashSet::default();
        for node in &self.scratch_nodes {
            debug_assert!(
                tracked_nodes.insert(*node),
                "layout node must not be tracked twice"
            );
        }
        for nodes in self.retained_layout_nodes_by_scope.values() {
            for node in nodes {
                debug_assert!(
                    tracked_nodes.insert(*node),
                    "layout node must not be tracked twice"
                );
            }
        }
        for (scope, root) in &self.retained_layout_root_by_scope {
            let nodes = self
                .retained_layout_nodes_by_scope
                .get(scope)
                .expect("retained layout root must belong to a retained scope");
            debug_assert!(
                nodes.contains(root),
                "retained layout root must be tracked in its retained scope"
            );
        }
        for nodes in self.retained_layout_nodes_by_scope.values() {
            for node in nodes {
                debug_assert!(
                    self.retained_layout_node_kinds.contains_key(node),
                    "retained layout node must have a recorded node kind"
                );
            }
        }
        debug_assert_eq!(
            self.taffy.total_node_count(),
            tracked_nodes.len(),
            "every live taffy node must be owned by scratch or retained layout tracking"
        );
        for parent in &tracked_nodes {
            for child in self.taffy.children((*parent).into()).expect(EXPECT_MESSAGE) {
                let child = LayoutId::from(child);
                debug_assert!(
                    tracked_nodes.contains(&child),
                    "every taffy child edge must point to a tracked live layout node"
                );
                debug_assert_eq!(
                    self.taffy.parent(child.into()),
                    Some((*parent).into()),
                    "every taffy child edge must agree with the child's parent edge"
                );
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_layout_node_creates(&self) -> usize {
        self.retained_layout_node_creates
    }

    #[cfg(test)]
    pub(crate) fn retained_layout_node_count(&self) -> usize {
        self.retained_layout_nodes_by_scope
            .values()
            .map(Vec::len)
            .sum()
    }

    fn next_retained_layout_node(&mut self) -> Option<Option<LayoutId>> {
        let active_scope = self.retained_layout_scope_stack.last_mut()?;
        let scope = active_scope.id;
        let node_index = active_scope.next_node_index;
        active_scope.next_node_index += 1;

        let nodes = self
            .retained_layout_nodes_by_scope
            .entry(scope)
            .or_default();
        Some(nodes.get(node_index).copied())
    }

    fn track_new_layout_node(&mut self, layout_id: LayoutId, kind: RetainedLayoutNodeKind) {
        if let Some(active_scope) = self.retained_layout_scope_stack.last() {
            self.retained_layout_nodes_by_scope
                .entry(active_scope.id)
                .or_default()
                .push(layout_id);
            self.retained_layout_node_kinds.insert(layout_id, kind);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.retained_layout_node_creates += 1;
            }
        } else {
            self.scratch_nodes.push(layout_id);
        }
    }

    fn remove_node(&mut self, node: LayoutId) {
        self.retained_layout_node_kinds.remove(&node);
        self.taffy
            .set_node_context(node.into(), None)
            .expect(EXPECT_MESSAGE);
        self.taffy.remove(node.into()).expect(EXPECT_MESSAGE);
    }

    fn node_children_match(&self, node: LayoutId, children: &[LayoutId]) -> bool {
        self.taffy.child_count(node.into()) == children.len()
            && children.iter().enumerate().all(|(ix, child)| {
                self.taffy
                    .child_at_index(node.into(), ix)
                    .expect(EXPECT_MESSAGE)
                    == (*child).into()
            })
    }

    fn update_retained_layout_node(
        &mut self,
        layout_id: LayoutId,
        style: taffy::style::Style,
        children: &[LayoutId],
    ) {
        let style_changed = self.taffy.style(layout_id.into()).expect(EXPECT_MESSAGE) != &style;
        let children_changed = !self.node_children_match(layout_id, children);
        trace_retained_layout!(
            "gpui retained taffy reuse-unmeasured node={:?} old_kind={:?} style_changed={} children_changed={} children={:?}",
            layout_id,
            self.retained_layout_node_kinds.get(&layout_id),
            style_changed,
            children_changed,
            children,
        );
        if style_changed {
            self.taffy
                .set_style(layout_id.into(), style)
                .expect(EXPECT_MESSAGE);
        }
        if self
            .retained_layout_node_kinds
            .insert(layout_id, RetainedLayoutNodeKind::Unmeasured)
            == Some(RetainedLayoutNodeKind::Measured)
        {
            self.taffy
                .set_node_context(layout_id.into(), None)
                .expect(EXPECT_MESSAGE);
        }
        if children_changed {
            self.taffy
                .set_children(layout_id.into(), LayoutId::to_taffy_slice(children))
                .expect(EXPECT_MESSAGE);
        }
        self.taffy
            .mark_dirty(layout_id.into())
            .expect(EXPECT_MESSAGE);
    }

    fn update_retained_measured_layout_node(
        &mut self,
        layout_id: LayoutId,
        style: taffy::style::Style,
        measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &mut App,
        ) -> Size<Pixels>
        + 'static,
    ) {
        let style_changed = self.taffy.style(layout_id.into()).expect(EXPECT_MESSAGE) != &style;
        trace_retained_layout!(
            "gpui retained taffy reuse-measured node={:?} old_kind={:?} style_changed={}",
            layout_id,
            self.retained_layout_node_kinds.get(&layout_id),
            style_changed,
        );
        if style_changed {
            self.taffy
                .set_style(layout_id.into(), style)
                .expect(EXPECT_MESSAGE);
        }
        self.taffy
            .set_node_context(
                layout_id.into(),
                Some(NodeContext {
                    measure: StackSafe::new(Box::new(measure)),
                }),
            )
            .expect(EXPECT_MESSAGE);
        self.retained_layout_node_kinds
            .insert(layout_id, RetainedLayoutNodeKind::Measured);
        if !self.node_children_match(layout_id, &[]) {
            self.taffy
                .set_children(layout_id.into(), &[])
                .expect(EXPECT_MESSAGE);
        }
        self.taffy
            .mark_dirty(layout_id.into())
            .expect(EXPECT_MESSAGE);
    }

    pub fn request_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        children: &[LayoutId],
    ) -> LayoutId {
        let taffy_style = style.to_taffy(rem_size, scale_factor);
        self.layout_work.layout_node_requests += 1;
        self.layout_work.child_edges += children.len() as u64;

        match self.next_retained_layout_node() {
            Some(Some(layout_id)) => {
                self.update_retained_layout_node(layout_id, taffy_style, children);
                layout_id
            }
            Some(None) | None => {
                let layout_id: LayoutId = self
                    .taffy
                    .new_leaf(taffy_style)
                    .expect(EXPECT_MESSAGE)
                    .into();
                if !children.is_empty() {
                    self.taffy
                        .set_children(layout_id.into(), LayoutId::to_taffy_slice(children))
                        .expect(EXPECT_MESSAGE);
                }
                trace_retained_layout!(
                    "gpui retained taffy create-unmeasured node={:?} children={:?} retained_scope_active={}",
                    layout_id,
                    children,
                    self.retained_layout_scope_stack.last().is_some(),
                );
                self.track_new_layout_node(layout_id, RetainedLayoutNodeKind::Unmeasured);
                layout_id
            }
        }
    }

    pub fn request_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &mut App,
        ) -> Size<Pixels>
        + 'static,
    ) -> LayoutId {
        let taffy_style = style.to_taffy(rem_size, scale_factor);
        self.layout_work.measured_layout_node_requests += 1;

        match self.next_retained_layout_node() {
            Some(Some(layout_id)) => {
                self.update_retained_measured_layout_node(layout_id, taffy_style, measure);
                layout_id
            }
            Some(None) | None => {
                let layout_id = self
                    .taffy
                    .new_leaf_with_context(
                        taffy_style,
                        NodeContext {
                            measure: StackSafe::new(Box::new(measure)),
                        },
                    )
                    .expect(EXPECT_MESSAGE)
                    .into();
                trace_retained_layout!(
                    "gpui retained taffy create-measured node={:?} retained_scope_active={}",
                    layout_id,
                    self.retained_layout_scope_stack.last().is_some(),
                );
                self.track_new_layout_node(layout_id, RetainedLayoutNodeKind::Measured);
                layout_id
            }
        }
    }

    // Used to understand performance
    #[allow(dead_code)]
    fn count_all_children(&self, parent: LayoutId) -> anyhow::Result<u32> {
        let mut count = 0;

        for child in self.taffy.children(parent.0)? {
            // Count this child.
            count += 1;

            // Count all of this child's children.
            count += self.count_all_children(LayoutId(child))?
        }

        Ok(count)
    }

    // Used to understand performance
    #[allow(dead_code)]
    fn max_depth(&self, depth: u32, parent: LayoutId) -> anyhow::Result<u32> {
        println!(
            "{parent:?} at depth {depth} has {} children",
            self.taffy.child_count(parent.0)
        );

        let mut max_child_depth = 0;

        for child in self.taffy.children(parent.0)? {
            max_child_depth = std::cmp::max(max_child_depth, self.max_depth(0, LayoutId(child))?);
        }

        Ok(depth + 1 + max_child_depth)
    }

    // Used to understand performance
    #[allow(dead_code)]
    fn get_edges(&self, parent: LayoutId) -> anyhow::Result<Vec<(LayoutId, LayoutId)>> {
        let mut edges = Vec::new();

        for child in self.taffy.children(parent.0)? {
            edges.push((parent, LayoutId(child)));

            edges.extend(self.get_edges(LayoutId(child))?);
        }

        Ok(edges)
    }

    fn clear_absolute_layout_bounds_for_subtree(&mut self, root: LayoutId) {
        let stack = &mut self.layout_bounds_scratch_space;
        stack.push(root);
        while let Some(id) = stack.pop() {
            self.absolute_layout_bounds.remove(&id);
            self.absolute_outer_origins.remove(&id);
            self.computed_layout_roots.remove(&id);
            stack.extend(
                self.taffy
                    .children(id.into())
                    .expect(EXPECT_MESSAGE)
                    .into_iter()
                    .map(LayoutId::from),
            );
        }
    }

    #[stacksafe]
    pub fn compute_layout(
        &mut self,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) {
        trace_retained_layout!(
            "gpui retained taffy compute root={:?} available={:?}",
            id,
            available_space,
        );
        // A retained node can be computed as a temporary root and later as a
        // descendant of another root in the same frame. Absolute bounds depend
        // on that root/available-space context, so clear the reachable subtree
        // before every compute.
        self.clear_absolute_layout_bounds_for_subtree(id);
        self.computed_layout_roots.insert(id);

        // Leaving this here until we have a better instrumentation approach.
        // println!("Laying out {} children", self.count_all_children(id)?);
        // println!("Max layout depth: {}", self.max_depth(0, id)?);

        // Output the edges (branches) of the tree in Mermaid format for visualization.
        // println!("Edges:");
        // for (a, b) in self.get_edges(id)? {
        //     println!("N{} --> N{}", u64::from(a), u64::from(b));
        // }
        //

        let scale_factor = window.scale_factor();

        let transform = |v: AvailableSpace| match v {
            AvailableSpace::Definite(pixels) => {
                AvailableSpace::Definite(Pixels(pixels.0 * scale_factor))
            }
            AvailableSpace::MinContent => AvailableSpace::MinContent,
            AvailableSpace::MaxContent => AvailableSpace::MaxContent,
        };
        let available_space = size(
            transform(available_space.width),
            transform(available_space.height),
        );

        self.layout_work.compute_layout_calls += 1;
        let compute_start = std::time::Instant::now();
        let mut measured_layout_calls = 0;
        let mut measured_layout_duration = Duration::default();

        self.taffy
            .compute_layout_with_measure(
                id.into(),
                available_space.into(),
                |known_dimensions, available_space, _id, node_context, _style| {
                    let Some(node_context) = node_context else {
                        return taffy::geometry::Size::default();
                    };

                    let known_dimensions = Size {
                        width: known_dimensions.width.map(|e| Pixels(e / scale_factor)),
                        height: known_dimensions.height.map(|e| Pixels(e / scale_factor)),
                    };

                    let available_space: Size<AvailableSpace> = available_space.into();
                    let untransform = |ev: AvailableSpace| match ev {
                        AvailableSpace::Definite(pixels) => {
                            AvailableSpace::Definite(Pixels(pixels.0 / scale_factor))
                        }
                        AvailableSpace::MinContent => AvailableSpace::MinContent,
                        AvailableSpace::MaxContent => AvailableSpace::MaxContent,
                    };
                    let available_space = size(
                        untransform(available_space.width),
                        untransform(available_space.height),
                    );

                    measured_layout_calls += 1;
                    let measure_start = std::time::Instant::now();
                    let measured_size: Size<Pixels> =
                        (node_context.measure)(known_dimensions, available_space, window, cx);
                    measured_layout_duration += measure_start.elapsed();
                    snap_measured_size_to_device_pixels(measured_size, scale_factor).into()
                },
            )
            .expect(EXPECT_MESSAGE);

        self.layout_work.compute_layout_duration += compute_start.elapsed();
        self.layout_work.measured_layout_calls += measured_layout_calls;
        self.layout_work.measured_layout_duration += measured_layout_duration;
    }

    // Pixel snapping
    //
    // Painting primitives at non-integer pixel coordinates produces blurry
    // output. Pixel snapping converts layout coordinates into integer
    // device-pixel coordinates so painted edges land exactly on physical
    // pixel boundaries.
    //
    // Non-integer coordinates can arise for several reasons, including:
    //   - flex distribution, percentages, centering, and text measurement
    //     can produce fractional element sizes and positions;
    //   - at fractional scale factors (for example 125% or 150%), integer
    //     logical-pixel values can map to non-integer device-pixel values.
    //
    // We pixel-snap by rounding in device-pixel space, after multiplying
    // by `scale_factor`, so that snapping targets physical pixels. Bounds
    // are divided by `scale_factor` before being returned to GPUI.
    //
    // Midpoints are rounded toward zero. This is a stylistic choice: a
    // 1-logical-pixel line at 150% scale should render as 1 dp rather than
    // 2 dp.
    //
    // Pixel snapping is done in two phases:
    //
    //  1. Pre-layout metric snapping. Before Taffy computes layout, all
    //     authored absolute lengths are rounded in `to_taffy`. This
    //     includes borders, padding, gaps, and explicit sizes.
    //     Custom-measured leaf nodes have their measured sizes rounded up
    //     to integer device-pixel lengths.
    //
    //  2. Post-layout edge snapping. After Taffy resolves the tree, layout
    //     relationships such as flex shares, grid tracks, percentages, and
    //     centering can produce new fractional edge positions. Boxes now
    //     have edges in absolute coordinates, and snapping must decide
    //     where those edges land on the device-pixel grid.
    //
    // Ideally, post-layout snapping would satisfy:
    //
    //  - Edge closure. Two raw layout edges at the same absolute position
    //    should snap to the same pixel column.
    //  - Translation stability. A component's internal geometry should not
    //    change when it moves to a new absolute position.
    //
    // These goals are in tension because rounding is not associative.
    // The simple local schemes make different tradeoffs:
    //
    //  - Absolute edge rounding gives each window coordinate one answer,
    //    so coincident edges always close globally. But a span's snapped
    //    length is `round(far) - round(near)`, which may change by 1 dp
    //    as its absolute origin moves.
    //
    //  - Parent-relative edge rounding rounds each child inside its
    //    parent's coordinate space. This guarantees translation stability,
    //    but a shared edge reached through different parents can
    //    accumulate different rounding, causing non-closure between
    //    cousins.
    //
    //  - Length rounding rounds each width, height, and thickness
    //    independently and then places boxes from those rounded lengths.
    //    Sizes stay stable under translation, but neighboring boxes derive
    //    their shared boundary from different sources, so closure is not
    //    guaranteed.
    //
    // We apply absolute edge rounding for each element's outer box in
    // post-layout rounding to preserve closure. Border and padding widths
    // are not touched by post-layout rounding; they keep their pre-layout
    // rounded value so that they remain stable under translation.
    //
    // This gives both closure and translation stability in the case that
    // all local metrics are integer device-pixel lengths. Pre-layout
    // rounding covers that in most cases. The exception is metrics
    // resolved by layout relationships, such as percentages. Outer box
    // edges will still close globally, and painted border widths are still
    // snapped independently, but the raw content-box origin can carry a
    // 1dp residual into descendants.

    pub fn layout_bounds(&mut self, id: LayoutId, scale_factor: f32) -> Bounds<Pixels> {
        if let Some(layout) = self.absolute_layout_bounds.get(&id).cloned() {
            return layout;
        }

        let layout = self.taffy.layout(id.into()).expect(EXPECT_MESSAGE);
        let layout_location = layout.location;
        let layout_size = layout.size;
        let parent = if self.computed_layout_roots.contains(&id) {
            None
        } else {
            self.taffy.parent(id.0)
        };

        let absolute_outer_origin = match parent {
            Some(parent_id) => {
                let parent_id = LayoutId::from(parent_id);
                self.layout_bounds(parent_id, scale_factor);
                let parent_origin = *self
                    .absolute_outer_origins
                    .get(&parent_id)
                    .expect("parent absolute outer origin should be cached");
                parent_origin + Point::from(layout_location)
            }
            None => Point::from(layout_location),
        };
        self.absolute_outer_origins
            .insert(id, absolute_outer_origin);

        let absolute_far = absolute_outer_origin + Point::from(Size::from(layout_size));
        let snapped_bounds = Bounds::from_corners(
            absolute_outer_origin.map(round_half_toward_zero),
            absolute_far.map(round_half_toward_zero),
        );

        let bounds = (snapped_bounds / scale_factor).map(Pixels);
        if bounds.size.width == Pixels::ZERO || bounds.size.height == Pixels::ZERO {
            trace_retained_layout!(
                "gpui retained taffy zero-bounds node={:?} parent={:?} raw_location={:?} raw_size={:?} bounds={:?} children={:?}",
                id,
                parent.map(LayoutId::from),
                layout_location,
                layout_size,
                bounds,
                self.taffy
                    .children(id.into())
                    .expect(EXPECT_MESSAGE)
                    .into_iter()
                    .map(LayoutId::from)
                    .collect::<Vec<_>>(),
            );
        }
        self.absolute_layout_bounds.insert(id, bounds);
        bounds
    }
}

/// A unique identifier for a layout node, generated when requesting a layout from Taffy
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
#[repr(transparent)]
pub struct LayoutId(NodeId);

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RetainedLayoutScopeId(u64);

impl RetainedLayoutScopeId {
    pub(crate) fn new(id: u64) -> Self {
        Self(id)
    }
}

impl LayoutId {
    fn to_taffy_slice(node_ids: &[Self]) -> &[taffy::NodeId] {
        // SAFETY: LayoutId is repr(transparent) to taffy::tree::NodeId.
        unsafe { std::mem::transmute::<&[LayoutId], &[taffy::NodeId]>(node_ids) }
    }
}

impl std::hash::Hash for LayoutId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        u64::from(self.0).hash(state);
    }
}

impl From<NodeId> for LayoutId {
    fn from(node_id: NodeId) -> Self {
        Self(node_id)
    }
}

impl From<LayoutId> for NodeId {
    fn from(layout_id: LayoutId) -> NodeId {
        layout_id.0
    }
}

fn snap_measured_size_to_device_pixels(size: Size<Pixels>, scale_factor: f32) -> Size<f32> {
    size.map(|d| ceil_to_device_pixel(d.0.max(0.0), scale_factor))
}

fn border_widths_to_taffy(
    widths: &Edges<AbsoluteLength>,
    rem_size: Pixels,
    scale_factor: f32,
) -> TaffyRect<taffy::style::LengthPercentage> {
    let snap = |w: &AbsoluteLength| {
        taffy::style::LengthPercentage::length(round_stroke_to_device_pixel(
            w.to_pixels(rem_size).0,
            scale_factor,
        ))
    };
    TaffyRect {
        top: snap(&widths.top),
        right: snap(&widths.right),
        bottom: snap(&widths.bottom),
        left: snap(&widths.left),
    }
}

trait ToTaffy<Output> {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> Output;
}

impl ToTaffy<taffy::style::Style> for Style {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Style {
        use taffy::style_helpers::{fr, length, minmax, repeat};

        fn to_grid_line(
            placement: &Range<crate::GridPlacement>,
        ) -> taffy::Line<taffy::GridPlacement> {
            taffy::Line {
                start: placement.start.into(),
                end: placement.end.into(),
            }
        }

        fn to_grid_repeat<T: taffy::style::CheapCloneStr>(
            unit: &Option<GridTemplate>,
        ) -> Vec<taffy::GridTemplateComponent<T>> {
            unit.map(|template| {
                match template.min_size {
                    // grid-template-*: repeat(<number>, minmax(0, 1fr));
                    crate::TemplateColumnMinSize::Zero => {
                        vec![repeat(
                            template.repeat,
                            vec![minmax(length(0.0_f32), fr(1.0_f32))],
                        )]
                    }
                    // grid-template-*: repeat(<number>, minmax(min-content, 1fr));
                    crate::TemplateColumnMinSize::MinContent => {
                        vec![repeat(
                            template.repeat,
                            vec![minmax(min_content(), fr(1.0_f32))],
                        )]
                    }
                    // grid-template-*: repeat(<number>, minmax(0, max-content))
                    crate::TemplateColumnMinSize::MaxContent => {
                        vec![repeat(
                            template.repeat,
                            vec![minmax(length(0.0_f32), max_content())],
                        )]
                    }
                }
            })
            .unwrap_or_default()
        }

        taffy::style::Style {
            display: self.display.into(),
            overflow: self.overflow.into(),
            scrollbar_width: self.scrollbar_width.to_taffy(rem_size, scale_factor),
            position: self.position.into(),
            inset: self.inset.to_taffy(rem_size, scale_factor),
            size: self.size.to_taffy(rem_size, scale_factor),
            min_size: self.min_size.to_taffy(rem_size, scale_factor),
            max_size: self.max_size.to_taffy(rem_size, scale_factor),
            aspect_ratio: self.aspect_ratio,
            margin: self.margin.to_taffy(rem_size, scale_factor),
            padding: self.padding.to_taffy(rem_size, scale_factor),
            border: border_widths_to_taffy(&self.border_widths, rem_size, scale_factor),
            align_items: self.align_items.map(|x| x.into()),
            align_self: self.align_self.map(|x| x.into()),
            align_content: self.align_content.map(|x| x.into()),
            justify_content: self.justify_content.map(|x| x.into()),
            gap: self.gap.to_taffy(rem_size, scale_factor),
            flex_direction: self.flex_direction.into(),
            flex_wrap: self.flex_wrap.into(),
            flex_basis: self.flex_basis.to_taffy(rem_size, scale_factor),
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink,
            grid_template_rows: to_grid_repeat(&self.grid_rows),
            grid_template_columns: to_grid_repeat(&self.grid_cols),
            grid_row: self
                .grid_location
                .as_ref()
                .map(|location| to_grid_line(&location.row))
                .unwrap_or_default(),
            grid_column: self
                .grid_location
                .as_ref()
                .map(|location| to_grid_line(&location.column))
                .unwrap_or_default(),
            ..Default::default()
        }
    }
}

impl ToTaffy<f32> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> f32 {
        round_to_device_pixel(self.to_pixels(rem_size).0, scale_factor)
    }
}

impl ToTaffy<taffy::style::LengthPercentageAuto> for Length {
    fn to_taffy(
        &self,
        rem_size: Pixels,
        scale_factor: f32,
    ) -> taffy::prelude::LengthPercentageAuto {
        match self {
            Length::Definite(length) => length.to_taffy(rem_size, scale_factor),
            Length::Auto => taffy::prelude::LengthPercentageAuto::auto(),
        }
    }
}

impl ToTaffy<taffy::style::Dimension> for Length {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::prelude::Dimension {
        match self {
            Length::Definite(length) => length.to_taffy(rem_size, scale_factor),
            Length::Auto => taffy::prelude::Dimension::auto(),
        }
    }
}

impl ToTaffy<taffy::style::LengthPercentage> for DefiniteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentage {
        match self {
            DefiniteLength::Absolute(length) => length.to_taffy(rem_size, scale_factor),
            DefiniteLength::Fraction(fraction) => {
                taffy::style::LengthPercentage::percent(*fraction)
            }
        }
    }
}

impl ToTaffy<taffy::style::LengthPercentageAuto> for DefiniteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentageAuto {
        match self {
            DefiniteLength::Absolute(length) => length.to_taffy(rem_size, scale_factor),
            DefiniteLength::Fraction(fraction) => {
                taffy::style::LengthPercentageAuto::percent(*fraction)
            }
        }
    }
}

impl ToTaffy<taffy::style::Dimension> for DefiniteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Dimension {
        match self {
            DefiniteLength::Absolute(length) => length.to_taffy(rem_size, scale_factor),
            DefiniteLength::Fraction(fraction) => taffy::style::Dimension::percent(*fraction),
        }
    }
}

impl ToTaffy<taffy::style::LengthPercentage> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentage {
        taffy::style::LengthPercentage::length(self.to_taffy(rem_size, scale_factor))
    }
}

impl ToTaffy<taffy::style::LengthPercentageAuto> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentageAuto {
        taffy::style::LengthPercentageAuto::length(self.to_taffy(rem_size, scale_factor))
    }
}

impl ToTaffy<taffy::style::Dimension> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Dimension {
        taffy::style::Dimension::length(self.to_taffy(rem_size, scale_factor))
    }
}

impl<T, T2> From<TaffyPoint<T>> for Point<T2>
where
    T: Into<T2>,
    T2: Clone + Debug + Default + PartialEq,
{
    fn from(point: TaffyPoint<T>) -> Point<T2> {
        Point {
            x: point.x.into(),
            y: point.y.into(),
        }
    }
}

impl<T, T2> From<Point<T>> for TaffyPoint<T2>
where
    T: Into<T2> + Clone + Debug + Default + PartialEq,
{
    fn from(val: Point<T>) -> Self {
        TaffyPoint {
            x: val.x.into(),
            y: val.y.into(),
        }
    }
}

impl<T, U> ToTaffy<TaffySize<U>> for Size<T>
where
    T: ToTaffy<U> + Clone + Debug + Default + PartialEq,
{
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> TaffySize<U> {
        TaffySize {
            width: self.width.to_taffy(rem_size, scale_factor),
            height: self.height.to_taffy(rem_size, scale_factor),
        }
    }
}

impl<T, U> ToTaffy<TaffyRect<U>> for Edges<T>
where
    T: ToTaffy<U> + Clone + Debug + Default + PartialEq,
{
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> TaffyRect<U> {
        TaffyRect {
            top: self.top.to_taffy(rem_size, scale_factor),
            right: self.right.to_taffy(rem_size, scale_factor),
            bottom: self.bottom.to_taffy(rem_size, scale_factor),
            left: self.left.to_taffy(rem_size, scale_factor),
        }
    }
}

impl<T, U> From<TaffySize<T>> for Size<U>
where
    T: Into<U>,
    U: Clone + Debug + Default + PartialEq,
{
    fn from(taffy_size: TaffySize<T>) -> Self {
        Size {
            width: taffy_size.width.into(),
            height: taffy_size.height.into(),
        }
    }
}

impl<T, U> From<Size<T>> for TaffySize<U>
where
    T: Into<U> + Clone + Debug + Default + PartialEq,
{
    fn from(size: Size<T>) -> Self {
        TaffySize {
            width: size.width.into(),
            height: size.height.into(),
        }
    }
}

/// The space available for an element to be laid out in
#[derive(Copy, Clone, Default, Debug, Eq, PartialEq)]
pub enum AvailableSpace {
    /// The amount of space available is the specified number of pixels
    Definite(Pixels),
    /// The amount of space available is indefinite and the node should be laid out under a min-content constraint
    #[default]
    MinContent,
    /// The amount of space available is indefinite and the node should be laid out under a max-content constraint
    MaxContent,
}

impl AvailableSpace {
    /// Returns a `Size` with both width and height set to `AvailableSpace::MinContent`.
    ///
    /// This function is useful when you want to create a `Size` with the minimum content constraints
    /// for both dimensions.
    ///
    /// # Examples
    ///
    /// ```
    /// use gpui::AvailableSpace;
    /// let min_content_size = AvailableSpace::min_size();
    /// assert_eq!(min_content_size.width, AvailableSpace::MinContent);
    /// assert_eq!(min_content_size.height, AvailableSpace::MinContent);
    /// ```
    pub const fn min_size() -> Size<Self> {
        Size {
            width: Self::MinContent,
            height: Self::MinContent,
        }
    }
}

impl From<AvailableSpace> for TaffyAvailableSpace {
    fn from(space: AvailableSpace) -> TaffyAvailableSpace {
        match space {
            AvailableSpace::Definite(Pixels(value)) => TaffyAvailableSpace::Definite(value),
            AvailableSpace::MinContent => TaffyAvailableSpace::MinContent,
            AvailableSpace::MaxContent => TaffyAvailableSpace::MaxContent,
        }
    }
}

impl From<TaffyAvailableSpace> for AvailableSpace {
    fn from(space: TaffyAvailableSpace) -> AvailableSpace {
        match space {
            TaffyAvailableSpace::Definite(value) => AvailableSpace::Definite(Pixels(value)),
            TaffyAvailableSpace::MinContent => AvailableSpace::MinContent,
            TaffyAvailableSpace::MaxContent => AvailableSpace::MaxContent,
        }
    }
}

impl From<Pixels> for AvailableSpace {
    fn from(pixels: Pixels) -> Self {
        AvailableSpace::Definite(pixels)
    }
}

impl From<Size<Pixels>> for Size<AvailableSpace> {
    fn from(size: Size<Pixels>) -> Self {
        Size {
            width: AvailableSpace::Definite(size.width),
            height: AvailableSpace::Definite(size.height),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppContext as _;
    use std::{cell::Cell, ops::Deref as _, rc::Rc, time::Duration};

    #[test]
    fn border_widths_to_taffy_use_stroke_snapping() {
        let border_widths = Edges {
            top: Pixels(0.0).into(),
            right: Pixels(0.4).into(),
            bottom: Pixels(0.5).into(),
            left: Pixels(1.6).into(),
        };
        let taffy_border = border_widths_to_taffy(&border_widths, Pixels(16.0), 1.0);

        assert_eq!(
            taffy_border.top,
            taffy::style::LengthPercentage::length(0.0)
        );
        assert_eq!(
            taffy_border.right,
            taffy::style::LengthPercentage::length(1.0)
        );
        assert_eq!(
            taffy_border.bottom,
            taffy::style::LengthPercentage::length(1.0)
        );
        assert_eq!(
            taffy_border.left,
            taffy::style::LengthPercentage::length(2.0)
        );
    }

    #[test]
    fn layout_work_sample_counts_requests_and_finish_frame_resets() {
        let mut engine = TaffyLayoutEngine::new();
        let first_child = engine.request_layout(Style::default(), Pixels(16.0), 1.0, &[]);
        let second_child = engine.request_layout(Style::default(), Pixels(16.0), 1.0, &[]);
        engine.request_layout(
            Style::default(),
            Pixels(16.0),
            1.0,
            &[first_child, second_child],
        );

        let expected_sample = LayoutWorkSample {
            draw_index: 0,
            layout_node_requests: 3,
            measured_layout_node_requests: 0,
            child_edges: 2,
            compute_layout_calls: 0,
            measured_layout_calls: 0,
            compute_layout_duration: Duration::default(),
            measured_layout_duration: Duration::default(),
        };
        assert_eq!(engine.layout_work_sample(), expected_sample);
        assert_eq!(engine.finish_frame(), expected_sample);
        assert_eq!(engine.layout_work_sample(), LayoutWorkSample::default());
    }

    #[test]
    fn retained_child_reparented_by_scratch_parent_detaches_from_old_retained_parent() {
        let mut engine = TaffyLayoutEngine::new();
        let child_scope = RetainedLayoutScopeId::new(0);
        let parent_scope = RetainedLayoutScopeId::new(1);

        engine.begin_retained_layout_scope(child_scope);
        let child = engine.request_layout(Style::default(), Pixels(16.0), 1.0, &[]);
        assert!(engine.finish_retained_layout_scope(child_scope, child));

        engine.begin_retained_layout_scope(parent_scope);
        let retained_parent = engine.request_layout(Style::default(), Pixels(16.0), 1.0, &[child]);
        assert!(engine.finish_retained_layout_scope(parent_scope, retained_parent));
        assert_eq!(
            engine.taffy.parent(child.into()),
            Some(retained_parent.into())
        );

        let scratch_parent = engine.request_layout(Style::default(), Pixels(16.0), 1.0, &[child]);
        assert_eq!(
            engine.taffy.parent(child.into()),
            Some(scratch_parent.into())
        );
        assert_eq!(
            engine
                .taffy
                .children(retained_parent.into())
                .expect(EXPECT_MESSAGE),
            Vec::<taffy::NodeId>::new()
        );

        engine.finish_frame();
        engine.remove_retained_layout_scope(child_scope);

        engine.begin_retained_layout_scope(parent_scope);
        let retained_parent = engine.request_layout(Style::default(), Pixels(16.0), 1.0, &[]);
        assert!(engine.finish_retained_layout_scope(parent_scope, retained_parent));
        assert_eq!(
            engine
                .taffy
                .children(retained_parent.into())
                .expect(EXPECT_MESSAGE),
            Vec::<taffy::NodeId>::new()
        );
    }

    #[test]
    fn retained_scope_revalidation_dirties_reused_nodes_even_when_descriptor_is_unchanged() {
        let mut test_app = crate::TestAppContext::single();
        let window = test_app.add_window(|_, _| crate::Empty);

        test_app
            .update_window(*window.deref(), |_, window, cx| {
                let mut engine = TaffyLayoutEngine::new();
                let scope = RetainedLayoutScopeId::new(0);
                let scale_factor = window.scale_factor();

                engine.begin_retained_layout_scope(scope);
                let child =
                    engine.request_layout(Style::default(), Pixels(16.0), scale_factor, &[]);
                let root =
                    engine.request_layout(Style::default(), Pixels(16.0), scale_factor, &[child]);
                assert!(engine.finish_retained_layout_scope(scope, root));

                engine.compute_layout(
                    root,
                    size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                    window,
                    cx,
                );
                assert_eq!(
                    (
                        engine.taffy.dirty(child.into()).expect(EXPECT_MESSAGE),
                        engine.taffy.dirty(root.into()).expect(EXPECT_MESSAGE),
                    ),
                    (false, false),
                );

                engine.begin_retained_layout_scope(scope);
                let reused_child =
                    engine.request_layout(Style::default(), Pixels(16.0), scale_factor, &[]);
                let reused_root = engine.request_layout(
                    Style::default(),
                    Pixels(16.0),
                    scale_factor,
                    &[reused_child],
                );
                assert_eq!((reused_child, reused_root), (child, root));
                assert!(engine.finish_retained_layout_scope(scope, reused_root));

                assert_eq!(
                    (
                        engine
                            .taffy
                            .dirty(reused_child.into())
                            .expect(EXPECT_MESSAGE),
                        engine
                            .taffy
                            .dirty(reused_root.into())
                            .expect(EXPECT_MESSAGE),
                    ),
                    (true, true),
                );
            })
            .unwrap();
    }

    #[test]
    fn compute_layout_clears_cached_descendant_bounds_from_previous_root_compute() {
        let mut test_app = crate::TestAppContext::single();
        let window = test_app.add_window(|_, _| crate::Empty);

        test_app
            .update_window(*window.deref(), |_, window, cx| {
                let mut engine = TaffyLayoutEngine::new();
                let scale_factor = window.scale_factor();

                let mut child_style = Style::default();
                child_style.size = size(
                    Length::Definite(DefiniteLength::Fraction(1.0)),
                    Length::Definite(DefiniteLength::Fraction(1.0)),
                );
                let child = engine.request_layout(child_style, Pixels(16.0), scale_factor, &[]);

                let mut previous_parent_style = Style::default();
                previous_parent_style.size = size(Pixels(100.0).into(), Pixels(100.0).into());
                let previous_parent = engine.request_layout(
                    previous_parent_style,
                    Pixels(16.0),
                    scale_factor,
                    &[child],
                );
                assert_eq!(
                    engine.taffy.parent(child.into()),
                    Some(previous_parent.into())
                );

                engine.compute_layout(
                    child,
                    size(
                        AvailableSpace::Definite(Pixels(0.0)),
                        AvailableSpace::Definite(Pixels(571.5)),
                    ),
                    window,
                    cx,
                );
                assert_eq!(
                    engine.layout_bounds(child, scale_factor),
                    Bounds {
                        origin: Point::default(),
                        size: size(Pixels(0.0), Pixels(571.5)),
                    }
                );

                let mut parent_style = Style::default();
                parent_style.size = size(Pixels(1077.0).into(), Pixels(0.0).into());
                let parent =
                    engine.request_layout(parent_style, Pixels(16.0), scale_factor, &[child]);

                engine.compute_layout(
                    parent,
                    size(
                        AvailableSpace::Definite(Pixels(1077.0)),
                        AvailableSpace::Definite(Pixels(0.0)),
                    ),
                    window,
                    cx,
                );

                assert_eq!(
                    engine.layout_bounds(parent, scale_factor),
                    Bounds {
                        origin: Point::default(),
                        size: size(Pixels(1077.0), Pixels(0.0)),
                    }
                );
                assert_eq!(
                    engine.layout_bounds(child, scale_factor),
                    Bounds {
                        origin: Point::default(),
                        size: size(Pixels(1077.0), Pixels(0.0)),
                    }
                );
            })
            .unwrap();
    }

    #[test]
    fn layout_work_sample_counts_compute_and_measure() {
        let measure_invocations = Rc::new(Cell::new(0));
        let measure_invocations_for_closure = measure_invocations.clone();
        let mut test_app = crate::TestAppContext::single();
        let window = test_app.add_window(|_, _| crate::Empty);

        let sample = test_app
            .update_window(*window.deref(), |_, window, cx| {
                let mut engine = TaffyLayoutEngine::new();
                let measured_layout = engine.request_measured_layout(
                    Style::default(),
                    Pixels(16.0),
                    1.0,
                    move |_, _, _, _| {
                        measure_invocations_for_closure
                            .set(measure_invocations_for_closure.get() + 1);
                        std::thread::sleep(Duration::from_micros(1));
                        size(Pixels(10.0), Pixels(20.0))
                    },
                );

                engine.compute_layout(
                    measured_layout,
                    size(
                        AvailableSpace::Definite(Pixels(100.0)),
                        AvailableSpace::Definite(Pixels(100.0)),
                    ),
                    window,
                    cx,
                );
                engine.finish_frame()
            })
            .unwrap();

        assert_eq!(
            sample,
            LayoutWorkSample {
                draw_index: 0,
                layout_node_requests: 0,
                measured_layout_node_requests: 1,
                child_edges: 0,
                compute_layout_calls: 1,
                measured_layout_calls: 1,
                compute_layout_duration: sample.compute_layout_duration,
                measured_layout_duration: sample.measured_layout_duration,
            }
        );
        assert_eq!(measure_invocations.get(), 1);
        assert!(sample.compute_layout_duration >= sample.measured_layout_duration);
        assert!(sample.measured_layout_duration > Duration::default());
    }
}
