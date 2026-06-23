//! Measurement support for the retained layout forest.
//!
//! This module keeps the three measured-node concepts separate:
//! pure comparable measure facts, current-frame executable producers, and GPUI
//! artifacts such as shaped text. Solver node context stays a private marker and
//! never stores GPUI paint or hit-test state.

mod artifacts;
mod producer_registry;
#[cfg(test)]
mod tests;

use super::super::LayoutId;
use super::AvailableSpace;
use super::solver::{SolverCacheEvent, SolverMeasureObservation, SolverNodeId};
use crate::{App, MeasureCx, Pixels, Size, Window, size};
use artifacts::{ArtifactCacheKey, ArtifactProof, ArtifactStore, ArtifactStoreCheckpoint};
pub(in crate::layout) use artifacts::{LayoutArtifact, LayoutArtifactKey};
use collections::FxHashMap;
use producer_registry::{ProducerRegistry, ProducerRegistryCheckpoint};
use stacksafe::StackSafe;
use std::rc::Rc;

/// Current-frame executable producer for measured layout.
///
/// The producer answers one solver size query through `MeasureCx`. It is not
/// comparable retained meaning; opaque producers are intentionally conservative
/// unless a call site supplies an explicit pure measure key.
pub(super) type NodeMeasureFn = StackSafe<
    Box<dyn FnMut(Size<Option<Pixels>>, Size<AvailableSpace>, &mut MeasureCx<'_>) -> Size<Pixels>>,
>;

type ArtifactMeasureFn = StackSafe<
    Box<
        dyn FnMut(Size<Option<Pixels>>, Size<AvailableSpace>, &mut MeasureCx<'_>) -> LayoutArtifact,
    >,
>;

/// Private marker installed on measured mirror nodes.
///
/// The marker only tells the solver that a node is measured. GPUI side effects and
/// artifacts live in forest-owned maps keyed by explicit retained facts.
#[derive(Clone)]
pub(super) struct NodeContext;

type ArtifactHydrator = Rc<dyn Fn(&LayoutArtifact)>;

/// Current-frame producer bundle registered for a measured facts.
///
/// Producer contexts are frame-local executable effects. The solver-facing
/// callback always receives a size; artifact producers are a separate family so
/// layout measurement cannot pretend that paint/hit-test artifacts are a solver
/// result type.
pub(in crate::layout) enum LayoutMeasureContext {
    Size(NodeMeasureFn),
    Artifact(ArtifactMeasureContext),
}

/// Current-frame executable producer for a measured node with a GPUI artifact.
pub(in crate::layout) struct ArtifactMeasureContext {
    pub(super) measure: ArtifactMeasureFn,
    pub(super) hydrate: ArtifactHydrator,
}

/// Measurement-owned request data for one measured layout facts.
///
/// The retained forest treats this as a generic measured node. This type owns
/// the distinction between opaque producers, pure size facts, and artifact
/// hydration so retained tree code cannot construct impossible combinations such
/// as a pure-size node with a callback or an artifact node without a producer.
pub(in crate::layout) struct MeasuredLayoutRequest(MeasuredLayoutRequestRepr);

enum MeasuredLayoutRequestRepr {
    Opaque(LayoutMeasureContext),
    PureSize(PureSizeMeasure),
    Artifact {
        facts: MeasuredLayoutFacts,
        key: LayoutArtifactKey,
        context: LayoutMeasureContext,
    },
}

impl MeasuredLayoutRequest {
    pub(in crate::layout) fn opaque(
        mut measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut MeasureCx<'_>,
        ) -> Size<Pixels>
        + 'static,
    ) -> Self {
        Self(MeasuredLayoutRequestRepr::Opaque(
            LayoutMeasureContext::Size(StackSafe::new(Box::new(
                move |known_dimensions, available_space, measure_cx| {
                    measure(known_dimensions, available_space, measure_cx)
                },
            ))),
        ))
    }

    pub(in crate::layout) fn pure_size(measure: PureSizeMeasure) -> Self {
        Self(MeasuredLayoutRequestRepr::PureSize(measure))
    }

    pub(in crate::layout) fn artifact(
        key: LayoutArtifactKey,
        hydrate: impl Fn(&LayoutArtifact) + 'static,
        mut measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut MeasureCx<'_>,
        ) -> LayoutArtifact
        + 'static,
    ) -> Self {
        Self::artifact_with_facts(
            MeasuredLayoutFacts::artifact(key.clone()),
            key,
            hydrate,
            measure,
        )
    }

    pub(in crate::layout) fn artifact_with_pure_size(
        layout_measure: PureSizeMeasure,
        key: LayoutArtifactKey,
        hydrate: impl Fn(&LayoutArtifact) + 'static,
        measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut MeasureCx<'_>,
        ) -> LayoutArtifact
        + 'static,
    ) -> Self {
        Self::artifact_with_facts(
            MeasuredLayoutFacts::pure_size(layout_measure),
            key,
            hydrate,
            measure,
        )
    }

    fn artifact_with_facts(
        facts: MeasuredLayoutFacts,
        key: LayoutArtifactKey,
        hydrate: impl Fn(&LayoutArtifact) + 'static,
        mut measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut MeasureCx<'_>,
        ) -> LayoutArtifact
        + 'static,
    ) -> Self {
        Self(MeasuredLayoutRequestRepr::Artifact {
            facts,
            key,
            context: LayoutMeasureContext::Artifact(ArtifactMeasureContext {
                measure: StackSafe::new(Box::new(
                    move |known_dimensions, available_space, measure_cx| {
                        measure(known_dimensions, available_space, measure_cx)
                    },
                )),
                hydrate: Rc::new(hydrate),
            }),
        })
    }
}

/// Explicit, comparable size functions for measured nodes.
///
/// A value of this type is layout-visible data. If it is unchanged and the
/// retained node is reused, the forest can let the solver reuse measurement
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
/// Artifact-producing measured nodes carry explicit layout identity, but their
/// render artifact also depends on the solver's measurement query. GPUI
/// therefore hydrates artifacts from callbacks or exact query-keyed observations;
/// retained node identity alone is never artifact proof.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct MeasuredLayoutFacts(MeasuredLayoutFactsRepr);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum MeasuredLayoutFactsRepr {
    Opaque,
    PureSize(PureSizeMeasure),
    Artifact(LayoutArtifactKey),
}

impl MeasuredLayoutFacts {
    pub(super) fn opaque() -> Self {
        Self(MeasuredLayoutFactsRepr::Opaque)
    }

    pub(super) fn pure_size(measure: PureSizeMeasure) -> Self {
        Self(MeasuredLayoutFactsRepr::PureSize(measure))
    }

    pub(super) fn artifact(key: LayoutArtifactKey) -> Self {
        Self(MeasuredLayoutFactsRepr::Artifact(key))
    }

    pub(super) fn can_reuse_solver_node_with(&self, current: &Self) -> bool {
        matches!(
            (&self.0, &current.0),
            (
                MeasuredLayoutFactsRepr::PureSize(_),
                MeasuredLayoutFactsRepr::PureSize(_)
            ) | (
                MeasuredLayoutFactsRepr::Artifact(_),
                MeasuredLayoutFactsRepr::Artifact(_)
            )
        )
    }

    pub(super) fn supports_fresh_compare(&self) -> bool {
        !matches!(self.0, MeasuredLayoutFactsRepr::Opaque)
    }

    pub(super) fn measure_for_fresh_compare(
        &self,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &App,
    ) -> Size<Pixels> {
        match &self.0 {
            MeasuredLayoutFactsRepr::Opaque => {
                unreachable!("opaque measured nodes skip fresh comparison")
            }
            MeasuredLayoutFactsRepr::PureSize(measure) => {
                measure.measure(known_dimensions, available_space)
            }
            MeasuredLayoutFactsRepr::Artifact(key) => {
                key.measure_for_fresh_compare(known_dimensions, available_space, window, cx)
            }
        }
    }
}

/// Registered current-frame measured request.
///
/// The retained fact tree stores only `MeasuredLayoutFacts`. This value remains
/// inside `MeasurementStore` until the current `LayoutId` is committed to one
/// solver node, preventing producer slots or artifacts from becoming part
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
    Artifact {
        facts: MeasuredLayoutFacts,
        key: LayoutArtifactKey,
        measure: usize,
    },
}

/// Telemetry classification for one measured solver query.
///
/// The measurement owner decides whether the query required GPUI measurement
/// work or was answered from an exact retained artifact/cache proof. Subtree
/// proof instrumentation should not know whether the producer is text, a list,
/// or any future measured kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MeasuredCallbackTelemetry {
    no_work_exempt: bool,
    counts_layout_work: bool,
}

impl MeasuredCallbackTelemetry {
    fn hard_work() -> Self {
        Self {
            no_work_exempt: false,
            counts_layout_work: true,
        }
    }

    fn no_work_exempt() -> Self {
        Self {
            no_work_exempt: true,
            counts_layout_work: false,
        }
    }

    pub(super) fn is_no_work_exempt(self) -> bool {
        self.no_work_exempt
    }

    pub(super) fn counts_layout_work(self) -> bool {
        self.counts_layout_work
    }
}

/// Size answer for one measured solver query plus GPUI work attribution.
pub(super) struct MeasuredQueryAnswer {
    size: Size<Pixels>,
    telemetry: MeasuredCallbackTelemetry,
}

impl MeasuredQueryAnswer {
    fn hard_work(size: Size<Pixels>) -> Self {
        Self {
            size,
            telemetry: MeasuredCallbackTelemetry::hard_work(),
        }
    }

    fn no_work(size: Size<Pixels>) -> Self {
        Self {
            size,
            telemetry: MeasuredCallbackTelemetry::no_work_exempt(),
        }
    }

    pub(super) fn size(&self) -> Size<Pixels> {
        self.size
    }

    pub(super) fn telemetry(&self) -> MeasuredCallbackTelemetry {
        self.telemetry
    }
}

/// Owner of measured producers and current-frame artifact hydration.
///
/// This is GPUI state, not solver state. The solver can decide whether a measured node
/// cache entry is valid, but GPUI owns the executable producer slots and
/// artifacts. Passive solver cache events may report that it reused a
/// measurement result or final-layout cache entry, but GPUI treats those only as
/// current-solve observations. Artifacts are reused through an exact query
/// key. Retained node identity is never artifact proof.
pub(super) struct MeasurementStore {
    producers: ProducerRegistry,
    pending_measurements: FxHashMap<LayoutId, RegisteredMeasuredLayout>,
    current_measurements: FxHashMap<SolverNodeId, CurrentMeasurement>,
    artifacts: ArtifactStore,
}

/// Transaction checkpoint for current-frame measurement producers and mappings.
pub(super) struct MeasurementStoreCheckpoint {
    producers: ProducerRegistryCheckpoint,
    pending_measurements: FxHashMap<LayoutId, RegisteredMeasuredLayout>,
    current_measurements: FxHashMap<SolverNodeId, CurrentMeasurement>,
    artifacts: ArtifactStoreCheckpoint,
}

/// Measurement-owned artifact obligation table for one legal solver root solve.
///
/// This records which measured nodes in the solved root require a current
/// artifact before prepaint. Cache events may satisfy those obligations only
/// when they report the exact measured-node query; container subtree hits are
/// not artifact proof.
pub(super) struct MeasurementSolveObserver {
    root_artifact_descendants: Vec<SolverNodeId>,
}

impl MeasurementSolveObserver {
    pub(super) fn has_artifact_obligations(&self) -> bool {
        !self.root_artifact_descendants.is_empty()
    }

    fn root_artifact_descendants(&self) -> &[SolverNodeId] {
        &self.root_artifact_descendants
    }
}

impl MeasurementStore {
    pub(super) fn new() -> Self {
        Self {
            producers: ProducerRegistry::new(),
            pending_measurements: FxHashMap::default(),
            current_measurements: FxHashMap::default(),
            artifacts: ArtifactStore::new(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.pending_measurements.clear();
        self.current_measurements.clear();
        self.artifacts.begin_frame();
    }

    pub(super) fn finish_frame(&mut self) {
        self.artifacts.finish_frame();
        self.producers.clear();
        self.pending_measurements.clear();
        self.current_measurements.clear();
    }

    pub(super) fn checkpoint(&self) -> MeasurementStoreCheckpoint {
        MeasurementStoreCheckpoint {
            producers: self.producers.checkpoint(),
            pending_measurements: self.pending_measurements.clone(),
            current_measurements: self.current_measurements.clone(),
            artifacts: self.artifacts.checkpoint(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: MeasurementStoreCheckpoint) {
        self.producers.rollback_to_checkpoint(checkpoint.producers);
        self.pending_measurements = checkpoint.pending_measurements;
        self.current_measurements = checkpoint.current_measurements;
        self.artifacts.rollback_to_checkpoint(checkpoint.artifacts);
    }

    pub(super) fn register_request(
        &mut self,
        request: MeasuredLayoutRequest,
    ) -> RegisteredMeasuredLayout {
        let (facts, measurement) = match request.0 {
            MeasuredLayoutRequestRepr::Opaque(context) => {
                let producer = self.producers.push(context);
                (
                    MeasuredLayoutFacts::opaque(),
                    CurrentMeasurement::Opaque(producer),
                )
            }
            MeasuredLayoutRequestRepr::PureSize(measure) => (
                MeasuredLayoutFacts::pure_size(measure.clone()),
                CurrentMeasurement::PureSize(measure),
            ),
            MeasuredLayoutRequestRepr::Artifact {
                facts,
                key,
                context,
            } => {
                let producer = self.producers.push(context);
                (
                    facts.clone(),
                    CurrentMeasurement::Artifact {
                        facts,
                        key,
                        measure: producer,
                    },
                )
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
            "registered measured request facts should match the committed facts"
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
            Some(CurrentMeasurement::Artifact { .. })
        )
    }

    pub(super) fn solve_observer(
        &self,
        root: SolverNodeId,
        mut children: impl FnMut(SolverNodeId) -> Vec<SolverNodeId>,
        mut can_produce_artifacts: impl FnMut(SolverNodeId) -> bool,
    ) -> MeasurementSolveObserver {
        let root_artifact_descendants =
            self.collect_artifact_descendants(root, &mut children, &mut can_produce_artifacts);
        MeasurementSolveObserver {
            root_artifact_descendants,
        }
    }

    fn collect_artifact_descendants(
        &self,
        node_id: SolverNodeId,
        children: &mut impl FnMut(SolverNodeId) -> Vec<SolverNodeId>,
        can_produce_artifacts: &mut impl FnMut(SolverNodeId) -> bool,
    ) -> Vec<SolverNodeId> {
        if !can_produce_artifacts(node_id) {
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
            ));
        }

        descendants
    }

    #[cfg(any(test, debug_assertions))]
    pub(super) fn debug_assert_current_measurement_matches(
        &self,
        node_id: SolverNodeId,
        facts: &MeasuredLayoutFacts,
    ) {
        match (&facts.0, facts, self.current_measurement(node_id)) {
            (MeasuredLayoutFactsRepr::Opaque, _, Some(CurrentMeasurement::Opaque(_))) => {}
            (
                MeasuredLayoutFactsRepr::PureSize(expected),
                _,
                Some(CurrentMeasurement::PureSize(actual)),
            ) => assert_eq!(actual, expected),
            (_, expected, Some(CurrentMeasurement::Artifact { facts: actual, .. })) => {
                assert_eq!(actual, expected)
            }
            _ => {
                panic!("measured facts should have matching current measurement state")
            }
        }
    }

    /// Hydrate artifact nodes from the artifacts selected by the completed solve.
    ///
    /// The solver may invoke a callback or report a passive cache hit/store event.
    /// Those events are artifact producers, not GPUI paint-state side effects.
    /// Hydration happens once here after the solve. There is no retained-node
    /// artifact hydration path; if the solver skips an artifact node without
    /// reporting an exact measurement query, GPUI has no artifact proof for that
    /// node.
    pub(super) fn finish_completed_solve(
        &mut self,
        observer: &MeasurementSolveObserver,
        scale_factor: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        for proof in self.artifacts.take_pending_proofs() {
            let cache_key = proof.cache_key();
            if let Some(artifact) = self.artifacts.current_artifact_for_query(&cache_key) {
                proof.assert_matches_artifact(&artifact, scale_factor);
                continue;
            }
            self.hydrate_artifact_from_current_producer(proof, scale_factor, window, cx);
        }

        self.assert_solved_root_artifacts_selected(observer);

        for node_id in observer.root_artifact_descendants() {
            let artifact = self
                .artifacts
                .current_artifact(*node_id)
                .expect("solved root artifact should have been selected before hydration");
            self.hydrate_artifact_node(*node_id, &artifact);
        }
    }

    fn assert_solved_root_artifacts_selected(&self, observer: &MeasurementSolveObserver) {
        let missing = observer
            .root_artifact_descendants()
            .iter()
            .copied()
            .filter(|node_id| !self.artifacts.has_current_artifact(*node_id))
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
            "completed retained layout solve did not select artifacts for solved root nodes: {missing}"
        );
    }

    pub(super) fn compute_state(&mut self) -> ComputeMeasurementState<'_> {
        ComputeMeasurementState::new(
            &mut self.producers,
            &mut self.current_measurements,
            &mut self.artifacts,
        )
    }

    #[stacksafe::stacksafe]
    fn hydrate_artifact_from_current_producer(
        &mut self,
        proof: ArtifactProof,
        scale_factor: f32,
        window: &mut Window,
        cx: &App,
    ) {
        let Some(CurrentMeasurement::Artifact { key, measure, .. }) =
            self.current_measurements.get(&proof.node_id).cloned()
        else {
            panic!("solver cache-hit text query should refer to a current text measurement");
        };
        assert_eq!(
            &key, &proof.artifact_key,
            "solver cache-hit artifact query should match the current artifact key"
        );

        let artifact = {
            let mut measure_cx = MeasureCx::new(window, cx);
            let context = self
                .producers
                .context_mut(measure)
                .expect("artifact measured layout should have a current producer and hydrator");
            let LayoutMeasureContext::Artifact(context) = context else {
                panic!("artifact measured layout should have an artifact producer");
            };
            (context.measure)(
                proof.known_dimensions,
                proof.available_space,
                &mut measure_cx,
            )
        };
        key.assert_matches_artifact(
            &artifact,
            "hydrated solver cache-hit artifact should match the current artifact key",
        );
        proof.assert_matches_artifact(&artifact, scale_factor);

        let cache_key = proof.cache_key();
        self.artifacts
            .record_for_query(proof.node_id, cache_key.clone(), &artifact);
        self.artifacts.cache_artifact(cache_key, artifact);
    }

    fn hydrate_artifact_node(&mut self, node_id: SolverNodeId, artifact: &LayoutArtifact) {
        let Some(CurrentMeasurement::Artifact { key, measure, .. }) =
            self.current_measurements.get_mut(&node_id)
        else {
            panic!("selected artifact should hydrate a current text measurement");
        };
        key.assert_matches_artifact(
            artifact,
            "hydrated artifact should match the current artifact key",
        );
        let context = self
            .producers
            .context(*measure)
            .expect("artifact measured layout should have a hydrator");
        let LayoutMeasureContext::Artifact(context) = context else {
            panic!("artifact measured layout should have an artifact hydrator");
        };
        let hydrate = Rc::clone(&context.hydrate);
        hydrate(artifact);
    }
}

/// Per-compute bridge between solver measurement callbacks and GPUI artifacts.
///
/// The solver owns whether a measured node callback runs. GPUI hydrates artifacts
/// from a callback result in the same compute. When the same retained artifact
/// node is measured with the same exact callback query, GPUI can hydrate from its
/// query-keyed artifact cache without running the text producer again.
pub(super) struct ComputeMeasurementState<'a> {
    producers: &'a mut ProducerRegistry,
    current_measurements: &'a mut FxHashMap<SolverNodeId, CurrentMeasurement>,
    artifacts: &'a mut ArtifactStore,
}

impl<'a> ComputeMeasurementState<'a> {
    fn new(
        producers: &'a mut ProducerRegistry,
        current_measurements: &'a mut FxHashMap<SolverNodeId, CurrentMeasurement>,
        artifacts: &'a mut ArtifactStore,
    ) -> Self {
        Self {
            producers,
            current_measurements,
            artifacts,
        }
    }

    pub(super) fn has_current_measurement(&self, node_id: SolverNodeId) -> bool {
        self.current_measurements.contains_key(&node_id)
    }

    #[stacksafe::stacksafe]
    pub(super) fn measure(
        &mut self,
        node_id: SolverNodeId,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &App,
    ) -> MeasuredQueryAnswer {
        let Some(current_measurement) = self.current_measurements.get(&node_id).cloned() else {
            panic!("measured layout mirror node should have a current GPUI measurement producer");
        };
        let mut measure_cx = MeasureCx::new(window, cx);

        match current_measurement {
            CurrentMeasurement::Opaque(measure) => {
                let context = self
                    .producers
                    .context_mut(measure)
                    .expect("opaque measured layout should have a current producer");
                let LayoutMeasureContext::Size(measure) = context else {
                    panic!("opaque measured layout should have a size producer");
                };
                MeasuredQueryAnswer::hard_work(measure(
                    known_dimensions,
                    available_space,
                    &mut measure_cx,
                ))
            }
            CurrentMeasurement::PureSize(measure) => {
                MeasuredQueryAnswer::hard_work(measure.measure(known_dimensions, available_space))
            }
            CurrentMeasurement::Artifact { key, measure, .. } => {
                let cache_key =
                    ArtifactCacheKey::new(node_id, key.clone(), known_dimensions, available_space);
                if let Some(artifact) = self.artifacts.artifact_for_query(&cache_key) {
                    let size = artifact.size();
                    self.record_artifact_for_query(node_id, cache_key, &artifact);
                    return MeasuredQueryAnswer::no_work(size);
                }

                let measure_id = measure;
                let artifact = {
                    let context = self.producers.context_mut(measure_id).expect(
                        "artifact measured layout should have a current producer and hydrator",
                    );
                    let LayoutMeasureContext::Artifact(context) = context else {
                        panic!("artifact measured layout should have an artifact producer");
                    };
                    (context.measure)(known_dimensions, available_space, &mut measure_cx)
                };
                key.assert_matches_artifact(
                    &artifact,
                    "artifact measured layout artifact should match the current artifact key",
                );
                let size = artifact.size();
                self.record_artifact_for_query(node_id, cache_key.clone(), &artifact);
                self.artifacts.cache_artifact(cache_key, artifact);
                MeasuredQueryAnswer::hard_work(size)
            }
        }
    }

    pub(super) fn observe_layout_cache_event(
        &mut self,
        event: SolverCacheEvent,
        scale_factor: f32,
    ) {
        match event {
            SolverCacheEvent::Measure(observation) => {
                self.record_artifact_for_measure_observation(observation, scale_factor);
            }
            SolverCacheEvent::Hit(_) | SolverCacheEvent::Stored(_) | SolverCacheEvent::Miss(_) => {}
            SolverCacheEvent::Cleared(_) => {}
        }
    }

    fn record_artifact_for_measure_observation(
        &mut self,
        observation: SolverMeasureObservation,
        scale_factor: f32,
    ) {
        let node_id = observation.node_id();
        let Some(CurrentMeasurement::Artifact { key, .. }) =
            self.current_measurements.get(&node_id)
        else {
            return;
        };
        let key = key.clone();
        let proof =
            ArtifactProof::from_solver_measure_observation(key.clone(), observation, scale_factor);
        let query_key = proof.cache_key();
        let Some(artifact) = self.artifacts.artifact_for_query(&query_key) else {
            self.artifacts.push_pending_proof(proof);
            return;
        };
        key.assert_matches_artifact(
            &artifact,
            "solver measure observation artifact should match the current artifact key",
        );
        proof.assert_matches_artifact(&artifact, scale_factor);
        self.record_artifact_for_query(node_id, query_key, &artifact);
    }

    fn record_artifact_for_query(
        &mut self,
        node_id: SolverNodeId,
        cache_key: ArtifactCacheKey,
        artifact: &LayoutArtifact,
    ) {
        self.artifacts
            .record_for_query(node_id, cache_key, artifact);
    }
}
