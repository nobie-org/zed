//! Retained GPUI layout forest and private solver mirror.
//!
//! The forest is the authority for retained layout occurrences. A solver mirror
//! is kept behind a private facade for layout execution and caching; callers may
//! request layout, compute roots, and read bounds, but cannot see or mutate
//! solver nodes directly.

use super::{
    AvailableSpace, EXPECT_MESSAGE, LayoutId, RetainedLayoutRootId, RetainedLayoutRootSite,
};
mod bounds_cache;
mod committed;
mod facts;
mod frame;
mod geometry;
mod measurement;
mod occurrence;
mod root_slots;
mod roots;
mod solver;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod state_tests;
mod subtree_probe;
mod trace;
mod work;
use crate::{
    App, Bounds, GlobalElementId, Pixels, Size, Style, Window, size,
    util::{ceil_to_device_pixel, round_half_toward_zero},
};
use bounds_cache::{BoundsCache, BoundsCacheCheckpoint};
use collections::{FxHashMap, FxHashSet};
use committed::{CommittedLayoutCheckpoint, CommittedLayoutState};
use facts::{LayoutArtifactPolicy, LayoutIntent, LayoutIntentKind};
use frame::{FrameIntents, FrameIntentsCheckpoint};
use geometry::{GeometryStore, GeometryStoreCheckpoint};
pub(super) use measurement::MeasuredLayoutRequest;
pub(crate) use measurement::PureSizeMeasure;
use measurement::{
    MeasuredLayoutFacts, MeasurementSolveObserver, MeasurementStore, MeasurementStoreCheckpoint,
};
use occurrence::{RetainedLayoutOccurrence, RetainedLayoutOccurrenceKind};
use root_slots::{RootSlots, RootSlotsCheckpoint};
use roots::{RootRegistry, RootRegistryCheckpoint};
use solver::{
    FreshLayoutSolver, FreshSolverNodeId, LayoutSolver, SolverLayout, SolverMeasureQuery,
    SolverNodeId, SolverStyle,
};
use std::{
    collections::hash_map::DefaultHasher,
    fmt::Debug,
    hash::{Hash, Hasher},
    time::Duration,
};
use subtree_probe::{SubtreeProbe, SubtreeProbeCheckpoint, SubtreeProbeComputeRecorder};
use trace::CacheEventTracer;
#[cfg(test)]
pub(super) use work::RetainedForestMutationSample;
pub(super) use work::{RetainedLayoutMissWork, RetainedLayoutWork};
use work::{RetainedWorkCheckpoint, RetainedWorkState};

/// Work observed while committing and computing one root layout.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ComputeLayoutWork {
    pub(super) solver_compute_layout_calls: u64,
    pub(super) measured_layout_calls: u64,
    pub(super) compute_layout_duration: Duration,
    pub(super) measured_layout_duration: Duration,
    pub(super) fresh_layout_comparison: Option<FreshLayoutComparisonSummary>,
}

#[derive(Clone)]
struct FreshLayoutCompareNodeContext {
    layout_id: LayoutId,
}

#[derive(Default)]
struct FreshLayoutComparison {
    summary: FreshLayoutComparisonSummary,
    mismatch: Option<String>,
    equal_zero: Option<String>,
    target_lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct FreshLayoutComparisonSummary {
    pub(super) checked_nodes: u64,
    pub(super) mismatches: u64,
    pub(super) equal_zero_nodes: u64,
    pub(super) target_nodes: u64,
    pub(super) target_mismatches: u64,
}

/// Opaque test handle for a retained occurrence.
///
/// Tests can compare or inspect retained forest behavior without depending on
/// the concrete solver node type or making it part of GPUI's public layout model.
#[cfg(test)]
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(super) struct RetainedNodeToken(SolverNodeId);

#[cfg(test)]
impl Debug for RetainedNodeToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RetainedNodeToken(..)")
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct RetainedStyleForTests(SolverStyle);

#[cfg(test)]
impl Debug for RetainedStyleForTests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RetainedStyleForTests(..)")
    }
}

#[cfg(test)]
impl PartialEq for RetainedStyleForTests {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RetainedLayoutShapeForTests {
    style: RetainedStyleForTests,
    has_measure_context: bool,
    children: Vec<RetainedLayoutShapeForTests>,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RetainedLayoutProjectionForTests {
    location: crate::Point<f32>,
    size: Size<f32>,
    children: Vec<RetainedLayoutProjectionForTests>,
}

/// Owner of retained GPUI layout occurrences and their private solver mirror.
///
/// All mirror mutations are encapsulated here. Methods such as
/// `request_layout`, `compute_layout`, and `finish_frame` move retained facts,
/// measurement state, bounds caches, and solver state together so the forest
/// stays observationally equivalent to a freshly built layout tree.
pub(super) struct RetainedLayoutForest {
    solver: LayoutSolver,
    roots: RootRegistry,
    frame: FrameIntents,
    measurements: MeasurementStore,
    geometry: GeometryStore,
    committed: CommittedLayoutState,
    bounds: BoundsCache,
    root_slots: RootSlots,
    subtree_probe: SubtreeProbe,
    work: RetainedWorkState,
}

/// Full rollback checkpoint for retryable layout transactions.
///
/// This intentionally checkpoints both retained GPUI authority and the mirror. A
/// speculative prepaint that fails must leave no retained layout side effects.
pub(super) struct RetainedLayoutForestCheckpoint {
    solver: LayoutSolver,
    roots: RootRegistryCheckpoint,
    frame: FrameIntentsCheckpoint,
    measurements: MeasurementStoreCheckpoint,
    geometry: GeometryStoreCheckpoint,
    committed: CommittedLayoutCheckpoint,
    bounds: BoundsCacheCheckpoint,
    root_slots: RootSlotsCheckpoint,
    subtree_probe: SubtreeProbeCheckpoint,
    work: RetainedWorkCheckpoint,
}

fn snap_measured_size_to_device_pixels(size: Size<Pixels>, scale_factor: f32) -> Size<f32> {
    size.map(|d| ceil_to_device_pixel(d.0.max(0.0), scale_factor))
}

fn retained_layout_fresh_compare_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("GPUI_TRACE_RETAINED_LAYOUT_FRESH_COMPARE").is_some())
}

impl RetainedLayoutForest {
    /// Create an empty retained forest and configure the mirror for GPUI snapping.
    pub(super) fn new() -> Self {
        Self {
            solver: LayoutSolver::new(),
            roots: RootRegistry::new(),
            frame: FrameIntents::new(),
            measurements: MeasurementStore::new(),
            geometry: GeometryStore::new(),
            committed: CommittedLayoutState::new(),
            bounds: BoundsCache::new(),
            root_slots: RootSlots::new(),
            subtree_probe: SubtreeProbe::new(),
            work: RetainedWorkState::new(),
        }
    }

    /// Reset frame-local inputs while keeping retained roots available.
    pub(super) fn begin_frame(&mut self) {
        self.measurements.begin_frame();
        self.geometry.begin_frame();
        self.subtree_probe.begin_frame();
        self.work.begin_frame();
    }

    /// Snapshot every retained and mirror field affected by speculative layout.
    pub(super) fn checkpoint(&self) -> RetainedLayoutForestCheckpoint {
        RetainedLayoutForestCheckpoint {
            solver: self.solver.clone(),
            roots: self.roots.checkpoint(),
            frame: self.frame.checkpoint(),
            measurements: self.measurements.checkpoint(),
            geometry: self.geometry.checkpoint(),
            committed: self.committed.checkpoint(),
            bounds: self.bounds.checkpoint(),
            root_slots: self.root_slots.checkpoint(),
            subtree_probe: self.subtree_probe.checkpoint(),
            work: self.work.checkpoint(),
        }
    }

    /// Restore the forest to a prior speculative-layout checkpoint.
    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: RetainedLayoutForestCheckpoint) {
        self.solver = checkpoint.solver;
        self.roots.rollback_to_checkpoint(checkpoint.roots);
        self.frame.rollback_to_checkpoint(checkpoint.frame);
        self.measurements
            .rollback_to_checkpoint(checkpoint.measurements);
        self.geometry.rollback_to_checkpoint(checkpoint.geometry);
        self.committed.rollback_to_checkpoint(checkpoint.committed);
        self.bounds.rollback_to_checkpoint(checkpoint.bounds);
        self.root_slots
            .rollback_to_checkpoint(checkpoint.root_slots);
        self.subtree_probe
            .rollback_to_checkpoint(checkpoint.subtree_probe);
        self.work.rollback_to_checkpoint(checkpoint.work);
    }

    /// Return a retained identity for a root compute site.
    ///
    /// This is the only root identity allocation path. It prevents callers from
    /// using root order as an implicit retention key.
    pub(super) fn retained_root_id(
        &mut self,
        root_site: RetainedLayoutRootSite,
        global_id: Option<&GlobalElementId>,
    ) -> RetainedLayoutRootId {
        self.roots.retained_root_id(root_site, global_id)
    }

    /// Promote successfully computed current roots and sweep everything else.
    ///
    /// After this call, current-frame intents, committed mappings, measurement
    /// producers, and bounds caches are gone. Only retained root occurrences
    /// survive to the next frame.
    pub(super) fn finish_frame(&mut self) -> (RetainedLayoutWork, RetainedLayoutMissWork) {
        self.flush_detached_subtree_removals();

        for retained_root in self.root_slots.take_retained_roots() {
            self.remove_retained_subtree(retained_root);
        }

        self.root_slots.promote_current_roots();
        self.geometry.finish_frame();
        self.frame.clear();
        self.measurements.finish_frame();
        self.committed.clear();
        self.bounds.clear();
        self.subtree_probe.finish_frame();
        let misses = self.work.miss_work();
        if trace::enabled() {
            if misses != RetainedLayoutMissWork::default() {
                eprintln!(
                    "gpui retained_layout match_miss_summary no_previous={} style={} kind={} measured_facts={} child_count={} child_subtree={} no_exact_child={}",
                    misses.no_previous,
                    misses.style,
                    misses.kind,
                    misses.measured_facts,
                    misses.child_count,
                    misses.child_subtree,
                    misses.no_exact_child,
                );
            }
        }
        self.work.finish_frame()
    }

    /// Store an unmeasured current-frame intent.
    pub(super) fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        children: &[LayoutId],
    ) -> LayoutId {
        self.push_intent(LayoutIntent {
            global_id: global_id.cloned(),
            artifact_policy: LayoutArtifactPolicy::from_style(&style),
            style: SolverStyle::from_gpui_style(&style, rem_size, scale_factor),
            kind: LayoutIntentKind::Unmeasured {
                children: children.to_vec(),
            },
        })
    }

    /// Store a measured current-frame intent.
    ///
    /// The request is produced by the measurement owner. The forest records only
    /// the generic measured shape, comparable measured facts, and optional
    /// current-frame producer slot.
    pub(super) fn request_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        request: MeasuredLayoutRequest,
    ) -> LayoutId {
        let measured = self.measurements.register_request(request);
        let measured_facts = measured.facts().clone();

        let id = self.push_intent(LayoutIntent {
            global_id: None,
            artifact_policy: LayoutArtifactPolicy::from_style(&style),
            style: SolverStyle::from_gpui_style(&style, rem_size, scale_factor),
            kind: LayoutIntentKind::Measured(measured_facts),
        });
        self.measurements.bind_registered_request(id, measured);
        id
    }

    /// Allocate the next current-frame `LayoutId`.
    fn push_intent(&mut self, intent: LayoutIntent) -> LayoutId {
        self.frame.push_intent(intent)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn set_retained_subtree_probe_targets_for_tests(&mut self, targets: Vec<String>) {
        self.subtree_probe.set_targets_for_tests(targets);
    }

    #[cfg(test)]
    pub(super) fn retained_subtree_work_samples_for_tests(
        &self,
    ) -> &[super::telemetry::RetainedSubtreeWorkSample] {
        self.subtree_probe.samples_for_tests()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn last_retained_subtree_work_samples_for_tests(
        &self,
    ) -> &[super::telemetry::RetainedSubtreeWorkSample] {
        self.subtree_probe.last_finished_samples_for_tests()
    }

    fn committed_node(&self, id: LayoutId) -> SolverNodeId {
        self.committed.node(id)
    }

    fn committed_layout_id_for_node(&self, node_id: SolverNodeId) -> Option<LayoutId> {
        self.committed.layout_id_for_node(node_id)
    }

    fn children(&self, node_id: SolverNodeId) -> Vec<SolverNodeId> {
        self.solver.children(node_id)
    }

    fn parent(&self, node_id: SolverNodeId) -> Option<SolverNodeId> {
        self.solver.parent(node_id)
    }

    fn geometry_layout(&self, node_id: SolverNodeId) -> SolverLayout {
        self.geometry.layout(node_id).unwrap_or_else(|| {
            panic!(
                "retained layout geometry should be captured before reading node {:?}",
                node_id
            )
        })
    }

    fn try_geometry_layout(&self, node_id: SolverNodeId) -> Option<SolverLayout> {
        self.geometry.layout(node_id)
    }

    fn try_style(&self, node_id: SolverNodeId) -> Option<SolverStyle> {
        self.solver.style(node_id)
    }

    /// Commit a root intent, compute the mirror, and finish measurement effects.
    pub(super) fn compute_layout(
        &mut self,
        root_id: RetainedLayoutRootId,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> ComputeLayoutWork {
        let node_id = self.commit_root_layout(root_id, id);
        self.geometry
            .begin_solve(root_id, node_id, available_space, scale_factor);

        if trace::detail_enabled() && trace::layout_id_is_targeted(Some(id.0)) {
            eprintln!(
                "gpui retained_layout compute_start layout_id={} node_id={:?} available_space={:?}",
                id.0, node_id, available_space
            );
        }

        let measurement_solve_observer = {
            let Self {
                solver,
                measurements,
                committed,
                frame,
                ..
            } = self;
            measurements.solve_observer(
                node_id,
                |node_id| solver.children(node_id),
                |node_id| {
                    committed
                        .layout_id_for_node(node_id)
                        .map(|layout_id| {
                            frame
                                .intent(layout_id)
                                .artifact_policy
                                .can_produce_artifacts()
                        })
                        .unwrap_or_else(|| {
                            panic!("solver node in legal root should have a committed layout id")
                        })
                },
            )
        };
        let mut cache_event_tracer = CacheEventTracer::new(if trace::detail_enabled() {
            self.committed.node_layout_ids_for_trace()
        } else {
            Vec::new()
        });
        let compute_start = std::time::Instant::now();
        let mut subtree_compute_recorder = self.subtree_probe.compute_recorder();
        let (measured_layout_calls, measured_layout_duration) = self.compute_layout_with_measure(
            node_id,
            available_space,
            scale_factor,
            window,
            cx,
            &mut subtree_compute_recorder,
            &measurement_solve_observer,
            &mut cache_event_tracer,
        );
        let compute_layout_duration = compute_start.elapsed();
        self.subtree_probe.record_compute(subtree_compute_recorder);

        {
            let Self {
                solver, geometry, ..
            } = self;
            geometry.capture_from_solver(
                root_id,
                node_id,
                available_space,
                scale_factor,
                solver.capture_layout_tree(node_id),
            );
        }
        self.measurements
            .finish_completed_solve(&measurement_solve_observer, window, cx);

        if trace::detail_enabled() && trace::layout_id_is_targeted(Some(id.0)) {
            let layout = self.geometry_layout(node_id);
            eprintln!(
                "gpui retained_layout compute_finish layout_id={} node_id={:?} root_layout={:?}",
                id.0, node_id, layout
            );
        }

        let work = ComputeLayoutWork {
            solver_compute_layout_calls: 1,
            measured_layout_calls,
            compute_layout_duration,
            measured_layout_duration,
            fresh_layout_comparison: if retained_layout_fresh_compare_enabled() {
                let target_layout_ids = trace::target_layout_ids();
                Some(self.trace_retained_fresh_layout_comparison(
                    id,
                    node_id,
                    available_space,
                    scale_factor,
                    window,
                    cx,
                    target_layout_ids,
                ))
            } else {
                None
            },
        };
        work
    }

    /// Read absolute, snapped bounds for a committed layout intent.
    pub(super) fn layout_bounds(&mut self, id: LayoutId, scale_factor: f32) -> Bounds<Pixels> {
        let node_id = self.committed_node(id);
        let bounds = self.layout_bounds_for_node(node_id, scale_factor);
        let has_zero_size = bounds.size.width.0 <= 0.0 || bounds.size.height.0 <= 0.0;
        let trace_targeted_zero_bounds =
            trace::detail_enabled() && trace::layout_id_is_targeted(Some(id.0));
        if has_zero_size && (trace_targeted_zero_bounds || trace::should_trace_zero_bounds()) {
            let layout = self.try_geometry_layout(node_id);
            let parent = self.parent(node_id);
            let parent_layout = parent.and_then(|parent| self.try_geometry_layout(parent));
            let children = self.children(node_id);
            let style = self.try_style(node_id);
            eprintln!(
                "gpui retained_layout zero_bounds layout_id={} node_id={:?} bounds={:?} geometry_layout={:?} parent={:?} parent_geometry_layout={:?} children={:?} style={:?}",
                id.0, node_id, bounds, layout, parent, parent_layout, children, style
            );
            let mut depth = 0;
            let mut ancestor = Some(node_id);
            while let Some(ancestor_node_id) = ancestor {
                let ancestor_layout_id = self
                    .committed_layout_id_for_node(ancestor_node_id)
                    .map(|layout_id| layout_id.0);
                let ancestor_layout = self.try_geometry_layout(ancestor_node_id);
                let ancestor_parent = self.parent(ancestor_node_id);
                let ancestor_children = self.children(ancestor_node_id);
                let ancestor_style = self.try_style(ancestor_node_id);
                eprintln!(
                    "gpui retained_layout zero_bounds_ancestor requested_layout_id={} depth={} layout_id={:?} node_id={:?} parent={:?} geometry_layout={:?} children={:?} style={:?}",
                    id.0,
                    depth,
                    ancestor_layout_id,
                    ancestor_node_id,
                    ancestor_parent,
                    ancestor_layout,
                    ancestor_children,
                    ancestor_style
                );
                depth += 1;
                if depth >= 12 {
                    break;
                }
                ancestor = ancestor_parent;
            }
        }
        bounds
    }

    fn layout_bounds_for_node(
        &mut self,
        node_id: SolverNodeId,
        scale_factor: f32,
    ) -> Bounds<Pixels> {
        let Self {
            solver,
            geometry,
            bounds,
            ..
        } = self;
        bounds.layout_bounds_for_node(
            node_id,
            scale_factor,
            |node_id| {
                geometry.layout(node_id).unwrap_or_else(|| {
                    panic!(
                        "retained layout geometry should be captured before reading bounds for {:?}",
                        node_id
                    )
                })
            },
            |node_id| solver.parent(node_id),
        )
    }

    fn compute_layout_with_measure(
        &mut self,
        node_id: SolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        window: &mut Window,
        cx: &mut App,
        subtree_compute_recorder: &mut SubtreeProbeComputeRecorder,
        measurement_solve_observer: &MeasurementSolveObserver,
        cache_event_tracer: &mut CacheEventTracer,
    ) -> (u64, std::time::Duration) {
        let mut measured_layout_calls = 0;
        let mut measured_layout_duration = std::time::Duration::default();

        let Self {
            solver,
            measurements,
            ..
        } = self;

        let compute_measurements = std::cell::RefCell::new(measurements.compute_state());
        let subtree_compute_recorder = std::cell::RefCell::new(subtree_compute_recorder);

        solver.compute_layout_with_measure_and_cache_events(
            node_id,
            available_space,
            scale_factor,
            |node_id, has_measure_context, query: SolverMeasureQuery| {
                if !has_measure_context {
                    assert!(
                        !compute_measurements
                            .borrow()
                            .has_current_measurement(node_id),
                        "measured solver node should have a stable measurement marker"
                    );
                    return size(0.0_f32, 0.0_f32).into();
                }

                measured_layout_calls += 1;
                let callback_telemetry = compute_measurements
                    .borrow()
                    .callback_telemetry(node_id)
                    .expect("measured solver node should have a current measurement");
                subtree_compute_recorder
                    .borrow_mut()
                    .record_measured_callback(node_id, callback_telemetry);
                let measure_start = std::time::Instant::now();
                let measured_size = compute_measurements.borrow_mut().measure(
                    node_id,
                    query.known_dimensions,
                    query.available_space,
                    window,
                    cx,
                );
                measured_layout_duration += measure_start.elapsed();
                snap_measured_size_to_device_pixels(measured_size, scale_factor)
            },
            |event| {
                cache_event_tracer.record(event);
                compute_measurements
                    .borrow_mut()
                    .observe_layout_cache_event(event, scale_factor, measurement_solve_observer);
            },
        );

        (measured_layout_calls, measured_layout_duration)
    }

    fn trace_retained_layout_miss(
        &mut self,
        reason: &'static str,
        id: LayoutId,
        previous: Option<&RetainedLayoutOccurrence>,
        detail: impl FnOnce() -> String,
    ) {
        let Some(limit) = trace::miss_trace_limit() else {
            return;
        };
        if !self.work.should_trace_miss(limit) {
            return;
        }
        let miss_trace_sample = self.work.take_miss_trace_sample_index();

        let intent = self.intent(id);
        let current = Self::layout_intent_summary(intent);
        let previous = previous
            .map(Self::retained_node_summary)
            .unwrap_or_else(|| "none".to_string());
        let detail = detail();
        eprintln!(
            "gpui retained_layout match_miss_sample sample={} layout_id={} reason={} current={} previous={} detail={}",
            miss_trace_sample, id.0, reason, current, previous, detail,
        );
    }

    fn trace_retained_style_update(
        &self,
        id: LayoutId,
        node_id: SolverNodeId,
        previous_style: &SolverStyle,
        current_style: &SolverStyle,
    ) {
        if !trace::should_trace_mutation() {
            return;
        }
        eprintln!(
            "gpui retained_layout mutation kind=style_update layout_id={} node_id={:?} intent={} previous_style={} current_style={}",
            id.0,
            node_id,
            Self::layout_intent_summary(self.intent(id)),
            Self::debug_fingerprint(previous_style),
            Self::debug_fingerprint(current_style)
        );
    }

    fn trace_retained_dirty_mark(&self, id: LayoutId, node_id: SolverNodeId, reason: &str) {
        if !trace::should_trace_mutation() {
            return;
        }
        eprintln!(
            "gpui retained_layout mutation kind=dirty_mark layout_id={} node_id={:?} reason={} intent={}",
            id.0,
            node_id,
            reason,
            Self::layout_intent_summary(self.intent(id))
        );
    }

    fn layout_intent_summary(intent: &LayoutIntent) -> String {
        format!(
            "{{global_id={:?}, kind={}, style={}}}",
            intent.global_id,
            Self::layout_intent_kind_summary(&intent.kind),
            Self::debug_fingerprint(&intent.style)
        )
    }

    fn layout_intent_kind_summary(kind: &LayoutIntentKind) -> String {
        match kind {
            LayoutIntentKind::Unmeasured { children } => {
                format!("unmeasured children={}", children.len())
            }
            LayoutIntentKind::Measured(measured) => {
                format!("measured facts={}", Self::debug_fingerprint(measured))
            }
        }
    }

    fn retained_node_summary(node: &RetainedLayoutOccurrence) -> String {
        format!(
            "{{node_id={:?}, kind={}, measured_facts={}, children={}, style={}}}",
            node.node_id,
            node.kind_name(),
            node.measured_facts()
                .map(Self::debug_fingerprint)
                .unwrap_or_else(|| "none".to_string()),
            node.children().len(),
            Self::debug_fingerprint(&node.style)
        )
    }

    fn debug_fingerprint(value: &impl Debug) -> String {
        let debug = format!("{value:?}");
        let mut hasher = DefaultHasher::new();
        debug.hash(&mut hasher);
        let preview = debug
            .chars()
            .take(180)
            .collect::<String>()
            .replace('\n', "\\n");
        let truncated = if debug.chars().count() > 180 {
            "..."
        } else {
            ""
        };
        format!(
            "hash={:016x} len={} preview={:?}{}",
            hasher.finish(),
            debug.len(),
            preview,
            truncated
        )
    }

    #[cfg(test)]
    pub(super) fn reset_retained_mutation_sample_for_tests(&mut self) {
        self.work.reset_mutation_sample_for_tests();
    }

    #[cfg(test)]
    pub(super) fn retained_mutation_sample_for_tests(&self) -> RetainedForestMutationSample {
        self.work.mutation_sample_for_tests()
    }

    #[cfg(test)]
    pub(super) fn retained_node_token_for_tests(&self, id: LayoutId) -> RetainedNodeToken {
        RetainedNodeToken(self.committed_node(id))
    }

    #[cfg(test)]
    pub(super) fn retained_layout_shape_for_tests(
        &self,
        token: RetainedNodeToken,
    ) -> RetainedLayoutShapeForTests {
        self.retained_layout_shape_for_node(token.0)
    }

    #[cfg(test)]
    fn retained_layout_shape_for_node(&self, node_id: SolverNodeId) -> RetainedLayoutShapeForTests {
        RetainedLayoutShapeForTests {
            style: RetainedStyleForTests(self.solver.style(node_id).expect(EXPECT_MESSAGE).clone()),
            has_measure_context: self.solver.has_measure_context(node_id),
            children: self
                .children(node_id)
                .into_iter()
                .map(|child| self.retained_layout_shape_for_node(child))
                .collect(),
        }
    }

    #[cfg(test)]
    pub(super) fn retained_layout_projection_for_tests(
        &self,
        token: RetainedNodeToken,
    ) -> RetainedLayoutProjectionForTests {
        self.retained_layout_projection_for_node(token.0)
    }

    #[cfg(test)]
    fn retained_layout_projection_for_node(
        &self,
        node_id: SolverNodeId,
    ) -> RetainedLayoutProjectionForTests {
        let layout = self.geometry_layout(node_id);
        RetainedLayoutProjectionForTests {
            location: layout.location.into(),
            size: layout.size.into(),
            children: self
                .children(node_id)
                .into_iter()
                .map(|child| self.retained_layout_projection_for_node(child))
                .collect(),
        }
    }

    #[cfg(test)]
    pub(super) fn retained_node_size_for_tests(&self, token: RetainedNodeToken) -> Size<f32> {
        self.geometry_layout(token.0).size.into()
    }

    #[cfg(test)]
    pub(super) fn retained_node_layout_bounds_for_tests(
        &mut self,
        token: RetainedNodeToken,
        scale_factor: f32,
    ) -> Bounds<Pixels> {
        self.layout_bounds_for_node(token.0, scale_factor)
    }

    #[cfg(test)]
    pub(super) fn retained_child_tokens_for_tests(
        &self,
        token: RetainedNodeToken,
    ) -> Vec<RetainedNodeToken> {
        self.children(token.0)
            .into_iter()
            .map(RetainedNodeToken)
            .collect()
    }

    #[cfg(test)]
    pub(super) fn retained_parent_token_for_tests(
        &self,
        token: RetainedNodeToken,
    ) -> Option<RetainedNodeToken> {
        self.parent(token.0).map(RetainedNodeToken)
    }

    #[cfg(test)]
    pub(super) fn retained_node_has_measure_context_for_tests(
        &self,
        token: RetainedNodeToken,
    ) -> bool {
        self.solver.has_measure_context(token.0)
    }

    #[cfg(test)]
    pub(super) fn compute_unmeasured_layout_for_tests(
        &mut self,
        root_id: RetainedLayoutRootId,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
    ) -> RetainedNodeToken {
        let root_node = self.commit_root_layout(root_id, id);
        self.geometry
            .begin_solve(root_id, root_node, available_space, scale_factor);
        self.solver.compute_layout_with_measure_and_cache_events(
            root_node,
            available_space,
            scale_factor,
            |_node_id, _has_measure_context, _query| size(0.0_f32, 0.0_f32),
            |_| {},
        );
        {
            let Self {
                solver, geometry, ..
            } = self;
            geometry.capture_from_solver(
                root_id,
                root_node,
                available_space,
                scale_factor,
                solver.capture_layout_tree(root_node),
            );
        }
        RetainedNodeToken(root_node)
    }

    #[cfg(test)]
    pub(super) fn assert_intent_committed_exactly_for_tests(&self, id: LayoutId) {
        let node_id = self.committed_node(id);
        let mut seen = FxHashSet::default();
        self.debug_assert_intent_node_matches(id, node_id, None, &mut seen);
    }

    #[cfg(test)]
    #[stacksafe::stacksafe]
    pub(super) fn retained_fresh_layout_comparison_for_tests(
        &mut self,
        root_id: RetainedLayoutRootId,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> FreshLayoutComparisonSummary {
        self.compute_layout(root_id, id, available_space, scale_factor, window, cx);
        let root_node_id = self.committed_node(id);
        self.trace_retained_fresh_layout_comparison(
            id,
            root_node_id,
            available_space,
            scale_factor,
            window,
            cx,
            None,
        )
    }

    fn trace_retained_fresh_layout_comparison(
        &self,
        root_layout_id: LayoutId,
        retained_root_node_id: SolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        window: &mut Window,
        cx: &mut App,
        target_layout_ids: Option<&[usize]>,
    ) -> FreshLayoutComparisonSummary {
        if !self.intent_subtree_supports_fresh_compare(root_layout_id) {
            eprintln!(
                "gpui retained_layout fresh_compare_skipped root_layout_id={} retained_root_node_id={:?} available_space={:?} reason=uncomparable_measured_node",
                root_layout_id.0, retained_root_node_id, available_space
            );
            return FreshLayoutComparisonSummary::default();
        }

        let mut fresh_solver = FreshLayoutSolver::<FreshLayoutCompareNodeContext>::new();
        let fresh_root_node_id =
            self.build_fresh_layout_compare_tree(&mut fresh_solver, root_layout_id);

        fresh_solver.compute_layout_with_measure(
            fresh_root_node_id,
            available_space,
            scale_factor,
            |_fresh_node_id, node_context, query| {
                let Some(node_context) = node_context else {
                    return size(0.0_f32, 0.0_f32);
                };
                let LayoutIntentKind::Measured(measured) =
                    &self.intent(node_context.layout_id).kind
                else {
                    return size(0.0_f32, 0.0_f32);
                };

                let measured_size = measured.measure_for_fresh_compare(
                    query.known_dimensions,
                    query.available_space,
                    window,
                    cx,
                );
                snap_measured_size_to_device_pixels(measured_size, scale_factor)
            },
        );

        let mut comparison = FreshLayoutComparison::default();
        self.observe_retained_fresh_layout_comparison(
            &fresh_solver,
            root_layout_id,
            fresh_root_node_id,
            &mut vec![root_layout_id],
            &mut comparison,
            target_layout_ids,
        );

        eprintln!(
            "gpui retained_layout fresh_compare_summary root_layout_id={} retained_root_node_id={:?} fresh_root_node_id={:?} available_space={:?} checked_nodes={} mismatches={} equal_zero_nodes={} target_nodes={} target_mismatches={}",
            root_layout_id.0,
            retained_root_node_id,
            fresh_root_node_id,
            available_space,
            comparison.summary.checked_nodes,
            comparison.summary.mismatches,
            comparison.summary.equal_zero_nodes,
            comparison.summary.target_nodes,
            comparison.summary.target_mismatches,
        );

        for target_line in comparison.target_lines {
            eprintln!(
                "gpui retained_layout fresh_compare_target root_layout_id={} retained_root_node_id={:?} fresh_root_node_id={:?} available_space={:?} {}",
                root_layout_id.0,
                retained_root_node_id,
                fresh_root_node_id,
                available_space,
                target_line
            );
        }

        if let Some(mismatch) = comparison.mismatch {
            eprintln!(
                "gpui retained_layout fresh_compare_mismatch root_layout_id={} retained_root_node_id={:?} fresh_root_node_id={:?} available_space={:?} {}",
                root_layout_id.0,
                retained_root_node_id,
                fresh_root_node_id,
                available_space,
                mismatch
            );
        } else if let Some(equal_zero) = comparison.equal_zero {
            eprintln!(
                "gpui retained_layout fresh_compare_equal_zero root_layout_id={} retained_root_node_id={:?} fresh_root_node_id={:?} available_space={:?} {}",
                root_layout_id.0,
                retained_root_node_id,
                fresh_root_node_id,
                available_space,
                equal_zero
            );
        }

        comparison.summary
    }

    fn build_fresh_layout_compare_tree(
        &self,
        fresh_solver: &mut FreshLayoutSolver<FreshLayoutCompareNodeContext>,
        id: LayoutId,
    ) -> FreshSolverNodeId {
        let intent = self.intent(id);
        match &intent.kind {
            LayoutIntentKind::Unmeasured { children } => {
                let child_node_ids = children
                    .iter()
                    .map(|child| self.build_fresh_layout_compare_tree(fresh_solver, *child))
                    .collect::<Vec<_>>();
                if child_node_ids.is_empty() {
                    fresh_solver.new_leaf(intent.style.clone())
                } else {
                    fresh_solver.new_with_children(intent.style.clone(), &child_node_ids)
                }
            }
            LayoutIntentKind::Measured(_) => fresh_solver.new_measured(
                intent.style.clone(),
                FreshLayoutCompareNodeContext { layout_id: id },
            ),
        }
    }

    fn intent_subtree_supports_fresh_compare(&self, id: LayoutId) -> bool {
        match &self.intent(id).kind {
            LayoutIntentKind::Unmeasured { children } => children
                .iter()
                .all(|child| self.intent_subtree_supports_fresh_compare(*child)),
            LayoutIntentKind::Measured(measured) => measured.supports_fresh_compare(),
        }
    }

    fn observe_retained_fresh_layout_comparison(
        &self,
        fresh_solver: &FreshLayoutSolver<FreshLayoutCompareNodeContext>,
        id: LayoutId,
        fresh_node_id: FreshSolverNodeId,
        path: &mut Vec<LayoutId>,
        comparison: &mut FreshLayoutComparison,
        target_layout_ids: Option<&[usize]>,
    ) {
        let retained_node_id = self.committed_node(id);
        let retained_layout = self.geometry_layout(retained_node_id);
        let fresh_layout = fresh_solver.layout(fresh_node_id);
        let layouts_match = retained_layout.location == fresh_layout.location
            && retained_layout.size == fresh_layout.size;
        let target_requested = target_layout_ids
            .map(|target_layout_ids| target_layout_ids.contains(&id.0))
            .unwrap_or(false);

        comparison.summary.checked_nodes += 1;
        if !layouts_match {
            comparison.summary.mismatches += 1;
        }

        if target_requested {
            comparison.summary.target_nodes += 1;
            if !layouts_match {
                comparison.summary.target_mismatches += 1;
            }
            let label = if layouts_match {
                "target_match"
            } else {
                "target_mismatch"
            };
            comparison
                .target_lines
                .push(self.fresh_layout_comparison_line(
                    label,
                    id,
                    retained_node_id,
                    fresh_node_id,
                    &retained_layout,
                    &fresh_layout,
                    path,
                ));
        }

        if !layouts_match && comparison.mismatch.is_none() {
            comparison.mismatch = Some(self.fresh_layout_comparison_line(
                "mismatch",
                id,
                retained_node_id,
                fresh_node_id,
                &retained_layout,
                &fresh_layout,
                path,
            ));
        }

        if layouts_match && Self::layout_has_zero_size(&retained_layout) {
            comparison.summary.equal_zero_nodes += 1;
            if comparison.equal_zero.is_none() {
                comparison.equal_zero = Some(self.fresh_layout_comparison_line(
                    "equal_zero",
                    id,
                    retained_node_id,
                    fresh_node_id,
                    &retained_layout,
                    &fresh_layout,
                    path,
                ));
            }
        }

        if let LayoutIntentKind::Unmeasured { children } = &self.intent(id).kind {
            let fresh_child_node_ids = fresh_solver.children(fresh_node_id);
            assert_eq!(
                fresh_child_node_ids.len(),
                children.len(),
                "fresh compare tree should mirror intent child count"
            );
            for (child, fresh_child_node_id) in children.iter().zip(fresh_child_node_ids) {
                path.push(*child);
                self.observe_retained_fresh_layout_comparison(
                    fresh_solver,
                    *child,
                    fresh_child_node_id,
                    path,
                    comparison,
                    target_layout_ids,
                );
                path.pop();
            }
        }
    }

    fn fresh_layout_comparison_line(
        &self,
        label: &'static str,
        id: LayoutId,
        retained_node_id: SolverNodeId,
        fresh_node_id: FreshSolverNodeId,
        retained_layout: &SolverLayout,
        fresh_layout: &SolverLayout,
        path: &[LayoutId],
    ) -> String {
        format!(
            "kind={} layout_id={} path={} retained_node_id={:?} fresh_node_id={:?} retained_location={:?} fresh_location={:?} retained_size={:?} fresh_size={:?} intent={}",
            label,
            id.0,
            Self::format_layout_id_path(path),
            retained_node_id,
            fresh_node_id,
            retained_layout.location,
            fresh_layout.location,
            retained_layout.size,
            fresh_layout.size,
            Self::layout_intent_summary(self.intent(id)),
        )
    }

    fn format_layout_id_path(path: &[LayoutId]) -> String {
        path.iter()
            .map(|id| id.0.to_string())
            .collect::<Vec<_>>()
            .join("/")
    }

    fn layout_has_zero_size(layout: &SolverLayout) -> bool {
        layout.size.width <= 0.0 || layout.size.height <= 0.0
    }
}

impl RetainedLayoutForest {
    /// Commit a current-frame intent into a retained root slot.
    ///
    /// This method is the root of the retained occurrence update. It may reuse a
    /// previous occurrence, build fresh mirror nodes, or detach obsolete
    /// subtrees, but all resulting solver mutations stay inside the forest.
    fn commit_layout(&mut self, root_id: RetainedLayoutRootId, id: LayoutId) -> SolverNodeId {
        if let Some(node_id) = self.committed.try_node(id) {
            return node_id;
        }

        assert!(
            !self.root_slots.has_current_root(root_id),
            "retained layout root should be committed at most once per frame"
        );
        let retained_root = self.root_slots.take_retained_root(root_id);
        if trace::detail_enabled() && trace::layout_id_is_targeted(Some(id.0)) {
            eprintln!(
                "gpui retained_layout commit_root_candidate root_id={:?} layout_id={} retained_root={}",
                root_id,
                id.0,
                retained_root.is_some()
            );
        }

        let retained_node = self.commit_intent(id, retained_root);
        let node_id = retained_node.node_id;
        self.flush_detached_subtree_removals();
        self.root_slots.insert_current_root(root_id, retained_node);
        if trace::detail_enabled() && trace::layout_id_is_targeted(Some(id.0)) {
            eprintln!(
                "gpui retained_layout commit_root_done root_id={:?} layout_id={} node_id={:?} current_roots={}",
                root_id,
                id.0,
                node_id,
                self.root_slots.current_root_count()
            );
        }
        #[cfg(any(test, debug_assertions))]
        self.debug_assert_committed_intent_matches(id, node_id);
        node_id
    }

    /// Commit an intent that must be a mirror root before computing layout.
    fn commit_root_layout(&mut self, root_id: RetainedLayoutRootId, id: LayoutId) -> SolverNodeId {
        let node_id = self.commit_layout(root_id, id);
        assert!(
            self.solver.parent(node_id).is_none(),
            "layout root must not already be committed under a parent"
        );
        node_id
    }

    /// Test-only retained commit probe that preserves solver privacy.
    #[cfg(test)]
    pub(super) fn commit_layout_for_tests(
        &mut self,
        root_id: RetainedLayoutRootId,
        id: LayoutId,
    ) -> RetainedNodeToken {
        RetainedNodeToken(self.commit_layout(root_id, id))
    }

    /// Test-only root commit probe that preserves solver privacy.
    #[cfg(test)]
    pub(super) fn commit_root_layout_for_tests(
        &mut self,
        root_id: RetainedLayoutRootId,
        id: LayoutId,
    ) -> RetainedNodeToken {
        RetainedNodeToken(self.commit_root_layout(root_id, id))
    }

    /// Commit one intent against an optional previous retained occurrence.
    fn commit_intent(
        &mut self,
        id: LayoutId,
        previous: Option<RetainedLayoutOccurrence>,
    ) -> RetainedLayoutOccurrence {
        assert!(
            !self.committed.contains_layout(id),
            "layout intent should appear only once in a committed layout tree"
        );

        let probe_global_id = self
            .subtree_probe
            .matched_global_id(self.intent(id).global_id.as_ref());
        let work_snapshot = probe_global_id.as_ref().map(|_| self.work.snapshot());
        let intent = self.intent(id);
        let retained_node = match intent.kind.clone() {
            LayoutIntentKind::Unmeasured { children } => {
                self.commit_unmeasured_intent(id, intent.style.clone(), children, previous)
            }
            LayoutIntentKind::Measured(measured) => {
                self.commit_measured_intent(id, intent.style.clone(), measured, previous)
            }
        };

        if let (Some(global_id), Some(work_snapshot)) = (probe_global_id, work_snapshot) {
            let work_delta = self.work.delta_since(work_snapshot);
            let node_ids = Self::retained_occurrence_node_ids(&retained_node);
            self.subtree_probe
                .record_committed_subtree(global_id, id, node_ids, work_delta);
        }

        retained_node
    }

    /// Build a retained occurrence with fresh mirror nodes only.
    fn build_fresh_occurrence(&mut self, id: LayoutId) -> RetainedLayoutOccurrence {
        assert!(
            !self.committed.contains_layout(id),
            "layout intent should appear only once in a committed layout tree"
        );

        match self.intent(id).kind.clone() {
            LayoutIntentKind::Unmeasured { children } => {
                self.build_fresh_unmeasured_occurrence(id, self.intent(id).style.clone(), children)
            }
            LayoutIntentKind::Measured(measured) => {
                self.build_fresh_measured_occurrence(id, self.intent(id).style.clone(), measured)
            }
        }
    }

    /// Build an unmeasured retained occurrence and mirror subtree from scratch.
    fn build_fresh_unmeasured_occurrence(
        &mut self,
        id: LayoutId,
        style: SolverStyle,
        children: Vec<LayoutId>,
    ) -> RetainedLayoutOccurrence {
        assert!(
            !self.committed.contains_layout(id),
            "layout intent should appear only once in a committed layout tree"
        );

        let mut retained_children = Vec::with_capacity(children.len());
        let mut child_node_ids = Vec::with_capacity(children.len());
        for child in children {
            let retained_child = self.build_fresh_occurrence(child);
            child_node_ids.push(retained_child.node_id);
            retained_children.push(retained_child);
        }

        let node_id = if child_node_ids.is_empty() {
            self.solver.new_leaf(style.clone())
        } else {
            self.solver
                .new_with_children(style.clone(), &child_node_ids)
        };
        self.work.record_create();
        self.mark_solver_node_committed(node_id);
        self.committed.insert(id, node_id);
        RetainedLayoutOccurrence {
            node_id,
            identity: self.intent(id).global_id.clone(),
            style,
            kind: RetainedLayoutOccurrenceKind::Unmeasured {
                children: retained_children,
            },
        }
    }

    /// Build a measured retained occurrence and mirror node from scratch.
    fn build_fresh_measured_occurrence(
        &mut self,
        id: LayoutId,
        style: SolverStyle,
        measured_facts: MeasuredLayoutFacts,
    ) -> RetainedLayoutOccurrence {
        assert!(
            !self.committed.contains_layout(id),
            "layout intent should appear only once in a committed layout tree"
        );

        let node_id = self.solver.new_measured(style.clone());
        self.work.record_create();
        self.mark_solver_node_committed(node_id);
        self.measurements
            .insert_current_measurement_for_layout(node_id, id, &measured_facts);
        self.committed.insert(id, node_id);
        RetainedLayoutOccurrence {
            node_id,
            identity: self.intent(id).global_id.clone(),
            style,
            kind: RetainedLayoutOccurrenceKind::Measured { measured_facts },
        }
    }

    /// Read a current-frame intent by id.
    fn intent(&self, id: LayoutId) -> &LayoutIntent {
        self.frame.intent(id)
    }

    /// Commit an unmeasured intent, reusing the previous occurrence when valid.
    fn commit_unmeasured_intent(
        &mut self,
        id: LayoutId,
        style: SolverStyle,
        children: Vec<LayoutId>,
        previous: Option<RetainedLayoutOccurrence>,
    ) -> RetainedLayoutOccurrence {
        assert!(
            !self.committed.contains_layout(id),
            "layout intent should appear only once in a committed layout tree"
        );

        let Some(previous) = previous else {
            self.work.record_no_previous_miss();
            self.trace_retained_layout_miss("no_previous", id, None, || String::new());
            return self.build_fresh_unmeasured_occurrence(id, style, children);
        };

        self.update_unmeasured_retained_occurrence(id, style, children, previous)
    }

    /// Update an unmeasured occurrence and mirror node to match the current intent.
    fn update_unmeasured_retained_occurrence(
        &mut self,
        id: LayoutId,
        style: SolverStyle,
        children: Vec<LayoutId>,
        previous: RetainedLayoutOccurrence,
    ) -> RetainedLayoutOccurrence {
        assert!(
            !self.committed.contains_layout(id),
            "layout intent should appear only once in a committed layout tree"
        );

        if !matches!(
            previous.kind,
            RetainedLayoutOccurrenceKind::Unmeasured { .. }
        ) {
            let fresh = self.build_fresh_unmeasured_occurrence(id, style, children);
            self.root_slots.detach_subtree(previous);
            return fresh;
        }

        let RetainedLayoutOccurrence {
            node_id,
            style: previous_style,
            kind:
                RetainedLayoutOccurrenceKind::Unmeasured {
                    children: previous_children,
                },
            ..
        } = previous
        else {
            unreachable!("measured previous occurrence handled above")
        };
        let previous_child_node_ids = previous_children
            .iter()
            .map(|child| child.node_id)
            .collect::<Vec<_>>();
        let mut previous_children = previous_children.into_iter().map(Some).collect::<Vec<_>>();
        let style_changed = previous_style != style;
        self.work.record_reuse();
        self.mark_solver_node_committed(node_id);
        self.committed.insert(id, node_id);

        let mut assigned_previous_children =
            self.assign_previous_child_occurrences(&children, previous_children.as_mut_slice());
        let mut retained_children = Vec::with_capacity(children.len());
        let mut child_node_ids = Vec::with_capacity(children.len());
        for (index, child) in children.into_iter().enumerate() {
            let assigned_previous_child = assigned_previous_children[index].take();
            let retained_child = self.commit_intent(child, assigned_previous_child);
            child_node_ids.push(retained_child.node_id);
            retained_children.push(retained_child);
        }

        if style_changed {
            self.trace_retained_style_update(id, node_id, &previous_style, &style);
            self.solver.set_style(node_id, style.clone());
            self.mark_mirror_path_dirty(node_id);
            self.work.record_style_update();
        }

        let children_changed = previous_child_node_ids != child_node_ids;
        if children_changed {
            self.solver.set_children(node_id, &child_node_ids);
            self.mark_mirror_path_dirty(node_id);
            self.work.record_child_list_update();
        }

        for previous_child in previous_children.into_iter().flatten() {
            self.root_slots.detach_subtree(previous_child);
        }

        let retained_node = RetainedLayoutOccurrence {
            node_id,
            identity: self.intent(id).global_id.clone(),
            style,
            kind: RetainedLayoutOccurrenceKind::Unmeasured {
                children: retained_children,
            },
        };
        retained_node
    }

    /// Assign previous child occurrences to current child intents.
    ///
    /// An occurrence may be preserved only when exact current facts prove it
    /// already represents the same subtree, or when a unique sibling identity
    /// proves it is the same semantic child whose current facts will be
    /// committed before the legal root solve. Same-position broad-kind reuse is
    /// deliberately absent.
    fn assign_previous_child_occurrences(
        &self,
        children: &[LayoutId],
        previous_children: &mut [Option<RetainedLayoutOccurrence>],
    ) -> Vec<Option<RetainedLayoutOccurrence>> {
        let mut assigned = std::iter::repeat_with(|| None)
            .take(children.len())
            .collect::<Vec<_>>();
        let unique_current_global_ids = self.unique_current_child_global_ids(children);
        let unique_previous_global_ids = Self::unique_previous_child_global_ids(previous_children);

        for (index, child) in children.iter().enumerate() {
            let Some(candidate) = previous_children
                .get(index)
                .and_then(|previous_child| previous_child.as_ref())
            else {
                continue;
            };
            let should_preserve = self
                .retained_occurrence_is_exact_current_intent(*child, candidate)
                && self.same_slot_exact_match_is_unambiguous(*child, children, previous_children);
            if should_preserve {
                let previous_child = previous_children
                    .get_mut(index)
                    .expect("previous child index should exist after immutable lookup");
                assigned[index] = previous_child.take();
            }
        }

        for (index, child) in children.iter().enumerate() {
            if assigned[index].is_none() {
                assigned[index] = self.take_semantic_previous_child(
                    *child,
                    index,
                    previous_children,
                    &unique_current_global_ids,
                    &unique_previous_global_ids,
                );
            }
        }

        for (index, child) in children.iter().enumerate() {
            if assigned[index].is_none() {
                assigned[index] =
                    self.take_unique_exact_previous_child(*child, children, previous_children);
            }
        }

        assigned
    }

    fn same_slot_exact_match_is_unambiguous(
        &self,
        child: LayoutId,
        current_children: &[LayoutId],
        previous_children: &[Option<RetainedLayoutOccurrence>],
    ) -> bool {
        self.current_exact_child_count(child, current_children)
            == self.previous_exact_child_count(child, previous_children)
    }

    /// Find unique current child identities. Duplicates are not semantic proof.
    fn unique_current_child_global_ids(
        &self,
        children: &[LayoutId],
    ) -> FxHashMap<GlobalElementId, Option<usize>> {
        let mut ids = FxHashMap::default();
        for (index, child) in children.iter().enumerate() {
            if let Some(global_id) = self.intent(*child).global_id.clone() {
                Self::insert_unique_global_id(&mut ids, global_id, index);
            }
        }
        ids
    }

    /// Find unique previous child identities. Duplicates are not semantic proof.
    fn unique_previous_child_global_ids(
        previous_children: &[Option<RetainedLayoutOccurrence>],
    ) -> FxHashMap<GlobalElementId, Option<usize>> {
        let mut ids = FxHashMap::default();
        for (index, previous_child) in previous_children.iter().enumerate() {
            let Some(previous_child) = previous_child else {
                continue;
            };
            if let Some(global_id) = previous_child.identity.clone() {
                Self::insert_unique_global_id(&mut ids, global_id, index);
            }
        }
        ids
    }

    fn insert_unique_global_id(
        ids: &mut FxHashMap<GlobalElementId, Option<usize>>,
        global_id: GlobalElementId,
        index: usize,
    ) {
        match ids.get_mut(&global_id) {
            Some(existing) => *existing = None,
            None => {
                ids.insert(global_id, Some(index));
            }
        }
    }

    /// Remove one unique semantic previous child from the available sibling set.
    fn take_semantic_previous_child(
        &self,
        child: LayoutId,
        current_index: usize,
        previous_children: &mut [Option<RetainedLayoutOccurrence>],
        unique_current_global_ids: &FxHashMap<GlobalElementId, Option<usize>>,
        unique_previous_global_ids: &FxHashMap<GlobalElementId, Option<usize>>,
    ) -> Option<RetainedLayoutOccurrence> {
        let global_id = self.intent(child).global_id.as_ref()?;
        if unique_current_global_ids.get(global_id).copied().flatten() != Some(current_index) {
            return None;
        }
        let previous_index = unique_previous_global_ids
            .get(global_id)
            .copied()
            .flatten()?;
        let previous_child = previous_children.get_mut(previous_index)?;
        let candidate = previous_child.as_ref()?;
        if self.retained_occurrence_can_preserve_unique_semantic_intent(child, candidate) {
            return previous_child.take();
        }
        None
    }

    /// Remove the only exact previous child match from the available sibling set.
    fn take_unique_exact_previous_child(
        &self,
        child: LayoutId,
        current_children: &[LayoutId],
        previous_children: &mut [Option<RetainedLayoutOccurrence>],
    ) -> Option<RetainedLayoutOccurrence> {
        if self.current_exact_child_count(child, current_children) != 1 {
            return None;
        }

        let mut matched_index = None;
        for (index, previous_child) in previous_children.iter().enumerate() {
            let Some(candidate) = previous_child.as_ref() else {
                continue;
            };
            if self.retained_occurrence_is_exact_current_intent(child, candidate) {
                if matched_index.is_some() {
                    return None;
                }
                matched_index = Some(index);
            }
        }
        previous_children.get_mut(matched_index?)?.take()
    }

    fn current_exact_child_count(&self, child: LayoutId, current_children: &[LayoutId]) -> usize {
        current_children
            .iter()
            .filter(|candidate| self.current_intent_subtrees_are_exact(child, **candidate))
            .count()
    }

    fn previous_exact_child_count(
        &self,
        child: LayoutId,
        previous_children: &[Option<RetainedLayoutOccurrence>],
    ) -> usize {
        previous_children
            .iter()
            .filter_map(Option::as_ref)
            .filter(|candidate| self.retained_occurrence_is_exact_current_intent(child, candidate))
            .count()
    }

    fn current_intent_subtrees_are_exact(&self, left: LayoutId, right: LayoutId) -> bool {
        let left_intent = self.intent(left);
        let right_intent = self.intent(right);
        if left_intent.global_id != right_intent.global_id {
            return false;
        }
        if left_intent.style != right_intent.style {
            return false;
        }
        if left_intent.artifact_policy != right_intent.artifact_policy {
            return false;
        }

        match (&left_intent.kind, &right_intent.kind) {
            (
                LayoutIntentKind::Unmeasured {
                    children: left_children,
                },
                LayoutIntentKind::Unmeasured {
                    children: right_children,
                },
            ) => {
                left_children.len() == right_children.len()
                    && left_children
                        .iter()
                        .zip(right_children)
                        .all(|(left_child, right_child)| {
                            self.current_intent_subtrees_are_exact(*left_child, *right_child)
                        })
            }
            (
                LayoutIntentKind::Measured(left_measured),
                LayoutIntentKind::Measured(right_measured),
            ) => left_measured == right_measured,
            _ => false,
        }
    }

    /// Commit a measured intent and register its current-frame producer.
    ///
    /// Pure-size and text measured nodes keep their mirror identity across
    /// explicit key changes; the key change dirties the node and replaces the
    /// comparable retained facts. Opaque producers are still rebuilt because
    /// their closure body is not layout-visible data.
    fn commit_measured_intent(
        &mut self,
        id: LayoutId,
        style: SolverStyle,
        measured_facts: MeasuredLayoutFacts,
        previous: Option<RetainedLayoutOccurrence>,
    ) -> RetainedLayoutOccurrence {
        assert!(
            !self.committed.contains_layout(id),
            "layout intent should appear only once in a committed layout tree"
        );

        let Some(previous) = previous else {
            self.work.record_no_previous_miss();
            self.trace_retained_layout_miss("no_previous", id, None, || String::new());
            return self.build_fresh_measured_occurrence(id, style, measured_facts);
        };

        let previous_measured_facts = previous.measured_facts().cloned();
        let compatible = previous
            .measured_facts()
            .map(|previous_measured_facts| {
                previous_measured_facts.can_reuse_solver_node_with(&measured_facts)
            })
            .unwrap_or(false);
        if !compatible {
            self.work.record_measured_facts_miss();
            self.trace_retained_layout_miss("measured_facts", id, Some(&previous), || {
                format!(
                    "previous_measured_facts={} current_measured_facts={}",
                    previous
                        .measured_facts()
                        .map(Self::debug_fingerprint)
                        .unwrap_or_else(|| "none".to_string()),
                    Self::debug_fingerprint(&measured_facts)
                )
            });
            let fresh = self.build_fresh_measured_occurrence(id, style, measured_facts);
            self.root_slots.detach_subtree(previous);
            return fresh;
        }

        let RetainedLayoutOccurrence {
            node_id,
            style: previous_style,
            kind: RetainedLayoutOccurrenceKind::Measured { .. },
            ..
        } = previous
        else {
            unreachable!("unmeasured previous occurrence handled by compatibility check")
        };
        self.work.record_reuse();
        self.mark_solver_node_committed(node_id);
        self.committed.insert(id, node_id);

        let style_changed = previous_style != style;
        let measured_facts_changed = previous_measured_facts.as_ref() != Some(&measured_facts);
        if style_changed {
            self.trace_retained_style_update(id, node_id, &previous_style, &style);
            self.solver.set_style(node_id, style.clone());
            self.mark_mirror_path_dirty(node_id);
            self.work.record_style_update();
        }
        if measured_facts_changed && !style_changed {
            self.trace_retained_dirty_mark(id, node_id, "measured_facts_changed");
            self.mark_solver_node_dirty(node_id);
        }

        self.measurements
            .insert_current_measurement_for_layout(node_id, id, &measured_facts);
        RetainedLayoutOccurrence {
            node_id,
            identity: self.intent(id).global_id.clone(),
            style,
            kind: RetainedLayoutOccurrenceKind::Measured { measured_facts },
        }
    }

    /// Check whether an existing occurrence subtree is an exact semantic match.
    ///
    /// This is a proof rule, not a heuristic. A `true` result means the
    /// occurrence's retained facts and child shape already match the current
    /// intent tree, so its solver cache can remain meaningful.
    fn retained_occurrence_is_exact_current_intent(
        &self,
        id: LayoutId,
        previous: &RetainedLayoutOccurrence,
    ) -> bool {
        let intent = self.intent(id);
        if previous.identity.as_ref() != intent.global_id.as_ref() {
            return false;
        }
        if previous.style != intent.style {
            return false;
        }

        match (&intent.kind, &previous.kind) {
            (
                LayoutIntentKind::Unmeasured { children },
                RetainedLayoutOccurrenceKind::Unmeasured {
                    children: previous_children,
                },
            ) => {
                previous_children.len() == children.len()
                    && children
                        .iter()
                        .zip(previous_children)
                        .all(|(child, previous_child)| {
                            self.retained_occurrence_is_exact_current_intent(*child, previous_child)
                        })
            }
            (
                LayoutIntentKind::Measured(measured),
                RetainedLayoutOccurrenceKind::Measured {
                    measured_facts: previous_measured_facts,
                },
            ) => previous_measured_facts == measured,
            _ => false,
        }
    }

    /// Return whether a unique semantic identity may preserve mirror identity.
    fn retained_occurrence_can_preserve_unique_semantic_intent(
        &self,
        id: LayoutId,
        previous: &RetainedLayoutOccurrence,
    ) -> bool {
        match (&self.intent(id).kind, &previous.kind) {
            (
                LayoutIntentKind::Unmeasured { .. },
                RetainedLayoutOccurrenceKind::Unmeasured { .. },
            ) => true,
            (
                LayoutIntentKind::Measured(measured),
                RetainedLayoutOccurrenceKind::Measured {
                    measured_facts: previous_measured_facts,
                },
            ) => previous_measured_facts.can_reuse_solver_node_with(measured),
            _ => false,
        }
    }

    fn retained_occurrence_node_ids(retained_node: &RetainedLayoutOccurrence) -> Vec<SolverNodeId> {
        let mut node_ids = vec![retained_node.node_id];
        for child in retained_node.children() {
            node_ids.extend(Self::retained_occurrence_node_ids(child));
        }
        node_ids
    }

    #[cfg(any(test, debug_assertions))]
    /// Assert that the private mirror is equivalent to the current intent tree.
    fn debug_assert_committed_intent_matches(&mut self, id: LayoutId, node_id: SolverNodeId) {
        let mut seen = FxHashSet::default();
        self.debug_assert_intent_node_matches(id, node_id, None, &mut seen);
    }

    #[cfg(any(test, debug_assertions))]
    fn debug_assert_intent_node_matches(
        &self,
        id: LayoutId,
        node_id: SolverNodeId,
        expected_parent: Option<SolverNodeId>,
        seen: &mut FxHashSet<SolverNodeId>,
    ) {
        assert!(
            seen.insert(node_id),
            "committed solver node should appear at only one current intent position"
        );
        assert_eq!(self.solver.parent(node_id), expected_parent);

        let intent = self.intent(id);
        let solver_style = self.solver.style(node_id).expect(EXPECT_MESSAGE);
        assert_eq!(&solver_style, &intent.style);

        match &intent.kind {
            LayoutIntentKind::Unmeasured { children } => {
                assert!(
                    !self.measurements.has_current_measurement(node_id),
                    "unmeasured intent should not have current measurement state"
                );
                assert!(!self.solver.has_measure_context(node_id));
                let child_node_ids = children
                    .iter()
                    .map(|child| self.committed.node(*child))
                    .collect::<Vec<_>>();
                assert_eq!(self.solver.children(node_id), child_node_ids);
                for (child, child_node_id) in children.iter().zip(child_node_ids) {
                    self.debug_assert_intent_node_matches(
                        *child,
                        child_node_id,
                        Some(node_id),
                        seen,
                    );
                }
            }
            LayoutIntentKind::Measured(measured) => {
                assert!(self.solver.has_measure_context(node_id));
                assert_eq!(self.solver.children(node_id), Vec::<SolverNodeId>::new());
                self.measurements
                    .debug_assert_current_measurement_matches(node_id, measured);
            }
        }
    }

    /// Mark a mirror node as used at one current-frame position.
    fn mark_solver_node_committed(&mut self, node_id: SolverNodeId) {
        self.committed.mark_solver_node_committed(node_id);
    }

    /// Mark a private mirror node dirty at most once in the current frame.
    fn mark_solver_node_dirty(&mut self, node_id: SolverNodeId) {
        if self.committed.mark_solver_node_dirty(node_id) {
            self.mark_mirror_path_dirty(node_id);
            self.work.record_dirty_mark();
        }
    }

    /// Make a mirror mutation visible to the next legal root solve.
    ///
    /// The current solver's dirty flag is implemented as cache clearing, so a direct
    /// `mark_dirty(node)` may stop when that node's cache is already empty.
    /// The retained forest owns the mutation transaction: once it changes a
    /// mirror node's style, children, or measurement facts, every ancestor on
    /// the retained parent path must be eligible for the scheduled root solve.
    fn mark_mirror_path_dirty(&mut self, node_id: SolverNodeId) {
        let mut current = Some(node_id);
        while let Some(node_id) = current {
            self.solver.mark_dirty(node_id);
            current = self.solver.parent(node_id);
        }
    }

    fn flush_detached_subtree_removals(&mut self) {
        for retained_node in self.root_slots.take_detached_subtree_removals() {
            self.remove_retained_subtree(retained_node);
        }
    }

    fn remove_retained_subtree(&mut self, retained_node: RetainedLayoutOccurrence) {
        let is_measured = matches!(
            retained_node.kind,
            RetainedLayoutOccurrenceKind::Measured { .. }
        );
        let node_id = retained_node.node_id;
        for child in retained_node.into_children() {
            self.remove_retained_subtree(child);
        }
        if is_measured {
            self.solver.clear_measure_context(node_id);
            self.work.record_measured_context_clear();
        }
        self.solver.remove(node_id);
        self.work.record_remove();
    }
}
