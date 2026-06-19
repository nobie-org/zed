//! GPUI's retained layout facade.
//!
//! Callers produce current-frame layout intents through `LayoutEngine`; the
//! retained forest owns the cross-frame layout nodes and the private Taffy
//! mirror. This module is the only facade GPUI code should use for layout.

use crate::{
    App, Bounds, ElementId, GlobalElementId, Pixels, Size, Style, TextLayoutArtifact,
    TextMeasureKey, Window,
};
use stacksafe::stacksafe;

mod retained_forest;
mod telemetry;
pub(crate) use retained_forest::PureSizeMeasure;
#[cfg(test)]
use retained_forest::{
    FreshLayoutComparisonSummary, RetainedForestMutationSample, RetainedLayoutProjectionForTests,
    RetainedLayoutShapeForTests, RetainedNodeToken, RetainedSubtreeWorkSample,
};
use retained_forest::{RetainedLayoutForest, RetainedLayoutForestCheckpoint};
pub use telemetry::LayoutWorkSample;

/// Layout entry point used by `Window`.
///
/// `LayoutEngine` accepts current-frame layout requests, delegates retained
/// ownership to `RetainedLayoutForest`, and accumulates frame-local work
/// telemetry. It intentionally exposes `LayoutId` and bounds, not Taffy node
/// handles, so callers cannot mutate or depend on the mirror directly.
pub(crate) struct LayoutEngine {
    forest: RetainedLayoutForest,
    layout_work: LayoutWorkSample,
    mode: LayoutEngineMode,
}

/// Retained-layout execution policy.
///
/// `Retained` is the production path. `Immediate` rebuilds the forest and its
/// private Taffy mirror at the start of every frame, which gives GPUI a
/// same-user-code baseline for debugging and correctness oracles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LayoutEngineMode {
    Retained,
    Immediate,
}

impl LayoutEngineMode {
    fn from_env() -> Self {
        if let Ok(mode) = std::env::var("GPUI_LAYOUT_MODE") {
            match mode.to_ascii_lowercase().as_str() {
                "immediate" | "fresh" | "fresh-taffy" => return Self::Immediate,
                _ => {}
            }
        }

        if std::env::var_os("GPUI_DISABLE_RETAINED_LAYOUT").is_some() {
            Self::Immediate
        } else {
            Self::Retained
        }
    }
}

/// Stable identity for a computed layout root across frames.
///
/// GPUI can compute multiple roots during one frame, including anonymous roots
/// created by deferred/prepaint work. The forest uses this id to decide which
/// retained root may be reused; root order alone is not a correctness proof.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RetainedLayoutRootId(u64);

impl RetainedLayoutRootId {
    fn new(id: u64) -> Self {
        Self(id)
    }
}

/// Exact retained-layout rollback point for retryable GPUI transactions.
///
/// `Window::transact` may speculatively run prepaint/layout and then abandon
/// it. The checkpoint must include both GPUI retained state and the Taffy
/// mirror, otherwise a failed attempt can poison a later successful frame.
pub(crate) struct LayoutCheckpoint {
    forest: RetainedLayoutForestCheckpoint,
    layout_work: LayoutWorkSample,
}

const EXPECT_MESSAGE: &str = "we should avoid taffy layout errors by construction if possible";

impl LayoutEngine {
    /// Create an empty retained layout engine.
    pub fn new() -> Self {
        LayoutEngine {
            forest: RetainedLayoutForest::new(),
            layout_work: LayoutWorkSample::default(),
            mode: LayoutEngineMode::from_env(),
        }
    }

    /// End the frame, promote successfully computed roots, and return work telemetry.
    pub fn finish_frame(&mut self) -> LayoutWorkSample {
        let (retained_layout_work, retained_layout_misses) = self.forest.finish_frame();
        let mut layout_work = self.layout_work;
        layout_work.record_retained_layout_work(retained_layout_work, retained_layout_misses);
        self.layout_work = LayoutWorkSample::default();
        layout_work
    }

    /// Reset frame-local request/measurement state before a new render pass.
    pub fn begin_frame(&mut self) {
        self.layout_work = LayoutWorkSample::default();
        match self.mode {
            LayoutEngineMode::Retained => self.forest.begin_frame(),
            LayoutEngineMode::Immediate => {
                self.forest = RetainedLayoutForest::new();
            }
        }
    }

    /// Allocate or look up a retained root id for a root compute site.
    ///
    /// A global id is authoritative when present. Anonymous roots are scoped by
    /// their element id stack plus per-frame occurrence, which keeps repeated
    /// anonymous roots from aliasing the same retained Taffy node.
    pub(crate) fn retained_root_id(
        &mut self,
        global_id: Option<&GlobalElementId>,
        element_id_stack: &[ElementId],
    ) -> RetainedLayoutRootId {
        self.forest.retained_root_id(global_id, element_id_stack)
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

    #[cfg(test)]
    fn set_retained_subtree_probe_targets_for_tests(&mut self, targets: Vec<String>) {
        self.forest
            .set_retained_subtree_probe_targets_for_tests(targets);
    }

    #[cfg(test)]
    fn retained_subtree_work_samples_for_tests(&self) -> &[RetainedSubtreeWorkSample] {
        self.forest.retained_subtree_work_samples_for_tests()
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

    /// Record an unmeasured current-frame layout intent with observation identity.
    ///
    /// `global_id` is used only for retained-layout diagnostics. It is not a
    /// retention key and cannot change which occurrence owns a Taffy node.
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
        mut measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &mut App,
        ) -> Size<Pixels>
        + 'static,
    ) -> LayoutId {
        self.layout_work.measured_layout_node_requests += 1;
        self.forest
            .request_opaque_measured_layout(style, rem_size, scale_factor, measure)
    }

    /// Record a measured layout intent whose size computation is explicit data.
    ///
    /// The `PureSizeMeasure` value is layout-visible and comparable, so Taffy can
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
        self.forest
            .request_pure_measured_layout(style, rem_size, scale_factor, measure)
    }

    /// Record a text measured layout intent with explicit artifact hydration.
    ///
    /// GPUI needs the shaped text artifact for paint and hit testing. Under
    /// stock Taffy, retained layout treats text conservatively: the hydrator
    /// installs artifacts produced by the current compute's callback, and the
    /// retained forest does not replay artifacts from `TextMeasureKey` alone.
    pub(crate) fn request_text_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measure_key: TextMeasureKey,
        hydrate: impl Fn(&TextLayoutArtifact) + 'static,
        mut measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &mut App,
        ) -> TextLayoutArtifact
        + 'static,
    ) -> LayoutId {
        self.layout_work.measured_layout_node_requests += 1;
        self.forest.request_text_measured_layout(
            style,
            rem_size,
            scale_factor,
            measure_key,
            hydrate,
            measure,
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
        id: LayoutId,
        root_id: RetainedLayoutRootId,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.compute_layout_in_root(id, root_id, available_space, window, cx);
    }

    /// Commit the current intent into the retained forest and ask the mirror to compute it.
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

    /// Return post-layout bounds for a committed current-frame intent.
    pub fn layout_bounds(&mut self, id: LayoutId, scale_factor: f32) -> Bounds<Pixels> {
        self.forest.layout_bounds(id, scale_factor)
    }

    #[cfg(test)]
    fn commit_layout(&mut self, id: LayoutId) -> RetainedNodeToken {
        self.forest
            .commit_layout_for_tests(RetainedLayoutRootId::new(0), id)
    }

    #[cfg(test)]
    fn commit_layout_in_test_root(&mut self, root_id: u64, id: LayoutId) -> RetainedNodeToken {
        self.forest
            .commit_layout_for_tests(RetainedLayoutRootId::new(root_id), id)
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
    fn compute_unmeasured_layout_for_tests(
        &mut self,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
    ) -> RetainedNodeToken {
        self.forest.compute_unmeasured_layout_for_tests(
            RetainedLayoutRootId::new(0),
            id,
            available_space,
        )
    }

    #[cfg(test)]
    fn assert_intent_committed_exactly_for_tests(&self, id: LayoutId) {
        self.forest.assert_intent_committed_exactly_for_tests(id);
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
