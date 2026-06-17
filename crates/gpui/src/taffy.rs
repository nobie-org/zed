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
use std::{
    any::Any,
    fmt::{self, Debug},
    mem,
    ops::Range,
    sync::Arc,
    time::Duration,
};
use taffy::{
    TaffyTree, TraversePartialTree as _,
    geometry::{Point as TaffyPoint, Rect as TaffyRect, Size as TaffySize},
    prelude::{max_content, min_content},
    style::AvailableSpace as TaffyAvailableSpace,
    tree::NodeId,
};

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

struct NodeContext;

#[derive(Clone, Debug, Eq, PartialEq)]
enum LayoutMeasureKey {
    Opaque(u64),
    Stable(MeasureKey),
}

trait MeasureKeyData: Debug {
    fn as_any(&self) -> &dyn Any;
    fn equals(&self, other: &dyn MeasureKeyData) -> bool;
}

impl<T> MeasureKeyData for T
where
    T: Debug + Eq + 'static,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn equals(&self, other: &dyn MeasureKeyData) -> bool {
        other.as_any().downcast_ref::<T>() == Some(self)
    }
}

/// A semantic key for a measured layout node.
///
/// The key must change whenever the measurement's layout-visible meaning can
/// change for the same style, known dimensions, and available space.
#[derive(Clone)]
pub(crate) struct MeasureKey(Arc<dyn MeasureKeyData>);

impl MeasureKey {
    pub(crate) fn new<T>(key: T) -> Self
    where
        T: Debug + Eq + 'static,
    {
        Self(Arc::new(key))
    }
}

impl Debug for MeasureKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialEq for MeasureKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.equals(&*other.0)
    }
}

impl Eq for MeasureKey {}

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
    /// Retained Taffy nodes created while committing descriptors.
    pub taffy_node_creates: u64,
    /// Retained Taffy nodes reused while committing descriptors.
    pub taffy_node_reuses: u64,
    /// Retained Taffy node styles updated while committing descriptors.
    pub taffy_style_updates: u64,
    /// Retained Taffy child lists updated while committing descriptors.
    pub taffy_children_updates: u64,
    /// Retained Taffy measured contexts updated while committing descriptors.
    pub taffy_measure_context_updates: u64,
    /// Retained Taffy measured nodes explicitly dirtied by changed measure keys.
    pub taffy_measure_dirty_marks: u64,
    /// Retained Taffy nodes removed while sweeping replaced descriptors.
    pub taffy_node_removes: u64,
    /// Wall time spent constructing non-measured layout descriptors.
    pub request_layout_duration: Duration,
    /// Wall time spent constructing measured layout descriptors.
    pub request_measured_layout_duration: Duration,
    /// Wall time spent committing descriptors onto retained Taffy nodes.
    pub descriptor_commit_duration: Duration,
    /// Wall time spent computing root layouts.
    pub compute_layout_duration: Duration,
    /// Wall time spent inside measured layout callbacks.
    pub measured_layout_duration: Duration,
}

struct LayoutDescriptor {
    style: taffy::style::Style,
    kind: LayoutDescriptorKind,
}

#[derive(Clone)]
enum LayoutDescriptorKind {
    Unmeasured {
        children: Vec<LayoutId>,
    },
    Measured {
        measure: usize,
        measure_key: LayoutMeasureKey,
    },
}

enum LayoutCommitKind {
    Unmeasured {
        children: Vec<LayoutId>,
    },
    Measured {
        measure: NodeMeasureFn,
        measure_key: LayoutMeasureKey,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetainedLayoutKind {
    Unmeasured,
    Measured,
}

struct RetainedLayoutDescriptor {
    style: taffy::style::Style,
    kind: RetainedLayoutKind,
    measure_key: Option<LayoutMeasureKey>,
}

struct RetainedLayoutNode {
    node_id: NodeId,
    descriptor: RetainedLayoutDescriptor,
    children: Vec<RetainedLayoutNode>,
}

struct CommitResult {
    retained_node: RetainedLayoutNode,
    replaced_previous: Option<RetainedLayoutNode>,
}

struct RetainedCandidate {
    node_id: NodeId,
    previous_descriptor: Option<RetainedLayoutDescriptor>,
    previous_children: Vec<RetainedLayoutNode>,
    replaced_previous: Option<RetainedLayoutNode>,
}

pub struct TaffyLayoutEngine {
    taffy: TaffyTree<NodeContext>,
    layout_descriptors: Vec<LayoutDescriptor>,
    layout_measure_contexts: Vec<Option<NodeMeasureFn>>,
    current_measure_contexts: FxHashMap<NodeId, NodeMeasureFn>,
    next_opaque_measure_key: u64,
    committed_layout_nodes: FxHashMap<LayoutId, NodeId>,
    #[cfg(any(test, debug_assertions))]
    committed_taffy_nodes: FxHashSet<NodeId>,
    retained_roots: Vec<Option<RetainedLayoutNode>>,
    current_roots: Vec<RetainedLayoutNode>,
    root_commit_index: usize,
    absolute_layout_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    /// Unrounded absolute border-box top-left per-node coordinate in device pixels.
    absolute_outer_origins: FxHashMap<NodeId, Point<f32>>,
    computed_layouts: FxHashSet<NodeId>,
    layout_bounds_scratch_space: Vec<NodeId>,
    layout_work: LayoutWorkSample,
}

const EXPECT_MESSAGE: &str = "we should avoid taffy layout errors by construction if possible";

impl TaffyLayoutEngine {
    pub fn new() -> Self {
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        TaffyLayoutEngine {
            taffy,
            layout_descriptors: Vec::new(),
            layout_measure_contexts: Vec::new(),
            current_measure_contexts: FxHashMap::default(),
            next_opaque_measure_key: 0,
            committed_layout_nodes: FxHashMap::default(),
            #[cfg(any(test, debug_assertions))]
            committed_taffy_nodes: FxHashSet::default(),
            retained_roots: Vec::new(),
            current_roots: Vec::new(),
            root_commit_index: 0,
            absolute_layout_bounds: FxHashMap::default(),
            absolute_outer_origins: FxHashMap::default(),
            computed_layouts: FxHashSet::default(),
            layout_bounds_scratch_space: Vec::new(),
            layout_work: LayoutWorkSample::default(),
        }
    }

    pub fn finish_frame(&mut self) -> LayoutWorkSample {
        for retained_root in mem::take(&mut self.retained_roots).into_iter().flatten() {
            self.remove_retained_subtree(retained_root);
        }

        let layout_work = self.layout_work;
        self.retained_roots = self.current_roots.drain(..).map(Some).collect();
        self.layout_descriptors.clear();
        self.layout_measure_contexts.clear();
        self.current_measure_contexts.clear();
        self.committed_layout_nodes.clear();
        #[cfg(any(test, debug_assertions))]
        self.committed_taffy_nodes.clear();
        self.root_commit_index = 0;
        self.absolute_layout_bounds.clear();
        self.absolute_outer_origins.clear();
        self.computed_layouts.clear();
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

    pub fn request_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        children: &[LayoutId],
    ) -> LayoutId {
        let request_start = std::time::Instant::now();
        let taffy_style = style.to_taffy(rem_size, scale_factor);
        self.layout_work.layout_node_requests += 1;
        self.layout_work.child_edges += children.len() as u64;

        let kind = LayoutDescriptorKind::Unmeasured {
            children: children.to_vec(),
        };

        let id = self.push_descriptor(LayoutDescriptor {
            style: taffy_style,
            kind,
        });
        self.layout_work.request_layout_duration += request_start.elapsed();
        id
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
        let measure_key = LayoutMeasureKey::Opaque(self.next_opaque_measure_key);
        self.next_opaque_measure_key = self.next_opaque_measure_key.wrapping_add(1);
        self.request_measured_layout_with_key(style, rem_size, scale_factor, measure_key, measure)
    }

    pub(crate) fn request_keyed_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measure_key: MeasureKey,
        measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &mut App,
        ) -> Size<Pixels>
        + 'static,
    ) -> LayoutId {
        self.request_measured_layout_with_key(
            style,
            rem_size,
            scale_factor,
            LayoutMeasureKey::Stable(measure_key),
            measure,
        )
    }

    fn request_measured_layout_with_key(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measure_key: LayoutMeasureKey,
        measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &mut App,
        ) -> Size<Pixels>
        + 'static,
    ) -> LayoutId {
        let request_start = std::time::Instant::now();
        let taffy_style = style.to_taffy(rem_size, scale_factor);
        self.layout_work.measured_layout_node_requests += 1;
        let measure_id = self.layout_measure_contexts.len();
        self.layout_measure_contexts
            .push(Some(StackSafe::new(Box::new(measure))));

        let id = self.push_descriptor(LayoutDescriptor {
            style: taffy_style,
            kind: LayoutDescriptorKind::Measured {
                measure: measure_id,
                measure_key,
            },
        });
        self.layout_work.request_measured_layout_duration += request_start.elapsed();
        id
    }

    fn push_descriptor(&mut self, descriptor: LayoutDescriptor) -> LayoutId {
        let id = LayoutId(self.layout_descriptors.len());
        self.layout_descriptors.push(descriptor);
        id
    }

    // Used to understand performance
    #[allow(dead_code)]
    fn count_all_children(&self, parent: NodeId) -> anyhow::Result<u32> {
        let mut count = 0;

        for child in self.taffy.children(parent)? {
            // Count this child.
            count += 1;

            // Count all of this child's children.
            count += self.count_all_children(child)?
        }

        Ok(count)
    }

    // Used to understand performance
    #[allow(dead_code)]
    fn max_depth(&self, depth: u32, parent: NodeId) -> anyhow::Result<u32> {
        println!(
            "{parent:?} at depth {depth} has {} children",
            self.taffy.child_count(parent)
        );

        let mut max_child_depth = 0;

        for child in self.taffy.children(parent)? {
            max_child_depth = std::cmp::max(max_child_depth, self.max_depth(0, child)?);
        }

        Ok(depth + 1 + max_child_depth)
    }

    // Used to understand performance
    #[allow(dead_code)]
    fn get_edges(&self, parent: NodeId) -> anyhow::Result<Vec<(NodeId, NodeId)>> {
        let mut edges = Vec::new();

        for child in self.taffy.children(parent)? {
            edges.push((parent, child));

            edges.extend(self.get_edges(child)?);
        }

        Ok(edges)
    }

    fn commit_layout(&mut self, id: LayoutId) -> NodeId {
        if let Some(node_id) = self.committed_layout_nodes.get(&id) {
            return *node_id;
        }

        let commit_start = std::time::Instant::now();
        let retained_root = self
            .retained_roots
            .get_mut(self.root_commit_index)
            .and_then(Option::take);
        self.root_commit_index += 1;

        let CommitResult {
            retained_node,
            replaced_previous,
        } = self.commit_descriptor(id, retained_root);
        if let Some(replaced_previous) = replaced_previous {
            self.remove_retained_subtree(replaced_previous);
        }

        let node_id = retained_node.node_id;
        self.current_roots.push(retained_node);
        self.layout_work.descriptor_commit_duration += commit_start.elapsed();
        #[cfg(any(test, debug_assertions))]
        self.debug_assert_committed_descriptor_matches(id, node_id);
        node_id
    }

    fn commit_root_layout(&mut self, id: LayoutId) -> NodeId {
        let node_id = self.commit_layout(id);
        assert!(
            self.taffy.parent(node_id).is_none(),
            "layout root must not already be committed under a parent"
        );
        node_id
    }

    fn commit_descriptor(
        &mut self,
        id: LayoutId,
        previous: Option<RetainedLayoutNode>,
    ) -> CommitResult {
        assert!(
            !self.committed_layout_nodes.contains_key(&id),
            "layout descriptor should appear only once in a committed layout tree"
        );

        let descriptor = self.descriptor(id);
        let style = descriptor.style.clone();
        let kind = match descriptor.kind.clone() {
            LayoutDescriptorKind::Unmeasured { children } => {
                LayoutCommitKind::Unmeasured { children }
            }
            LayoutDescriptorKind::Measured {
                measure,
                measure_key,
            } => LayoutCommitKind::Measured {
                measure: self.layout_measure_contexts[measure]
                    .take()
                    .expect("measured layout descriptor should be committed only once"),
                measure_key,
            },
        };

        match kind {
            LayoutCommitKind::Unmeasured { children } => {
                let candidate =
                    self.node_for_descriptor(previous, RetainedLayoutKind::Unmeasured, &style);
                self.apply_style_diff(
                    candidate.node_id,
                    candidate.previous_descriptor.as_ref(),
                    &style,
                );

                let previous_child_node_ids = candidate
                    .previous_children
                    .iter()
                    .map(|child| child.node_id)
                    .collect::<Vec<_>>();
                let mut previous_children = candidate.previous_children.into_iter();
                let mut retained_children = Vec::with_capacity(children.len());
                let mut child_node_ids = Vec::with_capacity(children.len());
                let mut detached_subtrees = Vec::new();
                for child in children {
                    let committed_child = self.commit_descriptor(child, previous_children.next());
                    child_node_ids.push(committed_child.retained_node.node_id);
                    retained_children.push(committed_child.retained_node);
                    if let Some(replaced_previous) = committed_child.replaced_previous {
                        detached_subtrees.push(replaced_previous);
                    }
                }
                for unused_child in previous_children {
                    detached_subtrees.push(unused_child);
                }

                self.apply_children_diff(
                    candidate.node_id,
                    &previous_child_node_ids,
                    &child_node_ids,
                );
                for detached_subtree in detached_subtrees {
                    self.remove_retained_subtree(detached_subtree);
                }

                self.committed_layout_nodes.insert(id, candidate.node_id);
                CommitResult {
                    retained_node: RetainedLayoutNode {
                        node_id: candidate.node_id,
                        descriptor: RetainedLayoutDescriptor {
                            style,
                            kind: RetainedLayoutKind::Unmeasured,
                            measure_key: None,
                        },
                        children: retained_children,
                    },
                    replaced_previous: candidate.replaced_previous,
                }
            }
            LayoutCommitKind::Measured {
                measure,
                measure_key,
            } => {
                let candidate =
                    self.node_for_descriptor(previous, RetainedLayoutKind::Measured, &style);
                self.apply_style_diff(
                    candidate.node_id,
                    candidate.previous_descriptor.as_ref(),
                    &style,
                );
                self.apply_measure_diff(
                    candidate.node_id,
                    candidate.previous_descriptor.as_ref(),
                    &measure_key,
                );
                if candidate.previous_descriptor.is_none() {
                    self.taffy
                        .set_node_context(candidate.node_id, Some(NodeContext))
                        .expect(EXPECT_MESSAGE);
                    self.layout_work.taffy_measure_context_updates += 1;
                }
                assert!(
                    self.current_measure_contexts
                        .insert(candidate.node_id, measure)
                        .is_none(),
                    "measured Taffy node should receive one current-frame callback"
                );
                self.committed_layout_nodes.insert(id, candidate.node_id);
                CommitResult {
                    retained_node: RetainedLayoutNode {
                        node_id: candidate.node_id,
                        descriptor: RetainedLayoutDescriptor {
                            style,
                            kind: RetainedLayoutKind::Measured,
                            measure_key: Some(measure_key),
                        },
                        children: Vec::new(),
                    },
                    replaced_previous: candidate.replaced_previous,
                }
            }
        }
    }

    fn descriptor(&self, id: LayoutId) -> &LayoutDescriptor {
        self.layout_descriptors
            .get(id.0)
            .expect("layout descriptor id should come from the current frame")
    }

    #[cfg(any(test, debug_assertions))]
    fn debug_assert_committed_descriptor_matches(&mut self, id: LayoutId, node_id: NodeId) {
        let mut seen = FxHashSet::default();
        self.debug_assert_descriptor_node_matches(id, node_id, None, &mut seen);
        for node_id in seen {
            assert!(
                self.committed_taffy_nodes.insert(node_id),
                "committed Taffy node should appear at only one current frame position"
            );
        }
    }

    #[cfg(any(test, debug_assertions))]
    fn debug_assert_descriptor_node_matches(
        &self,
        id: LayoutId,
        node_id: NodeId,
        expected_parent: Option<NodeId>,
        seen: &mut FxHashSet<NodeId>,
    ) {
        assert!(
            seen.insert(node_id),
            "committed Taffy node should appear at only one current descriptor position"
        );
        assert_eq!(self.taffy.parent(node_id), expected_parent);

        let descriptor = self.descriptor(id);
        assert_eq!(
            self.taffy.style(node_id).expect(EXPECT_MESSAGE),
            &descriptor.style
        );

        match &descriptor.kind {
            LayoutDescriptorKind::Unmeasured { children } => {
                assert!(self.taffy.get_node_context(node_id).is_none());
                let child_node_ids = children
                    .iter()
                    .map(|child| self.committed_layout_nodes[child])
                    .collect::<Vec<_>>();
                assert_eq!(
                    self.taffy.children(node_id).expect(EXPECT_MESSAGE),
                    child_node_ids
                );
                for (child, child_node_id) in children.iter().zip(child_node_ids) {
                    self.debug_assert_descriptor_node_matches(
                        *child,
                        child_node_id,
                        Some(node_id),
                        seen,
                    );
                }
            }
            LayoutDescriptorKind::Measured { .. } => {
                assert!(self.taffy.get_node_context(node_id).is_some());
                assert_eq!(
                    self.taffy.children(node_id).expect(EXPECT_MESSAGE),
                    Vec::<NodeId>::new()
                );
            }
        }
    }

    fn node_for_descriptor(
        &mut self,
        previous: Option<RetainedLayoutNode>,
        kind: RetainedLayoutKind,
        style: &taffy::style::Style,
    ) -> RetainedCandidate {
        match previous {
            Some(previous) if previous.descriptor.kind == kind => {
                self.layout_work.taffy_node_reuses += 1;
                RetainedCandidate {
                    node_id: previous.node_id,
                    previous_descriptor: Some(previous.descriptor),
                    previous_children: previous.children,
                    replaced_previous: None,
                }
            }
            Some(previous) => {
                let node_id = self.taffy.new_leaf(style.clone()).expect(EXPECT_MESSAGE);
                self.layout_work.taffy_node_creates += 1;
                RetainedCandidate {
                    node_id,
                    previous_descriptor: None,
                    previous_children: Vec::new(),
                    replaced_previous: Some(previous),
                }
            }
            None => {
                let node_id = self.taffy.new_leaf(style.clone()).expect(EXPECT_MESSAGE);
                self.layout_work.taffy_node_creates += 1;
                RetainedCandidate {
                    node_id,
                    previous_descriptor: None,
                    previous_children: Vec::new(),
                    replaced_previous: None,
                }
            }
        }
    }

    fn apply_style_diff(
        &mut self,
        node_id: NodeId,
        previous: Option<&RetainedLayoutDescriptor>,
        style: &taffy::style::Style,
    ) {
        if let Some(previous) = previous {
            if previous.style != *style {
                self.taffy
                    .set_style(node_id, style.clone())
                    .expect(EXPECT_MESSAGE);
                self.layout_work.taffy_style_updates += 1;
            }
        }
    }

    fn apply_children_diff(
        &mut self,
        node_id: NodeId,
        previous_children: &[NodeId],
        children: &[NodeId],
    ) {
        if previous_children != children {
            self.taffy
                .set_children(node_id, children)
                .expect(EXPECT_MESSAGE);
            self.layout_work.taffy_children_updates += 1;
        }
    }

    fn apply_measure_diff(
        &mut self,
        node_id: NodeId,
        previous: Option<&RetainedLayoutDescriptor>,
        measure_key: &LayoutMeasureKey,
    ) {
        if let Some(previous) = previous
            && previous.measure_key.as_ref() != Some(measure_key)
        {
            self.taffy.mark_dirty(node_id).expect(EXPECT_MESSAGE);
            self.layout_work.taffy_measure_dirty_marks += 1;
        }
    }

    fn remove_retained_subtree(&mut self, retained_node: RetainedLayoutNode) {
        for child in retained_node.children {
            self.remove_retained_subtree(child);
        }
        if retained_node.descriptor.kind == RetainedLayoutKind::Measured {
            self.taffy
                .set_node_context(retained_node.node_id, None)
                .expect(EXPECT_MESSAGE);
            self.layout_work.taffy_measure_context_updates += 1;
        }
        self.taffy
            .remove(retained_node.node_id)
            .expect(EXPECT_MESSAGE);
        self.layout_work.taffy_node_removes += 1;
    }

    fn invalidate_cached_bounds_for_subtree(&mut self, node_id: NodeId) {
        let stack = &mut self.layout_bounds_scratch_space;
        stack.push(node_id);
        while let Some(node_id) = stack.pop() {
            self.absolute_layout_bounds.remove(&node_id);
            self.absolute_outer_origins.remove(&node_id);
            stack.extend(
                self.taffy
                    .children(node_id)
                    .expect(EXPECT_MESSAGE)
                    .into_iter(),
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
        let node_id = self.commit_root_layout(id);
        // Leaving this here until we have a better instrumentation approach.
        // println!("Laying out {} children", self.count_all_children(id)?);
        // println!("Max layout depth: {}", self.max_depth(0, id)?);

        // Output the edges (branches) of the tree in Mermaid format for visualization.
        // println!("Edges:");
        // for (a, b) in self.get_edges(id)? {
        //     println!("N{} --> N{}", u64::from(a), u64::from(b));
        // }
        //

        if !self.computed_layouts.insert(node_id) {
            self.invalidate_cached_bounds_for_subtree(node_id);
        }

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

        let current_measure_contexts = &mut self.current_measure_contexts;

        self.taffy
            .compute_layout_with_measure(
                node_id,
                available_space.into(),
                |known_dimensions, available_space, id, node_context, _style| {
                    let Some(node_context) = node_context else {
                        return taffy::geometry::Size::default();
                    };
                    let _node_context = node_context;

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
                    let measure = current_measure_contexts
                        .get_mut(&id)
                        .expect("measured Taffy node should have a current-frame callback");
                    let measured_size: Size<Pixels> =
                        measure(known_dimensions, available_space, window, cx);
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
        let node_id = *self
            .committed_layout_nodes
            .get(&id)
            .expect("layout bounds should only be requested after layout is committed");
        self.layout_bounds_for_node(node_id, scale_factor)
    }

    fn layout_bounds_for_node(&mut self, node_id: NodeId, scale_factor: f32) -> Bounds<Pixels> {
        if let Some(layout) = self.absolute_layout_bounds.get(&node_id).cloned() {
            return layout;
        }

        let layout = self.taffy.layout(node_id).expect(EXPECT_MESSAGE);
        let layout_location = layout.location;
        let layout_size = layout.size;
        let parent = self.taffy.parent(node_id);

        let absolute_outer_origin = match parent {
            Some(parent_id) => {
                self.layout_bounds_for_node(parent_id, scale_factor);
                let parent_origin = *self
                    .absolute_outer_origins
                    .get(&parent_id)
                    .expect("parent absolute outer origin should be cached");
                parent_origin + Point::from(layout_location)
            }
            None => Point::from(layout_location),
        };
        self.absolute_outer_origins
            .insert(node_id, absolute_outer_origin);

        let absolute_far = absolute_outer_origin + Point::from(Size::from(layout_size));
        let snapped_bounds = Bounds::from_corners(
            absolute_outer_origin.map(round_half_toward_zero),
            absolute_far.map(round_half_toward_zero),
        );

        let bounds = (snapped_bounds / scale_factor).map(Pixels);
        self.absolute_layout_bounds.insert(node_id, bounds);
        bounds
    }
}

/// A unique identifier for a layout descriptor generated during the current frame.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct LayoutId(usize);

impl std::hash::Hash for LayoutId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[cfg(test)]
mod retained_layout_tests {
    use super::*;
    use crate::{
        AbsoluteLength, DefiniteLength, Display, FlexDirection, Length, TestAppContext, px,
    };
    use std::{cell::Cell, rc::Rc};

    #[derive(Debug, PartialEq)]
    struct TaffyShape {
        style: taffy::style::Style,
        has_measure_context: bool,
        children: Vec<TaffyShape>,
    }

    fn style_with_width(width: f32) -> Style {
        let mut style = Style::default();
        style.size.width =
            Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(width))));
        style
    }

    fn request_leaf(engine: &mut TaffyLayoutEngine, width: f32) -> LayoutId {
        engine.request_layout(style_with_width(width), px(16.0), 1.0, &[])
    }

    fn request_container(engine: &mut TaffyLayoutEngine, children: &[LayoutId]) -> LayoutId {
        engine.request_layout(Style::default(), px(16.0), 1.0, children)
    }

    fn request_flex_container(engine: &mut TaffyLayoutEngine, children: &[LayoutId]) -> LayoutId {
        let mut style = Style::default();
        style.display = Display::Flex;
        style.flex_direction = FlexDirection::Row;
        engine.request_layout(style, px(16.0), 1.0, children)
    }

    fn request_full_container(engine: &mut TaffyLayoutEngine, children: &[LayoutId]) -> LayoutId {
        let mut style = Style::default();
        style.size = Size::full();
        engine.request_layout(style, px(16.0), 1.0, children)
    }

    fn request_full_leaf(engine: &mut TaffyLayoutEngine) -> LayoutId {
        request_full_container(engine, &[])
    }

    fn request_measured(engine: &mut TaffyLayoutEngine, width: f32) -> LayoutId {
        engine.request_measured_layout(style_with_width(width), px(16.0), 1.0, move |_, _, _, _| {
            size(px(width), px(10.0))
        })
    }

    fn request_auto_measured(engine: &mut TaffyLayoutEngine, width: f32) -> LayoutId {
        engine.request_measured_layout(Style::default(), px(16.0), 1.0, move |_, _, _, _| {
            size(px(width), px(10.0))
        })
    }

    fn request_keyed_auto_measured(
        engine: &mut TaffyLayoutEngine,
        key: u64,
        width: f32,
        invocations: Rc<Cell<usize>>,
    ) -> LayoutId {
        engine.request_keyed_measured_layout(
            Style::default(),
            px(16.0),
            1.0,
            MeasureKey::new(key),
            move |_, _, _, _| {
                invocations.set(invocations.get() + 1);
                size(px(width), px(10.0))
            },
        )
    }

    struct DropCounter(Rc<Cell<usize>>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    fn request_counted_measured(
        engine: &mut TaffyLayoutEngine,
        drops: Rc<Cell<usize>>,
    ) -> LayoutId {
        let counter = DropCounter(drops);
        engine.request_measured_layout(Style::default(), px(16.0), 1.0, move |_, _, _, _| {
            let _keep_counter_alive = &counter;
            size(px(10.0), px(10.0))
        })
    }

    fn request_row(engine: &mut TaffyLayoutEngine, widths: &[f32]) -> LayoutId {
        let children = widths
            .iter()
            .map(|width| request_leaf(engine, *width))
            .collect::<Vec<_>>();
        request_container(engine, &children)
    }

    fn request_flex_row(engine: &mut TaffyLayoutEngine, widths: &[f32]) -> LayoutId {
        let children = widths
            .iter()
            .map(|width| request_leaf(engine, *width))
            .collect::<Vec<_>>();
        request_flex_container(engine, &children)
    }

    fn taffy_shape(engine: &TaffyLayoutEngine, node_id: NodeId) -> TaffyShape {
        TaffyShape {
            style: engine.taffy.style(node_id).expect(EXPECT_MESSAGE).clone(),
            has_measure_context: engine.taffy.get_node_context(node_id).is_some(),
            children: engine
                .taffy
                .children(node_id)
                .expect(EXPECT_MESSAGE)
                .into_iter()
                .map(|child| taffy_shape(engine, child))
                .collect(),
        }
    }

    fn compute_layout_without_measure(
        engine: &mut TaffyLayoutEngine,
        root: LayoutId,
        width: f32,
        height: f32,
    ) -> NodeId {
        compute_layout_without_measure_with_available_space(
            engine,
            root,
            AvailableSpace::Definite(px(width)),
            AvailableSpace::Definite(px(height)),
        )
    }

    fn compute_layout_without_measure_with_available_space(
        engine: &mut TaffyLayoutEngine,
        root: LayoutId,
        width: AvailableSpace,
        height: AvailableSpace,
    ) -> NodeId {
        let root_node = engine.commit_root_layout(root);
        if !engine.computed_layouts.insert(root_node) {
            engine.invalidate_cached_bounds_for_subtree(root_node);
        }
        engine
            .taffy
            .compute_layout_with_measure(
                root_node,
                size(width, height).into(),
                |_known_dimensions, _available_space, _id, _node_context, _style| {
                    taffy::geometry::Size::default()
                },
            )
            .expect(EXPECT_MESSAGE);
        root_node
    }

    fn taffy_node_size(engine: &TaffyLayoutEngine, node_id: NodeId) -> Size<f32> {
        engine
            .taffy
            .layout(node_id)
            .expect(EXPECT_MESSAGE)
            .size
            .into()
    }

    fn assert_descriptor_committed_exactly(engine: &TaffyLayoutEngine, id: LayoutId) {
        let node_id = engine.committed_layout_nodes[&id];
        let descriptor = &engine.layout_descriptors[id.0];
        assert_eq!(
            engine.taffy.style(node_id).expect(EXPECT_MESSAGE),
            &descriptor.style
        );

        match &descriptor.kind {
            LayoutDescriptorKind::Unmeasured { children } => {
                assert!(engine.taffy.get_node_context(node_id).is_none());
                let child_node_ids = children
                    .iter()
                    .map(|child| engine.committed_layout_nodes[child])
                    .collect::<Vec<_>>();
                assert_eq!(
                    engine.taffy.children(node_id).expect(EXPECT_MESSAGE),
                    child_node_ids
                );
                for child in children {
                    assert_descriptor_committed_exactly(engine, *child);
                }
            }
            LayoutDescriptorKind::Measured { .. } => {
                assert!(engine.taffy.get_node_context(node_id).is_some());
                assert_eq!(
                    engine.taffy.children(node_id).expect(EXPECT_MESSAGE),
                    Vec::<NodeId>::new()
                );
            }
        }
    }

    #[test]
    fn retained_commit_matches_fresh_commit_after_insert_delete_and_reorder() {
        let mut retained = TaffyLayoutEngine::new();
        let retained_first_root = request_row(&mut retained, &[10.0, 20.0, 30.0]);
        retained.commit_layout(retained_first_root);
        retained.finish_frame();

        let retained_second_root = request_row(&mut retained, &[30.0, 10.0, 40.0, 20.0]);
        let retained_root_node = retained.commit_layout(retained_second_root);
        assert_descriptor_committed_exactly(&retained, retained_second_root);

        let mut fresh = TaffyLayoutEngine::new();
        let fresh_root = request_row(&mut fresh, &[30.0, 10.0, 40.0, 20.0]);
        let fresh_root_node = fresh.commit_layout(fresh_root);

        assert_eq!(
            taffy_shape(&retained, retained_root_node),
            taffy_shape(&fresh, fresh_root_node)
        );
    }

    #[test]
    fn retained_commit_matches_fresh_commit_for_frame_sequence_gallery() {
        let frames: &[&[f32]] = &[
            &[],
            &[10.0],
            &[10.0, 20.0],
            &[20.0, 10.0],
            &[10.0],
            &[30.0, 10.0, 20.0],
            &[],
            &[40.0, 10.0],
        ];
        let mut retained = TaffyLayoutEngine::new();

        for widths in frames {
            let retained_root = request_row(&mut retained, widths);
            let retained_root_node = retained.commit_layout(retained_root);
            assert_descriptor_committed_exactly(&retained, retained_root);

            let mut fresh = TaffyLayoutEngine::new();
            let fresh_root = request_row(&mut fresh, widths);
            let fresh_root_node = fresh.commit_layout(fresh_root);
            assert_descriptor_committed_exactly(&fresh, fresh_root);

            assert_eq!(
                taffy_shape(&retained, retained_root_node),
                taffy_shape(&fresh, fresh_root_node)
            );

            retained.finish_frame();
        }
    }

    #[test]
    fn retained_layout_recomputes_after_child_delete_from_content_sized_parent() {
        let mut retained = TaffyLayoutEngine::new();
        let root = request_flex_row(&mut retained, &[100.0, 200.0]);
        compute_layout_without_measure_with_available_space(
            &mut retained,
            root,
            AvailableSpace::MaxContent,
            AvailableSpace::Definite(px(100.0)),
        );
        retained.finish_frame();

        let root = request_flex_row(&mut retained, &[100.0]);
        let retained_root = compute_layout_without_measure_with_available_space(
            &mut retained,
            root,
            AvailableSpace::MaxContent,
            AvailableSpace::Definite(px(100.0)),
        );

        let mut fresh = TaffyLayoutEngine::new();
        let fresh_root = request_flex_row(&mut fresh, &[100.0]);
        let fresh_root = compute_layout_without_measure_with_available_space(
            &mut fresh,
            fresh_root,
            AvailableSpace::MaxContent,
            AvailableSpace::Definite(px(100.0)),
        );

        assert_eq!(
            (
                taffy_node_size(&retained, retained_root),
                taffy_node_size(&fresh, fresh_root),
            ),
            (size(100.0, 0.0), size(100.0, 0.0))
        );
    }

    #[test]
    fn moved_child_has_exactly_the_current_descriptor_parent() {
        let mut engine = TaffyLayoutEngine::new();
        let child = request_leaf(&mut engine, 10.0);
        let left = request_container(&mut engine, &[child]);
        let right = request_container(&mut engine, &[]);
        let root = request_container(&mut engine, &[left, right]);
        engine.commit_layout(root);
        engine.finish_frame();

        let left = request_container(&mut engine, &[]);
        let child = request_leaf(&mut engine, 10.0);
        let right = request_container(&mut engine, &[child]);
        let root = request_container(&mut engine, &[left, right]);
        engine.commit_layout(root);
        assert_descriptor_committed_exactly(&engine, root);

        let left_node = engine.committed_layout_nodes[&left];
        let right_node = engine.committed_layout_nodes[&right];
        let child_node = engine.committed_layout_nodes[&child];

        assert_eq!(
            engine.taffy.children(left_node).expect(EXPECT_MESSAGE),
            Vec::<NodeId>::new()
        );
        assert_eq!(
            engine.taffy.children(right_node).expect(EXPECT_MESSAGE),
            vec![child_node]
        );
        assert_eq!(engine.taffy.parent(child_node), Some(right_node));
    }

    #[test]
    fn node_kind_changes_do_not_reuse_stale_measured_context() {
        let mut engine = TaffyLayoutEngine::new();
        let measured = request_measured(&mut engine, 10.0);
        engine.commit_layout(measured);
        engine.finish_frame();

        let leaf = request_leaf(&mut engine, 10.0);
        engine.commit_layout(leaf);
        assert_descriptor_committed_exactly(&engine, leaf);

        let leaf_node = engine.committed_layout_nodes[&leaf];
        assert!(engine.taffy.get_node_context(leaf_node).is_none());
        assert_eq!(
            engine.taffy.children(leaf_node).expect(EXPECT_MESSAGE),
            Vec::<NodeId>::new()
        );
    }

    #[test]
    fn removed_measured_node_drops_its_measure_context() {
        let mut engine = TaffyLayoutEngine::new();
        let drops = Rc::new(Cell::new(0));

        let measured = request_counted_measured(&mut engine, drops.clone());
        engine.commit_layout(measured);
        assert_eq!(drops.get(), 0);

        engine.finish_frame();
        assert_eq!(drops.get(), 1);

        let leaf = request_leaf(&mut engine, 10.0);
        engine.commit_layout(leaf);
        assert_descriptor_committed_exactly(&engine, leaf);
        assert_eq!(drops.get(), 1);
    }

    #[gpui::test]
    fn retained_measured_node_recomputes_when_measure_result_changes(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let scale_factor = cx.update(|window, _| window.scale_factor());

        let mut retained = TaffyLayoutEngine::new();
        let root = request_auto_measured(&mut retained, 0.0);
        cx.update(|window, app| {
            retained.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });
        retained.finish_frame();

        let root = request_auto_measured(&mut retained, 100.0);
        let retained_root = cx.update(|window, app| {
            retained.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
            retained.committed_layout_nodes[&root]
        });

        let mut fresh = TaffyLayoutEngine::new();
        let fresh_root = request_auto_measured(&mut fresh, 100.0);
        let fresh_root = cx.update(|window, app| {
            fresh.compute_layout(
                fresh_root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
            fresh.committed_layout_nodes[&fresh_root]
        });

        let expected_size = size(100.0 * scale_factor, 10.0 * scale_factor);
        assert_eq!(
            (
                taffy_node_size(&retained, retained_root),
                taffy_node_size(&fresh, fresh_root),
            ),
            (expected_size, expected_size)
        );
    }

    #[gpui::test]
    fn keyed_measured_node_reuse_does_not_replace_taffy_context(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let invocations = Rc::new(Cell::new(0));
        let mut engine = TaffyLayoutEngine::new();

        let root = request_keyed_auto_measured(&mut engine, 1, 100.0, invocations.clone());
        cx.update(|window, app| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });
        let initial = engine.finish_frame();
        assert_eq!(initial.taffy_measure_context_updates, 1);
        assert_eq!(initial.taffy_measure_dirty_marks, 0);
        assert_eq!(initial.measured_layout_calls, 1);

        let root = request_keyed_auto_measured(&mut engine, 1, 100.0, invocations.clone());
        cx.update(|window, app| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });
        let reused = engine.finish_frame();

        assert_eq!(reused.taffy_node_reuses, 1);
        assert_eq!(reused.taffy_measure_context_updates, 0);
        assert_eq!(reused.taffy_measure_dirty_marks, 0);
        assert_eq!(reused.measured_layout_calls, 0);
        assert_eq!(invocations.get(), 1);
    }

    #[gpui::test]
    fn keyed_measured_node_recomputes_when_key_changes(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let scale_factor = cx.update(|window, _| window.scale_factor());
        let invocations = Rc::new(Cell::new(0));
        let mut engine = TaffyLayoutEngine::new();

        let root = request_keyed_auto_measured(&mut engine, 1, 0.0, invocations.clone());
        cx.update(|window, app| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });
        engine.finish_frame();

        let root = request_keyed_auto_measured(&mut engine, 2, 100.0, invocations.clone());
        let retained_root = cx.update(|window, app| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
            engine.committed_layout_nodes[&root]
        });
        let changed = engine.finish_frame();

        assert_eq!(changed.taffy_node_reuses, 1);
        assert_eq!(changed.taffy_measure_context_updates, 0);
        assert_eq!(changed.taffy_measure_dirty_marks, 1);
        assert_eq!(changed.measured_layout_calls, 1);
        assert_eq!(invocations.get(), 2);
        assert_eq!(
            taffy_node_size(&engine, retained_root),
            size(100.0 * scale_factor, 10.0 * scale_factor)
        );
    }

    #[test]
    #[should_panic(expected = "layout descriptor should appear only once")]
    fn duplicate_descriptor_references_fail_loudly() {
        let mut engine = TaffyLayoutEngine::new();
        let child = request_leaf(&mut engine, 10.0);
        let root = request_container(&mut engine, &[child, child]);

        engine.commit_layout(root);
    }

    #[test]
    fn retained_layout_recomputes_when_root_available_space_changes() {
        let mut retained = TaffyLayoutEngine::new();
        let child = request_full_leaf(&mut retained);
        let root = request_full_container(&mut retained, &[child]);
        compute_layout_without_measure(&mut retained, root, 0.0, 100.0);
        retained.finish_frame();

        let child = request_full_leaf(&mut retained);
        let root = request_full_container(&mut retained, &[child]);
        let retained_root = compute_layout_without_measure(&mut retained, root, 800.0, 100.0);
        let retained_child = retained.committed_layout_nodes[&child];

        let mut fresh = TaffyLayoutEngine::new();
        let child = request_full_leaf(&mut fresh);
        let root = request_full_container(&mut fresh, &[child]);
        let fresh_root = compute_layout_without_measure(&mut fresh, root, 800.0, 100.0);
        let fresh_child = fresh.committed_layout_nodes[&child];

        assert_eq!(
            taffy_node_size(&retained, retained_root),
            taffy_node_size(&fresh, fresh_root)
        );
        assert_eq!(
            taffy_node_size(&retained, retained_child),
            taffy_node_size(&fresh, fresh_child)
        );
        assert_eq!(
            taffy_node_size(&retained, retained_child),
            size(800.0, 100.0)
        );
    }

    #[test]
    fn layout_work_sample_counts_retained_taffy_mutations() {
        let mut engine = TaffyLayoutEngine::new();

        let root = request_leaf(&mut engine, 10.0);
        compute_layout_without_measure(&mut engine, root, 100.0, 100.0);
        let initial = engine.finish_frame();
        assert_eq!(initial.taffy_node_creates, 1);
        assert_eq!(initial.taffy_node_reuses, 0);
        assert_eq!(initial.taffy_style_updates, 0);
        assert_eq!(initial.taffy_children_updates, 0);
        assert_eq!(initial.taffy_node_removes, 0);

        let root = request_leaf(&mut engine, 10.0);
        compute_layout_without_measure(&mut engine, root, 100.0, 100.0);
        let reused = engine.finish_frame();
        assert_eq!(reused.taffy_node_creates, 0);
        assert_eq!(reused.taffy_node_reuses, 1);
        assert_eq!(reused.taffy_style_updates, 0);
        assert_eq!(reused.taffy_children_updates, 0);
        assert_eq!(reused.taffy_node_removes, 0);

        let root = request_leaf(&mut engine, 20.0);
        compute_layout_without_measure(&mut engine, root, 100.0, 100.0);
        let style_changed = engine.finish_frame();
        assert_eq!(style_changed.taffy_node_creates, 0);
        assert_eq!(style_changed.taffy_node_reuses, 1);
        assert_eq!(style_changed.taffy_style_updates, 1);
        assert_eq!(style_changed.taffy_children_updates, 0);
        assert_eq!(style_changed.taffy_node_removes, 0);

        let child = request_leaf(&mut engine, 10.0);
        let root = request_container(&mut engine, &[child]);
        compute_layout_without_measure(&mut engine, root, 100.0, 100.0);
        let child_inserted = engine.finish_frame();
        assert_eq!(child_inserted.taffy_node_creates, 1);
        assert_eq!(child_inserted.taffy_node_reuses, 1);
        assert_eq!(child_inserted.taffy_style_updates, 1);
        assert_eq!(child_inserted.taffy_children_updates, 1);
        assert_eq!(child_inserted.taffy_node_removes, 0);

        let root = request_container(&mut engine, &[]);
        compute_layout_without_measure(&mut engine, root, 100.0, 100.0);
        let child_removed = engine.finish_frame();
        assert_eq!(child_removed.taffy_node_creates, 0);
        assert_eq!(child_removed.taffy_node_reuses, 1);
        assert_eq!(child_removed.taffy_style_updates, 0);
        assert_eq!(child_removed.taffy_children_updates, 1);
        assert_eq!(child_removed.taffy_node_removes, 1);
    }

    #[test]
    fn repeated_root_layout_recompute_is_allowed() {
        let mut engine = TaffyLayoutEngine::new();
        let child = request_full_leaf(&mut engine);
        let root = request_full_container(&mut engine, &[child]);

        compute_layout_without_measure(&mut engine, root, 0.0, 100.0);
        let child_node = engine.committed_layout_nodes[&child];
        assert_eq!(
            engine.layout_bounds_for_node(child_node, 1.0).size,
            size(px(0.0), px(100.0))
        );

        let root_node = compute_layout_without_measure(&mut engine, root, 800.0, 100.0);

        assert_eq!(taffy_node_size(&engine, root_node), size(800.0, 100.0));
        assert_eq!(taffy_node_size(&engine, child_node), size(800.0, 100.0));
        assert_eq!(
            engine.layout_bounds_for_node(child_node, 1.0).size,
            size(px(800.0), px(100.0))
        );
    }

    #[test]
    #[should_panic(expected = "layout root must not already be committed under a parent")]
    fn descriptor_committed_under_parent_cannot_be_computed_as_root() {
        let mut engine = TaffyLayoutEngine::new();
        let child = request_full_leaf(&mut engine);
        let root = request_full_container(&mut engine, &[child]);

        compute_layout_without_measure(&mut engine, root, 800.0, 100.0);
        engine.commit_root_layout(child);
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

        let sample = engine.layout_work_sample();
        assert_eq!(
            sample,
            LayoutWorkSample {
                draw_index: 0,
                layout_node_requests: 3,
                measured_layout_node_requests: 0,
                child_edges: 2,
                compute_layout_calls: 0,
                measured_layout_calls: 0,
                request_layout_duration: sample.request_layout_duration,
                compute_layout_duration: Duration::default(),
                measured_layout_duration: Duration::default(),
                ..Default::default()
            }
        );
        assert!(sample.request_layout_duration > Duration::default());
        assert_eq!(engine.finish_frame(), sample);
        assert_eq!(engine.layout_work_sample(), LayoutWorkSample::default());
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
                taffy_node_creates: 1,
                taffy_measure_context_updates: 1,
                request_measured_layout_duration: sample.request_measured_layout_duration,
                descriptor_commit_duration: sample.descriptor_commit_duration,
                compute_layout_duration: sample.compute_layout_duration,
                measured_layout_duration: sample.measured_layout_duration,
                ..Default::default()
            }
        );
        assert_eq!(measure_invocations.get(), 1);
        assert!(sample.request_measured_layout_duration > Duration::default());
        assert!(sample.descriptor_commit_duration > Duration::default());
        assert!(sample.compute_layout_duration >= sample.measured_layout_duration);
        assert!(sample.measured_layout_duration > Duration::default());
    }
}
