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
/// `Text` is still an explicit layout fact, but it is conservative under stock
/// Taffy because the shaped artifact also depends on Taffy's measurement query
/// (`known_dimensions` and `available_space`), which GPUI cannot observe when
/// Taffy reuses an internal measurement result.
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
/// artifacts. Under stock Taffy, text artifacts are hydrated only from
/// callbacks that run during the current compute because `TextMeasureKey` does
/// not include the full Taffy measurement query.
pub(super) struct MeasurementStore {
    producer_contexts: Vec<Option<LayoutMeasureContext>>,
    current_measurements: FxHashMap<NodeId, CurrentMeasurement>,
    current_text_artifacts: FxHashMap<NodeId, TextLayoutArtifact>,
}

/// Transaction checkpoint for current-frame measurement producers and mappings.
pub(super) struct MeasurementStoreCheckpoint {
    producer_contexts_len: usize,
    current_measurements: FxHashMap<NodeId, CurrentMeasurement>,
    current_text_artifacts: FxHashMap<NodeId, TextLayoutArtifact>,
}

impl MeasurementStore {
    pub(super) fn new() -> Self {
        Self {
            producer_contexts: Vec::new(),
            current_measurements: FxHashMap::default(),
            current_text_artifacts: FxHashMap::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.current_measurements.clear();
        self.current_text_artifacts.clear();
    }

    pub(super) fn finish_frame(&mut self) {
        self.producer_contexts.clear();
        self.current_measurements.clear();
        self.current_text_artifacts.clear();
    }

    pub(super) fn checkpoint(&self) -> MeasurementStoreCheckpoint {
        MeasurementStoreCheckpoint {
            producer_contexts_len: self.producer_contexts.len(),
            current_measurements: self.current_measurements.clone(),
            current_text_artifacts: self.current_text_artifacts.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: MeasurementStoreCheckpoint) {
        self.producer_contexts
            .truncate(checkpoint.producer_contexts_len);
        self.current_measurements = checkpoint.current_measurements;
        self.current_text_artifacts = checkpoint.current_text_artifacts;
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

    pub(super) fn compute_state(&mut self) -> ComputeMeasurementState<'_> {
        ComputeMeasurementState::new(
            &mut self.producer_contexts,
            &mut self.current_measurements,
            &mut self.current_text_artifacts,
        )
    }

    pub(super) fn current_text_artifacts(&self) -> FxHashMap<NodeId, TextLayoutArtifact> {
        self.current_text_artifacts.clone()
    }

    pub(super) fn hydrate_text_artifacts(
        &self,
        artifacts: &FxHashMap<NodeId, TextLayoutArtifact>,
    ) -> u64 {
        let mut hydrated = 0;
        for (node_id, artifact) in artifacts {
            let Some(CurrentMeasurement::Text { key, measure }) =
                self.current_measurements.get(node_id)
            else {
                panic!("snapshot text artifact should correspond to a current text measured node");
            };
            assert_eq!(
                artifact.key(),
                key,
                "snapshot text artifact should match the current text measure key"
            );
            let hydrate = self.producer_contexts[*measure]
                .as_ref()
                .and_then(|measure| measure.text_hydrator.as_ref())
                .map(Rc::clone)
                .expect("text measured layout should have a current hydrator");
            hydrate(artifact);
            hydrated += 1;
        }
        hydrated
    }
}

/// Per-compute bridge between Taffy measurement callbacks and GPUI artifacts.
///
/// Taffy owns whether a measured node callback runs. GPUI hydrates text only
/// from a callback result in the same compute because stock Taffy does not
/// expose the measurement query that selected a cached text result. Replaying a
/// text artifact from `TextMeasureKey` alone is under-keyed and therefore
/// forbidden.
pub(super) struct ComputeMeasurementState<'a> {
    producer_contexts: &'a mut Vec<Option<LayoutMeasureContext>>,
    current_measurements: &'a mut FxHashMap<NodeId, CurrentMeasurement>,
    current_text_artifacts: &'a mut FxHashMap<NodeId, TextLayoutArtifact>,
}

impl<'a> ComputeMeasurementState<'a> {
    pub(super) fn new(
        producer_contexts: &'a mut Vec<Option<LayoutMeasureContext>>,
        current_measurements: &'a mut FxHashMap<NodeId, CurrentMeasurement>,
        current_text_artifacts: &'a mut FxHashMap<NodeId, TextLayoutArtifact>,
    ) -> Self {
        Self {
            producer_contexts,
            current_measurements,
            current_text_artifacts,
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
                self.current_text_artifacts.insert(node_id, artifact);
                size
            }
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
