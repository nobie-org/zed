//! Measurement support for the retained layout forest.
//!
//! This module keeps the three measured-node concepts separate:
//! pure comparable measure facts, current-frame executable producers, and GPUI
//! artifacts such as shaped text. Taffy node context stays a private marker and
//! never stores GPUI paint or hit-test state.

use super::AvailableSpace;
use crate::{App, Pixels, Size, TextLayoutArtifact, TextMeasureKey, Window, size};
use collections::FxHashMap;
use stacksafe::StackSafe;
use std::{collections::VecDeque, mem, rc::Rc};
use taffy::tree::{LayoutCacheEntry, LayoutCacheEntryId, LayoutCacheEvent, NodeId, RunMode};

/// Current-frame executable producer for measured layout.
///
/// This callback can call into GPUI/window state while Taffy asks for a size. It
/// is not comparable retained meaning; opaque producers are intentionally
/// conservative unless a call site supplies an explicit pure measure key.
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

/// Private marker installed on measured mirror nodes.
///
/// The marker only tells Taffy that a node is measured. GPUI side effects and
/// artifacts live in forest-owned maps keyed by explicit retained facts.
#[derive(Clone)]
pub(super) struct NodeContext;

type TextHydrator = Rc<dyn Fn(&TextLayoutArtifact)>;

/// Current-frame producer bundle registered for a measured intent.
///
/// Text measured nodes also carry a hydrator that installs an artifact into the
/// current element state after Taffy either measures or reuses cached layout.
pub(super) struct LayoutMeasureContext {
    pub(super) measure: NodeMeasureFn,
    pub(super) text_hydrator: Option<TextHydrator>,
}

/// Output produced by a measured layout callback.
///
/// Size-only results are enough for pure or opaque measured nodes. Text returns
/// an artifact because GPUI must retain shaped lines for paint and hit testing
/// even when Taffy later skips the measurement callback.
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

/// Explicit, comparable size functions for measured nodes.
///
/// A value of this type is layout-visible data. If it is unchanged and the
/// retained occurrence is reused, the forest can let Taffy reuse measurement
/// cache without re-running an arbitrary closure.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum PureSizeMeasure {
    List(ListPureSizeMeasure),
    UniformList(UniformListPureSizeMeasure),
    ContentSize(ContentSizePureSizeMeasure),
}

impl PureSizeMeasure {
    /// Build the pure size summary used by variable-height lists.
    pub(crate) fn list(max_element_width: Pixels, total_height: Pixels, scale_factor: f32) -> Self {
        Self::List(ListPureSizeMeasure {
            max_element_width,
            total_height,
            scale_factor_bits: scale_factor.to_bits(),
        })
    }

    /// Build the pure size summary used by uniform lists.
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

    /// Build a pure measured node from an already known content size.
    pub(crate) fn content_size(content_size: Size<Pixels>, scale_factor: f32) -> Self {
        Self::ContentSize(ContentSizePureSizeMeasure {
            content_size,
            scale_factor_bits: scale_factor.to_bits(),
        })
    }

    /// Evaluate the pure measure data for Taffy's current measurement query.
    pub(super) fn measure(
        &self,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
    ) -> Size<Pixels> {
        match self {
            Self::List(measure) => measure.measure(known_dimensions, available_space),
            Self::UniformList(measure) => measure.measure(known_dimensions, available_space),
            Self::ContentSize(measure) => measure.measure(known_dimensions, available_space),
        }
    }
}

/// Pure list measurement input.
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

/// Pure uniform-list measurement input.
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

/// Pure fixed-content measurement input.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ContentSizePureSizeMeasure {
    content_size: Size<Pixels>,
    scale_factor_bits: u32,
}

impl ContentSizePureSizeMeasure {
    fn measure(
        &self,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
    ) -> Size<Pixels> {
        let width = known_dimensions
            .width
            .unwrap_or(match available_space.width {
                AvailableSpace::Definite(width) => width,
                AvailableSpace::MinContent | AvailableSpace::MaxContent => self.content_size.width,
            });
        let height = known_dimensions
            .height
            .unwrap_or(match available_space.height {
                AvailableSpace::Definite(height) => height,
                AvailableSpace::MinContent | AvailableSpace::MaxContent => self.content_size.height,
            });
        size(width, height)
    }
}

/// Comparable measured-node identity.
///
/// `Opaque` means no retained measurement identity has been proven. `PureSize`
/// and `Text` are explicit facts that may preserve Taffy measurement cache and
/// associated GPUI artifacts when unchanged.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum MeasuredLayoutKind {
    Opaque,
    PureSize(PureSizeMeasure),
    Text(TextMeasureKey),
}

impl MeasuredLayoutKind {
    pub(super) fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque)
    }
}

/// Current-frame measurement source for a committed mirror node.
///
/// This map is rebuilt while committing the current frame. It is deliberately
/// separate from retained facts because producers and hydrators are executable
/// frame-local state.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum CurrentMeasurement {
    Opaque(usize),
    PureSize(PureSizeMeasure),
    Text { key: TextMeasureKey, measure: usize },
}

/// Owner of measured producers and text layout artifacts.
///
/// This is GPUI state, not Taffy state. Taffy can decide whether a measured
/// node cache entry is valid, but GPUI owns the executable producer slots and
/// the paint/hit-test artifacts needed when that cache entry is reused.
pub(super) struct MeasurementStore {
    producer_contexts: Vec<Option<LayoutMeasureContext>>,
    current_measurements: FxHashMap<NodeId, CurrentMeasurement>,
    pending_text_artifacts: FxHashMap<NodeId, VecDeque<TextLayoutArtifact>>,
    text_cache_artifacts: FxHashMap<TextCacheArtifactKey, TextLayoutArtifact>,
    text_subtree_cache_artifacts: FxHashMap<TextSubtreeCacheArtifactKey, Vec<TextSubtreeArtifact>>,
    current_text_descendants_by_node: FxHashMap<NodeId, Vec<NodeId>>,
}

/// Transaction checkpoint for measurement producers and artifacts.
pub(super) struct MeasurementStoreCheckpoint {
    producer_contexts_len: usize,
    current_measurements: FxHashMap<NodeId, CurrentMeasurement>,
    pending_text_artifacts: FxHashMap<NodeId, VecDeque<TextLayoutArtifact>>,
    text_cache_artifacts: FxHashMap<TextCacheArtifactKey, TextLayoutArtifact>,
    text_subtree_cache_artifacts: FxHashMap<TextSubtreeCacheArtifactKey, Vec<TextSubtreeArtifact>>,
    current_text_descendants_by_node: FxHashMap<NodeId, Vec<NodeId>>,
}

impl MeasurementStore {
    pub(super) fn new() -> Self {
        Self {
            producer_contexts: Vec::new(),
            current_measurements: FxHashMap::default(),
            pending_text_artifacts: FxHashMap::default(),
            text_cache_artifacts: FxHashMap::default(),
            text_subtree_cache_artifacts: FxHashMap::default(),
            current_text_descendants_by_node: FxHashMap::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.current_measurements.clear();
        self.pending_text_artifacts.clear();
    }

    pub(super) fn finish_frame(&mut self) {
        self.producer_contexts.clear();
        self.current_measurements.clear();
        self.pending_text_artifacts.clear();
        self.current_text_descendants_by_node.clear();
    }

    pub(super) fn checkpoint(&self) -> MeasurementStoreCheckpoint {
        MeasurementStoreCheckpoint {
            producer_contexts_len: self.producer_contexts.len(),
            current_measurements: self.current_measurements.clone(),
            pending_text_artifacts: self.pending_text_artifacts.clone(),
            text_cache_artifacts: self.text_cache_artifacts.clone(),
            text_subtree_cache_artifacts: self.text_subtree_cache_artifacts.clone(),
            current_text_descendants_by_node: self.current_text_descendants_by_node.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: MeasurementStoreCheckpoint) {
        self.producer_contexts
            .truncate(checkpoint.producer_contexts_len);
        self.current_measurements = checkpoint.current_measurements;
        self.pending_text_artifacts = checkpoint.pending_text_artifacts;
        self.text_cache_artifacts = checkpoint.text_cache_artifacts;
        self.text_subtree_cache_artifacts = checkpoint.text_subtree_cache_artifacts;
        self.current_text_descendants_by_node = checkpoint.current_text_descendants_by_node;
    }

    pub(super) fn push_producer_context(&mut self, measure_context: LayoutMeasureContext) -> usize {
        let measure_id = self.producer_contexts.len();
        self.producer_contexts.push(Some(measure_context));
        measure_id
    }

    pub(super) fn insert_current_measurement(
        &mut self,
        node_id: NodeId,
        measurement: CurrentMeasurement,
    ) {
        self.current_measurements.insert(node_id, measurement);
    }

    pub(super) fn has_current_measurement(&self, node_id: NodeId) -> bool {
        self.current_measurements.contains_key(&node_id)
    }

    pub(super) fn current_measurement(&self, node_id: NodeId) -> Option<&CurrentMeasurement> {
        self.current_measurements.get(&node_id)
    }

    pub(super) fn record_text_descendants(
        &mut self,
        node_id: NodeId,
        text_descendants: Vec<NodeId>,
    ) {
        self.current_text_descendants_by_node
            .insert(node_id, text_descendants);
    }

    pub(super) fn remove_text_artifacts_for_node(&mut self, node_id: NodeId) {
        self.pending_text_artifacts.remove(&node_id);
        self.text_cache_artifacts
            .retain(|key, _| key.node_id != node_id);
        self.text_subtree_cache_artifacts.retain(|key, artifacts| {
            key.node_id != node_id && !artifacts.iter().any(|artifact| artifact.node_id == node_id)
        });
    }

    pub(super) fn compute_state(&mut self) -> ComputeMeasurementState<'_> {
        ComputeMeasurementState::new(
            &mut self.producer_contexts,
            &mut self.current_measurements,
            &mut self.pending_text_artifacts,
            &mut self.text_cache_artifacts,
            &mut self.text_subtree_cache_artifacts,
            &self.current_text_descendants_by_node,
        )
    }
}

/// Cache key for one text measurement artifact stored in a Taffy cache entry.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct TextCacheArtifactKey {
    pub(super) node_id: NodeId,
    measure_key: TextMeasureKey,
    entry_id: LayoutCacheEntryId,
}

/// Cache key for the final text artifact chosen during a perform-layout pass.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct TextFinalArtifactKey {
    pub(super) node_id: NodeId,
    measure_key: TextMeasureKey,
}

/// Cache key for text descendants hydrated from a retained subtree cache hit.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct TextSubtreeCacheArtifactKey {
    pub(super) node_id: NodeId,
    entry_id: LayoutCacheEntryId,
}

/// Text artifact retained for a descendant of a cached subtree.
#[derive(Clone)]
pub(super) struct TextSubtreeArtifact {
    pub(super) node_id: NodeId,
    pub(super) measure_key: TextMeasureKey,
    pub(super) artifact: TextLayoutArtifact,
}

/// Per-compute bridge between Taffy measurement/cache events and GPUI artifacts.
///
/// Taffy owns layout cache hits/stores, but GPUI owns text artifacts. This state
/// listens to cache events, records artifacts produced by measurement, and
/// hydrates current-frame `TextLayout` handles before compute returns.
pub(super) struct ComputeMeasurementState<'a> {
    producer_contexts: &'a mut Vec<Option<LayoutMeasureContext>>,
    current_measurements: &'a mut FxHashMap<NodeId, CurrentMeasurement>,
    pending_text_artifacts: &'a mut FxHashMap<NodeId, VecDeque<TextLayoutArtifact>>,
    text_cache_artifacts: &'a mut FxHashMap<TextCacheArtifactKey, TextLayoutArtifact>,
    text_subtree_cache_artifacts:
        &'a mut FxHashMap<TextSubtreeCacheArtifactKey, Vec<TextSubtreeArtifact>>,
    current_text_descendants_by_node: &'a FxHashMap<NodeId, Vec<NodeId>>,
    final_text_artifacts: FxHashMap<TextFinalArtifactKey, TextLayoutArtifact>,
}

impl<'a> ComputeMeasurementState<'a> {
    pub(super) fn new(
        producer_contexts: &'a mut Vec<Option<LayoutMeasureContext>>,
        current_measurements: &'a mut FxHashMap<NodeId, CurrentMeasurement>,
        pending_text_artifacts: &'a mut FxHashMap<NodeId, VecDeque<TextLayoutArtifact>>,
        text_cache_artifacts: &'a mut FxHashMap<TextCacheArtifactKey, TextLayoutArtifact>,
        text_subtree_cache_artifacts: &'a mut FxHashMap<
            TextSubtreeCacheArtifactKey,
            Vec<TextSubtreeArtifact>,
        >,
        current_text_descendants_by_node: &'a FxHashMap<NodeId, Vec<NodeId>>,
    ) -> Self {
        Self {
            producer_contexts,
            current_measurements,
            pending_text_artifacts,
            text_cache_artifacts,
            text_subtree_cache_artifacts,
            current_text_descendants_by_node,
            final_text_artifacts: FxHashMap::default(),
        }
    }

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
            panic!("measured layout mirror node should have a current GPUI measurement producer");
        };

        match current_measurement {
            CurrentMeasurement::Opaque(measure) => {
                let measure = &mut self.producer_contexts[*measure]
                    .as_mut()
                    .expect("opaque measured layout should have a current producer")
                    .measure;
                measure(known_dimensions, available_space, window, cx).size()
            }
            CurrentMeasurement::PureSize(measure) => {
                measure.measure(known_dimensions, available_space)
            }
            CurrentMeasurement::Text { key, measure, .. } => {
                let measure = &mut self.producer_contexts[*measure]
                    .as_mut()
                    .expect("text measured layout should have a current producer and hydrator")
                    .measure;
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
                self.final_text_artifacts
                    .retain(|key, _| key.node_id != node_id);
                self.remove_text_subtree_cache_artifacts_for_node(node_id);
            }
            _ => panic!("unsupported retained-layout mirror cache event"),
        }
    }

    fn handle_cache_hit(&mut self, entry: LayoutCacheEntry) {
        if entry.requested_input().run_mode != RunMode::PerformLayout {
            return;
        }
        let node_id = entry.node_id();
        if let Some(key) = self.current_text_key(node_id).cloned() {
            let artifact = self
                .text_cache_artifacts
                .get(&TextCacheArtifactKey {
                    node_id,
                    measure_key: key.clone(),
                    entry_id: entry.entry_id(),
                })
                .cloned()
                .expect("text final-layout cache hit should have a retained text layout artifact");
            self.record_final_text_artifact(node_id, key, artifact);
        }

        if let Some(artifacts) = self
            .text_subtree_cache_artifacts
            .get(&TextSubtreeCacheArtifactKey {
                node_id,
                entry_id: entry.entry_id(),
            })
            .cloned()
        {
            for artifact in artifacts {
                if self.current_text_key(artifact.node_id) == Some(&artifact.measure_key) {
                    self.record_final_text_artifact(
                        artifact.node_id,
                        artifact.measure_key,
                        artifact.artifact,
                    );
                }
            }
        }
    }

    fn handle_cache_store(&mut self, entry: LayoutCacheEntry) {
        let node_id = entry.node_id();
        if let Some(key) = self.current_text_key(node_id).cloned() {
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
                self.record_final_text_artifact(node_id, key, artifact);
            }
        }

        if entry.requested_input().run_mode == RunMode::PerformLayout {
            self.record_text_subtree_cache_artifacts(node_id, entry.entry_id());
        }
    }

    fn record_final_text_artifact(
        &mut self,
        node_id: NodeId,
        key: TextMeasureKey,
        artifact: TextLayoutArtifact,
    ) {
        self.final_text_artifacts.insert(
            TextFinalArtifactKey {
                node_id,
                measure_key: key,
            },
            artifact,
        );
    }

    fn record_text_subtree_cache_artifacts(
        &mut self,
        node_id: NodeId,
        entry_id: LayoutCacheEntryId,
    ) {
        let Some(text_descendants) = self.current_text_descendants_by_node.get(&node_id) else {
            return;
        };
        let artifacts = text_descendants
            .iter()
            .filter_map(|text_node_id| {
                let measure_key = self.current_text_key(*text_node_id)?.clone();
                let artifact = self
                    .final_text_artifacts
                    .get(&TextFinalArtifactKey {
                        node_id: *text_node_id,
                        measure_key: measure_key.clone(),
                    })?
                    .clone();
                Some(TextSubtreeArtifact {
                    node_id: *text_node_id,
                    measure_key,
                    artifact,
                })
            })
            .collect::<Vec<_>>();
        if !artifacts.is_empty() {
            self.text_subtree_cache_artifacts
                .insert(TextSubtreeCacheArtifactKey { node_id, entry_id }, artifacts);
        }
    }

    fn remove_text_subtree_cache_artifacts_for_node(&mut self, node_id: NodeId) {
        self.text_subtree_cache_artifacts.retain(|key, artifacts| {
            key.node_id != node_id && !artifacts.iter().any(|artifact| artifact.node_id == node_id)
        });
    }

    pub(super) fn finish_compute(&mut self) {
        for (key, artifact) in mem::take(&mut self.final_text_artifacts) {
            let node_id = key.node_id;
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
        let Some(CurrentMeasurement::Text { key, measure }) =
            self.current_measurements.get_mut(&node_id)
        else {
            return;
        };
        assert_eq!(
            artifact.key(),
            key,
            "hydrated text artifact should match the current text measure key"
        );
        let hydrate = self.producer_contexts[*measure]
            .as_ref()
            .and_then(|measure| measure.text_hydrator.as_ref())
            .map(Rc::clone)
            .expect("text measured layout should have a hydrator");
        hydrate(artifact);
    }
}
