use super::{AvailableSpace, EXPECT_MESSAGE, LayoutId};
use crate::{App, Pixels, Size, TextLayoutArtifact, TextMeasureKey, Window, size};
use collections::{FxHashMap, FxHashSet};
use stacksafe::StackSafe;
use std::{collections::VecDeque, fmt::Debug, mem, rc::Rc};
use taffy::{
    TaffyTree,
    tree::{LayoutCacheEntry, LayoutCacheEntryId, LayoutCacheEvent, NodeId, RunMode},
};

pub(super) type NodeMeasureFn = StackSafe<
    Box<
        dyn FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &mut App,
        ) -> MeasuredLayoutResult,
    >,
>;

pub(super) struct NodeContext;

type TextHydrator = Rc<dyn Fn(&TextLayoutArtifact)>;

pub(super) struct LayoutMeasureContext {
    pub(super) measure: NodeMeasureFn,
    pub(super) text_hydrator: Option<TextHydrator>,
}

pub(super) enum MeasuredLayoutResult {
    Size(Size<Pixels>),
    Text(TextLayoutArtifact),
}

impl MeasuredLayoutResult {
    fn size(&self) -> Size<Pixels> {
        match self {
            Self::Size(size) => *size,
            Self::Text(artifact) => artifact.size(),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum PureSizeMeasure {
    List(ListPureSizeMeasure),
    UniformList(UniformListPureSizeMeasure),
}

impl PureSizeMeasure {
    pub(crate) fn list(max_element_width: Pixels, total_height: Pixels, scale_factor: f32) -> Self {
        Self::List(ListPureSizeMeasure {
            max_element_width,
            total_height,
            scale_factor_bits: scale_factor.to_bits(),
        })
    }

    pub(crate) fn uniform_list(
        item_size: Size<Pixels>,
        item_count: usize,
        scale_factor: f32,
    ) -> Self {
        Self::UniformList(UniformListPureSizeMeasure {
            item_size,
            item_count,
            scale_factor_bits: scale_factor.to_bits(),
        })
    }

    fn measure(
        &self,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
    ) -> Size<Pixels> {
        match self {
            Self::List(measure) => measure.measure(known_dimensions, available_space),
            Self::UniformList(measure) => measure.measure(known_dimensions, available_space),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ListPureSizeMeasure {
    max_element_width: Pixels,
    total_height: Pixels,
    scale_factor_bits: u32,
}

impl ListPureSizeMeasure {
    fn measure(
        &self,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
    ) -> Size<Pixels> {
        let width = known_dimensions
            .width
            .unwrap_or(match available_space.width {
                AvailableSpace::Definite(width) => width,
                AvailableSpace::MinContent | AvailableSpace::MaxContent => self.max_element_width,
            });
        let height = match available_space.height {
            AvailableSpace::Definite(height) => self.total_height.min(height),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => self.total_height,
        };
        size(width, height)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct UniformListPureSizeMeasure {
    item_size: Size<Pixels>,
    item_count: usize,
    scale_factor_bits: u32,
}

impl UniformListPureSizeMeasure {
    fn measure(
        &self,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
    ) -> Size<Pixels> {
        let desired_height = self.item_size.height * self.item_count;
        let width = known_dimensions
            .width
            .unwrap_or(match available_space.width {
                AvailableSpace::Definite(width) => width,
                AvailableSpace::MinContent | AvailableSpace::MaxContent => self.item_size.width,
            });
        let height = match available_space.height {
            AvailableSpace::Definite(height) => desired_height.min(height),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => desired_height,
        };
        size(width, height)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum MeasuredLayoutKind {
    Opaque,
    PureSize(PureSizeMeasure),
    Text(TextMeasureKey),
}

impl MeasuredLayoutKind {
    fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque)
    }
}

pub(super) enum CurrentMeasurement {
    Opaque(NodeMeasureFn),
    PureSize(PureSizeMeasure),
    Text {
        key: TextMeasureKey,
        measure: NodeMeasureFn,
        hydrate: TextHydrator,
        hydrated: bool,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct TextCacheArtifactKey {
    pub(super) node_id: NodeId,
    measure_key: TextMeasureKey,
    entry_id: LayoutCacheEntryId,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct TextFinalArtifactKey {
    pub(super) node_id: NodeId,
    measure_key: TextMeasureKey,
}

pub(super) struct LayoutDescriptor {
    pub(super) style: taffy::style::Style,
    pub(super) kind: LayoutDescriptorKind,
}

#[derive(Clone)]
pub(super) enum LayoutDescriptorKind {
    Unmeasured {
        children: Vec<LayoutId>,
    },
    Measured {
        measure: Option<usize>,
        measured_kind: MeasuredLayoutKind,
    },
}

enum LayoutCommitKind {
    Unmeasured {
        children: Vec<LayoutId>,
    },
    Measured {
        measure: Option<LayoutMeasureContext>,
        measured_kind: MeasuredLayoutKind,
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
    measured_kind: Option<MeasuredLayoutKind>,
}

pub(super) struct RetainedLayoutNode {
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

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct TaffyMutationCountsForTests {
    creates: u64,
    style_updates: u64,
    children_updates: u64,
    context_updates: u64,
    removes: u64,
}

pub(super) struct ReconcileState {
    layout_descriptors: Vec<LayoutDescriptor>,
    layout_measure_contexts: Vec<Option<LayoutMeasureContext>>,
    current_measurements: FxHashMap<NodeId, CurrentMeasurement>,
    pending_text_artifacts: FxHashMap<NodeId, VecDeque<TextLayoutArtifact>>,
    text_cache_artifacts: FxHashMap<TextCacheArtifactKey, TextLayoutArtifact>,
    text_final_artifacts: FxHashMap<TextFinalArtifactKey, TextLayoutArtifact>,
    committed_layout_nodes: FxHashMap<LayoutId, NodeId>,
    #[cfg(any(test, debug_assertions))]
    committed_taffy_nodes: FxHashSet<NodeId>,
    retained_roots: Vec<Option<RetainedLayoutNode>>,
    current_roots: Vec<RetainedLayoutNode>,
    root_commit_index: usize,
    #[cfg(test)]
    taffy_mutation_counts_for_tests: TaffyMutationCountsForTests,
}

impl ReconcileState {
    pub(super) fn new() -> Self {
        Self {
            layout_descriptors: Vec::new(),
            layout_measure_contexts: Vec::new(),
            current_measurements: FxHashMap::default(),
            pending_text_artifacts: FxHashMap::default(),
            text_cache_artifacts: FxHashMap::default(),
            text_final_artifacts: FxHashMap::default(),
            committed_layout_nodes: FxHashMap::default(),
            #[cfg(any(test, debug_assertions))]
            committed_taffy_nodes: FxHashSet::default(),
            retained_roots: Vec::new(),
            current_roots: Vec::new(),
            root_commit_index: 0,
            #[cfg(test)]
            taffy_mutation_counts_for_tests: TaffyMutationCountsForTests::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.current_measurements.clear();
        self.pending_text_artifacts.clear();
    }

    pub(super) fn finish_frame(&mut self, taffy: &mut TaffyTree<NodeContext>) {
        for retained_root in mem::take(&mut self.retained_roots).into_iter().flatten() {
            self.remove_retained_subtree(taffy, retained_root);
        }

        self.retained_roots = self.current_roots.drain(..).map(Some).collect();
        self.layout_descriptors.clear();
        self.layout_measure_contexts.clear();
        self.current_measurements.clear();
        self.pending_text_artifacts.clear();
        self.committed_layout_nodes.clear();
        #[cfg(any(test, debug_assertions))]
        self.committed_taffy_nodes.clear();
        self.root_commit_index = 0;
    }

    pub(super) fn request_layout(
        &mut self,
        style: taffy::style::Style,
        children: &[LayoutId],
    ) -> LayoutId {
        self.push_descriptor(LayoutDescriptor {
            style,
            kind: LayoutDescriptorKind::Unmeasured {
                children: children.to_vec(),
            },
        })
    }

    pub(super) fn request_measured_layout(
        &mut self,
        style: taffy::style::Style,
        measured_kind: MeasuredLayoutKind,
        measure_context: Option<LayoutMeasureContext>,
    ) -> LayoutId {
        let measure = measure_context.map(|measure_context| {
            let measure_id = self.layout_measure_contexts.len();
            self.layout_measure_contexts.push(Some(measure_context));
            measure_id
        });

        self.push_descriptor(LayoutDescriptor {
            style,
            kind: LayoutDescriptorKind::Measured {
                measure,
                measured_kind,
            },
        })
    }

    fn push_descriptor(&mut self, descriptor: LayoutDescriptor) -> LayoutId {
        let id = LayoutId(self.layout_descriptors.len());
        self.layout_descriptors.push(descriptor);
        id
    }

    pub(super) fn committed_node(&self, id: LayoutId) -> NodeId {
        *self
            .committed_layout_nodes
            .get(&id)
            .expect("layout bounds should only be requested after layout is committed")
    }

    pub(super) fn begin_compute_measurements(&mut self) -> ComputeMeasurementState<'_> {
        ComputeMeasurementState {
            current_measurements: &mut self.current_measurements,
            pending_text_artifacts: &mut self.pending_text_artifacts,
            text_cache_artifacts: &mut self.text_cache_artifacts,
            text_final_artifacts: &mut self.text_final_artifacts,
        }
    }

    #[cfg(test)]
    pub(super) fn reset_taffy_mutation_counts_for_tests(&mut self) {
        self.taffy_mutation_counts_for_tests = TaffyMutationCountsForTests::default();
    }

    #[cfg(test)]
    pub(super) fn taffy_mutation_counts_for_tests(&self) -> TaffyMutationCountsForTests {
        self.taffy_mutation_counts_for_tests
    }

    #[cfg(test)]
    pub(super) fn assert_descriptor_committed_exactly_for_tests(
        &self,
        taffy: &TaffyTree<NodeContext>,
        id: LayoutId,
    ) {
        let node_id = self.committed_node(id);
        let mut seen = FxHashSet::default();
        self.debug_assert_descriptor_node_matches(taffy, id, node_id, None, &mut seen);
    }
}

pub(super) struct ComputeMeasurementState<'a> {
    current_measurements: &'a mut FxHashMap<NodeId, CurrentMeasurement>,
    pending_text_artifacts: &'a mut FxHashMap<NodeId, VecDeque<TextLayoutArtifact>>,
    text_cache_artifacts: &'a mut FxHashMap<TextCacheArtifactKey, TextLayoutArtifact>,
    text_final_artifacts: &'a mut FxHashMap<TextFinalArtifactKey, TextLayoutArtifact>,
}

impl ComputeMeasurementState<'_> {
    pub(super) fn has_current_measurement(&self, node_id: NodeId) -> bool {
        self.current_measurements.contains_key(&node_id)
    }

    pub(super) fn measure(
        &mut self,
        node_id: NodeId,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        let Some(current_measurement) = self.current_measurements.get_mut(&node_id) else {
            panic!("measured Taffy node should have a current GPUI measurement producer");
        };

        match current_measurement {
            CurrentMeasurement::Opaque(measure) => {
                measure(known_dimensions, available_space, window, cx).size()
            }
            CurrentMeasurement::PureSize(measure) => {
                measure.measure(known_dimensions, available_space)
            }
            CurrentMeasurement::Text { key, measure, .. } => {
                let artifact = match measure(known_dimensions, available_space, window, cx) {
                    MeasuredLayoutResult::Text(artifact) => artifact,
                    MeasuredLayoutResult::Size(_) => {
                        panic!("text measured layout producer returned a size-only result");
                    }
                };
                assert_eq!(
                    artifact.key(),
                    key,
                    "text measured layout artifact should match the current text measure key"
                );
                let size = artifact.size();
                self.pending_text_artifacts
                    .entry(node_id)
                    .or_default()
                    .push_back(artifact);
                size
            }
        }
    }

    pub(super) fn handle_cache_event(&mut self, event: LayoutCacheEvent) {
        match event {
            LayoutCacheEvent::Hit(entry) => self.handle_cache_hit(entry),
            LayoutCacheEvent::Stored(entry) => self.handle_cache_store(entry),
            LayoutCacheEvent::Cleared(clear) => {
                let node_id = clear.node_id();
                self.pending_text_artifacts.remove(&node_id);
                self.text_cache_artifacts
                    .retain(|key, _| key.node_id != node_id);
                self.text_final_artifacts
                    .retain(|key, _| key.node_id != node_id);
            }
            _ => panic!("unsupported Taffy layout cache event"),
        }
    }

    fn handle_cache_hit(&mut self, entry: LayoutCacheEntry) {
        if entry.requested_input().run_mode != RunMode::PerformLayout {
            return;
        }
        let node_id = entry.node_id();
        let Some(key) = self.current_text_key(node_id).cloned() else {
            return;
        };
        let artifact = self
            .text_cache_artifacts
            .get(&TextCacheArtifactKey {
                node_id,
                measure_key: key,
                entry_id: entry.entry_id(),
            })
            .cloned()
            .expect("text final-layout cache hit should have a retained text layout artifact");
        self.hydrate_text_node(node_id, &artifact);
    }

    fn handle_cache_store(&mut self, entry: LayoutCacheEntry) {
        let node_id = entry.node_id();
        let Some(key) = self.current_text_key(node_id).cloned() else {
            return;
        };
        let artifact = self
            .pending_text_artifacts
            .get_mut(&node_id)
            .and_then(VecDeque::pop_front)
            .expect("text cache store should follow a text measurement producer call");
        assert_eq!(
            artifact.key(),
            &key,
            "stored text artifact should match the current text measure key"
        );

        self.text_cache_artifacts.insert(
            TextCacheArtifactKey {
                node_id,
                measure_key: key.clone(),
                entry_id: entry.entry_id(),
            },
            artifact.clone(),
        );

        if entry.requested_input().run_mode == RunMode::PerformLayout {
            self.text_final_artifacts.insert(
                TextFinalArtifactKey {
                    node_id,
                    measure_key: key,
                },
                artifact.clone(),
            );
            self.hydrate_text_node(node_id, &artifact);
        }
    }

    pub(super) fn finish_compute(&mut self) {
        let pending_hydrations = self
            .current_measurements
            .iter()
            .filter_map(|(node_id, measurement)| match measurement {
                CurrentMeasurement::Text { key, hydrated, .. } if !*hydrated => {
                    Some((*node_id, key.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        for (node_id, key) in pending_hydrations {
            let Some(artifact) = self
                .text_final_artifacts
                .get(&TextFinalArtifactKey {
                    node_id,
                    measure_key: key,
                })
                .cloned()
            else {
                // Taffy may skip descendants below display-none/hidden ancestors.
                // Visible text still fails loudly during prepaint if no artifact exists.
                continue;
            };
            self.hydrate_text_node(node_id, &artifact);
        }

        self.pending_text_artifacts.clear();
    }

    fn current_text_key(&self, node_id: NodeId) -> Option<&TextMeasureKey> {
        match self.current_measurements.get(&node_id) {
            Some(CurrentMeasurement::Text { key, .. }) => Some(key),
            _ => None,
        }
    }

    fn hydrate_text_node(&mut self, node_id: NodeId, artifact: &TextLayoutArtifact) {
        let Some(CurrentMeasurement::Text {
            key,
            hydrate,
            hydrated,
            ..
        }) = self.current_measurements.get_mut(&node_id)
        else {
            return;
        };
        assert_eq!(
            artifact.key(),
            key,
            "hydrated text artifact should match the current text measure key"
        );
        *hydrated = true;
        hydrate(artifact);
    }
}

impl ReconcileState {
    pub(super) fn commit_layout(
        &mut self,
        taffy: &mut TaffyTree<NodeContext>,
        id: LayoutId,
    ) -> NodeId {
        if let Some(node_id) = self.committed_layout_nodes.get(&id) {
            return *node_id;
        }

        let retained_root = self
            .retained_roots
            .get_mut(self.root_commit_index)
            .and_then(Option::take);
        self.root_commit_index += 1;

        let CommitResult {
            retained_node,
            replaced_previous,
        } = self.commit_descriptor(taffy, id, retained_root);
        if let Some(replaced_previous) = replaced_previous {
            self.remove_retained_subtree(taffy, replaced_previous);
        }

        let node_id = retained_node.node_id;
        self.current_roots.push(retained_node);
        #[cfg(any(test, debug_assertions))]
        self.debug_assert_committed_descriptor_matches(taffy, id, node_id);
        node_id
    }

    pub(super) fn commit_root_layout(
        &mut self,
        taffy: &mut TaffyTree<NodeContext>,
        id: LayoutId,
    ) -> NodeId {
        let node_id = self.commit_layout(taffy, id);
        assert!(
            taffy.parent(node_id).is_none(),
            "layout root must not already be committed under a parent"
        );
        node_id
    }

    fn commit_descriptor(
        &mut self,
        taffy: &mut TaffyTree<NodeContext>,
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
                measured_kind,
            } => LayoutCommitKind::Measured {
                measure: measure.map(|measure| {
                    self.layout_measure_contexts[measure]
                        .take()
                        .expect("measured layout descriptor should be committed only once")
                }),
                measured_kind,
            },
        };

        match kind {
            LayoutCommitKind::Unmeasured { children } => {
                let candidate = self.node_for_descriptor(
                    taffy,
                    previous,
                    RetainedLayoutKind::Unmeasured,
                    &style,
                );
                self.apply_style_diff(
                    taffy,
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
                    let committed_child =
                        self.commit_descriptor(taffy, child, previous_children.next());
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
                    taffy,
                    candidate.node_id,
                    &previous_child_node_ids,
                    &child_node_ids,
                );
                for detached_subtree in detached_subtrees {
                    self.remove_retained_subtree(taffy, detached_subtree);
                }

                self.committed_layout_nodes.insert(id, candidate.node_id);
                CommitResult {
                    retained_node: RetainedLayoutNode {
                        node_id: candidate.node_id,
                        descriptor: RetainedLayoutDescriptor {
                            style,
                            kind: RetainedLayoutKind::Unmeasured,
                            measured_kind: None,
                        },
                        children: retained_children,
                    },
                    replaced_previous: candidate.replaced_previous,
                }
            }
            LayoutCommitKind::Measured {
                measure,
                measured_kind,
            } => {
                let candidate =
                    self.node_for_descriptor(taffy, previous, RetainedLayoutKind::Measured, &style);
                let style_changed = self.apply_style_diff(
                    taffy,
                    candidate.node_id,
                    candidate.previous_descriptor.as_ref(),
                    &style,
                );
                self.apply_measured_diff(
                    taffy,
                    candidate.node_id,
                    candidate.previous_descriptor.as_ref(),
                    &measured_kind,
                    style_changed,
                );
                self.current_measurements.insert(
                    candidate.node_id,
                    Self::current_measurement(measured_kind.clone(), measure),
                );
                self.committed_layout_nodes.insert(id, candidate.node_id);
                CommitResult {
                    retained_node: RetainedLayoutNode {
                        node_id: candidate.node_id,
                        descriptor: RetainedLayoutDescriptor {
                            style,
                            kind: RetainedLayoutKind::Measured,
                            measured_kind: Some(measured_kind),
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
    fn debug_assert_committed_descriptor_matches(
        &mut self,
        taffy: &TaffyTree<NodeContext>,
        id: LayoutId,
        node_id: NodeId,
    ) {
        let mut seen = FxHashSet::default();
        self.debug_assert_descriptor_node_matches(taffy, id, node_id, None, &mut seen);
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
        taffy: &TaffyTree<NodeContext>,
        id: LayoutId,
        node_id: NodeId,
        expected_parent: Option<NodeId>,
        seen: &mut FxHashSet<NodeId>,
    ) {
        assert!(
            seen.insert(node_id),
            "committed Taffy node should appear at only one current descriptor position"
        );
        assert_eq!(taffy.parent(node_id), expected_parent);

        let descriptor = self.descriptor(id);
        assert_eq!(
            taffy.style(node_id).expect(EXPECT_MESSAGE),
            &descriptor.style
        );

        match &descriptor.kind {
            LayoutDescriptorKind::Unmeasured { children } => {
                assert!(taffy.get_node_context(node_id).is_none());
                let child_node_ids = children
                    .iter()
                    .map(|child| self.committed_layout_nodes[child])
                    .collect::<Vec<_>>();
                assert_eq!(
                    taffy.children(node_id).expect(EXPECT_MESSAGE),
                    child_node_ids
                );
                for (child, child_node_id) in children.iter().zip(child_node_ids) {
                    self.debug_assert_descriptor_node_matches(
                        taffy,
                        *child,
                        child_node_id,
                        Some(node_id),
                        seen,
                    );
                }
            }
            LayoutDescriptorKind::Measured { .. } => {
                assert!(taffy.get_node_context(node_id).is_some());
                assert_eq!(
                    taffy.children(node_id).expect(EXPECT_MESSAGE),
                    Vec::<NodeId>::new()
                );
            }
        }
    }

    fn node_for_descriptor(
        &mut self,
        taffy: &mut TaffyTree<NodeContext>,
        previous: Option<RetainedLayoutNode>,
        kind: RetainedLayoutKind,
        style: &taffy::style::Style,
    ) -> RetainedCandidate {
        match previous {
            Some(previous) if previous.descriptor.kind == kind => RetainedCandidate {
                node_id: previous.node_id,
                previous_descriptor: Some(previous.descriptor),
                previous_children: previous.children,
                replaced_previous: None,
            },
            Some(previous) => {
                let node_id = taffy.new_leaf(style.clone()).expect(EXPECT_MESSAGE);
                #[cfg(test)]
                {
                    self.taffy_mutation_counts_for_tests.creates += 1;
                }
                RetainedCandidate {
                    node_id,
                    previous_descriptor: None,
                    previous_children: Vec::new(),
                    replaced_previous: Some(previous),
                }
            }
            None => {
                let node_id = taffy.new_leaf(style.clone()).expect(EXPECT_MESSAGE);
                #[cfg(test)]
                {
                    self.taffy_mutation_counts_for_tests.creates += 1;
                }
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
        taffy: &mut TaffyTree<NodeContext>,
        node_id: NodeId,
        previous: Option<&RetainedLayoutDescriptor>,
        style: &taffy::style::Style,
    ) -> bool {
        if let Some(previous) = previous {
            if previous.style != *style {
                taffy
                    .set_style(node_id, style.clone())
                    .expect(EXPECT_MESSAGE);
                #[cfg(test)]
                {
                    self.taffy_mutation_counts_for_tests.style_updates += 1;
                }
                return true;
            }
        }
        false
    }

    fn apply_measured_diff(
        &mut self,
        taffy: &mut TaffyTree<NodeContext>,
        node_id: NodeId,
        previous: Option<&RetainedLayoutDescriptor>,
        measured_kind: &MeasuredLayoutKind,
        style_changed: bool,
    ) {
        if taffy.get_node_context(node_id).is_none() {
            taffy
                .set_node_context(node_id, Some(NodeContext))
                .expect(EXPECT_MESSAGE);
            #[cfg(test)]
            {
                self.taffy_mutation_counts_for_tests.context_updates += 1;
            }
            return;
        }

        let measured_kind_changed =
            previous.and_then(|previous| previous.measured_kind.as_ref()) != Some(measured_kind);

        if style_changed || measured_kind_changed {
            self.remove_text_artifacts_for_node(node_id);
        }

        if measured_kind_changed {
            taffy.mark_dirty(node_id).expect(EXPECT_MESSAGE);
        } else if measured_kind.is_opaque() && previous.is_some() && !style_changed {
            taffy.mark_dirty(node_id).expect(EXPECT_MESSAGE);
        }
    }

    fn current_measurement(
        measured_kind: MeasuredLayoutKind,
        measure: Option<LayoutMeasureContext>,
    ) -> CurrentMeasurement {
        match measured_kind {
            MeasuredLayoutKind::Opaque => CurrentMeasurement::Opaque(
                measure
                    .expect("opaque measured layout should have a current producer")
                    .measure,
            ),
            MeasuredLayoutKind::PureSize(measure) => CurrentMeasurement::PureSize(measure),
            MeasuredLayoutKind::Text(key) => {
                let measure = measure
                    .expect("text measured layout should have a current producer and hydrator");
                CurrentMeasurement::Text {
                    key,
                    measure: measure.measure,
                    hydrate: measure
                        .text_hydrator
                        .expect("text measured layout should have a hydrator"),
                    hydrated: false,
                }
            }
        }
    }

    fn apply_children_diff(
        &mut self,
        taffy: &mut TaffyTree<NodeContext>,
        node_id: NodeId,
        previous_children: &[NodeId],
        children: &[NodeId],
    ) {
        if previous_children != children {
            taffy.set_children(node_id, children).expect(EXPECT_MESSAGE);
            #[cfg(test)]
            {
                self.taffy_mutation_counts_for_tests.children_updates += 1;
            }
        }
    }

    fn remove_retained_subtree(
        &mut self,
        taffy: &mut TaffyTree<NodeContext>,
        retained_node: RetainedLayoutNode,
    ) {
        for child in retained_node.children {
            self.remove_retained_subtree(taffy, child);
        }
        self.remove_text_artifacts_for_node(retained_node.node_id);
        if retained_node.descriptor.kind == RetainedLayoutKind::Measured {
            taffy
                .set_node_context(retained_node.node_id, None)
                .expect(EXPECT_MESSAGE);
            #[cfg(test)]
            {
                self.taffy_mutation_counts_for_tests.context_updates += 1;
            }
        }
        taffy.remove(retained_node.node_id).expect(EXPECT_MESSAGE);
        #[cfg(test)]
        {
            self.taffy_mutation_counts_for_tests.removes += 1;
        }
    }

    fn remove_text_artifacts_for_node(&mut self, node_id: NodeId) {
        self.pending_text_artifacts.remove(&node_id);
        self.text_cache_artifacts
            .retain(|key, _| key.node_id != node_id);
        self.text_final_artifacts
            .retain(|key, _| key.node_id != node_id);
    }
}
