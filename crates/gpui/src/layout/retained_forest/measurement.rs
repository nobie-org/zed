//! Measurement support for the retained layout forest.
//!
//! This module keeps the three measured-node concepts separate:
//! pure comparable measure facts, current-frame executable producers, and GPUI
//! artifacts such as shaped text. Solver node context stays a private marker and
//! never stores GPUI paint or hit-test state.

mod producer_registry;
mod text_artifacts;

use super::super::LayoutId;
use super::AvailableSpace;
use super::solver::{SolverCacheEntry, SolverCacheEvent, SolverNodeId};
use crate::{App, Pixels, Size, TextLayoutArtifact, TextMeasureKey, Window, size};
use collections::FxHashMap;
use producer_registry::{ProducerRegistry, ProducerRegistryCheckpoint};
use stacksafe::StackSafe;
use std::rc::Rc;
use text_artifacts::{
    PendingTextArtifactQuery, TextArtifactCacheKey, TextArtifactStore, TextArtifactStoreCheckpoint,
};

/// Current-frame executable producer for measured layout.
///
/// This callback can call into GPUI/window state while the solver asks for a size. It
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
/// The marker only tells the solver that a node is measured. GPUI side effects and
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
/// from the exact measurement query the solver asked in the current compute.
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

/// Measurement-owned request data for one measured layout intent.
///
/// The retained forest treats this as a generic measured node. This type owns
/// the distinction between opaque producers, pure size facts, and text artifact
/// hydration so retained tree code does not need text-specific constructors.
pub(in crate::layout) struct MeasuredLayoutRequest {
    facts: MeasuredLayoutFacts,
    context: Option<LayoutMeasureContext>,
}

impl MeasuredLayoutRequest {
    pub(in crate::layout) fn opaque(
        mut measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &mut App,
        ) -> Size<Pixels>
        + 'static,
    ) -> Self {
        Self {
            facts: MeasuredLayoutFacts::opaque(),
            context: Some(LayoutMeasureContext {
                measure: StackSafe::new(Box::new(
                    move |known_dimensions, available_space, window, cx| {
                        MeasuredLayoutResult::Size(measure(
                            known_dimensions,
                            available_space,
                            window,
                            cx,
                        ))
                    },
                )),
                text_hydrator: None,
            }),
        }
    }

    pub(in crate::layout) fn pure_size(measure: PureSizeMeasure) -> Self {
        Self {
            facts: MeasuredLayoutFacts::pure_size(measure),
            context: None,
        }
    }

    pub(in crate::layout) fn text(
        measure_key: TextMeasureKey,
        hydrate: impl Fn(&TextLayoutArtifact) + 'static,
        mut measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &mut App,
        ) -> TextLayoutArtifact
        + 'static,
    ) -> Self {
        Self {
            facts: MeasuredLayoutFacts::text(measure_key),
            context: Some(LayoutMeasureContext {
                measure: StackSafe::new(Box::new(
                    move |known_dimensions, available_space, window, cx| {
                        MeasuredLayoutResult::Text(measure(
                            known_dimensions,
                            available_space,
                            window,
                            cx,
                        ))
                    },
                )),
                text_hydrator: Some(Rc::new(hydrate)),
            }),
        }
    }

    fn into_parts(self) -> (MeasuredLayoutFacts, Option<LayoutMeasureContext>) {
        (self.facts, self.context)
    }
}

/// Explicit, comparable size functions for measured nodes.
///
/// A value of this type is layout-visible data. If it is unchanged and the
/// retained occurrence is reused, the forest can let the solver reuse measurement
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

    /// Evaluate the pure measure data for the solver's current measurement query.
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

/// Comparable measured-node facts.
///
/// `Opaque` means no retained measurement identity has been proven. `PureSize`
/// is explicit data and may preserve solver measurement cache when unchanged.
/// `Text` is explicit layout identity, but the shaped artifact also depends on
/// the solver's measurement query (`known_dimensions` and `available_space`). GPUI
/// therefore hydrates artifacts from callbacks or exact query-keyed cache
/// observations; it never treats `TextMeasureKey` alone as artifact proof.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct MeasuredLayoutFacts(MeasuredLayoutFactsRepr);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum MeasuredLayoutFactsRepr {
    Opaque,
    PureSize(PureSizeMeasure),
    Text(TextMeasureKey),
}

impl MeasuredLayoutFacts {
    pub(super) fn opaque() -> Self {
        Self(MeasuredLayoutFactsRepr::Opaque)
    }

    pub(super) fn pure_size(measure: PureSizeMeasure) -> Self {
        Self(MeasuredLayoutFactsRepr::PureSize(measure))
    }

    pub(super) fn text(key: TextMeasureKey) -> Self {
        Self(MeasuredLayoutFactsRepr::Text(key))
    }

    pub(super) fn can_reuse_solver_node_with(&self, current: &Self) -> bool {
        matches!(
            (&self.0, &current.0),
            (
                MeasuredLayoutFactsRepr::PureSize(_),
                MeasuredLayoutFactsRepr::PureSize(_)
            ) | (
                MeasuredLayoutFactsRepr::Text(_),
                MeasuredLayoutFactsRepr::Text(_)
            )
        )
    }

    pub(super) fn is_opaque(&self) -> bool {
        matches!(self.0, MeasuredLayoutFactsRepr::Opaque)
    }

    pub(super) fn measure_for_fresh_compare(
        &self,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        match &self.0 {
            MeasuredLayoutFactsRepr::Opaque => {
                unreachable!("opaque measured nodes skip fresh comparison")
            }
            MeasuredLayoutFactsRepr::PureSize(measure) => {
                measure.measure(known_dimensions, available_space)
            }
            MeasuredLayoutFactsRepr::Text(key) => key
                .measure(known_dimensions, available_space, window, cx)
                .size(),
        }
    }
}

/// Registered current-frame measured request.
///
/// The retained fact tree stores only `MeasuredLayoutFacts`. This value remains
/// inside `MeasurementStore` until the current `LayoutId` is committed to one
/// solver node, preventing producer slots or text artifacts from becoming part
/// of the descriptor tree.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RegisteredMeasuredLayout {
    facts: MeasuredLayoutFacts,
    measurement: CurrentMeasurement,
}

impl RegisteredMeasuredLayout {
    pub(super) fn facts(&self) -> &MeasuredLayoutFacts {
        &self.facts
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

/// Telemetry classification for one measured callback.
///
/// The measurement owner decides how callback work should be counted. Subtree
/// proof instrumentation should not know whether the producer is text, a list,
/// or any future measured kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MeasuredCallbackTelemetry {
    no_work_exempt: bool,
}

impl MeasuredCallbackTelemetry {
    fn hard_work() -> Self {
        Self {
            no_work_exempt: false,
        }
    }

    fn no_work_exempt() -> Self {
        Self {
            no_work_exempt: true,
        }
    }

    pub(super) fn is_no_work_exempt(self) -> bool {
        self.no_work_exempt
    }
}

/// Owner of measured producers and current-frame text layout hydration.
///
/// This is GPUI state, not solver state. The solver can decide whether a measured node
/// cache entry is valid, but GPUI owns the executable producer slots and text
/// artifacts. Passive solver cache events may report that it reused a
/// measurement result or final-layout cache entry, but GPUI treats those only as
/// current-solve observations. Text artifacts are reused through an exact query
/// key or through a final-layout cache entry that selected the same current text
/// descendants. Retained node identity and `TextMeasureKey` alone are never
/// artifact proof.
pub(super) struct MeasurementStore {
    producers: ProducerRegistry,
    pending_measurements: FxHashMap<LayoutId, RegisteredMeasuredLayout>,
    current_measurements: FxHashMap<SolverNodeId, CurrentMeasurement>,
    text_artifacts: TextArtifactStore,
}

/// Transaction checkpoint for current-frame measurement producers and mappings.
pub(super) struct MeasurementStoreCheckpoint {
    producers: ProducerRegistryCheckpoint,
    pending_measurements: FxHashMap<LayoutId, RegisteredMeasuredLayout>,
    current_measurements: FxHashMap<SolverNodeId, CurrentMeasurement>,
    text_artifacts: TextArtifactStoreCheckpoint,
}

/// Measurement-owned observation table for one legal solver root solve.
///
/// Solver cache events name a node. This table maps that node to the current
/// measured descendants whose paint/hit-test artifacts may be selected by the
/// event. Keeping it here prevents the retained forest root from knowing which
/// measured-node kind produces artifacts.
pub(super) struct MeasurementSolveObserver {
    artifact_descendants_by_node: FxHashMap<SolverNodeId, Vec<SolverNodeId>>,
    root_artifact_descendants: Vec<SolverNodeId>,
}

impl MeasurementSolveObserver {
    fn root_artifact_descendants(&self) -> &[SolverNodeId] {
        &self.root_artifact_descendants
    }

    fn artifact_descendants_for_event(&self, event: SolverCacheEvent) -> &[SolverNodeId] {
        let event_node_id = match event {
            SolverCacheEvent::Hit(entry) | SolverCacheEvent::Stored(entry) => entry.node_id(),
            SolverCacheEvent::Cleared(clear) => clear.node_id(),
        };
        self.artifact_descendants_by_node
            .get(&event_node_id)
            .map(Vec::as_slice)
            .expect("solver cache event node should belong to the current layout root")
    }
}

impl MeasurementStore {
    pub(super) fn new() -> Self {
        Self {
            producers: ProducerRegistry::new(),
            pending_measurements: FxHashMap::default(),
            current_measurements: FxHashMap::default(),
            text_artifacts: TextArtifactStore::new(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.pending_measurements.clear();
        self.current_measurements.clear();
        self.text_artifacts.begin_frame();
    }

    pub(super) fn finish_frame(&mut self) {
        self.text_artifacts.finish_frame();
        self.producers.clear();
        self.pending_measurements.clear();
        self.current_measurements.clear();
    }

    pub(super) fn checkpoint(&self) -> MeasurementStoreCheckpoint {
        MeasurementStoreCheckpoint {
            producers: self.producers.checkpoint(),
            pending_measurements: self.pending_measurements.clone(),
            current_measurements: self.current_measurements.clone(),
            text_artifacts: self.text_artifacts.checkpoint(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: MeasurementStoreCheckpoint) {
        self.producers.rollback_to_checkpoint(checkpoint.producers);
        self.pending_measurements = checkpoint.pending_measurements;
        self.current_measurements = checkpoint.current_measurements;
        self.text_artifacts
            .rollback_to_checkpoint(checkpoint.text_artifacts);
    }

    pub(super) fn register_request(
        &mut self,
        request: MeasuredLayoutRequest,
    ) -> RegisteredMeasuredLayout {
        let (facts, context) = request.into_parts();
        let measurement = match facts.0.clone() {
            MeasuredLayoutFactsRepr::Opaque => {
                let producer = self
                    .producers
                    .push(context.expect("opaque measured layout request should have a producer"));
                CurrentMeasurement::Opaque(producer)
            }
            MeasuredLayoutFactsRepr::PureSize(measure) => {
                assert!(
                    context.is_none(),
                    "pure-size measured layout request should not have a producer"
                );
                CurrentMeasurement::PureSize(measure)
            }
            MeasuredLayoutFactsRepr::Text(key) => {
                let producer = self
                    .producers
                    .push(context.expect("text measured layout request should have a producer"));
                CurrentMeasurement::Text {
                    key,
                    measure: producer,
                }
            }
        };
        RegisteredMeasuredLayout { facts, measurement }
    }

    pub(super) fn bind_registered_request(
        &mut self,
        layout_id: LayoutId,
        request: RegisteredMeasuredLayout,
    ) {
        let previous = self.pending_measurements.insert(layout_id, request);
        assert!(
            previous.is_none(),
            "measured layout id should have one current measurement request"
        );
    }

    #[cfg(test)]
    pub(super) fn push_producer_context_for_tests(
        &mut self,
        measure_context: LayoutMeasureContext,
    ) -> usize {
        self.producers.push(measure_context)
    }

    #[cfg(test)]
    pub(super) fn insert_current_measurement(
        &mut self,
        node_id: SolverNodeId,
        measurement: CurrentMeasurement,
    ) {
        self.current_measurements.insert(node_id, measurement);
    }

    pub(super) fn insert_current_measurement_for_layout(
        &mut self,
        node_id: SolverNodeId,
        layout_id: LayoutId,
        facts: &MeasuredLayoutFacts,
    ) {
        let measured = self
            .pending_measurements
            .remove(&layout_id)
            .expect("measured layout id should have a registered current measurement request");
        assert_eq!(
            &measured.facts, facts,
            "registered measured request facts should match the committed intent"
        );
        self.current_measurements
            .insert(node_id, measured.measurement);
    }

    pub(super) fn has_current_measurement(&self, node_id: SolverNodeId) -> bool {
        self.current_measurements.contains_key(&node_id)
    }

    pub(super) fn current_measurement(&self, node_id: SolverNodeId) -> Option<&CurrentMeasurement> {
        self.current_measurements.get(&node_id)
    }

    fn has_current_artifact_measurement(&self, node_id: SolverNodeId) -> bool {
        matches!(
            self.current_measurement(node_id),
            Some(CurrentMeasurement::Text { .. })
        )
    }

    pub(super) fn solve_observer(
        &self,
        root: SolverNodeId,
        mut children: impl FnMut(SolverNodeId) -> Vec<SolverNodeId>,
        mut can_produce_artifacts: impl FnMut(SolverNodeId) -> bool,
    ) -> MeasurementSolveObserver {
        let mut artifact_descendants_by_node = FxHashMap::default();
        let root_artifact_descendants = self.collect_artifact_descendants(
            root,
            &mut children,
            &mut can_produce_artifacts,
            &mut artifact_descendants_by_node,
        );
        MeasurementSolveObserver {
            artifact_descendants_by_node,
            root_artifact_descendants,
        }
    }

    fn collect_artifact_descendants(
        &self,
        node_id: SolverNodeId,
        children: &mut impl FnMut(SolverNodeId) -> Vec<SolverNodeId>,
        can_produce_artifacts: &mut impl FnMut(SolverNodeId) -> bool,
        descendants_by_node: &mut FxHashMap<SolverNodeId, Vec<SolverNodeId>>,
    ) -> Vec<SolverNodeId> {
        if !can_produce_artifacts(node_id) {
            for child in children(node_id) {
                Self::collect_artifact_ineligible_descendants(child, children, descendants_by_node);
            }
            descendants_by_node.insert(node_id, Vec::new());
            return Vec::new();
        }

        let mut descendants = Vec::new();
        if self.has_current_artifact_measurement(node_id) {
            descendants.push(node_id);
        }

        for child in children(node_id) {
            descendants.extend(self.collect_artifact_descendants(
                child,
                children,
                can_produce_artifacts,
                descendants_by_node,
            ));
        }

        descendants_by_node.insert(node_id, descendants.clone());
        descendants
    }

    fn collect_artifact_ineligible_descendants(
        node_id: SolverNodeId,
        children: &mut impl FnMut(SolverNodeId) -> Vec<SolverNodeId>,
        descendants_by_node: &mut FxHashMap<SolverNodeId, Vec<SolverNodeId>>,
    ) {
        for child in children(node_id) {
            Self::collect_artifact_ineligible_descendants(child, children, descendants_by_node);
        }
        descendants_by_node.insert(node_id, Vec::new());
    }

    #[cfg(any(test, debug_assertions))]
    pub(super) fn debug_assert_current_measurement_matches(
        &self,
        node_id: SolverNodeId,
        facts: &MeasuredLayoutFacts,
    ) {
        match (&facts.0, self.current_measurement(node_id)) {
            (MeasuredLayoutFactsRepr::Opaque, Some(CurrentMeasurement::Opaque(_))) => {}
            (
                MeasuredLayoutFactsRepr::PureSize(expected),
                Some(CurrentMeasurement::PureSize(actual)),
            ) => assert_eq!(actual, expected),
            (
                MeasuredLayoutFactsRepr::Text(expected),
                Some(CurrentMeasurement::Text { key: actual, .. }),
            ) => assert_eq!(actual, expected),
            _ => {
                panic!("measured intent should have matching current measurement state")
            }
        }
    }

    /// Hydrate text nodes from the artifacts selected by the completed solve.
    ///
    /// The solver may invoke a callback or report a passive cache hit/store event.
    /// Those events are artifact producers, not GPUI paint-state side effects.
    /// Hydration happens once here after the solve. There is no retained-node
    /// artifact hydration path; if the solver skips a text node without reporting an
    /// exact measurement query, GPUI has no artifact proof for that node.
    pub(super) fn finish_completed_solve(
        &mut self,
        observer: &MeasurementSolveObserver,
        window: &mut Window,
        cx: &mut App,
    ) {
        for query in self.text_artifacts.take_pending_queries() {
            let cache_key = query.cache_key();
            if let Some(artifact) = self.text_artifacts.current_artifact_for_query(&cache_key) {
                text_artifact_matches_query(&artifact, &query);
                continue;
            }
            self.hydrate_text_query_from_current_producer(query, window, cx);
        }

        let current_measurements = &self.current_measurements;
        self.text_artifacts.flush_pending_subtree_stores(|node_id| {
            match current_measurements.get(&node_id) {
                Some(CurrentMeasurement::Text { key, .. }) => Some(key.clone()),
                _ => None,
            }
        });

        self.assert_solved_root_text_artifacts_selected(observer);

        for (node_id, artifact) in self.text_artifacts.current_artifacts() {
            self.hydrate_text_node(node_id, &artifact);
        }
    }

    fn assert_solved_root_text_artifacts_selected(&self, observer: &MeasurementSolveObserver) {
        let missing = observer
            .root_artifact_descendants()
            .iter()
            .copied()
            .filter(|node_id| !self.text_artifacts.has_current_artifact(*node_id))
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return;
        }

        let missing = missing
            .iter()
            .map(|node_id| format!("{:?}", node_id))
            .collect::<Vec<_>>()
            .join(", ");
        panic!(
            "completed retained layout solve did not select text artifacts for solved root nodes: {missing}"
        );
    }

    pub(super) fn compute_state(&mut self) -> ComputeMeasurementState<'_> {
        ComputeMeasurementState::new(
            &mut self.producers,
            &mut self.current_measurements,
            &mut self.text_artifacts,
        )
    }

    #[stacksafe::stacksafe]
    fn hydrate_text_query_from_current_producer(
        &mut self,
        query: PendingTextArtifactQuery,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(CurrentMeasurement::Text { key, measure }) =
            self.current_measurements.get(&query.node_id).cloned()
        else {
            panic!("solver cache-hit text query should refer to a current text measurement");
        };
        assert_eq!(
            &key, &query.text_key,
            "solver cache-hit text query should match the current text measure key"
        );

        let artifact = {
            let measure = &mut self
                .producers
                .context_mut(measure)
                .expect("text measured layout should have a current producer and hydrator")
                .measure;
            match measure(query.known_dimensions, query.available_space, window, cx) {
                MeasuredLayoutResult::Text(artifact) => artifact,
                MeasuredLayoutResult::Size(_) => {
                    panic!("text measured layout producer returned a size-only result");
                }
            }
        };
        assert_eq!(
            artifact.key(),
            &key,
            "hydrated solver cache-hit text artifact should match the current text measure key"
        );
        text_artifact_matches_query(&artifact, &query);

        let cache_key = query.cache_key();
        self.text_artifacts
            .record_for_query(query.node_id, cache_key.clone(), &artifact);
        self.text_artifacts.cache_artifact(cache_key, artifact);
    }

    fn hydrate_text_node(&mut self, node_id: SolverNodeId, artifact: &TextLayoutArtifact) {
        let Some(CurrentMeasurement::Text { key, measure }) =
            self.current_measurements.get_mut(&node_id)
        else {
            panic!("selected text artifact should hydrate a current text measurement");
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

/// Per-compute bridge between solver measurement callbacks and GPUI artifacts.
///
/// The solver owns whether a measured node callback runs. GPUI hydrates text from a
/// callback result in the same compute. When the same retained text node is
/// measured with the same exact callback query, GPUI can hydrate from its
/// query-keyed artifact cache without running the text producer again.
pub(super) struct ComputeMeasurementState<'a> {
    producers: &'a mut ProducerRegistry,
    current_measurements: &'a mut FxHashMap<SolverNodeId, CurrentMeasurement>,
    text_artifacts: &'a mut TextArtifactStore,
}

impl<'a> ComputeMeasurementState<'a> {
    fn new(
        producers: &'a mut ProducerRegistry,
        current_measurements: &'a mut FxHashMap<SolverNodeId, CurrentMeasurement>,
        text_artifacts: &'a mut TextArtifactStore,
    ) -> Self {
        Self {
            producers,
            current_measurements,
            text_artifacts,
        }
    }

    pub(super) fn has_current_measurement(&self, node_id: SolverNodeId) -> bool {
        self.current_measurements.contains_key(&node_id)
    }

    pub(super) fn callback_telemetry(
        &self,
        node_id: SolverNodeId,
    ) -> Option<MeasuredCallbackTelemetry> {
        match self.current_measurements.get(&node_id) {
            Some(CurrentMeasurement::Opaque(_)) => Some(MeasuredCallbackTelemetry::hard_work()),
            Some(CurrentMeasurement::PureSize(_)) => Some(MeasuredCallbackTelemetry::hard_work()),
            Some(CurrentMeasurement::Text { .. }) => {
                Some(MeasuredCallbackTelemetry::no_work_exempt())
            }
            None => None,
        }
    }

    #[stacksafe::stacksafe]
    pub(super) fn measure(
        &mut self,
        node_id: SolverNodeId,
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
                self.text_artifacts.cache_artifact(cache_key, artifact);
                size
            }
        }
    }

    pub(super) fn observe_layout_cache_event(
        &mut self,
        event: SolverCacheEvent,
        scale_factor: f32,
        observer: &MeasurementSolveObserver,
    ) {
        let text_descendants = observer.artifact_descendants_for_event(event);
        match event {
            SolverCacheEvent::Hit(entry) => {
                self.record_text_artifact_for_cache_entry_hit(entry, scale_factor);
                self.record_text_artifacts_for_subtree_cache_hit(entry, text_descendants);
            }
            SolverCacheEvent::Stored(entry) => {
                self.store_text_artifact_for_cache_entry(entry, scale_factor);
                self.text_artifacts
                    .push_pending_subtree_store(entry, text_descendants);
            }
            SolverCacheEvent::Cleared(clear) => {
                self.text_artifacts
                    .clear_subtree_cache_entry(clear.node_id());
            }
        }
    }

    fn record_text_artifact_for_cache_entry_hit(
        &mut self,
        entry: SolverCacheEntry,
        scale_factor: f32,
    ) {
        if !entry.is_compute_size() {
            return;
        }
        let node_id = entry.node_id();
        let Some(CurrentMeasurement::Text { key, .. }) = self.current_measurements.get(&node_id)
        else {
            return;
        };
        let key = key.clone();
        let pending_query =
            PendingTextArtifactQuery::from_solver_entry(node_id, key.clone(), entry, scale_factor);
        let query_key = pending_query.cache_key();
        let Some(artifact) = self.text_artifacts.artifact_for_query(&query_key) else {
            self.text_artifacts.push_pending_query(pending_query);
            return;
        };
        assert_eq!(
            artifact.key(),
            &key,
            "solver cache-hit text artifact should match the current text measure key"
        );
        text_artifact_matches_query(&artifact, &pending_query);
        self.record_text_artifact_for_query(node_id, query_key, &artifact);
    }

    fn store_text_artifact_for_cache_entry(&mut self, entry: SolverCacheEntry, scale_factor: f32) {
        if !entry.is_compute_size() {
            return;
        }
        let node_id = entry.node_id();
        let Some(CurrentMeasurement::Text { key, .. }) = self.current_measurements.get(&node_id)
        else {
            return;
        };
        let key = key.clone();
        let query_key =
            TextArtifactCacheKey::from_solver_entry(node_id, key.clone(), entry, scale_factor);
        let Some(artifact) = self.text_artifacts.current_artifact_for_query(&query_key) else {
            // The solver may store final-layout entries for a measured text node
            // whose paint artifact came from a different current query. That
            // is not exact query proof, so GPUI must not persist it under this
            // query key. A later hit for this query will hydrate explicitly.
            return;
        };
        assert_eq!(
            artifact.key(),
            &key,
            "stored solver cache-event text artifact should match the current text measure key"
        );
        self.record_text_artifact_for_query(node_id, query_key.clone(), &artifact);
        self.text_artifacts.cache_artifact(query_key, artifact);
    }

    fn record_text_artifacts_for_subtree_cache_hit(
        &mut self,
        entry: SolverCacheEntry,
        text_descendants: &[SolverNodeId],
    ) {
        let current_measurements = &self.current_measurements;
        self.text_artifacts
            .hydrate_from_subtree_cache_hit(entry, text_descendants, |node_id| {
                match current_measurements.get(&node_id) {
                    Some(CurrentMeasurement::Text { key, .. }) => Some(key.clone()),
                    _ => None,
                }
            });
    }

    fn record_text_artifact_for_query(
        &mut self,
        node_id: SolverNodeId,
        cache_key: TextArtifactCacheKey,
        artifact: &TextLayoutArtifact,
    ) {
        self.text_artifacts
            .record_for_query(node_id, cache_key, artifact);
    }
}

fn text_artifact_matches_query(
    artifact: &TextLayoutArtifact,
    query: &PendingTextArtifactQuery,
) -> bool {
    assert_eq!(
        artifact.key(),
        &query.text_key,
        "current text artifact should match the exact solver query text key"
    );
    true
}
