//! Measurement support for the retained layout forest.
//!
//! This module keeps the three measured-node concepts separate:
//! pure comparable measure facts, current-frame executable producers, and GPUI
//! artifacts such as shaped text. Taffy node context stays a private marker and
//! never stores GPUI paint or hit-test state.

mod producer_registry;
mod text_artifacts;

use super::AvailableSpace;
use crate::{App, Pixels, Size, TextLayoutArtifact, TextMeasureKey, Window, size};
use collections::{FxHashMap, FxHashSet};
use producer_registry::{ProducerRegistry, ProducerRegistryCheckpoint};
use stacksafe::StackSafe;
use std::rc::Rc;
use taffy::{LayoutCacheEntry, LayoutCacheEvent, tree::NodeId};
use text_artifacts::{TextArtifactCacheKey, TextArtifactStore, TextArtifactStoreCheckpoint};

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
/// current element state when the current compute invokes the text producer.
pub(super) struct LayoutMeasureContext {
    pub(super) measure: NodeMeasureFn,
    pub(super) text_hydrator: Option<TextHydrator>,
}

/// Output produced by a measured layout callback.
///
/// Size-only results are enough for pure or opaque measured nodes. Text returns
/// an artifact because GPUI must hydrate shaped lines for paint and hit testing
/// from the exact measurement query Taffy asked in the current compute.
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
/// is explicit data and may preserve Taffy measurement cache when unchanged.
/// `Text` is explicit layout identity, but the shaped artifact also depends on
/// Taffy's measurement query (`known_dimensions` and `available_space`). GPUI
/// therefore hydrates artifacts from callbacks, exact query-keyed cache
/// observations, or retained unchanged-node replay; it never treats
/// `TextMeasureKey` alone as artifact proof.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum MeasuredLayoutKind {
    Opaque,
    PureSize(PureSizeMeasure),
    Text(TextMeasureKey),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MeasurementCallbackKind {
    Opaque,
    PureSize,
    Text,
}

/// Owner of measured producers and current-frame text layout hydration.
///
/// This is GPUI state, not Taffy state. Taffy can decide whether a measured node
/// cache entry is valid, but GPUI owns the executable producer slots and text
/// artifacts. Taffy's passive cache events may report that it reused a
/// measurement result, but GPUI treats that only as a query observation. Text
/// artifacts are reused only through the exact query key GPUI observed from a
/// callback or cache event; `TextMeasureKey` alone is never enough.
pub(super) struct MeasurementStore {
    producers: ProducerRegistry,
    current_measurements: FxHashMap<NodeId, CurrentMeasurement>,
    text_artifacts: TextArtifactStore,
}

/// Transaction checkpoint for current-frame measurement producers and mappings.
pub(super) struct MeasurementStoreCheckpoint {
    producers: ProducerRegistryCheckpoint,
    current_measurements: FxHashMap<NodeId, CurrentMeasurement>,
    text_artifacts: TextArtifactStoreCheckpoint,
}

impl MeasurementStore {
    pub(super) fn new() -> Self {
        Self {
            producers: ProducerRegistry::new(),
            current_measurements: FxHashMap::default(),
            text_artifacts: TextArtifactStore::new(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.current_measurements.clear();
        self.text_artifacts.begin_frame();
    }

    pub(super) fn finish_frame(&mut self) {
        let current_text_nodes = self
            .current_measurements
            .iter()
            .filter_map(|(node_id, measurement)| {
                matches!(measurement, CurrentMeasurement::Text { .. }).then_some(*node_id)
            })
            .collect::<FxHashSet<_>>();
        self.text_artifacts.finish_frame(&current_text_nodes);
        self.producers.clear();
        self.current_measurements.clear();
    }

    pub(super) fn checkpoint(&self) -> MeasurementStoreCheckpoint {
        MeasurementStoreCheckpoint {
            producers: self.producers.checkpoint(),
            current_measurements: self.current_measurements.clone(),
            text_artifacts: self.text_artifacts.checkpoint(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: MeasurementStoreCheckpoint) {
        self.producers.rollback_to_checkpoint(checkpoint.producers);
        self.current_measurements = checkpoint.current_measurements;
        self.text_artifacts
            .rollback_to_checkpoint(checkpoint.text_artifacts);
    }

    pub(super) fn push_producer_context(&mut self, measure_context: LayoutMeasureContext) -> usize {
        self.producers.push(measure_context)
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

    /// Register that an unchanged text node may replay its previous artifact after the solve.
    ///
    /// The caller may only register a retained text node when the retained
    /// occurrence, text key, style, and incoming layout context are unchanged.
    /// This does not hydrate during commit: commit only records the current
    /// retained-tree fact. After stock Taffy solves, GPUI replays the artifact only
    /// if Taffy skipped the text callback and therefore did not already produce a
    /// current artifact.
    pub(super) fn register_unchanged_text_artifact_replay(
        &mut self,
        node_id: NodeId,
        expected_key: &TextMeasureKey,
    ) {
        self.text_artifacts.register_replay(node_id, expected_key);
    }

    /// Hydrate text nodes from the artifact selected by the completed solve.
    ///
    /// Taffy may invoke a callback, reuse one or more cache entries, or skip an
    /// unchanged subtree entirely. Those events are artifact producers, not GPUI
    /// paint-state side effects. Hydration happens once here after the solve,
    /// from `current_text_artifacts`.
    pub(super) fn hydrate_registered_text_artifact_replays(&mut self) {
        let replay_candidates = self.text_artifacts.take_replay_candidates();
        for (node_id, expected_key) in replay_candidates {
            if self.text_artifacts.has_current_artifact(node_id) {
                continue;
            }
            let Some(artifact) = self.text_artifacts.retained_artifact(node_id) else {
                continue;
            };
            assert_eq!(
                artifact.key(),
                &expected_key,
                "retained text artifact should match the current text measure key"
            );
            self.text_artifacts
                .insert_current_artifact(node_id, artifact);
        }

        for (node_id, artifact) in self.text_artifacts.current_artifacts() {
            self.hydrate_text_node(node_id, &artifact);
        }
    }

    pub(super) fn compute_state(&mut self) -> ComputeMeasurementState<'_> {
        ComputeMeasurementState::new(
            &mut self.producers,
            &mut self.current_measurements,
            &mut self.text_artifacts,
        )
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
        let hydrate = self
            .producers
            .context(*measure)
            .and_then(|measure| measure.text_hydrator.as_ref())
            .map(Rc::clone)
            .expect("text measured layout should have a hydrator");
        hydrate(artifact);
    }
}

/// Per-compute bridge between Taffy measurement callbacks and GPUI artifacts.
///
/// Taffy owns whether a measured node callback runs. GPUI hydrates text from a
/// callback result in the same compute. When the same retained text node is
/// measured with the same exact callback query, GPUI can hydrate from its
/// query-keyed artifact cache without running the text producer again.
pub(super) struct ComputeMeasurementState<'a> {
    producers: &'a mut ProducerRegistry,
    current_measurements: &'a mut FxHashMap<NodeId, CurrentMeasurement>,
    text_artifacts: &'a mut TextArtifactStore,
}

impl<'a> ComputeMeasurementState<'a> {
    fn new(
        producers: &'a mut ProducerRegistry,
        current_measurements: &'a mut FxHashMap<NodeId, CurrentMeasurement>,
        text_artifacts: &'a mut TextArtifactStore,
    ) -> Self {
        Self {
            producers,
            current_measurements,
            text_artifacts,
        }
    }

    pub(super) fn has_current_measurement(&self, node_id: NodeId) -> bool {
        self.current_measurements.contains_key(&node_id)
    }

    pub(super) fn callback_kind(&self, node_id: NodeId) -> Option<MeasurementCallbackKind> {
        match self.current_measurements.get(&node_id) {
            Some(CurrentMeasurement::Opaque(_)) => Some(MeasurementCallbackKind::Opaque),
            Some(CurrentMeasurement::PureSize(_)) => Some(MeasurementCallbackKind::PureSize),
            Some(CurrentMeasurement::Text { .. }) => Some(MeasurementCallbackKind::Text),
            None => None,
        }
    }

    pub(super) fn measure(
        &mut self,
        node_id: NodeId,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        let Some(current_measurement) = self.current_measurements.get(&node_id).cloned() else {
            panic!("measured layout mirror node should have a current GPUI measurement producer");
        };

        match current_measurement {
            CurrentMeasurement::Opaque(measure) => {
                let measure = &mut self
                    .producers
                    .context_mut(measure)
                    .expect("opaque measured layout should have a current producer")
                    .measure;
                measure(known_dimensions, available_space, window, cx).size()
            }
            CurrentMeasurement::PureSize(measure) => {
                measure.measure(known_dimensions, available_space)
            }
            CurrentMeasurement::Text { key, measure, .. } => {
                let cache_key = TextArtifactCacheKey::new(
                    node_id,
                    key.clone(),
                    known_dimensions,
                    available_space,
                );
                if let Some(artifact) = self.text_artifacts.artifact_for_query(&cache_key) {
                    let size = artifact.size();
                    self.record_text_artifact_for_query(node_id, cache_key, &artifact);
                    return size;
                }

                let measure_id = measure;
                let artifact = {
                    let measure = &mut self
                        .producers
                        .context_mut(measure_id)
                        .expect("text measured layout should have a current producer and hydrator")
                        .measure;
                    match measure(known_dimensions, available_space, window, cx) {
                        MeasuredLayoutResult::Text(artifact) => artifact,
                        MeasuredLayoutResult::Size(_) => {
                            panic!("text measured layout producer returned a size-only result");
                        }
                    }
                };
                assert_eq!(
                    artifact.key(),
                    &key,
                    "text measured layout artifact should match the current text measure key"
                );
                let size = artifact.size();
                self.record_text_artifact_for_query(node_id, cache_key.clone(), &artifact);
                self.text_artifacts
                    .insert_current_artifact(node_id, artifact.clone());
                self.text_artifacts.cache_artifact(cache_key, artifact);
                size
            }
        }
    }

    pub(super) fn observe_layout_cache_event(
        &mut self,
        event: LayoutCacheEvent,
        scale_factor: f32,
    ) {
        match event {
            LayoutCacheEvent::Hit(entry) => {
                self.record_text_artifact_for_cache_entry_hit(entry, scale_factor)
            }
            LayoutCacheEvent::Stored(entry) => {
                self.store_text_artifact_for_cache_entry(entry, scale_factor)
            }
            LayoutCacheEvent::Cleared(_) => {}
            _ => {}
        }
    }

    fn record_text_artifact_for_cache_entry_hit(
        &mut self,
        entry: LayoutCacheEntry,
        scale_factor: f32,
    ) {
        let node_id = entry.node_id();
        let Some(CurrentMeasurement::Text { key, .. }) = self.current_measurements.get(&node_id)
        else {
            return;
        };
        let key = key.clone();
        let query_key =
            TextArtifactCacheKey::from_taffy_entry(node_id, key.clone(), entry, scale_factor);
        let Some(artifact) = self.text_artifacts.artifact_for_query(&query_key) else {
            return;
        };
        assert_eq!(
            artifact.key(),
            &key,
            "Taffy cache-hit text artifact should match the current text measure key"
        );
        self.record_text_artifact_for_query(node_id, query_key, &artifact);
    }

    fn store_text_artifact_for_cache_entry(&mut self, entry: LayoutCacheEntry, scale_factor: f32) {
        let node_id = entry.node_id();
        let Some(CurrentMeasurement::Text { key, .. }) = self.current_measurements.get(&node_id)
        else {
            return;
        };
        let key = key.clone();
        let Some(artifact) = self.text_artifacts.current_artifact(node_id) else {
            debug_assert!(
                false,
                "text cache store should follow a text measurement callback or cache hydration"
            );
            return;
        };
        assert_eq!(
            artifact.key(),
            &key,
            "stored Taffy cache-event text artifact should match the current text measure key"
        );
        let query_key =
            TextArtifactCacheKey::from_taffy_entry(node_id, key.clone(), entry, scale_factor);
        self.record_text_artifact_for_query(node_id, query_key.clone(), &artifact);
        self.text_artifacts.cache_artifact(query_key, artifact);
    }

    fn record_text_artifact_for_query(
        &mut self,
        node_id: NodeId,
        cache_key: TextArtifactCacheKey,
        artifact: &TextLayoutArtifact,
    ) {
        self.text_artifacts
            .record_for_query(node_id, cache_key, artifact);
    }
}
