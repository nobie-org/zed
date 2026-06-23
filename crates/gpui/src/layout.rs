//! GPUI's retained layout facade.
//!
//! Callers produce current-frame layout intents through `LayoutEngine`; the
//! retained forest owns the cross-frame layout nodes and the private solver
//! mirror. This module is the only facade GPUI code should use for layout.

use crate::{
    App, Bounds, GlobalElementId, MeasureCx, Pixels, Size, Style, TextLayoutArtifact,
    TextMeasureKey, Window,
};
use core::panic::Location;
use stacksafe::stacksafe;

mod retained_forest;
mod telemetry;
pub(crate) use retained_forest::PureSizeMeasure;
#[cfg(test)]
use retained_forest::{
    FreshLayoutComparisonSummary, RetainedForestMutationSample, RetainedLayoutProjectionForTests,
    RetainedLayoutShapeForTests, RetainedNodeToken,
};
use retained_forest::{
    LayoutArtifact, LayoutArtifactKey, MeasuredLayoutRequest, RetainedLayoutForest,
    RetainedLayoutForestCheckpoint,
};
pub use telemetry::LayoutWorkSample;
#[cfg(any(test, feature = "test-support"))]
pub use telemetry::RetainedSubtreeWorkSample;

/// Layout entry point used by `Window`.
///
/// `LayoutEngine` accepts current-frame layout requests, delegates retained
/// ownership to `RetainedLayoutForest`, and accumulates frame-local work
/// telemetry. It intentionally exposes `LayoutId` and bounds, not solver node
/// handles, so callers cannot mutate or depend on the mirror directly.
pub(crate) struct LayoutEngine {
    mode: LayoutEngineMode,
    forest: RetainedLayoutForest,
    layout_work: LayoutWorkSample,
}

/// Runtime layout retention mode.
///
/// This is a diagnostic switch, not a second layout authority. Both modes use
/// the same GPUI frame lifecycle and private root solve path. `Immediate`
/// discards retained state at frame finish so the app can be run against a
/// fresh-tree baseline while still exercising the retained-layout facade.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LayoutEngineMode {
    Retained,
    Immediate,
}

impl LayoutEngineMode {
    fn from_env() -> Self {
        static MODE: std::sync::OnceLock<LayoutEngineMode> = std::sync::OnceLock::new();
        *MODE.get_or_init(|| match std::env::var("GPUI_LAYOUT_MODE") {
            Ok(mode) if mode == "retained" => Self::Retained,
            Ok(mode) if mode == "immediate" || mode == "fresh" => Self::Immediate,
            Ok(mode) => panic!(
                "unsupported GPUI_LAYOUT_MODE={mode:?}; expected `retained`, `immediate`, or `fresh`"
            ),
            Err(_) => Self::Retained,
        })
    }
}

/// Stable framework identity for one legal root compute site.
///
/// A root site is not user-authored element identity. It identifies the GPUI
/// code path that is allowed to solve an independent layout root, so private
/// sizing probes, visible list items, tooltips, and the window root cannot
/// accidentally reuse the same retained root just because they are anonymous
/// under the same element stack.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RetainedLayoutRootSite(&'static Location<'static>);

impl RetainedLayoutRootSite {
    pub(crate) fn caller(location: &'static Location<'static>) -> Self {
        Self(location)
    }

    pub(crate) fn location(&self) -> &'static Location<'static> {
        self.0
    }
}

/// Stable identity for a computed layout root across frames.
///
/// GPUI can compute multiple roots during one frame, including anonymous roots
/// created by deferred/prepaint work. The forest uses this id to decide which
/// retained root may be reused; root order alone is not a correctness proof.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct RetainedLayoutRootId(u64);

impl RetainedLayoutRootId {
    fn new(id: u64) -> Self {
        Self(id)
    }
}

fn text_layout_artifact_key(measure_key: TextMeasureKey) -> LayoutArtifactKey {
    LayoutArtifactKey::new(
        measure_key,
        |measure_key, known_dimensions, available_space, window, cx| {
            measure_key
                .measure(
                    known_dimensions,
                    available_space,
                    &mut MeasureCx::new(window, cx),
                )
                .size()
        },
    )
}

/// One-shot authority to solve one retained layout root.
///
/// The token binds root identity to the `LayoutId` that created it and is
/// consumed by `compute_retained_layout`, so callers cannot freely recombine
/// arbitrary layout ids and retained root ids.
pub(crate) struct RetainedLayoutRoot {
    id: RetainedLayoutRootId,
    layout_id: LayoutId,
}

/// Exact retained-layout rollback point for retryable GPUI transactions.
///
/// `Window::transact` may speculatively run prepaint/layout and then abandon
/// it. The checkpoint must include both GPUI retained state and the solver
/// mirror, otherwise a failed attempt can poison a later successful frame.
pub(crate) struct LayoutCheckpoint {
    forest: RetainedLayoutForestCheckpoint,
    layout_work: LayoutWorkSample,
}

const EXPECT_MESSAGE: &str = "we should avoid layout solver errors by construction if possible";

impl LayoutEngine {
    /// Create an empty retained layout engine.
    pub fn new() -> Self {
        Self::new_with_mode(LayoutEngineMode::from_env())
    }

    fn new_with_mode(mode: LayoutEngineMode) -> Self {
        LayoutEngine {
            mode,
            forest: RetainedLayoutForest::new(),
            layout_work: LayoutWorkSample::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_force_fresh_for_tests() -> Self {
        Self::new_with_mode(LayoutEngineMode::Immediate)
    }

    /// End the frame, promote successfully computed roots, and return work telemetry.
    pub fn finish_frame(&mut self) -> LayoutWorkSample {
        let finish_frame_start = std::time::Instant::now();
        let (retained_layout_work, retained_layout_misses) = self.forest.finish_frame();
        let retained_layout_finish_frame_duration = finish_frame_start.elapsed();
        let mut layout_work = self.layout_work;
        layout_work.retained_layout_finish_frame_duration += retained_layout_finish_frame_duration;
        layout_work.record_retained_layout_work(retained_layout_work, retained_layout_misses);
        if self.mode == LayoutEngineMode::Immediate {
            self.forest.reset_retained_state_for_fresh_frame();
            layout_work.force_fresh_frame_resets = 1;
        }
        self.layout_work = LayoutWorkSample::default();
        layout_work
    }

    /// Reset frame-local request/measurement state before a new render pass.
    pub fn begin_frame(&mut self) {
        self.layout_work = LayoutWorkSample::default();
        self.forest.begin_frame();
    }

    /// Allocate or look up a retained root id for a root compute site.
    ///
    /// A global id is authoritative when present. Anonymous roots are scratch
    /// roots: without explicit identity, there is no cross-frame retained root
    /// identity to look up.
    pub(crate) fn retained_root(
        &mut self,
        layout_id: LayoutId,
        root_site: RetainedLayoutRootSite,
        global_id: Option<&GlobalElementId>,
    ) -> RetainedLayoutRoot {
        let id = self.forest.retained_root_id(root_site, global_id);
        RetainedLayoutRoot { id, layout_id }
    }

    /// Snapshot all retained layout state affected by speculative layout.
    pub(crate) fn checkpoint(&self) -> LayoutCheckpoint {
        LayoutCheckpoint {
            forest: self.forest.checkpoint(),
            layout_work: self.layout_work,
        }
    }

    /// Restore a speculative layout attempt exactly.
    pub(crate) fn rollback_to_checkpoint(&mut self, checkpoint: LayoutCheckpoint) {
        self.forest.rollback_to_checkpoint(checkpoint.forest);
        self.layout_work = checkpoint.layout_work;
    }

    #[cfg(test)]
    fn layout_work_sample(&self) -> LayoutWorkSample {
        self.layout_work
    }

    #[cfg(test)]
    fn reset_retained_mutation_sample_for_tests(&mut self) {
        self.forest.reset_retained_mutation_sample_for_tests();
    }

    #[cfg(test)]
    fn retained_mutation_sample_for_tests(&self) -> RetainedForestMutationSample {
        self.forest.retained_mutation_sample_for_tests()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn set_retained_subtree_probe_targets_for_tests(&mut self, targets: Vec<String>) {
        self.forest
            .set_retained_subtree_probe_targets_for_tests(targets);
    }

    #[cfg(test)]
    pub(crate) fn compare_with_fresh_for_tests(&mut self) {
        self.forest.compare_with_fresh_for_tests();
    }

    #[cfg(test)]
    fn retained_subtree_work_samples_for_tests(&self) -> &[RetainedSubtreeWorkSample] {
        self.forest.retained_subtree_work_samples_for_tests()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn last_retained_subtree_work_samples_for_tests(
        &self,
    ) -> &[RetainedSubtreeWorkSample] {
        self.forest.last_retained_subtree_work_samples_for_tests()
    }

    /// Test-only helper for recording an anonymous unmeasured layout intent.
    #[cfg(test)]
    pub fn request_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        children: &[LayoutId],
    ) -> LayoutId {
        self.request_layout_with_global_id(None, style, rem_size, scale_factor, children)
    }

    /// Record an unmeasured current-frame layout intent with optional semantic identity.
    ///
    /// `global_id` is a candidate retention key, not proof by itself. The
    /// retained forest may use it only when the id is unique among both current
    /// and previous siblings; duplicate or missing ids fall back to exact
    /// subtree matching or fresh construction.
    pub(crate) fn request_layout_with_global_id(
        &mut self,
        global_id: Option<&GlobalElementId>,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        children: &[LayoutId],
    ) -> LayoutId {
        self.layout_work.layout_node_requests += 1;
        self.layout_work.child_edges += children.len() as u64;

        self.forest
            .request_layout(global_id, style, rem_size, scale_factor, children)
    }

    /// Record an opaque measured layout intent.
    ///
    /// The closure is an executable producer for this frame, not comparable
    /// layout meaning. Opaque measured nodes are therefore conservative: the
    /// forest keeps the current producer available but does not treat the
    /// closure object as a retained identity.
    pub fn request_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut MeasureCx<'_>,
        ) -> Size<Pixels>
        + 'static,
    ) -> LayoutId {
        self.layout_work.measured_layout_node_requests += 1;
        self.forest.request_measured_layout(
            style,
            rem_size,
            scale_factor,
            MeasuredLayoutRequest::opaque(measure),
        )
    }

    /// Record a measured layout intent whose size computation is explicit data.
    ///
    /// The `PureSizeMeasure` value is layout-visible and comparable, so the solver can
    /// reuse measurement cache when the retained node and measure input are both
    /// unchanged.
    pub(crate) fn request_pure_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measure: PureSizeMeasure,
    ) -> LayoutId {
        self.layout_work.measured_layout_node_requests += 1;
        self.forest.request_measured_layout(
            style,
            rem_size,
            scale_factor,
            MeasuredLayoutRequest::pure_size(measure),
        )
    }

    /// Record a text measured layout intent with explicit artifact facts.
    ///
    /// The solver callback answers size. Text prepaint owns shaped paint and
    /// hit-test artifacts from final solved bounds, so layout measurement never
    /// needs a side-effecting hydrator to make text paintable.
    pub(crate) fn request_text_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measure_key: TextMeasureKey,
        mut measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut MeasureCx<'_>,
        ) -> TextLayoutArtifact
        + 'static,
    ) -> LayoutId {
        self.layout_work.measured_layout_node_requests += 1;
        let artifact_key = text_layout_artifact_key(measure_key);
        self.forest.request_measured_layout(
            style,
            rem_size,
            scale_factor,
            MeasuredLayoutRequest::artifact(
                artifact_key,
                move |known_dimensions, available_space, measure_cx| {
                    let artifact = measure(known_dimensions, available_space, measure_cx);
                    let artifact_key = text_layout_artifact_key(artifact.key().clone());
                    LayoutArtifact::new(artifact_key, artifact.size())
                },
            ),
        )
    }

    /// Test-only anonymous root compute helper.
    ///
    /// Production callers go through `Window`, which allocates retained root
    /// identity before computing. Tests use this helper when they exercise
    /// `LayoutEngine` without a `Window` root site.
    #[cfg(test)]
    #[stacksafe]
    pub fn compute_layout(
        &mut self,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.compute_layout_in_root(
            id,
            RetainedLayoutRootId::new(id.0 as u64),
            available_space,
            window,
            cx,
        );
    }

    /// Compute a root with explicit retained identity.
    #[stacksafe]
    pub(crate) fn compute_retained_layout(
        &mut self,
        root: RetainedLayoutRoot,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.compute_layout_in_root(root.layout_id, root.id, available_space, window, cx);
    }

    #[cfg(test)]
    fn compute_retained_layout_for_tests(
        &mut self,
        id: LayoutId,
        root_id: RetainedLayoutRootId,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.compute_layout_in_root(id, root_id, available_space, window, cx);
    }

    /// Commit the current facts into the retained forest and ask the mirror to compute them.
    fn compute_layout_in_root(
        &mut self,
        id: LayoutId,
        root_id: RetainedLayoutRootId,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let scale_factor = window.scale_factor();
        self.layout_work.compute_layout_calls += 1;
        let compute_work =
            self.forest
                .compute_layout(root_id, id, available_space, scale_factor, window, cx);
        self.layout_work.solver_compute_layout_calls += compute_work.solver_compute_layout_calls;
        self.layout_work.solver_cache_hits += compute_work.solver_cache_events.hits;
        self.layout_work.solver_cache_stores += compute_work.solver_cache_events.stores;
        self.layout_work.solver_cache_clears += compute_work.solver_cache_events.clears;
        self.layout_work.solver_cache_measure_observations +=
            compute_work.solver_cache_events.measure_observations;
        self.layout_work.retained_layout_commit_duration +=
            compute_work.retained_layout_commit_duration;
        self.layout_work.solver_observation_setup_duration +=
            compute_work.solver_observation_setup_duration;
        self.layout_work.solver_layout_duration += compute_work.solver_layout_duration;
        self.layout_work.geometry_capture_duration += compute_work.geometry_capture_duration;
        self.layout_work.artifact_completion_duration += compute_work.artifact_completion_duration;
        self.layout_work.fresh_compare_duration += compute_work.fresh_compare_duration;
        self.layout_work.compute_layout_duration += compute_work.compute_layout_duration;
        self.layout_work.measured_layout_calls += compute_work.measured_layout_calls;
        self.layout_work.measured_layout_duration += compute_work.measured_layout_duration;
        if let Some(comparison) = compute_work.fresh_layout_comparison {
            self.layout_work
                .record_retained_fresh_layout_comparison(comparison);
        }
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
    //  1. Pre-layout metric snapping. Before the solver computes layout, all
    //     authored absolute lengths are rounded during style lowering. This
    //     includes borders, padding, gaps, and explicit sizes.
    //     Custom-measured leaf nodes have their measured sizes rounded up
    //     to integer device-pixel lengths.
    //
    //  2. Post-layout edge snapping. After the solver resolves the tree, layout
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

    /// Return post-layout bounds for a committed current-frame intent.
    pub fn layout_bounds(&self, id: LayoutId) -> Bounds<Pixels> {
        self.forest.layout_bounds(id)
    }

    #[cfg(test)]
    fn commit_layout(&mut self, id: LayoutId) -> RetainedNodeToken {
        self.forest
            .commit_layout_for_tests(RetainedLayoutRootId::new(0), id)
    }

    #[cfg(test)]
    fn commit_root_layout(&mut self, id: LayoutId) -> RetainedNodeToken {
        self.forest
            .commit_root_layout_for_tests(RetainedLayoutRootId::new(0), id)
    }

    #[cfg(test)]
    fn retained_node_token_for_tests(&self, id: LayoutId) -> RetainedNodeToken {
        self.forest.retained_node_token_for_tests(id)
    }

    #[cfg(test)]
    fn retained_layout_shape_for_tests(
        &self,
        token: RetainedNodeToken,
    ) -> RetainedLayoutShapeForTests {
        self.forest.retained_layout_shape_for_tests(token)
    }

    #[cfg(test)]
    fn retained_layout_projection_for_tests(
        &self,
        token: RetainedNodeToken,
    ) -> RetainedLayoutProjectionForTests {
        self.forest.retained_layout_projection_for_tests(token)
    }

    #[cfg(test)]
    fn retained_node_size_for_tests(&self, token: RetainedNodeToken) -> Size<f32> {
        self.forest.retained_node_size_for_tests(token)
    }

    #[cfg(test)]
    fn retained_node_layout_bounds_for_tests(
        &mut self,
        token: RetainedNodeToken,
        scale_factor: f32,
    ) -> Bounds<Pixels> {
        self.forest
            .retained_node_layout_bounds_for_tests(token, scale_factor)
    }

    #[cfg(test)]
    fn retained_child_tokens_for_tests(&self, token: RetainedNodeToken) -> Vec<RetainedNodeToken> {
        self.forest.retained_child_tokens_for_tests(token)
    }

    #[cfg(test)]
    fn retained_parent_token_for_tests(
        &self,
        token: RetainedNodeToken,
    ) -> Option<RetainedNodeToken> {
        self.forest.retained_parent_token_for_tests(token)
    }

    #[cfg(test)]
    fn retained_node_has_measure_context_for_tests(&self, token: RetainedNodeToken) -> bool {
        self.forest
            .retained_node_has_measure_context_for_tests(token)
    }

    #[cfg(test)]
    fn compute_unmeasured_layout_with_scale_for_tests(
        &mut self,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
    ) -> RetainedNodeToken {
        self.forest.compute_unmeasured_layout_for_tests(
            RetainedLayoutRootId::new(0),
            id,
            available_space,
            scale_factor,
        )
    }

    #[cfg(test)]
    fn assert_facts_committed_exactly_for_tests(&self, id: LayoutId) {
        self.forest.assert_facts_committed_exactly_for_tests(id);
    }

    #[cfg(test)]
    fn retained_fresh_layout_comparison_for_tests(
        &mut self,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> FreshLayoutComparisonSummary {
        self.forest.retained_fresh_layout_comparison_for_tests(
            RetainedLayoutRootId::new(0),
            id,
            available_space,
            window.scale_factor(),
            window,
            cx,
        )
    }
}

/// A unique identifier for a layout intent generated during the current frame.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct LayoutId(usize);

impl std::hash::Hash for LayoutId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[cfg(test)]
mod retained_framework_tests;
#[cfg(test)]
mod retained_layout_tests;

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
mod tests;
