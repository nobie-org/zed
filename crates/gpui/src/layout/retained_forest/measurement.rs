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
use std::rc::Rc;
use taffy::tree::NodeId;

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
    text_artifacts: FxHashMap<TextArtifactKey, TextLayoutArtifact>,
    current_text_descendants_by_node: FxHashMap<NodeId, Vec<NodeId>>,
}

/// Transaction checkpoint for measurement producers and artifacts.
pub(super) struct MeasurementStoreCheckpoint {
    producer_contexts_len: usize,
    current_measurements: FxHashMap<NodeId, CurrentMeasurement>,
    text_artifacts: FxHashMap<TextArtifactKey, TextLayoutArtifact>,
    current_text_descendants_by_node: FxHashMap<NodeId, Vec<NodeId>>,
}

impl MeasurementStore {
    pub(super) fn new() -> Self {
        Self {
            producer_contexts: Vec::new(),
            current_measurements: FxHashMap::default(),
            text_artifacts: FxHashMap::default(),
            current_text_descendants_by_node: FxHashMap::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.current_measurements.clear();
    }

    pub(super) fn finish_frame(&mut self) {
        self.producer_contexts.clear();
        self.current_measurements.clear();
        self.current_text_descendants_by_node.clear();
    }

    pub(super) fn checkpoint(&self) -> MeasurementStoreCheckpoint {
        MeasurementStoreCheckpoint {
            producer_contexts_len: self.producer_contexts.len(),
            current_measurements: self.current_measurements.clone(),
            text_artifacts: self.text_artifacts.clone(),
            current_text_descendants_by_node: self.current_text_descendants_by_node.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: MeasurementStoreCheckpoint) {
        self.producer_contexts
            .truncate(checkpoint.producer_contexts_len);
        self.current_measurements = checkpoint.current_measurements;
        self.text_artifacts = checkpoint.text_artifacts;
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
        self.text_artifacts.retain(|key, _| key.node_id != node_id);
    }

    pub(super) fn compute_state(&mut self) -> ComputeMeasurementState<'_> {
        ComputeMeasurementState::new(
            &mut self.producer_contexts,
            &mut self.current_measurements,
            &mut self.text_artifacts,
            &self.current_text_descendants_by_node,
        )
    }
}

/// Cache key for a retained text artifact owned by GPUI.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct TextArtifactKey {
    pub(super) node_id: NodeId,
    measure_key: TextMeasureKey,
}

/// Per-compute bridge between Taffy measurement callbacks and GPUI artifacts.
///
/// Taffy owns whether a measured node callback runs. GPUI owns text artifacts:
/// when a callback runs, the returned artifact is hydrated and retained; when a
/// callback is skipped, the retained artifact for the same node/key is replayed
/// into the current frame's `TextLayout`.
pub(super) struct ComputeMeasurementState<'a> {
    producer_contexts: &'a mut Vec<Option<LayoutMeasureContext>>,
    current_measurements: &'a mut FxHashMap<NodeId, CurrentMeasurement>,
    text_artifacts: &'a mut FxHashMap<TextArtifactKey, TextLayoutArtifact>,
    current_text_descendants_by_node: &'a FxHashMap<NodeId, Vec<NodeId>>,
    hydrated_text_nodes: FxHashMap<NodeId, TextMeasureKey>,
}

impl<'a> ComputeMeasurementState<'a> {
    pub(super) fn new(
        producer_contexts: &'a mut Vec<Option<LayoutMeasureContext>>,
        current_measurements: &'a mut FxHashMap<NodeId, CurrentMeasurement>,
        text_artifacts: &'a mut FxHashMap<TextArtifactKey, TextLayoutArtifact>,
        current_text_descendants_by_node: &'a FxHashMap<NodeId, Vec<NodeId>>,
    ) -> Self {
        Self {
            producer_contexts,
            current_measurements,
            text_artifacts,
            current_text_descendants_by_node,
            hydrated_text_nodes: FxHashMap::default(),
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
        let Some(current_measurement) = self.current_measurements.get(&node_id).cloned() else {
            panic!("measured layout mirror node should have a current GPUI measurement producer");
        };

        match current_measurement {
            CurrentMeasurement::Opaque(measure) => {
                let measure = &mut self.producer_contexts[measure]
                    .as_mut()
                    .expect("opaque measured layout should have a current producer")
                    .measure;
                measure(known_dimensions, available_space, window, cx).size()
            }
            CurrentMeasurement::PureSize(measure) => {
                measure.measure(known_dimensions, available_space)
            }
            CurrentMeasurement::Text { key, measure, .. } => {
                let measure_id = measure;
                let artifact = {
                    let measure = &mut self.producer_contexts[measure_id]
                        .as_mut()
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
                self.hydrate_text_node(node_id, &artifact);
                self.text_artifacts.insert(
                    TextArtifactKey {
                        node_id,
                        measure_key: key.clone(),
                    },
                    artifact,
                );
                self.hydrated_text_nodes.insert(node_id, key.clone());
                size
            }
        }
    }

    pub(super) fn finish_compute(&mut self, root_node_id: NodeId) {
        let Some(text_descendants) = self.current_text_descendants_by_node.get(&root_node_id)
        else {
            return;
        };
        for node_id in text_descendants.clone() {
            let Some(key) = self.current_text_key(node_id).cloned() else {
                continue;
            };
            if self.hydrated_text_nodes.get(&node_id) == Some(&key) {
                continue;
            }
            let Some(artifact) = self
                .text_artifacts
                .get(&TextArtifactKey {
                    node_id,
                    measure_key: key.clone(),
                })
                .cloned()
            else {
                continue;
            };
            self.hydrate_text_node(node_id, &artifact);
            self.hydrated_text_nodes.insert(node_id, key);
        }
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
