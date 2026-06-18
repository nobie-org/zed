use crate::{
    AbsoluteLength, App, Bounds, DefiniteLength, Edges, GridTemplate, Length, Pixels, Point, Size,
    Style, TextLayoutArtifact, TextMeasureKey, Window, size,
    util::{
        ceil_to_device_pixel, round_half_toward_zero, round_stroke_to_device_pixel,
        round_to_device_pixel,
    },
};
use collections::{FxHashMap, FxHashSet};
use stacksafe::{StackSafe, stacksafe};
use std::{cell::RefCell, fmt::Debug, ops::Range, rc::Rc, time::Duration};
use taffy::{
    TaffyTree,
    geometry::{Point as TaffyPoint, Rect as TaffyRect, Size as TaffySize},
    prelude::{max_content, min_content},
    style::AvailableSpace as TaffyAvailableSpace,
    tree::NodeId,
};

mod reconcile;
pub(crate) use reconcile::PureSizeMeasure;
#[cfg(test)]
use reconcile::TaffyMutationCountsForTests;
use reconcile::{
    LayoutMeasureContext, MeasuredLayoutKind, MeasuredLayoutResult, NodeContext, ReconcileState,
};

/// Layout work performed by GPUI for one completed window draw.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LayoutWorkSample {
    /// Monotonic window-local draw index that produced this sample.
    pub draw_index: u64,
    /// Non-measured nodes requested through [`Window::request_layout`](crate::Window::request_layout).
    pub layout_node_requests: u64,
    /// Measured nodes requested through [`Window::request_measured_layout`](crate::Window::request_measured_layout).
    pub measured_layout_node_requests: u64,
    /// Parent-to-child layout edges passed to Taffy.
    pub child_edges: u64,
    /// Root layout computations requested for the draw.
    pub compute_layout_calls: u64,
    /// Measured layout callbacks invoked by Taffy.
    pub measured_layout_calls: u64,
    /// Wall time spent computing root layouts.
    pub compute_layout_duration: Duration,
    /// Wall time spent inside measured layout callbacks.
    pub measured_layout_duration: Duration,
}

pub struct TaffyLayoutEngine {
    taffy: TaffyTree<NodeContext>,
    reconcile: ReconcileState,
    absolute_layout_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    /// Unrounded absolute border-box top-left per-node coordinate in device pixels.
    absolute_outer_origins: FxHashMap<NodeId, Point<f32>>,
    computed_layouts: FxHashSet<NodeId>,
    layout_bounds_scratch_space: Vec<NodeId>,
    layout_work: LayoutWorkSample,
}

const EXPECT_MESSAGE: &str = "we should avoid taffy layout errors by construction if possible";

impl TaffyLayoutEngine {
    pub fn new() -> Self {
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        TaffyLayoutEngine {
            taffy,
            reconcile: ReconcileState::new(),
            absolute_layout_bounds: FxHashMap::default(),
            absolute_outer_origins: FxHashMap::default(),
            computed_layouts: FxHashSet::default(),
            layout_bounds_scratch_space: Vec::new(),
            layout_work: LayoutWorkSample::default(),
        }
    }

    pub fn finish_frame(&mut self) -> LayoutWorkSample {
        let layout_work = self.layout_work;
        self.reconcile.finish_frame(&mut self.taffy);
        self.absolute_layout_bounds.clear();
        self.absolute_outer_origins.clear();
        self.computed_layouts.clear();
        self.layout_work = LayoutWorkSample::default();
        layout_work
    }

    pub fn begin_frame(&mut self) {
        self.layout_work = LayoutWorkSample::default();
        self.reconcile.begin_frame();
    }

    #[cfg(test)]
    fn layout_work_sample(&self) -> LayoutWorkSample {
        self.layout_work
    }

    #[cfg(test)]
    fn reset_taffy_mutation_counts_for_tests(&mut self) {
        self.reconcile.reset_taffy_mutation_counts_for_tests();
    }

    #[cfg(test)]
    fn taffy_mutation_counts_for_tests(&self) -> TaffyMutationCountsForTests {
        self.reconcile.taffy_mutation_counts_for_tests()
    }

    pub fn request_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        children: &[LayoutId],
    ) -> LayoutId {
        let taffy_style = style.to_taffy(rem_size, scale_factor);
        self.layout_work.layout_node_requests += 1;
        self.layout_work.child_edges += children.len() as u64;

        self.reconcile.request_layout(taffy_style, children)
    }

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
        self.request_measured_layout_internal(
            style,
            rem_size,
            scale_factor,
            MeasuredLayoutKind::Opaque,
            Some(LayoutMeasureContext {
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
        )
    }

    pub(crate) fn request_pure_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measure: PureSizeMeasure,
    ) -> LayoutId {
        self.request_measured_layout_internal(
            style,
            rem_size,
            scale_factor,
            MeasuredLayoutKind::PureSize(measure),
            None,
        )
    }

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
        self.request_measured_layout_internal(
            style,
            rem_size,
            scale_factor,
            MeasuredLayoutKind::Text(measure_key),
            Some(LayoutMeasureContext {
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
        )
    }

    fn request_measured_layout_internal(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measured_kind: MeasuredLayoutKind,
        measure_context: Option<LayoutMeasureContext>,
    ) -> LayoutId {
        let taffy_style = style.to_taffy(rem_size, scale_factor);
        self.layout_work.measured_layout_node_requests += 1;
        self.reconcile
            .request_measured_layout(taffy_style, measured_kind, measure_context)
    }

    fn invalidate_cached_bounds_for_subtree(&mut self, node_id: NodeId) {
        let stack = &mut self.layout_bounds_scratch_space;
        stack.push(node_id);
        while let Some(node_id) = stack.pop() {
            self.absolute_layout_bounds.remove(&node_id);
            self.absolute_outer_origins.remove(&node_id);
            stack.extend(
                self.taffy
                    .children(node_id)
                    .expect(EXPECT_MESSAGE)
                    .into_iter(),
            );
        }
    }

    #[stacksafe]
    pub fn compute_layout(
        &mut self,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let node_id = self.reconcile.commit_root_layout(&mut self.taffy, id);

        if !self.computed_layouts.insert(node_id) {
            self.invalidate_cached_bounds_for_subtree(node_id);
        }

        let scale_factor = window.scale_factor();

        let transform = |v: AvailableSpace| match v {
            AvailableSpace::Definite(pixels) => {
                AvailableSpace::Definite(Pixels(pixels.0 * scale_factor))
            }
            AvailableSpace::MinContent => AvailableSpace::MinContent,
            AvailableSpace::MaxContent => AvailableSpace::MaxContent,
        };
        let available_space = size(
            transform(available_space.width),
            transform(available_space.height),
        );

        self.layout_work.compute_layout_calls += 1;
        let compute_start = std::time::Instant::now();
        let mut measured_layout_calls = 0;
        let mut measured_layout_duration = Duration::default();

        let compute_measurements = RefCell::new(self.reconcile.begin_compute_measurements());

        self.taffy
            .compute_layout_with_measure_and_cache_events(
                node_id,
                available_space.into(),
                |known_dimensions, available_space, node_id, node_context, _style| {
                    if node_context.is_none() {
                        assert!(
                            !compute_measurements
                                .borrow()
                                .has_current_measurement(node_id),
                            "measured Taffy node should have a stable measurement marker"
                        );
                        return size(0.0_f32, 0.0_f32).into();
                    }

                    let known_dimensions = Size {
                        width: known_dimensions.width.map(|e| Pixels(e / scale_factor)),
                        height: known_dimensions.height.map(|e| Pixels(e / scale_factor)),
                    };

                    let available_space: Size<AvailableSpace> = available_space.into();
                    let untransform = |ev: AvailableSpace| match ev {
                        AvailableSpace::Definite(pixels) => {
                            AvailableSpace::Definite(Pixels(pixels.0 / scale_factor))
                        }
                        AvailableSpace::MinContent => AvailableSpace::MinContent,
                        AvailableSpace::MaxContent => AvailableSpace::MaxContent,
                    };
                    let available_space = size(
                        untransform(available_space.width),
                        untransform(available_space.height),
                    );

                    measured_layout_calls += 1;
                    let measure_start = std::time::Instant::now();
                    let measured_size = compute_measurements.borrow_mut().measure(
                        node_id,
                        known_dimensions,
                        available_space,
                        window,
                        cx,
                    );
                    measured_layout_duration += measure_start.elapsed();
                    snap_measured_size_to_device_pixels(measured_size, scale_factor).into()
                },
                |event| {
                    compute_measurements.borrow_mut().handle_cache_event(event);
                },
            )
            .expect(EXPECT_MESSAGE);
        compute_measurements.into_inner().finish_compute();

        self.layout_work.compute_layout_duration += compute_start.elapsed();
        self.layout_work.measured_layout_calls += measured_layout_calls;
        self.layout_work.measured_layout_duration += measured_layout_duration;
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

    pub fn layout_bounds(&mut self, id: LayoutId, scale_factor: f32) -> Bounds<Pixels> {
        let node_id = self.reconcile.committed_node(id);
        self.layout_bounds_for_node(node_id, scale_factor)
    }

    #[cfg(test)]
    fn commit_layout(&mut self, id: LayoutId) -> NodeId {
        self.reconcile.commit_layout(&mut self.taffy, id)
    }

    #[cfg(test)]
    fn commit_root_layout(&mut self, id: LayoutId) -> NodeId {
        self.reconcile.commit_root_layout(&mut self.taffy, id)
    }

    #[cfg(test)]
    fn committed_node_for_tests(&self, id: LayoutId) -> NodeId {
        self.reconcile.committed_node(id)
    }

    #[cfg(test)]
    fn assert_descriptor_committed_exactly_for_tests(&self, id: LayoutId) {
        self.reconcile
            .assert_descriptor_committed_exactly_for_tests(&self.taffy, id);
    }

    fn layout_bounds_for_node(&mut self, node_id: NodeId, scale_factor: f32) -> Bounds<Pixels> {
        if let Some(layout) = self.absolute_layout_bounds.get(&node_id).cloned() {
            return layout;
        }

        let layout = self.taffy.layout(node_id).expect(EXPECT_MESSAGE);
        let layout_location = layout.location;
        let layout_size = layout.size;
        let parent = self.taffy.parent(node_id);

        let absolute_outer_origin = match parent {
            Some(parent_id) => {
                self.layout_bounds_for_node(parent_id, scale_factor);
                let parent_origin = *self
                    .absolute_outer_origins
                    .get(&parent_id)
                    .expect("parent absolute outer origin should be cached");
                parent_origin + Point::from(layout_location)
            }
            None => Point::from(layout_location),
        };
        self.absolute_outer_origins
            .insert(node_id, absolute_outer_origin);

        let absolute_far = absolute_outer_origin + Point::from(Size::from(layout_size));
        let snapped_bounds = Bounds::from_corners(
            absolute_outer_origin.map(round_half_toward_zero),
            absolute_far.map(round_half_toward_zero),
        );

        let bounds = (snapped_bounds / scale_factor).map(Pixels);
        self.absolute_layout_bounds.insert(node_id, bounds);
        bounds
    }
}

/// A unique identifier for a layout descriptor generated during the current frame.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct LayoutId(usize);

impl std::hash::Hash for LayoutId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[cfg(test)]
mod retained_layout_tests {
    use super::*;
    use crate::{
        AbsoluteLength, DefiniteLength, Display, FlexDirection, IntoElement, Length,
        ParentElement as _, SharedString, Styled as _, TestAppContext, TextStyle, div, point, px,
    };
    use std::{cell::Cell, rc::Rc};

    #[derive(Debug, PartialEq)]
    struct TaffyShape {
        style: taffy::style::Style,
        has_measure_context: bool,
        children: Vec<TaffyShape>,
    }

    fn style_with_width(width: f32) -> Style {
        let mut style = Style::default();
        style.size.width =
            Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(width))));
        style
    }

    fn request_leaf(engine: &mut TaffyLayoutEngine, width: f32) -> LayoutId {
        engine.request_layout(style_with_width(width), px(16.0), 1.0, &[])
    }

    fn request_container(engine: &mut TaffyLayoutEngine, children: &[LayoutId]) -> LayoutId {
        engine.request_layout(Style::default(), px(16.0), 1.0, children)
    }

    fn request_flex_container(engine: &mut TaffyLayoutEngine, children: &[LayoutId]) -> LayoutId {
        let mut style = Style::default();
        style.display = Display::Flex;
        style.flex_direction = FlexDirection::Row;
        engine.request_layout(style, px(16.0), 1.0, children)
    }

    fn request_full_container(engine: &mut TaffyLayoutEngine, children: &[LayoutId]) -> LayoutId {
        let mut style = Style::default();
        style.size = Size::full();
        engine.request_layout(style, px(16.0), 1.0, children)
    }

    fn request_full_leaf(engine: &mut TaffyLayoutEngine) -> LayoutId {
        request_full_container(engine, &[])
    }

    fn request_measured(engine: &mut TaffyLayoutEngine, width: f32) -> LayoutId {
        engine.request_measured_layout(style_with_width(width), px(16.0), 1.0, move |_, _, _, _| {
            size(px(width), px(10.0))
        })
    }

    fn request_auto_measured(engine: &mut TaffyLayoutEngine, width: f32) -> LayoutId {
        engine.request_measured_layout(Style::default(), px(16.0), 1.0, move |_, _, _, _| {
            size(px(width), px(10.0))
        })
    }

    struct DropCounter(Rc<Cell<usize>>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    fn request_counted_measured(
        engine: &mut TaffyLayoutEngine,
        drops: Rc<Cell<usize>>,
    ) -> LayoutId {
        let counter = DropCounter(drops);
        engine.request_measured_layout(Style::default(), px(16.0), 1.0, move |_, _, _, _| {
            let _keep_counter_alive = &counter;
            size(px(10.0), px(10.0))
        })
    }

    fn request_pure_list_measured(
        engine: &mut TaffyLayoutEngine,
        width: f32,
        height: f32,
    ) -> LayoutId {
        engine.request_pure_measured_layout(
            Style::default(),
            px(16.0),
            1.0,
            PureSizeMeasure::list(px(width), px(height), 1.0),
        )
    }

    fn text_measure_key(text: &'static str) -> TextMeasureKey {
        let text_style = TextStyle::default();
        let text = SharedString::new_static(text);
        TextMeasureKey::new(
            text.clone(),
            vec![text_style.to_run(text.len())],
            &text_style,
            px(16.0),
            px(20.0),
            1.0,
            0,
        )
    }

    fn request_text_measured(
        engine: &mut TaffyLayoutEngine,
        key: TextMeasureKey,
        size: Size<Pixels>,
        measure_invocations: Rc<Cell<usize>>,
        hydrations: Rc<Cell<usize>>,
    ) -> LayoutId {
        request_text_measured_with_hydration_log(
            engine,
            key,
            size,
            measure_invocations,
            hydrations,
            None,
        )
    }

    #[derive(Clone, Debug, PartialEq)]
    struct HydratedTextArtifact {
        key: TextMeasureKey,
        size: Size<Pixels>,
    }

    fn request_text_measured_with_hydration_log(
        engine: &mut TaffyLayoutEngine,
        key: TextMeasureKey,
        size: Size<Pixels>,
        measure_invocations: Rc<Cell<usize>>,
        hydrations: Rc<Cell<usize>>,
        hydrated_artifacts: Option<Rc<RefCell<Vec<HydratedTextArtifact>>>>,
    ) -> LayoutId {
        let artifact = TextLayoutArtifact::for_tests(key.clone(), size);
        engine.request_text_measured_layout(
            Style::default(),
            px(16.0),
            1.0,
            key,
            move |artifact| {
                hydrations.set(hydrations.get() + 1);
                if let Some(hydrated_artifacts) = hydrated_artifacts.as_ref() {
                    hydrated_artifacts.borrow_mut().push(HydratedTextArtifact {
                        key: artifact.key().clone(),
                        size: artifact.size(),
                    });
                }
            },
            move |_, _, _, _| {
                measure_invocations.set(measure_invocations.get() + 1);
                artifact.clone()
            },
        )
    }

    fn request_row(engine: &mut TaffyLayoutEngine, widths: &[f32]) -> LayoutId {
        let children = widths
            .iter()
            .map(|width| request_leaf(engine, *width))
            .collect::<Vec<_>>();
        request_container(engine, &children)
    }

    fn request_flex_row(engine: &mut TaffyLayoutEngine, widths: &[f32]) -> LayoutId {
        let children = widths
            .iter()
            .map(|width| request_leaf(engine, *width))
            .collect::<Vec<_>>();
        request_flex_container(engine, &children)
    }

    fn taffy_shape(engine: &TaffyLayoutEngine, node_id: NodeId) -> TaffyShape {
        TaffyShape {
            style: engine.taffy.style(node_id).expect(EXPECT_MESSAGE).clone(),
            has_measure_context: engine.taffy.get_node_context(node_id).is_some(),
            children: engine
                .taffy
                .children(node_id)
                .expect(EXPECT_MESSAGE)
                .into_iter()
                .map(|child| taffy_shape(engine, child))
                .collect(),
        }
    }

    fn compute_layout_without_measure(
        engine: &mut TaffyLayoutEngine,
        root: LayoutId,
        width: f32,
        height: f32,
    ) -> NodeId {
        compute_layout_without_measure_with_available_space(
            engine,
            root,
            AvailableSpace::Definite(px(width)),
            AvailableSpace::Definite(px(height)),
        )
    }

    fn compute_layout_without_measure_with_available_space(
        engine: &mut TaffyLayoutEngine,
        root: LayoutId,
        width: AvailableSpace,
        height: AvailableSpace,
    ) -> NodeId {
        let root_node = engine.commit_root_layout(root);
        if !engine.computed_layouts.insert(root_node) {
            engine.invalidate_cached_bounds_for_subtree(root_node);
        }
        engine
            .taffy
            .compute_layout_with_measure(
                root_node,
                size(width, height).into(),
                |_known_dimensions, _available_space, _id, _node_context, _style| {
                    taffy::geometry::Size::default()
                },
            )
            .expect(EXPECT_MESSAGE);
        root_node
    }

    fn taffy_node_size(engine: &TaffyLayoutEngine, node_id: NodeId) -> Size<f32> {
        engine
            .taffy
            .layout(node_id)
            .expect(EXPECT_MESSAGE)
            .size
            .into()
    }

    fn assert_descriptor_committed_exactly(engine: &TaffyLayoutEngine, id: LayoutId) {
        engine.assert_descriptor_committed_exactly_for_tests(id);
    }

    #[test]
    fn retained_commit_matches_fresh_commit_after_insert_delete_and_reorder() {
        let mut retained = TaffyLayoutEngine::new();
        let retained_first_root = request_row(&mut retained, &[10.0, 20.0, 30.0]);
        retained.commit_layout(retained_first_root);
        retained.finish_frame();

        let retained_second_root = request_row(&mut retained, &[30.0, 10.0, 40.0, 20.0]);
        let retained_root_node = retained.commit_layout(retained_second_root);
        assert_descriptor_committed_exactly(&retained, retained_second_root);

        let mut fresh = TaffyLayoutEngine::new();
        let fresh_root = request_row(&mut fresh, &[30.0, 10.0, 40.0, 20.0]);
        let fresh_root_node = fresh.commit_layout(fresh_root);

        assert_eq!(
            taffy_shape(&retained, retained_root_node),
            taffy_shape(&fresh, fresh_root_node)
        );
    }

    #[test]
    fn unchanged_unmeasured_tree_emits_no_taffy_mutations_on_second_frame() {
        let mut engine = TaffyLayoutEngine::new();
        let root = request_row(&mut engine, &[10.0, 20.0, 30.0]);
        engine.commit_layout(root);
        engine.finish_frame();

        engine.reset_taffy_mutation_counts_for_tests();
        let root = request_row(&mut engine, &[10.0, 20.0, 30.0]);
        engine.commit_layout(root);
        engine.finish_frame();

        assert_eq!(
            engine.taffy_mutation_counts_for_tests(),
            TaffyMutationCountsForTests::default()
        );
    }

    #[test]
    fn retained_commit_matches_fresh_commit_for_frame_sequence_gallery() {
        let frames: &[&[f32]] = &[
            &[],
            &[10.0],
            &[10.0, 20.0],
            &[20.0, 10.0],
            &[10.0],
            &[30.0, 10.0, 20.0],
            &[],
            &[40.0, 10.0],
        ];
        let mut retained = TaffyLayoutEngine::new();

        for widths in frames {
            let retained_root = request_row(&mut retained, widths);
            let retained_root_node = retained.commit_layout(retained_root);
            assert_descriptor_committed_exactly(&retained, retained_root);

            let mut fresh = TaffyLayoutEngine::new();
            let fresh_root = request_row(&mut fresh, widths);
            let fresh_root_node = fresh.commit_layout(fresh_root);
            assert_descriptor_committed_exactly(&fresh, fresh_root);

            assert_eq!(
                taffy_shape(&retained, retained_root_node),
                taffy_shape(&fresh, fresh_root_node)
            );

            retained.finish_frame();
        }
    }

    #[test]
    fn retained_layout_recomputes_after_child_delete_from_content_sized_parent() {
        let mut retained = TaffyLayoutEngine::new();
        let root = request_flex_row(&mut retained, &[100.0, 200.0]);
        compute_layout_without_measure_with_available_space(
            &mut retained,
            root,
            AvailableSpace::MaxContent,
            AvailableSpace::Definite(px(100.0)),
        );
        retained.finish_frame();

        let root = request_flex_row(&mut retained, &[100.0]);
        let retained_root = compute_layout_without_measure_with_available_space(
            &mut retained,
            root,
            AvailableSpace::MaxContent,
            AvailableSpace::Definite(px(100.0)),
        );

        let mut fresh = TaffyLayoutEngine::new();
        let fresh_root = request_flex_row(&mut fresh, &[100.0]);
        let fresh_root = compute_layout_without_measure_with_available_space(
            &mut fresh,
            fresh_root,
            AvailableSpace::MaxContent,
            AvailableSpace::Definite(px(100.0)),
        );

        assert_eq!(
            (
                taffy_node_size(&retained, retained_root),
                taffy_node_size(&fresh, fresh_root),
            ),
            (size(100.0, 0.0), size(100.0, 0.0))
        );
    }

    #[test]
    fn moved_child_has_exactly_the_current_descriptor_parent() {
        let mut engine = TaffyLayoutEngine::new();
        let child = request_leaf(&mut engine, 10.0);
        let left = request_container(&mut engine, &[child]);
        let right = request_container(&mut engine, &[]);
        let root = request_container(&mut engine, &[left, right]);
        engine.commit_layout(root);
        engine.finish_frame();

        let left = request_container(&mut engine, &[]);
        let child = request_leaf(&mut engine, 10.0);
        let right = request_container(&mut engine, &[child]);
        let root = request_container(&mut engine, &[left, right]);
        engine.commit_layout(root);
        assert_descriptor_committed_exactly(&engine, root);

        let left_node = engine.committed_node_for_tests(left);
        let right_node = engine.committed_node_for_tests(right);
        let child_node = engine.committed_node_for_tests(child);

        assert_eq!(
            engine.taffy.children(left_node).expect(EXPECT_MESSAGE),
            Vec::<NodeId>::new()
        );
        assert_eq!(
            engine.taffy.children(right_node).expect(EXPECT_MESSAGE),
            vec![child_node]
        );
        assert_eq!(engine.taffy.parent(child_node), Some(right_node));
    }

    #[test]
    fn node_kind_changes_do_not_reuse_stale_measured_context() {
        let mut engine = TaffyLayoutEngine::new();
        let measured = request_measured(&mut engine, 10.0);
        engine.commit_layout(measured);
        engine.finish_frame();

        let leaf = request_leaf(&mut engine, 10.0);
        engine.commit_layout(leaf);
        assert_descriptor_committed_exactly(&engine, leaf);

        let leaf_node = engine.committed_node_for_tests(leaf);
        assert!(engine.taffy.get_node_context(leaf_node).is_none());
        assert_eq!(
            engine.taffy.children(leaf_node).expect(EXPECT_MESSAGE),
            Vec::<NodeId>::new()
        );
    }

    #[test]
    fn measured_producer_drops_at_frame_finish_and_marker_drops_on_removal() {
        let mut engine = TaffyLayoutEngine::new();
        let drops = Rc::new(Cell::new(0));

        let measured = request_counted_measured(&mut engine, drops.clone());
        engine.commit_layout(measured);
        let measured_node = engine.committed_node_for_tests(measured);
        assert!(engine.taffy.get_node_context(measured_node).is_some());
        assert_eq!(drops.get(), 0);

        engine.finish_frame();
        assert_eq!(drops.get(), 1);
        assert!(engine.taffy.get_node_context(measured_node).is_some());

        let leaf = request_leaf(&mut engine, 10.0);
        engine.commit_layout(leaf);
        assert_descriptor_committed_exactly(&engine, leaf);
        assert_eq!(drops.get(), 1);
        let leaf_node = engine.committed_node_for_tests(leaf);
        assert!(engine.taffy.get_node_context(leaf_node).is_none());
    }

    #[gpui::test]
    fn retained_measured_node_recomputes_when_measure_result_changes(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let scale_factor = cx.update(|window, _| window.scale_factor());

        let mut retained = TaffyLayoutEngine::new();
        let root = request_auto_measured(&mut retained, 0.0);
        cx.update(|window, app| {
            retained.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });
        retained.finish_frame();

        let root = request_auto_measured(&mut retained, 100.0);
        let retained_root = cx.update(|window, app| {
            retained.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
            retained.committed_node_for_tests(root)
        });

        let mut fresh = TaffyLayoutEngine::new();
        let fresh_root = request_auto_measured(&mut fresh, 100.0);
        let fresh_root = cx.update(|window, app| {
            fresh.compute_layout(
                fresh_root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
            fresh.committed_node_for_tests(fresh_root)
        });

        let expected_size = size(100.0 * scale_factor, 10.0 * scale_factor);
        assert_eq!(
            (
                taffy_node_size(&retained, retained_root),
                taffy_node_size(&fresh, fresh_root),
            ),
            (expected_size, expected_size)
        );
    }

    #[gpui::test]
    fn unchanged_pure_size_measure_reuses_taffy_cache(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let mut engine = TaffyLayoutEngine::new();

        let root = request_pure_list_measured(&mut engine, 10.0, 20.0);
        cx.update(|window, app| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });
        engine.finish_frame();

        let root = request_pure_list_measured(&mut engine, 10.0, 20.0);
        cx.update(|window, app| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });

        assert_eq!(
            engine.layout_work_sample(),
            LayoutWorkSample {
                draw_index: 0,
                layout_node_requests: 0,
                measured_layout_node_requests: 1,
                child_edges: 0,
                compute_layout_calls: 1,
                measured_layout_calls: 0,
                compute_layout_duration: engine.layout_work_sample().compute_layout_duration,
                measured_layout_duration: Duration::default(),
            }
        );
    }

    #[gpui::test]
    fn unchanged_text_measure_hydrates_from_retained_artifact(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let key = text_measure_key("hello");
        let measure_invocations = Rc::new(Cell::new(0));
        let hydrations = Rc::new(Cell::new(0));
        let mut engine = TaffyLayoutEngine::new();

        let root = request_text_measured(
            &mut engine,
            key.clone(),
            size(px(40.0), px(20.0)),
            measure_invocations.clone(),
            hydrations.clone(),
        );
        cx.update(|window, app| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });
        engine.finish_frame();

        let root = request_text_measured(
            &mut engine,
            key,
            size(px(40.0), px(20.0)),
            measure_invocations.clone(),
            hydrations.clone(),
        );
        cx.update(|window, app| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });

        assert_eq!((measure_invocations.get(), hydrations.get()), (1, 2));
        assert_eq!(engine.layout_work_sample().measured_layout_calls, 0);
    }

    #[gpui::test]
    fn changed_text_measure_key_remeasures(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let first_key = text_measure_key("hello");
        let second_key = text_measure_key("world");
        let measure_invocations = Rc::new(Cell::new(0));
        let hydrations = Rc::new(Cell::new(0));
        let hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
        let mut engine = TaffyLayoutEngine::new();

        let root = request_text_measured_with_hydration_log(
            &mut engine,
            first_key.clone(),
            size(px(40.0), px(20.0)),
            measure_invocations.clone(),
            hydrations.clone(),
            Some(hydrated_artifacts.clone()),
        );
        cx.update(|window, app| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });
        engine.finish_frame();

        let root = request_text_measured_with_hydration_log(
            &mut engine,
            second_key.clone(),
            size(px(60.0), px(20.0)),
            measure_invocations.clone(),
            hydrations.clone(),
            Some(hydrated_artifacts.clone()),
        );
        cx.update(|window, app| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });

        assert_eq!((measure_invocations.get(), hydrations.get()), (2, 2));
        assert_eq!(engine.layout_work_sample().measured_layout_calls, 1);
        assert_eq!(
            hydrated_artifacts.borrow().as_slice(),
            [
                HydratedTextArtifact {
                    key: first_key,
                    size: size(px(40.0), px(20.0)),
                },
                HydratedTextArtifact {
                    key: second_key,
                    size: size(px(60.0), px(20.0)),
                },
            ]
        );
    }

    #[gpui::test]
    fn hidden_text_does_not_require_a_measurement_artifact(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(100.0), px(100.0)),
            |_, _| div().hidden().child("hello").into_any_element(),
        );
    }

    #[test]
    #[should_panic(expected = "layout descriptor should appear only once")]
    fn duplicate_descriptor_references_fail_loudly() {
        let mut engine = TaffyLayoutEngine::new();
        let child = request_leaf(&mut engine, 10.0);
        let root = request_container(&mut engine, &[child, child]);

        engine.commit_layout(root);
    }

    #[test]
    fn retained_layout_recomputes_when_root_available_space_changes() {
        let mut retained = TaffyLayoutEngine::new();
        let child = request_full_leaf(&mut retained);
        let root = request_full_container(&mut retained, &[child]);
        compute_layout_without_measure(&mut retained, root, 0.0, 100.0);
        retained.finish_frame();

        let child = request_full_leaf(&mut retained);
        let root = request_full_container(&mut retained, &[child]);
        let retained_root = compute_layout_without_measure(&mut retained, root, 800.0, 100.0);
        let retained_child = retained.committed_node_for_tests(child);

        let mut fresh = TaffyLayoutEngine::new();
        let child = request_full_leaf(&mut fresh);
        let root = request_full_container(&mut fresh, &[child]);
        let fresh_root = compute_layout_without_measure(&mut fresh, root, 800.0, 100.0);
        let fresh_child = fresh.committed_node_for_tests(child);

        assert_eq!(
            taffy_node_size(&retained, retained_root),
            taffy_node_size(&fresh, fresh_root)
        );
        assert_eq!(
            taffy_node_size(&retained, retained_child),
            taffy_node_size(&fresh, fresh_child)
        );
        assert_eq!(
            taffy_node_size(&retained, retained_child),
            size(800.0, 100.0)
        );
    }

    #[test]
    fn repeated_root_layout_recompute_is_allowed() {
        let mut engine = TaffyLayoutEngine::new();
        let child = request_full_leaf(&mut engine);
        let root = request_full_container(&mut engine, &[child]);

        compute_layout_without_measure(&mut engine, root, 0.0, 100.0);
        let child_node = engine.committed_node_for_tests(child);
        assert_eq!(
            engine.layout_bounds_for_node(child_node, 1.0).size,
            size(px(0.0), px(100.0))
        );

        let root_node = compute_layout_without_measure(&mut engine, root, 800.0, 100.0);

        assert_eq!(taffy_node_size(&engine, root_node), size(800.0, 100.0));
        assert_eq!(taffy_node_size(&engine, child_node), size(800.0, 100.0));
        assert_eq!(
            engine.layout_bounds_for_node(child_node, 1.0).size,
            size(px(800.0), px(100.0))
        );
    }

    #[test]
    #[should_panic(expected = "layout root must not already be committed under a parent")]
    fn descriptor_committed_under_parent_cannot_be_computed_as_root() {
        let mut engine = TaffyLayoutEngine::new();
        let child = request_full_leaf(&mut engine);
        let root = request_full_container(&mut engine, &[child]);

        compute_layout_without_measure(&mut engine, root, 800.0, 100.0);
        engine.commit_root_layout(child);
    }
}

fn snap_measured_size_to_device_pixels(size: Size<Pixels>, scale_factor: f32) -> Size<f32> {
    size.map(|d| ceil_to_device_pixel(d.0.max(0.0), scale_factor))
}

fn border_widths_to_taffy(
    widths: &Edges<AbsoluteLength>,
    rem_size: Pixels,
    scale_factor: f32,
) -> TaffyRect<taffy::style::LengthPercentage> {
    let snap = |w: &AbsoluteLength| {
        taffy::style::LengthPercentage::length(round_stroke_to_device_pixel(
            w.to_pixels(rem_size).0,
            scale_factor,
        ))
    };
    TaffyRect {
        top: snap(&widths.top),
        right: snap(&widths.right),
        bottom: snap(&widths.bottom),
        left: snap(&widths.left),
    }
}

trait ToTaffy<Output> {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> Output;
}

impl ToTaffy<taffy::style::Style> for Style {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Style {
        use taffy::style_helpers::{fr, length, minmax, repeat};

        fn to_grid_line(
            placement: &Range<crate::GridPlacement>,
        ) -> taffy::Line<taffy::GridPlacement> {
            taffy::Line {
                start: placement.start.into(),
                end: placement.end.into(),
            }
        }

        fn to_grid_repeat<T: taffy::style::CheapCloneStr>(
            unit: &Option<GridTemplate>,
        ) -> Vec<taffy::GridTemplateComponent<T>> {
            unit.map(|template| {
                match template.min_size {
                    // grid-template-*: repeat(<number>, minmax(0, 1fr));
                    crate::TemplateColumnMinSize::Zero => {
                        vec![repeat(
                            template.repeat,
                            vec![minmax(length(0.0_f32), fr(1.0_f32))],
                        )]
                    }
                    // grid-template-*: repeat(<number>, minmax(min-content, 1fr));
                    crate::TemplateColumnMinSize::MinContent => {
                        vec![repeat(
                            template.repeat,
                            vec![minmax(min_content(), fr(1.0_f32))],
                        )]
                    }
                    // grid-template-*: repeat(<number>, minmax(0, max-content))
                    crate::TemplateColumnMinSize::MaxContent => {
                        vec![repeat(
                            template.repeat,
                            vec![minmax(length(0.0_f32), max_content())],
                        )]
                    }
                }
            })
            .unwrap_or_default()
        }

        taffy::style::Style {
            display: self.display.into(),
            overflow: self.overflow.into(),
            scrollbar_width: self.scrollbar_width.to_taffy(rem_size, scale_factor),
            position: self.position.into(),
            inset: self.inset.to_taffy(rem_size, scale_factor),
            size: self.size.to_taffy(rem_size, scale_factor),
            min_size: self.min_size.to_taffy(rem_size, scale_factor),
            max_size: self.max_size.to_taffy(rem_size, scale_factor),
            aspect_ratio: self.aspect_ratio,
            margin: self.margin.to_taffy(rem_size, scale_factor),
            padding: self.padding.to_taffy(rem_size, scale_factor),
            border: border_widths_to_taffy(&self.border_widths, rem_size, scale_factor),
            align_items: self.align_items.map(|x| x.into()),
            align_self: self.align_self.map(|x| x.into()),
            align_content: self.align_content.map(|x| x.into()),
            justify_content: self.justify_content.map(|x| x.into()),
            gap: self.gap.to_taffy(rem_size, scale_factor),
            flex_direction: self.flex_direction.into(),
            flex_wrap: self.flex_wrap.into(),
            flex_basis: self.flex_basis.to_taffy(rem_size, scale_factor),
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink,
            grid_template_rows: to_grid_repeat(&self.grid_rows),
            grid_template_columns: to_grid_repeat(&self.grid_cols),
            grid_row: self
                .grid_location
                .as_ref()
                .map(|location| to_grid_line(&location.row))
                .unwrap_or_default(),
            grid_column: self
                .grid_location
                .as_ref()
                .map(|location| to_grid_line(&location.column))
                .unwrap_or_default(),
            ..Default::default()
        }
    }
}

impl ToTaffy<f32> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> f32 {
        round_to_device_pixel(self.to_pixels(rem_size).0, scale_factor)
    }
}

impl ToTaffy<taffy::style::LengthPercentageAuto> for Length {
    fn to_taffy(
        &self,
        rem_size: Pixels,
        scale_factor: f32,
    ) -> taffy::prelude::LengthPercentageAuto {
        match self {
            Length::Definite(length) => length.to_taffy(rem_size, scale_factor),
            Length::Auto => taffy::prelude::LengthPercentageAuto::auto(),
        }
    }
}

impl ToTaffy<taffy::style::Dimension> for Length {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::prelude::Dimension {
        match self {
            Length::Definite(length) => length.to_taffy(rem_size, scale_factor),
            Length::Auto => taffy::prelude::Dimension::auto(),
        }
    }
}

impl ToTaffy<taffy::style::LengthPercentage> for DefiniteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentage {
        match self {
            DefiniteLength::Absolute(length) => length.to_taffy(rem_size, scale_factor),
            DefiniteLength::Fraction(fraction) => {
                taffy::style::LengthPercentage::percent(*fraction)
            }
        }
    }
}

impl ToTaffy<taffy::style::LengthPercentageAuto> for DefiniteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentageAuto {
        match self {
            DefiniteLength::Absolute(length) => length.to_taffy(rem_size, scale_factor),
            DefiniteLength::Fraction(fraction) => {
                taffy::style::LengthPercentageAuto::percent(*fraction)
            }
        }
    }
}

impl ToTaffy<taffy::style::Dimension> for DefiniteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Dimension {
        match self {
            DefiniteLength::Absolute(length) => length.to_taffy(rem_size, scale_factor),
            DefiniteLength::Fraction(fraction) => taffy::style::Dimension::percent(*fraction),
        }
    }
}

impl ToTaffy<taffy::style::LengthPercentage> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentage {
        taffy::style::LengthPercentage::length(self.to_taffy(rem_size, scale_factor))
    }
}

impl ToTaffy<taffy::style::LengthPercentageAuto> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentageAuto {
        taffy::style::LengthPercentageAuto::length(self.to_taffy(rem_size, scale_factor))
    }
}

impl ToTaffy<taffy::style::Dimension> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Dimension {
        taffy::style::Dimension::length(self.to_taffy(rem_size, scale_factor))
    }
}

impl<T, T2> From<TaffyPoint<T>> for Point<T2>
where
    T: Into<T2>,
    T2: Clone + Debug + Default + PartialEq,
{
    fn from(point: TaffyPoint<T>) -> Point<T2> {
        Point {
            x: point.x.into(),
            y: point.y.into(),
        }
    }
}

impl<T, T2> From<Point<T>> for TaffyPoint<T2>
where
    T: Into<T2> + Clone + Debug + Default + PartialEq,
{
    fn from(val: Point<T>) -> Self {
        TaffyPoint {
            x: val.x.into(),
            y: val.y.into(),
        }
    }
}

impl<T, U> ToTaffy<TaffySize<U>> for Size<T>
where
    T: ToTaffy<U> + Clone + Debug + Default + PartialEq,
{
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> TaffySize<U> {
        TaffySize {
            width: self.width.to_taffy(rem_size, scale_factor),
            height: self.height.to_taffy(rem_size, scale_factor),
        }
    }
}

impl<T, U> ToTaffy<TaffyRect<U>> for Edges<T>
where
    T: ToTaffy<U> + Clone + Debug + Default + PartialEq,
{
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> TaffyRect<U> {
        TaffyRect {
            top: self.top.to_taffy(rem_size, scale_factor),
            right: self.right.to_taffy(rem_size, scale_factor),
            bottom: self.bottom.to_taffy(rem_size, scale_factor),
            left: self.left.to_taffy(rem_size, scale_factor),
        }
    }
}

impl<T, U> From<TaffySize<T>> for Size<U>
where
    T: Into<U>,
    U: Clone + Debug + Default + PartialEq,
{
    fn from(taffy_size: TaffySize<T>) -> Self {
        Size {
            width: taffy_size.width.into(),
            height: taffy_size.height.into(),
        }
    }
}

impl<T, U> From<Size<T>> for TaffySize<U>
where
    T: Into<U> + Clone + Debug + Default + PartialEq,
{
    fn from(size: Size<T>) -> Self {
        TaffySize {
            width: size.width.into(),
            height: size.height.into(),
        }
    }
}

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

impl From<AvailableSpace> for TaffyAvailableSpace {
    fn from(space: AvailableSpace) -> TaffyAvailableSpace {
        match space {
            AvailableSpace::Definite(Pixels(value)) => TaffyAvailableSpace::Definite(value),
            AvailableSpace::MinContent => TaffyAvailableSpace::MinContent,
            AvailableSpace::MaxContent => TaffyAvailableSpace::MaxContent,
        }
    }
}

impl From<TaffyAvailableSpace> for AvailableSpace {
    fn from(space: TaffyAvailableSpace) -> AvailableSpace {
        match space {
            TaffyAvailableSpace::Definite(value) => AvailableSpace::Definite(Pixels(value)),
            TaffyAvailableSpace::MinContent => AvailableSpace::MinContent,
            TaffyAvailableSpace::MaxContent => AvailableSpace::MaxContent,
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
mod tests {
    use super::*;
    use crate::AppContext as _;
    use std::{cell::Cell, ops::Deref as _, rc::Rc, time::Duration};

    #[test]
    fn border_widths_to_taffy_use_stroke_snapping() {
        let border_widths = Edges {
            top: Pixels(0.0).into(),
            right: Pixels(0.4).into(),
            bottom: Pixels(0.5).into(),
            left: Pixels(1.6).into(),
        };
        let taffy_border = border_widths_to_taffy(&border_widths, Pixels(16.0), 1.0);

        assert_eq!(
            taffy_border.top,
            taffy::style::LengthPercentage::length(0.0)
        );
        assert_eq!(
            taffy_border.right,
            taffy::style::LengthPercentage::length(1.0)
        );
        assert_eq!(
            taffy_border.bottom,
            taffy::style::LengthPercentage::length(1.0)
        );
        assert_eq!(
            taffy_border.left,
            taffy::style::LengthPercentage::length(2.0)
        );
    }

    #[test]
    fn layout_work_sample_counts_requests_and_finish_frame_resets() {
        let mut engine = TaffyLayoutEngine::new();
        let first_child = engine.request_layout(Style::default(), Pixels(16.0), 1.0, &[]);
        let second_child = engine.request_layout(Style::default(), Pixels(16.0), 1.0, &[]);
        engine.request_layout(
            Style::default(),
            Pixels(16.0),
            1.0,
            &[first_child, second_child],
        );

        let expected_sample = LayoutWorkSample {
            draw_index: 0,
            layout_node_requests: 3,
            measured_layout_node_requests: 0,
            child_edges: 2,
            compute_layout_calls: 0,
            measured_layout_calls: 0,
            compute_layout_duration: Duration::default(),
            measured_layout_duration: Duration::default(),
        };
        assert_eq!(engine.layout_work_sample(), expected_sample);
        assert_eq!(engine.finish_frame(), expected_sample);
        assert_eq!(engine.layout_work_sample(), LayoutWorkSample::default());
    }

    #[test]
    fn layout_work_sample_counts_compute_and_measure() {
        let measure_invocations = Rc::new(Cell::new(0));
        let measure_invocations_for_closure = measure_invocations.clone();
        let mut test_app = crate::TestAppContext::single();
        let window = test_app.add_window(|_, _| crate::Empty);

        let sample = test_app
            .update_window(*window.deref(), |_, window, cx| {
                let mut engine = TaffyLayoutEngine::new();
                let measured_layout = engine.request_measured_layout(
                    Style::default(),
                    Pixels(16.0),
                    1.0,
                    move |_, _, _, _| {
                        measure_invocations_for_closure
                            .set(measure_invocations_for_closure.get() + 1);
                        std::thread::sleep(Duration::from_micros(1));
                        size(Pixels(10.0), Pixels(20.0))
                    },
                );

                engine.compute_layout(
                    measured_layout,
                    size(
                        AvailableSpace::Definite(Pixels(100.0)),
                        AvailableSpace::Definite(Pixels(100.0)),
                    ),
                    window,
                    cx,
                );
                engine.finish_frame()
            })
            .unwrap();

        assert_eq!(
            sample,
            LayoutWorkSample {
                draw_index: 0,
                layout_node_requests: 0,
                measured_layout_node_requests: 1,
                child_edges: 0,
                compute_layout_calls: 1,
                measured_layout_calls: 1,
                compute_layout_duration: sample.compute_layout_duration,
                measured_layout_duration: sample.measured_layout_duration,
            }
        );
        assert_eq!(measure_invocations.get(), 1);
        assert!(sample.compute_layout_duration >= sample.measured_layout_duration);
        assert!(sample.measured_layout_duration > Duration::default());
    }
}
