use super::*;
use crate::{
    AbsoluteLength, DefiniteLength, Display, Drawable, Edges, Element, ElementId, FlexDirection,
    FlexWrap, GlobalElementId, GridPlacement, GridTemplate, InspectorElementId, IntoElement,
    LayoutRequestCx, Length, Overflow, PaintCx, ParentElement as _, Position, PrepaintCx,
    SharedString, Styled as _, TemplateColumnMinSize, TestAppContext, TextOverflow, TextStyle,
    VisualTestContext, WhiteSpace, div, point, px, size,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

fn style_with_width(width: f32) -> Style {
    let mut style = Style::default();
    style.size.width = length_px(width);
    style
}

fn absolute_px(value: f32) -> AbsoluteLength {
    AbsoluteLength::Pixels(px(value))
}

fn definite_px(value: f32) -> DefiniteLength {
    DefiniteLength::Absolute(absolute_px(value))
}

fn length_px(value: f32) -> Length {
    Length::Definite(definite_px(value))
}

fn length_fraction(percent: u16) -> Length {
    Length::Definite(DefiniteLength::Fraction(percent as f32 / 100.0))
}

fn request_leaf(engine: &mut LayoutEngine, width: f32) -> LayoutId {
    engine.request_layout(style_with_width(width), px(16.0), 1.0, &[])
}

fn test_global_id(id: u64) -> GlobalElementId {
    GlobalElementId(Arc::from([ElementId::Integer(id.into())]))
}

fn request_keyed_leaf(engine: &mut LayoutEngine, key: u64, width: f32) -> LayoutId {
    let global_id = test_global_id(key);
    engine.request_layout_with_global_id(
        Some(&global_id),
        style_with_width(width),
        px(16.0),
        1.0,
        &[],
    )
}

fn request_container(engine: &mut LayoutEngine, children: &[LayoutId]) -> LayoutId {
    engine.request_layout(Style::default(), px(16.0), 1.0, children)
}

fn request_flex_container(engine: &mut LayoutEngine, children: &[LayoutId]) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Flex;
    style.flex_direction = FlexDirection::Row;
    engine.request_layout(style, px(16.0), 1.0, children)
}

fn request_grid_container(engine: &mut LayoutEngine, children: &[LayoutId]) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Grid;
    style.grid_cols = Some(GridTemplate {
        repeat: 3,
        min_size: TemplateColumnMinSize::Zero,
    });
    style.grid_rows = Some(GridTemplate {
        repeat: 2,
        min_size: TemplateColumnMinSize::Zero,
    });
    engine.request_layout(style, px(16.0), 1.0, children)
}

fn request_grid_item(engine: &mut LayoutEngine, width: f32, column: i16, row: i16) -> LayoutId {
    let mut style = style_with_width(width);
    let grid_location = style.grid_location.get_or_insert_default();
    grid_location.column = GridPlacement::Line(column)..GridPlacement::Line(column + 1);
    grid_location.row = GridPlacement::Line(row)..GridPlacement::Line(row + 1);
    engine.request_layout(style, px(16.0), 1.0, &[])
}

fn request_full_container(engine: &mut LayoutEngine, children: &[LayoutId]) -> LayoutId {
    let mut style = Style::default();
    style.size = Size::full();
    engine.request_layout(style, px(16.0), 1.0, children)
}

fn request_full_leaf(engine: &mut LayoutEngine) -> LayoutId {
    request_full_container(engine, &[])
}

fn compute_stable_test_root(
    cx: &mut VisualTestContext,
    engine: &mut LayoutEngine,
    root: LayoutId,
    available_space: Size<AvailableSpace>,
) {
    cx.update(|window, app| {
        engine.compute_retained_layout_for_tests(
            root,
            RetainedLayoutRootId::new(0),
            available_space,
            window,
            app,
        );
    });
}

fn request_keyed_layout(
    engine: &mut LayoutEngine,
    key: u64,
    style: Style,
    children: &[LayoutId],
) -> LayoutId {
    let global_id = test_global_id(key);
    engine.request_layout_with_global_id(Some(&global_id), style, px(16.0), 1.0, children)
}

fn request_keyed_full_block_leaf(engine: &mut LayoutEngine, key: u64) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.size = Size::full();
    request_keyed_layout(engine, key, style, &[])
}

fn request_keyed_absolute_leaf(engine: &mut LayoutEngine, key: u64) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.position = Position::Absolute;
    request_keyed_layout(engine, key, style, &[])
}

fn request_keyed_fixed_height_leaf(engine: &mut LayoutEngine, key: u64, height: f32) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.size.height =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(height))));
    request_keyed_layout(engine, key, style, &[])
}

fn request_keyed_flex_column_container(
    engine: &mut LayoutEngine,
    key: u64,
    children: &[LayoutId],
) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Flex;
    style.flex_direction = FlexDirection::Column;
    style.size = Size::full();
    request_keyed_layout(engine, key, style, children)
}

fn request_keyed_growing_block_container(
    engine: &mut LayoutEngine,
    key: u64,
    children: &[LayoutId],
) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.min_size.height =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.0))));
    style.flex_grow = 1.0;
    style.flex_shrink = 1.0;
    request_keyed_layout(engine, key, style, children)
}

fn request_keyed_live_panel_container(
    engine: &mut LayoutEngine,
    key: u64,
    children: &[LayoutId],
) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Flex;
    style.flex_direction = FlexDirection::Column;
    style.overflow = point(Overflow::Hidden, Overflow::Hidden);
    style.border_widths.left = AbsoluteLength::Pixels(px(2.0));
    style.border_widths.top = AbsoluteLength::Pixels(px(2.0));
    style.border_widths.bottom = AbsoluteLength::Pixels(px(2.0));
    style.flex_grow = 1.0;
    style.flex_shrink = 1.0;
    request_keyed_layout(engine, key, style, children)
}

fn request_keyed_full_growing_hidden_block_container(
    engine: &mut LayoutEngine,
    key: u64,
    children: &[LayoutId],
) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.size = Size::full();
    style.flex_grow = 1.0;
    style.flex_shrink = 1.0;
    style.overflow = point(Overflow::Hidden, Overflow::Hidden);
    request_keyed_layout(engine, key, style, children)
}

fn request_keyed_fixed_width_sidebar(engine: &mut LayoutEngine, key: u64, width: f32) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.size.width =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(width))));
    style.size.height = Length::Definite(DefiniteLength::Fraction(1.0));
    request_keyed_layout(engine, key, style, &[])
}

fn request_keyed_fixed_width_sidebar_with_content(
    engine: &mut LayoutEngine,
    key: u64,
    content_key: u64,
    width: f32,
    content_height: f32,
) -> LayoutId {
    let content = request_keyed_fixed_height_leaf(engine, content_key, content_height);
    let mut style = Style::default();
    style.display = Display::Block;
    style.size.width =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(width))));
    style.size.height = Length::Definite(DefiniteLength::Fraction(1.0));
    request_keyed_layout(engine, key, style, &[content])
}

fn request_keyed_flex_row_root(
    engine: &mut LayoutEngine,
    key: u64,
    children: &[LayoutId],
) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Flex;
    style.flex_direction = FlexDirection::Row;
    style.size = Size::full();
    request_keyed_layout(engine, key, style, children)
}

fn request_keyed_growing_flex_row_container(
    engine: &mut LayoutEngine,
    key: u64,
    children: &[LayoutId],
) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Flex;
    style.flex_direction = FlexDirection::Row;
    style.min_size.width =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.0))));
    style.min_size.height =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.0))));
    style.flex_grow = 1.0;
    style.flex_shrink = 1.0;
    request_keyed_layout(engine, key, style, children)
}

fn request_keyed_gapped_growing_flex_row_container(
    engine: &mut LayoutEngine,
    key: u64,
    gap: f32,
    children: &[LayoutId],
) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Flex;
    style.flex_direction = FlexDirection::Row;
    style.min_size.width =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.0))));
    style.min_size.height =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.0))));
    style.gap.width = DefiniteLength::Absolute(AbsoluteLength::Pixels(px(gap)));
    style.gap.height = DefiniteLength::Absolute(AbsoluteLength::Pixels(px(gap)));
    style.flex_grow = 1.0;
    style.flex_shrink = 1.0;
    request_keyed_layout(engine, key, style, children)
}

fn request_keyed_growing_flex_column_container(
    engine: &mut LayoutEngine,
    key: u64,
    children: &[LayoutId],
) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Flex;
    style.flex_direction = FlexDirection::Column;
    style.min_size.width =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.0))));
    style.min_size.height =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.0))));
    style.flex_grow = 1.0;
    style.flex_shrink = 1.0;
    request_keyed_layout(engine, key, style, children)
}

fn request_keyed_padded_growing_flex_row_container(
    engine: &mut LayoutEngine,
    key: u64,
    children: &[LayoutId],
) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Flex;
    style.flex_direction = FlexDirection::Row;
    style.min_size.width =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.0))));
    style.min_size.height =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.0))));
    style.padding.left = DefiniteLength::Absolute(AbsoluteLength::Pixels(px(21.0)));
    style.padding.right = DefiniteLength::Absolute(AbsoluteLength::Pixels(px(21.0)));
    style.padding.top = DefiniteLength::Absolute(AbsoluteLength::Pixels(px(28.0)));
    style.padding.bottom = DefiniteLength::Absolute(AbsoluteLength::Pixels(px(21.0)));
    style.flex_grow = 1.0;
    style.flex_shrink = 1.0;
    request_keyed_layout(engine, key, style, children)
}

fn request_canvas_like_flex_frame(
    engine: &mut LayoutEngine,
    header_height: f32,
) -> (LayoutId, LayoutId, LayoutId, LayoutId) {
    let canvas = request_keyed_full_block_leaf(engine, 10_001);
    let overlay = request_keyed_absolute_leaf(engine, 10_002);
    let canvas_host =
        request_keyed_full_growing_hidden_block_container(engine, 10_003, &[canvas, overlay]);
    let flex_child = request_keyed_growing_block_container(engine, 10_004, &[canvas_host]);
    let header = request_keyed_fixed_height_leaf(engine, 10_005, header_height);
    let panel = request_keyed_live_panel_container(engine, 10_006, &[header, flex_child]);
    let root = request_keyed_flex_column_container(engine, 10_007, &[panel]);
    (root, flex_child, canvas_host, canvas)
}

fn request_canvas_like_workspace_row(
    engine: &mut LayoutEngine,
    header_height: f32,
    sidebar_width: Option<f32>,
) -> (LayoutId, LayoutId, LayoutId, LayoutId, LayoutId) {
    let canvas = request_keyed_full_block_leaf(engine, 11_001);
    let overlay = request_keyed_absolute_leaf(engine, 11_002);
    let canvas_host =
        request_keyed_full_growing_hidden_block_container(engine, 11_003, &[canvas, overlay]);
    let flex_child = request_keyed_growing_block_container(engine, 11_004, &[canvas_host]);
    let header = request_keyed_fixed_height_leaf(engine, 11_005, header_height);
    let panel = request_keyed_live_panel_container(engine, 11_006, &[header, flex_child]);
    let root = match sidebar_width {
        Some(sidebar_width) => {
            let sidebar = request_keyed_fixed_width_sidebar(engine, 11_007, sidebar_width);
            request_keyed_flex_row_root(engine, 11_008, &[panel, sidebar])
        }
        None => request_keyed_flex_row_root(engine, 11_008, &[panel]),
    };
    (root, panel, flex_child, canvas_host, canvas)
}

fn request_canvas_like_chrome_frame(
    engine: &mut LayoutEngine,
    sidebar_width: f32,
) -> (LayoutId, LayoutId, LayoutId, LayoutId, LayoutId) {
    request_canvas_like_chrome_frame_with_sidebar(engine, |engine| {
        request_keyed_fixed_width_sidebar(engine, 12_014, sidebar_width)
    })
}

fn request_canvas_like_chrome_frame_with_sidebar_content(
    engine: &mut LayoutEngine,
    sidebar_width: f32,
    sidebar_content_height: f32,
) -> (LayoutId, LayoutId, LayoutId, LayoutId, LayoutId) {
    request_canvas_like_chrome_frame_with_sidebar(engine, |engine| {
        request_keyed_fixed_width_sidebar_with_content(
            engine,
            12_014,
            12_015,
            sidebar_width,
            sidebar_content_height,
        )
    })
}

fn request_canvas_like_chrome_frame_with_sidebar(
    engine: &mut LayoutEngine,
    request_sidebar: impl FnOnce(&mut LayoutEngine) -> LayoutId,
) -> (LayoutId, LayoutId, LayoutId, LayoutId, LayoutId) {
    request_canvas_like_chrome_frame_with_sidebar_and_gap(engine, request_sidebar, 0.0)
}

fn request_canvas_like_chrome_frame_with_sidebar_and_gap(
    engine: &mut LayoutEngine,
    request_sidebar: impl FnOnce(&mut LayoutEngine) -> LayoutId,
    content_gap: f32,
) -> (LayoutId, LayoutId, LayoutId, LayoutId, LayoutId) {
    let canvas = request_keyed_full_block_leaf(engine, 12_001);
    let overlay = request_keyed_absolute_leaf(engine, 12_002);
    let canvas_host =
        request_keyed_full_growing_hidden_block_container(engine, 12_003, &[canvas, overlay]);
    let flex_child = request_keyed_growing_block_container(engine, 12_004, &[canvas_host]);
    let header = request_keyed_fixed_height_leaf(engine, 12_005, 70.0);
    let panel = request_keyed_live_panel_container(engine, 12_006, &[header, flex_child]);
    let sidebar = request_sidebar(engine);
    let content_row = request_keyed_gapped_growing_flex_row_container(
        engine,
        12_007,
        content_gap,
        &[panel, sidebar],
    );
    let padded_row =
        request_keyed_padded_growing_flex_row_container(engine, 12_008, &[content_row]);
    let footer = request_keyed_fixed_height_leaf(engine, 12_009, 98.0);
    let lower_column =
        request_keyed_growing_flex_column_container(engine, 12_010, &[padded_row, footer]);
    let main_row = request_keyed_growing_flex_row_container(engine, 12_011, &[lower_column]);
    let top_chrome = request_keyed_fixed_height_leaf(engine, 12_012, 156.0);
    let root = request_keyed_flex_column_container(engine, 12_013, &[top_chrome, main_row]);
    (root, panel, flex_child, canvas_host, canvas)
}

#[cfg(not(target_arch = "wasm32"))]
fn request_canvas_like_chrome_frame_from_spec(
    engine: &mut LayoutEngine,
    spec: GeneratedCanvasChromeFrameSpec,
) -> (LayoutId, LayoutId, LayoutId, LayoutId, LayoutId) {
    request_canvas_like_chrome_frame_with_sidebar_and_gap(
        engine,
        |engine| {
            request_keyed_fixed_width_sidebar_with_content(
                engine,
                12_014,
                12_015,
                spec.sidebar_width as f32,
                spec.sidebar_content_height as f32,
            )
        },
        spec.content_gap as f32,
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn compute_canvas_chrome_frame_output(
    engine: &mut LayoutEngine,
    spec: GeneratedCanvasChromeFrameSpec,
) -> CanvasChromeFrameOutputForTests {
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame_from_spec(engine, spec);
    let retained_root = compute_layout_without_measure_with_scale(
        engine,
        root,
        spec.root_width as f32,
        spec.root_height as f32,
        spec.scale_factor,
    );
    assert_facts_committed_exactly(engine, root);

    let panel = engine.retained_node_token_for_tests(panel);
    let flex_child = engine.retained_node_token_for_tests(flex_child);
    let canvas_host = engine.retained_node_token_for_tests(canvas_host);
    let canvas = engine.retained_node_token_for_tests(canvas);
    let canvas_size = retained_node_size(engine, canvas);

    CanvasChromeFrameOutputForTests {
        shape: retained_layout_shape(engine, retained_root),
        projection: retained_layout_projection(engine, retained_root),
        bounds: retained_layout_bounds_tree(engine, retained_root, spec.scale_factor),
        canvas_chain_sizes: (
            retained_node_size(engine, panel),
            retained_node_size(engine, flex_child),
            retained_node_size(engine, canvas_host),
            canvas_size,
        ),
        canvas_is_paintable: (canvas_size.width > 0.0, canvas_size.height > 0.0),
    }
}

fn request_measured(engine: &mut LayoutEngine, width: f32) -> LayoutId {
    engine.request_measured_layout(style_with_width(width), px(16.0), 1.0, move |_, _, _| {
        size(px(width), px(10.0))
    })
}

fn request_auto_measured(engine: &mut LayoutEngine, width: f32) -> LayoutId {
    engine.request_measured_layout(Style::default(), px(16.0), 1.0, move |_, _, _| {
        size(px(width), px(10.0))
    })
}

struct DropCounter(Rc<Cell<usize>>);

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

fn request_counted_measured(engine: &mut LayoutEngine, drops: Rc<Cell<usize>>) -> LayoutId {
    let counter = DropCounter(drops);
    engine.request_measured_layout(Style::default(), px(16.0), 1.0, move |_, _, _| {
        let _keep_counter_alive = &counter;
        size(px(10.0), px(10.0))
    })
}

fn request_pure_list_measured(engine: &mut LayoutEngine, width: f32, height: f32) -> LayoutId {
    engine.request_pure_measured_layout(
        Style::default(),
        px(16.0),
        1.0,
        PureSizeMeasure::list(px(width), px(height), 1.0),
    )
}

fn text_measure_key(text: &'static str) -> TextMeasureKey {
    text_measure_key_with(text, |_| {}, px(16.0), px(20.0), 1.0, 0)
}

fn text_measure_key_with(
    text: &'static str,
    configure_style: impl FnOnce(&mut TextStyle),
    font_size: Pixels,
    line_height: Pixels,
    scale_factor: f32,
    shaping_epoch: u64,
) -> TextMeasureKey {
    let text_style = TextStyle::default();
    let mut text_style = text_style;
    configure_style(&mut text_style);
    let text = SharedString::new_static(text);
    TextMeasureKey::new(
        text.clone(),
        vec![text_style.to_run(text.len())],
        &text_style,
        font_size,
        line_height,
        scale_factor,
        shaping_epoch,
    )
}

fn request_text_measured(
    engine: &mut LayoutEngine,
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

fn request_text_measured_with_style(
    engine: &mut LayoutEngine,
    style: Style,
    key: TextMeasureKey,
    size: Size<Pixels>,
) -> LayoutId {
    engine.request_text_measured_layout(
        style,
        px(16.0),
        1.0,
        key.clone(),
        |_| {},
        move |known_dimensions, available_space, _| {
            TextLayoutArtifact::for_tests(
                key.clone(),
                text_artifact_size_for_query(size, known_dimensions, available_space),
            )
        },
    )
}

fn text_artifact_size_for_query(
    fallback_size: Size<Pixels>,
    known_dimensions: Size<Option<Pixels>>,
    available_space: Size<AvailableSpace>,
) -> Size<Pixels> {
    let width = known_dimensions
        .width
        .unwrap_or(match available_space.width {
            AvailableSpace::Definite(width) => fallback_size.width.min(width),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => fallback_size.width,
        });
    let height = known_dimensions
        .height
        .unwrap_or(match available_space.height {
            AvailableSpace::Definite(height) => fallback_size.height.min(height),
            AvailableSpace::MinContent | AvailableSpace::MaxContent => fallback_size.height,
        });
    size(width, height)
}

#[derive(Clone, Debug, PartialEq)]
struct HydratedTextArtifact {
    key: TextMeasureKey,
    size: Size<Pixels>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq)]
enum GeneratedStyle {
    Default,
    FixedWidth(u16),
    FixedSize {
        width: u16,
        height: u16,
    },
    FlexRow {
        gap: u16,
        wrap: bool,
    },
    Grid {
        cols: u16,
        rows: u16,
    },
    GridItem {
        column: i16,
        row: i16,
        width: u16,
    },
    Full,
    PaddedBox {
        width: u16,
        height: u16,
        padding: u16,
        border: u16,
    },
    MarginBox {
        width: u16,
        height: u16,
        margin: u16,
    },
    MinMaxWidth {
        width: u16,
        min_width: u16,
        max_width: u16,
    },
    AbsoluteBox {
        left: u16,
        top: u16,
        width: u16,
        height: u16,
    },
    PercentWidth {
        percent: u16,
        height: u16,
    },
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq)]
enum GeneratedTree {
    Unmeasured {
        style: GeneratedStyle,
        children: Vec<GeneratedTree>,
    },
    PureSize {
        width: u16,
        height: u16,
    },
    Text {
        key_index: u8,
        width: u16,
        height: u16,
    },
    Opaque {
        width: u16,
    },
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq)]
struct GeneratedFrame {
    roots: Vec<GeneratedTree>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug)]
struct GeneratedCanvasChromeFrameSpec {
    root_width: u16,
    root_height: u16,
    sidebar_width: u16,
    sidebar_content_height: u16,
    content_gap: u8,
    scale_factor: f32,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug)]
enum GeneratedSemanticChildKind {
    AnonymousLeaf,
    UniqueLeaf,
    DuplicateLeaf,
    AnonymousPanel,
    UniquePanel,
    DuplicatePanel,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug)]
struct GeneratedSemanticChildTemplate {
    kind: GeneratedSemanticChildKind,
    identity: u8,
    width: u16,
    child_width: u16,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug)]
struct GeneratedSemanticChildUse {
    template_index: usize,
    width_delta: u16,
    child_width_delta: u16,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
struct GeneratedSemanticFrame {
    children: Vec<GeneratedSemanticChildUse>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq)]
struct CanvasChromeFrameOutputForTests {
    shape: RetainedLayoutShapeForTests,
    projection: RetainedLayoutProjectionForTests,
    bounds: RetainedLayoutBoundsTreeForTests,
    canvas_chain_sizes: (Size<f32>, Size<f32>, Size<f32>, Size<f32>),
    canvas_is_paintable: (bool, bool),
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Eq, PartialEq)]
struct GeneratedTextHydration {
    label: u16,
    key_index: u8,
    width: u16,
    height: u16,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq)]
struct RetainedLayoutBoundsTreeForTests {
    bounds: Bounds<Pixels>,
    children: Vec<RetainedLayoutBoundsTreeForTests>,
}

#[cfg(not(target_arch = "wasm32"))]
fn hegel_settings(test_cases: u64) -> hegel::Settings {
    hegel::Settings::new().test_cases(test_cases)
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_u8(tc: &hegel::TestCase, min: u8, max: u8) -> u8 {
    tc.draw(
        hegel::generators::integers::<u8>()
            .min_value(min)
            .max_value(max),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_u16(tc: &hegel::TestCase, min: u16, max: u16) -> u16 {
    tc.draw(
        hegel::generators::integers::<u16>()
            .min_value(min)
            .max_value(max),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_usize(tc: &hegel::TestCase, min: usize, max: usize) -> usize {
    tc.draw(
        hegel::generators::integers::<usize>()
            .min_value(min)
            .max_value(max),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_generated_style(tc: &hegel::TestCase) -> GeneratedStyle {
    match draw_u8(tc, 0, 13) {
        0 => GeneratedStyle::Default,
        1 => GeneratedStyle::FixedWidth(draw_u16(tc, 0, 240)),
        2 => GeneratedStyle::FixedSize {
            width: draw_u16(tc, 0, 240),
            height: draw_u16(tc, 0, 160),
        },
        3 => GeneratedStyle::FlexRow {
            gap: draw_u16(tc, 0, 24),
            wrap: draw_u8(tc, 0, 1) == 1,
        },
        4 => GeneratedStyle::Grid {
            cols: draw_u16(tc, 1, 4),
            rows: draw_u16(tc, 1, 4),
        },
        5 => GeneratedStyle::GridItem {
            column: draw_u8(tc, 1, 4) as i16,
            row: draw_u8(tc, 1, 4) as i16,
            width: draw_u16(tc, 0, 240),
        },
        6 => GeneratedStyle::Full,
        7 => GeneratedStyle::PaddedBox {
            width: draw_u16(tc, 0, 240),
            height: draw_u16(tc, 0, 160),
            padding: draw_u16(tc, 0, 24),
            border: draw_u16(tc, 0, 12),
        },
        8 => GeneratedStyle::MarginBox {
            width: draw_u16(tc, 0, 240),
            height: draw_u16(tc, 0, 160),
            margin: draw_u16(tc, 0, 32),
        },
        9 => {
            let min_width = draw_u16(tc, 0, 120);
            let max_width = draw_u16(tc, min_width, 260);
            GeneratedStyle::MinMaxWidth {
                width: draw_u16(tc, 0, 300),
                min_width,
                max_width,
            }
        }
        10 => GeneratedStyle::AbsoluteBox {
            left: draw_u16(tc, 0, 80),
            top: draw_u16(tc, 0, 80),
            width: draw_u16(tc, 0, 200),
            height: draw_u16(tc, 0, 120),
        },
        11 => GeneratedStyle::PercentWidth {
            percent: draw_u16(tc, 0, 100),
            height: draw_u16(tc, 0, 160),
        },
        12 => GeneratedStyle::Grid {
            cols: draw_u16(tc, 1, 4),
            rows: draw_u16(tc, 1, 4),
        },
        _ => GeneratedStyle::FlexRow {
            gap: draw_u16(tc, 0, 24),
            wrap: true,
        },
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_generated_tree(tc: &hegel::TestCase, depth: u8, allow_opaque: bool) -> GeneratedTree {
    let kind = if depth == 0 {
        draw_u8(tc, 0, if allow_opaque { 3 } else { 2 })
    } else {
        draw_u8(tc, 0, if allow_opaque { 5 } else { 4 })
    };

    match kind {
        0 => GeneratedTree::Unmeasured {
            style: GeneratedStyle::FixedWidth(draw_u16(tc, 0, 240)),
            children: Vec::new(),
        },
        1 => GeneratedTree::PureSize {
            width: draw_u16(tc, 0, 240),
            height: draw_u16(tc, 0, 120),
        },
        2 => GeneratedTree::Text {
            key_index: draw_u8(tc, 0, 3),
            width: draw_u16(tc, 1, 240),
            height: draw_u16(tc, 1, 120),
        },
        3 if allow_opaque => GeneratedTree::Opaque {
            width: draw_u16(tc, 0, 240),
        },
        _ => {
            let child_count = draw_usize(tc, 0, 5);
            let children = (0..child_count)
                .map(|_| draw_generated_tree(tc, depth.saturating_sub(1), allow_opaque))
                .collect();
            GeneratedTree::Unmeasured {
                style: draw_generated_style(tc),
                children,
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_generated_frame(
    tc: &hegel::TestCase,
    allow_opaque: bool,
    max_roots: usize,
) -> GeneratedFrame {
    let root_count = draw_usize(tc, 0, max_roots);
    GeneratedFrame {
        roots: (0..root_count)
            .map(|_| draw_generated_tree(tc, 3, allow_opaque))
            .collect(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_generated_frames(
    tc: &hegel::TestCase,
    allow_opaque: bool,
    min_frames: usize,
    max_frames: usize,
) -> Vec<GeneratedFrame> {
    let frame_count = draw_usize(tc, min_frames, max_frames);
    (0..frame_count)
        .map(|_| draw_generated_frame(tc, allow_opaque, 5))
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_canvas_chrome_frame_specs(
    tc: &hegel::TestCase,
    min_frames: usize,
    max_frames: usize,
) -> Vec<GeneratedCanvasChromeFrameSpec> {
    let frame_count = draw_usize(tc, min_frames, max_frames);
    (0..frame_count)
        .map(|_| GeneratedCanvasChromeFrameSpec {
            root_width: draw_u16(tc, 1100, 2400),
            root_height: draw_u16(tc, 760, 1600),
            sidebar_width: draw_u16(tc, 0, 640),
            sidebar_content_height: draw_u16(tc, 0, 360),
            content_gap: draw_u8(tc, 0, 32),
            scale_factor: if draw_u8(tc, 0, 1) == 0 { 1.0 } else { 2.0 },
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_generated_semantic_child_kind(tc: &hegel::TestCase) -> GeneratedSemanticChildKind {
    match draw_u8(tc, 0, 5) {
        0 => GeneratedSemanticChildKind::AnonymousLeaf,
        1 => GeneratedSemanticChildKind::UniqueLeaf,
        2 => GeneratedSemanticChildKind::DuplicateLeaf,
        3 => GeneratedSemanticChildKind::AnonymousPanel,
        4 => GeneratedSemanticChildKind::UniquePanel,
        _ => GeneratedSemanticChildKind::DuplicatePanel,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_generated_semantic_child_templates(
    tc: &hegel::TestCase,
) -> Vec<GeneratedSemanticChildTemplate> {
    let template_count = draw_usize(tc, 2, 8);
    (0..template_count)
        .map(|index| GeneratedSemanticChildTemplate {
            kind: draw_generated_semantic_child_kind(tc),
            identity: index as u8,
            width: draw_u16(tc, 1, 140),
            child_width: draw_u16(tc, 1, 140),
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_generated_semantic_frames(
    tc: &hegel::TestCase,
    template_count: usize,
) -> Vec<GeneratedSemanticFrame> {
    let frame_count = draw_usize(tc, 3, 8);
    (0..frame_count)
        .map(|_| {
            let child_count = draw_usize(tc, 0, template_count + 3);
            let children = (0..child_count)
                .map(|_| GeneratedSemanticChildUse {
                    template_index: draw_usize(tc, 0, template_count - 1),
                    width_delta: draw_u16(tc, 0, 32),
                    child_width_delta: draw_u16(tc, 0, 32),
                })
                .collect();
            GeneratedSemanticFrame { children }
        })
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
impl GeneratedStyle {
    fn to_style(&self) -> Style {
        match self {
            Self::Default => Style::default(),
            Self::FixedWidth(width) => style_with_width(*width as f32),
            Self::FixedSize { width, height } => {
                let mut style = style_with_width(*width as f32);
                style.size.height = length_px(*height as f32);
                style
            }
            Self::FlexRow { gap, wrap } => {
                let mut style = Style::default();
                style.display = Display::Flex;
                style.flex_direction = FlexDirection::Row;
                style.flex_wrap = if *wrap {
                    FlexWrap::Wrap
                } else {
                    FlexWrap::NoWrap
                };
                style.gap = size(definite_px(*gap as f32), definite_px(*gap as f32));
                style
            }
            Self::Grid { cols, rows } => {
                let mut style = Style::default();
                style.display = Display::Grid;
                style.grid_cols = Some(GridTemplate {
                    repeat: *cols,
                    min_size: TemplateColumnMinSize::Zero,
                });
                style.grid_rows = Some(GridTemplate {
                    repeat: *rows,
                    min_size: TemplateColumnMinSize::Zero,
                });
                style
            }
            Self::GridItem { column, row, width } => {
                let mut style = style_with_width(*width as f32);
                let grid_location = style.grid_location.get_or_insert_default();
                grid_location.column =
                    GridPlacement::Line(*column)..GridPlacement::Line(*column + 1);
                grid_location.row = GridPlacement::Line(*row)..GridPlacement::Line(*row + 1);
                style
            }
            Self::Full => {
                let mut style = Style::default();
                style.size = Size::full();
                style
            }
            Self::PaddedBox {
                width,
                height,
                padding,
                border,
            } => {
                let mut style = style_with_width(*width as f32);
                style.size.height = length_px(*height as f32);
                style.padding = Edges::all(definite_px(*padding as f32));
                style.border_widths = Edges::all(absolute_px(*border as f32));
                style
            }
            Self::MarginBox {
                width,
                height,
                margin,
            } => {
                let mut style = style_with_width(*width as f32);
                style.size.height = length_px(*height as f32);
                style.margin = Edges::all(length_px(*margin as f32));
                style
            }
            Self::MinMaxWidth {
                width,
                min_width,
                max_width,
            } => {
                let mut style = style_with_width(*width as f32);
                style.min_size.width = length_px(*min_width as f32);
                style.max_size.width = length_px(*max_width as f32);
                style
            }
            Self::AbsoluteBox {
                left,
                top,
                width,
                height,
            } => {
                let mut style = style_with_width(*width as f32);
                style.position = Position::Absolute;
                style.inset.left = length_px(*left as f32);
                style.inset.top = length_px(*top as f32);
                style.size.height = length_px(*height as f32);
                style
            }
            Self::PercentWidth { percent, height } => {
                let mut style = Style::default();
                style.size.width = length_fraction(*percent);
                style.size.height = length_px(*height as f32);
                style
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl GeneratedTree {
    fn node_count(&self) -> u64 {
        match self {
            Self::Unmeasured { children, .. } => {
                1 + children.iter().map(Self::node_count).sum::<u64>()
            }
            Self::PureSize { .. } | Self::Text { .. } | Self::Opaque { .. } => 1,
        }
    }

    fn measured_count(&self) -> u64 {
        match self {
            Self::Unmeasured { children, .. } => children.iter().map(Self::measured_count).sum(),
            Self::PureSize { .. } | Self::Text { .. } | Self::Opaque { .. } => 1,
        }
    }

    fn retained_node_matches_current_facts(&self, current: &Self) -> bool {
        match (self, current) {
            (
                Self::Unmeasured {
                    style: previous_style,
                    children: previous_children,
                },
                Self::Unmeasured {
                    style: current_style,
                    children: current_children,
                },
            ) => {
                previous_style == current_style
                    && previous_children.len() == current_children.len()
                    && previous_children
                        .iter()
                        .zip(current_children)
                        .all(|(previous, current)| {
                            previous.retained_node_matches_current_facts(current)
                        })
            }
            (
                Self::PureSize {
                    width: left_width,
                    height: left_height,
                },
                Self::PureSize {
                    width: right_width,
                    height: right_height,
                },
            ) => left_width == right_width && left_height == right_height,
            (
                Self::Text {
                    key_index: left_key,
                    width: left_width,
                    height: left_height,
                },
                Self::Text {
                    key_index: right_key,
                    width: right_width,
                    height: right_height,
                },
            ) => left_key == right_key && left_width == right_width && left_height == right_height,
            _ => false,
        }
    }

    fn same_position_facts_match(&self, current: &Self) -> bool {
        match (self, current) {
            (
                Self::Unmeasured {
                    style: previous_style,
                    children: previous_children,
                },
                Self::Unmeasured {
                    style: current_style,
                    children: current_children,
                },
            ) => {
                previous_style == current_style && previous_children.len() == current_children.len()
            }
            (
                Self::PureSize {
                    width: left_width,
                    height: left_height,
                },
                Self::PureSize {
                    width: right_width,
                    height: right_height,
                },
            ) => left_width == right_width && left_height == right_height,
            (
                Self::Text {
                    key_index: left_key,
                    width: left_width,
                    height: left_height,
                },
                Self::Text {
                    key_index: right_key,
                    width: right_width,
                    height: right_height,
                },
            ) => left_key == right_key && left_width == right_width && left_height == right_height,
            _ => false,
        }
    }

    fn can_host_recommitted_facts(&self, current: &Self) -> bool {
        matches!(
            (self, current),
            (Self::Unmeasured { .. }, Self::Unmeasured { .. })
                | (Self::PureSize { .. }, Self::PureSize { .. })
                | (Self::Text { .. }, Self::Text { .. })
        )
    }

    fn exact_child_count(child: &Self, children: &[Self]) -> usize {
        children
            .iter()
            .filter(|candidate| child.retained_node_matches_current_facts(candidate))
            .count()
    }

    fn exact_unused_previous_child_count(
        child: &Self,
        previous_children: &[Self],
        previous_used: &[bool],
    ) -> usize {
        previous_children
            .iter()
            .zip(previous_used)
            .filter(|(_, used)| !**used)
            .filter(|(candidate, _)| candidate.retained_node_matches_current_facts(child))
            .count()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn generated_text_measure_key(key_index: u8, width: u16, height: u16) -> TextMeasureKey {
    let text = SharedString::from(format!("generated-text-{key_index}-{width}-{height}"));
    let text_style = TextStyle::default();
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

#[cfg(not(target_arch = "wasm32"))]
fn request_generated_tree(engine: &mut LayoutEngine, tree: &GeneratedTree) -> LayoutId {
    match tree {
        GeneratedTree::Unmeasured { style, children } => {
            let children = children
                .iter()
                .map(|child| request_generated_tree(engine, child))
                .collect::<Vec<_>>();
            engine.request_layout(style.to_style(), px(16.0), 1.0, &children)
        }
        GeneratedTree::PureSize { width, height } => engine.request_pure_measured_layout(
            Style::default(),
            px(16.0),
            1.0,
            PureSizeMeasure::list(px(*width as f32), px(*height as f32), 1.0),
        ),
        GeneratedTree::Text {
            key_index,
            width,
            height,
        } => {
            let key = generated_text_measure_key(*key_index, *width, *height);
            let fallback_size = size(px(*width as f32), px(*height as f32));
            engine.request_text_measured_layout(
                Style::default(),
                px(16.0),
                1.0,
                key.clone(),
                |_| {},
                move |known_dimensions, available_space, _| {
                    TextLayoutArtifact::for_tests(
                        key.clone(),
                        text_artifact_size_for_query(
                            fallback_size,
                            known_dimensions,
                            available_space,
                        ),
                    )
                },
            )
        }
        GeneratedTree::Opaque { width } => {
            let width = *width;
            engine.request_measured_layout(
                style_with_width(width as f32),
                px(16.0),
                1.0,
                move |_, _, _| size(px(width as f32), px(10.0)),
            )
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn request_generated_frame(engine: &mut LayoutEngine, frame: &GeneratedFrame) -> Vec<LayoutId> {
    frame
        .roots
        .iter()
        .map(|root| request_generated_tree(engine, root))
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn semantic_width(base: u16, delta: u16) -> f32 {
    (base + delta) as f32
}

#[cfg(not(target_arch = "wasm32"))]
fn request_generated_semantic_child(
    engine: &mut LayoutEngine,
    template: GeneratedSemanticChildTemplate,
    child_use: GeneratedSemanticChildUse,
) -> LayoutId {
    let width = semantic_width(template.width, child_use.width_delta);
    let child_width = semantic_width(template.child_width, child_use.child_width_delta);
    match template.kind {
        GeneratedSemanticChildKind::AnonymousLeaf => request_leaf(engine, width),
        GeneratedSemanticChildKind::UniqueLeaf => {
            request_keyed_leaf(engine, 50_000 + template.identity as u64, width)
        }
        GeneratedSemanticChildKind::DuplicateLeaf => {
            request_keyed_leaf(engine, 50_900 + u64::from(template.identity % 2), width)
        }
        GeneratedSemanticChildKind::AnonymousPanel => {
            let child = request_leaf(engine, child_width);
            request_container(engine, &[child])
        }
        GeneratedSemanticChildKind::UniquePanel => {
            let child = request_leaf(engine, child_width);
            request_keyed_layout(
                engine,
                51_000 + template.identity as u64,
                style_with_width(width),
                &[child],
            )
        }
        GeneratedSemanticChildKind::DuplicatePanel => {
            let child = request_leaf(engine, child_width);
            request_keyed_layout(
                engine,
                51_900 + u64::from(template.identity % 2),
                style_with_width(width),
                &[child],
            )
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn request_generated_semantic_frame(
    engine: &mut LayoutEngine,
    templates: &[GeneratedSemanticChildTemplate],
    frame: &GeneratedSemanticFrame,
) -> (LayoutId, Vec<LayoutId>) {
    let children = frame
        .children
        .iter()
        .map(|child_use| {
            request_generated_semantic_child(
                engine,
                templates[child_use.template_index],
                *child_use,
            )
        })
        .collect::<Vec<_>>();
    let root = request_flex_container(engine, &children);
    (root, children)
}

#[cfg(not(target_arch = "wasm32"))]
fn retained_layout_shapes(
    engine: &LayoutEngine,
    roots: &[RetainedNodeToken],
) -> Vec<RetainedLayoutShapeForTests> {
    roots
        .iter()
        .map(|root| retained_layout_shape(engine, *root))
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn retained_layout_projection(
    engine: &LayoutEngine,
    node_id: RetainedNodeToken,
) -> RetainedLayoutProjectionForTests {
    engine.retained_layout_projection_for_tests(node_id)
}

#[cfg(not(target_arch = "wasm32"))]
fn retained_layout_projections(
    engine: &LayoutEngine,
    roots: &[RetainedNodeToken],
) -> Vec<RetainedLayoutProjectionForTests> {
    roots
        .iter()
        .map(|root| retained_layout_projection(engine, *root))
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn retained_layout_bounds_tree(
    engine: &mut LayoutEngine,
    root: RetainedNodeToken,
    scale_factor: f32,
) -> RetainedLayoutBoundsTreeForTests {
    let children = engine.retained_child_tokens_for_tests(root);
    let bounds = engine.retained_node_layout_bounds_for_tests(root, scale_factor);
    RetainedLayoutBoundsTreeForTests {
        bounds,
        children: children
            .into_iter()
            .map(|child| retained_layout_bounds_tree(engine, child, scale_factor))
            .collect(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn retained_layout_bounds_trees(
    engine: &mut LayoutEngine,
    roots: &[RetainedNodeToken],
    scale_factor: f32,
) -> Vec<RetainedLayoutBoundsTreeForTests> {
    roots
        .iter()
        .map(|root| retained_layout_bounds_tree(engine, *root, scale_factor))
        .collect()
}

#[cfg(not(target_arch = "wasm32"))]
fn compute_generated_roots(
    cx: &mut VisualTestContext,
    engine: &mut LayoutEngine,
    roots: &[LayoutId],
    available_width: u16,
    available_height: u16,
) -> Vec<RetainedNodeToken> {
    cx.update(|window, app| {
        roots
            .iter()
            .enumerate()
            .map(|(index, root)| {
                engine.compute_retained_layout_for_tests(
                    *root,
                    RetainedLayoutRootId::new(index as u64),
                    size(
                        AvailableSpace::Definite(px(available_width as f32)),
                        AvailableSpace::Definite(px(available_height as f32)),
                    ),
                    window,
                    app,
                );
                engine.retained_node_token_for_tests(*root)
            })
            .collect()
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn expected_mutations(
    previous: &[GeneratedTree],
    current: &[GeneratedTree],
) -> RetainedForestMutationSample {
    let mut expected = RetainedForestMutationSample::default();
    for index in 0..previous.len().max(current.len()) {
        match (previous.get(index), current.get(index)) {
            (Some(previous), Some(current)) => {
                expected.add_commit(previous, current);
            }
            (Some(previous), None) => {
                expected.removes += previous.node_count();
                expected.context_clears += previous.measured_count();
            }
            (None, Some(current)) => {
                expected.creates += current.node_count();
            }
            (None, None) => {}
        }
    }
    expected
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy)]
struct ExpectedCommitResult {
    retained_same_node: bool,
}

#[cfg(not(target_arch = "wasm32"))]
trait ExpectedMutationCountsExt {
    fn add_commit(&mut self, previous: &GeneratedTree, current: &GeneratedTree);
    fn add_commit_with_context(
        &mut self,
        previous: &GeneratedTree,
        current: &GeneratedTree,
    ) -> ExpectedCommitResult;
    fn add_fresh_tree(&mut self, current: &GeneratedTree);
    fn add_remove_tree(&mut self, previous: &GeneratedTree);
    fn add_reused_tree(&mut self, current: &GeneratedTree) -> ExpectedCommitResult;
}

#[cfg(not(target_arch = "wasm32"))]
impl ExpectedMutationCountsExt for RetainedForestMutationSample {
    fn add_commit(&mut self, previous: &GeneratedTree, current: &GeneratedTree) {
        self.add_commit_with_context(previous, current);
    }

    fn add_commit_with_context(
        &mut self,
        previous: &GeneratedTree,
        current: &GeneratedTree,
    ) -> ExpectedCommitResult {
        if previous.retained_node_matches_current_facts(current) {
            return self.add_reused_tree(current);
        }

        match (previous, current) {
            (
                GeneratedTree::Unmeasured {
                    style: previous_style,
                    children: previous_children,
                },
                GeneratedTree::Unmeasured {
                    style: current_style,
                    children: current_children,
                },
            ) => {
                self.reuses += 1;
                if previous_style != current_style {
                    self.style_updates += 1;
                }

                let mut previous_used = vec![false; previous_children.len()];
                let mut assigned_previous_indices = vec![None; current_children.len()];

                for (current_index, current_child) in current_children.iter().enumerate() {
                    if let Some(previous_child) = previous_children.get(current_index) {
                        if previous_child.same_position_facts_match(current_child) {
                            previous_used[current_index] = true;
                            assigned_previous_indices[current_index] = Some(current_index);
                        }
                    }
                }

                for (current_index, current_child) in current_children.iter().enumerate() {
                    if assigned_previous_indices[current_index].is_some() {
                        continue;
                    }
                    if GeneratedTree::exact_child_count(current_child, current_children) != 1
                        || GeneratedTree::exact_unused_previous_child_count(
                            current_child,
                            previous_children,
                            &previous_used,
                        ) != 1
                    {
                        continue;
                    }
                    let matching_previous_index =
                        previous_children
                            .iter()
                            .enumerate()
                            .position(|(index, previous_child)| {
                                !previous_used[index]
                                    && previous_child
                                        .retained_node_matches_current_facts(current_child)
                            });
                    if let Some(index) = matching_previous_index {
                        previous_used[index] = true;
                        assigned_previous_indices[current_index] = Some(index);
                    }
                }

                for (current_index, current_child) in current_children.iter().enumerate() {
                    if assigned_previous_indices[current_index].is_some() {
                        continue;
                    }
                    let Some(previous_child) = previous_children.get(current_index) else {
                        continue;
                    };
                    if !previous_used[current_index]
                        && previous_child.can_host_recommitted_facts(current_child)
                    {
                        previous_used[current_index] = true;
                        assigned_previous_indices[current_index] = Some(current_index);
                    }
                }

                let mut child_list_changed = previous_children.len() != current_children.len();
                for (current_index, matching_previous_index) in
                    assigned_previous_indices.iter().enumerate()
                {
                    if *matching_previous_index
                        != previous_children
                            .get(current_index)
                            .and_then(|_| Some(current_index))
                    {
                        child_list_changed = true;
                    }
                }
                for (current_index, current_child) in current_children.iter().enumerate() {
                    let matching_previous_index = assigned_previous_indices[current_index];
                    if let Some(index) = matching_previous_index {
                        let child_result =
                            self.add_commit_with_context(&previous_children[index], current_child);
                        if !child_result.retained_same_node || index != current_index {
                            child_list_changed = true;
                        }
                    } else {
                        self.add_fresh_tree(current_child);
                        child_list_changed = true;
                    }
                }
                if child_list_changed {
                    self.child_list_updates += 1;
                }

                for (previous_child, used) in previous_children.iter().zip(previous_used) {
                    if !used {
                        self.add_remove_tree(previous_child);
                    }
                }
                ExpectedCommitResult {
                    retained_same_node: true,
                }
            }
            (GeneratedTree::PureSize { .. }, GeneratedTree::PureSize { .. })
            | (GeneratedTree::Text { .. }, GeneratedTree::Text { .. }) => {
                self.reuses += 1;
                let measured_facts_changed = match (previous, current) {
                    (
                        GeneratedTree::PureSize {
                            width: previous_width,
                            height: previous_height,
                        },
                        GeneratedTree::PureSize {
                            width: current_width,
                            height: current_height,
                        },
                    ) => previous_width != current_width || previous_height != current_height,
                    (
                        GeneratedTree::Text {
                            key_index: previous_key,
                            width: previous_width,
                            height: previous_height,
                        },
                        GeneratedTree::Text {
                            key_index: current_key,
                            width: current_width,
                            height: current_height,
                        },
                    ) => {
                        previous_key != current_key
                            || previous_width != current_width
                            || previous_height != current_height
                    }
                    _ => false,
                };
                if measured_facts_changed {
                    self.dirty_marks += 1;
                }
                ExpectedCommitResult {
                    retained_same_node: true,
                }
            }
            _ => {
                self.add_remove_tree(previous);
                self.add_fresh_tree(current);
                ExpectedCommitResult {
                    retained_same_node: false,
                }
            }
        }
    }

    fn add_fresh_tree(&mut self, current: &GeneratedTree) {
        self.creates += current.node_count();
    }

    fn add_remove_tree(&mut self, previous: &GeneratedTree) {
        self.removes += previous.node_count();
        self.context_clears += previous.measured_count();
    }

    fn add_reused_tree(&mut self, current: &GeneratedTree) -> ExpectedCommitResult {
        self.reuses += current.node_count();
        ExpectedCommitResult {
            retained_same_node: true,
        }
    }
}

fn request_text_measured_with_hydration_log(
    engine: &mut LayoutEngine,
    key: TextMeasureKey,
    size: Size<Pixels>,
    measure_invocations: Rc<Cell<usize>>,
    hydrations: Rc<Cell<usize>>,
    hydrated_artifacts: Option<Rc<RefCell<Vec<HydratedTextArtifact>>>>,
) -> LayoutId {
    engine.request_text_measured_layout(
        Style::default(),
        px(16.0),
        1.0,
        key.clone(),
        move |artifact| {
            hydrations.set(hydrations.get() + 1);
            if let Some(hydrated_artifacts) = hydrated_artifacts.as_ref() {
                hydrated_artifacts.borrow_mut().push(HydratedTextArtifact {
                    key: artifact.key().clone(),
                    size: artifact.size(),
                });
            }
        },
        move |known_dimensions, available_space, _| {
            measure_invocations.set(measure_invocations.get() + 1);
            TextLayoutArtifact::for_tests(
                key.clone(),
                text_artifact_size_for_query(size, known_dimensions, available_space),
            )
        },
    )
}

fn request_fixed_size_text_measured_with_hydration_log(
    engine: &mut LayoutEngine,
    key: TextMeasureKey,
    layout_size: Size<Pixels>,
    artifact_size: Size<Pixels>,
    measure_invocations: Rc<Cell<usize>>,
    hydrations: Rc<Cell<usize>>,
    hydrated_artifacts: Rc<RefCell<Vec<HydratedTextArtifact>>>,
) -> LayoutId {
    engine.request_fixed_size_text_measured_layout(
        Style::default(),
        px(16.0),
        1.0,
        layout_size,
        key.clone(),
        move |artifact| {
            hydrations.set(hydrations.get() + 1);
            hydrated_artifacts.borrow_mut().push(HydratedTextArtifact {
                key: artifact.key().clone(),
                size: artifact.size(),
            });
        },
        move |known_dimensions, available_space, _| {
            measure_invocations.set(measure_invocations.get() + 1);
            TextLayoutArtifact::for_tests(
                key.clone(),
                text_artifact_size_for_query(artifact_size, known_dimensions, available_space),
            )
        },
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn request_input_sensitive_text_measured(
    engine: &mut LayoutEngine,
    label: u16,
    key_index: u8,
    fallback_width: u16,
    height: u16,
    measure_invocations: Rc<Cell<usize>>,
    hydrated_artifacts: Rc<RefCell<Vec<GeneratedTextHydration>>>,
) -> LayoutId {
    let key = generated_text_measure_key(key_index, fallback_width, height);
    let measure_key = key.clone();
    engine.request_text_measured_layout(
        Style::default(),
        px(16.0),
        1.0,
        key,
        move |artifact| {
            let size = artifact.size();
            hydrated_artifacts
                .borrow_mut()
                .push(GeneratedTextHydration {
                    label,
                    key_index,
                    width: size.width.0.round() as u16,
                    height: size.height.0.round() as u16,
                });
        },
        move |known_dimensions, available_space, _| {
            measure_invocations.set(measure_invocations.get() + 1);
            let measured_width = known_dimensions
                .width
                .unwrap_or(match available_space.width {
                    AvailableSpace::Definite(width) => width,
                    AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                        px(fallback_width as f32)
                    }
                });
            let measured_height = known_dimensions.height.unwrap_or(px(height as f32));
            TextLayoutArtifact::for_tests(
                measure_key.clone(),
                size(measured_width, measured_height),
            )
        },
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn request_growing_input_sensitive_text_measured(
    engine: &mut LayoutEngine,
    label: u16,
    key_index: u8,
    fallback_width: u16,
    height: u16,
    measure_invocations: Rc<Cell<usize>>,
    hydrated_artifacts: Rc<RefCell<Vec<GeneratedTextHydration>>>,
) -> LayoutId {
    let key = generated_text_measure_key(key_index, fallback_width, height);
    let measure_key = key.clone();
    let mut style = Style::default();
    style.flex_grow = 1.0;
    style.flex_shrink = 1.0;
    style.min_size.width =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.0))));
    engine.request_text_measured_layout(
        style,
        px(16.0),
        1.0,
        key,
        move |artifact| {
            let size = artifact.size();
            hydrated_artifacts
                .borrow_mut()
                .push(GeneratedTextHydration {
                    label,
                    key_index,
                    width: size.width.0.round() as u16,
                    height: size.height.0.round() as u16,
                });
        },
        move |known_dimensions, available_space, _| {
            measure_invocations.set(measure_invocations.get() + 1);
            let measured_width = known_dimensions
                .width
                .unwrap_or(match available_space.width {
                    AvailableSpace::Definite(width) => width,
                    AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                        px(fallback_width as f32)
                    }
                });
            let measured_height = known_dimensions.height.unwrap_or(px(height as f32));
            TextLayoutArtifact::for_tests(
                measure_key.clone(),
                size(measured_width, measured_height),
            )
        },
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn compute_generated_text_root(
    cx: &mut VisualTestContext,
    engine: &mut LayoutEngine,
    root: LayoutId,
    available_width: u16,
) {
    compute_stable_test_root(
        cx,
        engine,
        root,
        size(
            AvailableSpace::Definite(px(available_width as f32)),
            AvailableSpace::MaxContent,
        ),
    );
}

fn request_row(engine: &mut LayoutEngine, widths: &[f32]) -> LayoutId {
    let children = widths
        .iter()
        .map(|width| request_leaf(engine, *width))
        .collect::<Vec<_>>();
    request_container(engine, &children)
}

fn request_flex_row(engine: &mut LayoutEngine, widths: &[f32]) -> LayoutId {
    let children = widths
        .iter()
        .map(|width| request_leaf(engine, *width))
        .collect::<Vec<_>>();
    request_flex_container(engine, &children)
}

fn retained_layout_shape(
    engine: &LayoutEngine,
    node_id: RetainedNodeToken,
) -> RetainedLayoutShapeForTests {
    engine.retained_layout_shape_for_tests(node_id)
}

fn compute_layout_without_measure(
    engine: &mut LayoutEngine,
    root: LayoutId,
    width: f32,
    height: f32,
) -> RetainedNodeToken {
    compute_layout_without_measure_with_scale(engine, root, width, height, 1.0)
}

fn compute_layout_without_measure_with_scale(
    engine: &mut LayoutEngine,
    root: LayoutId,
    width: f32,
    height: f32,
    scale_factor: f32,
) -> RetainedNodeToken {
    compute_layout_without_measure_with_available_space(
        engine,
        root,
        AvailableSpace::Definite(px(width)),
        AvailableSpace::Definite(px(height)),
        scale_factor,
    )
}

fn compute_layout_without_measure_with_available_space(
    engine: &mut LayoutEngine,
    root: LayoutId,
    width: AvailableSpace,
    height: AvailableSpace,
    scale_factor: f32,
) -> RetainedNodeToken {
    engine.compute_unmeasured_layout_with_scale_for_tests(root, size(width, height), scale_factor)
}

fn retained_node_size(engine: &LayoutEngine, node_id: RetainedNodeToken) -> Size<f32> {
    engine.retained_node_size_for_tests(node_id)
}

fn assert_facts_committed_exactly(engine: &LayoutEngine, id: LayoutId) {
    engine.assert_facts_committed_exactly_for_tests(id);
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_retained_commit_matches_fresh_outputs(cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let mut cx = cx.add_empty_window();
        let frames = draw_generated_frames(&tc, true, 1, 5);
        let available_width = draw_u16(&tc, 1, 360);
        let available_height = draw_u16(&tc, 1, 240);
        let mut retained = LayoutEngine::new();
        let mut previous_roots = Vec::new();

        for frame in frames {
            retained.reset_retained_mutation_sample_for_tests();
            let retained_ids = request_generated_frame(&mut retained, &frame);
            let retained_roots = compute_generated_roots(
                cx,
                &mut retained,
                &retained_ids,
                available_width,
                available_height,
            );
            for root in &retained_ids {
                assert_facts_committed_exactly(&retained, *root);
            }

            let mut fresh = LayoutEngine::new();
            let fresh_ids = request_generated_frame(&mut fresh, &frame);
            let fresh_roots = compute_generated_roots(
                cx,
                &mut fresh,
                &fresh_ids,
                available_width,
                available_height,
            );
            for root in &fresh_ids {
                assert_facts_committed_exactly(&fresh, *root);
            }

            assert_eq!(
                retained_layout_shapes(&retained, &retained_roots),
                retained_layout_shapes(&fresh, &fresh_roots)
            );
            assert_eq!(
                retained_layout_projections(&retained, &retained_roots),
                retained_layout_projections(&fresh, &fresh_roots)
            );
            assert_eq!(
                retained_layout_bounds_trees(&mut retained, &retained_roots, 1.0),
                retained_layout_bounds_trees(&mut fresh, &fresh_roots, 1.0)
            );
            retained.finish_frame();
            let actual = retained.retained_mutation_sample_for_tests();
            let expected = expected_mutations(&previous_roots, &frame.roots);
            assert_eq!(actual, expected);
            previous_roots = frame.roots;
        }
    })
    .settings(hegel_settings(100))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_retained_layout_matches_fresh_across_changing_root_constraints(
    cx: &mut TestAppContext,
) {
    hegel::Hegel::new(|tc| {
        let mut cx = cx.add_empty_window();
        let frame = draw_generated_frame(&tc, true, 3);
        let first_width = draw_u16(&tc, 1, 360);
        let first_height = draw_u16(&tc, 1, 240);
        let second_width = draw_u16(&tc, 1, 360);
        let second_height = draw_u16(&tc, 1, 240);
        let root_constraints = [
            (first_width, first_height),
            (second_width, second_height),
            (first_width, first_height),
        ];
        let mut retained = LayoutEngine::new();

        for (available_width, available_height) in root_constraints {
            let retained_ids = request_generated_frame(&mut retained, &frame);
            let retained_roots = compute_generated_roots(
                cx,
                &mut retained,
                &retained_ids,
                available_width,
                available_height,
            );
            for root in &retained_ids {
                assert_facts_committed_exactly(&retained, *root);
            }

            let mut fresh = LayoutEngine::new();
            let fresh_ids = request_generated_frame(&mut fresh, &frame);
            let fresh_roots = compute_generated_roots(
                cx,
                &mut fresh,
                &fresh_ids,
                available_width,
                available_height,
            );
            for root in &fresh_ids {
                assert_facts_committed_exactly(&fresh, *root);
            }

            assert_eq!(
                retained_layout_shapes(&retained, &retained_roots),
                retained_layout_shapes(&fresh, &fresh_roots)
            );
            assert_eq!(
                retained_layout_projections(&retained, &retained_roots),
                retained_layout_projections(&fresh, &fresh_roots)
            );
            assert_eq!(
                retained_layout_bounds_trees(&mut retained, &retained_roots, 1.0),
                retained_layout_bounds_trees(&mut fresh, &fresh_roots, 1.0)
            );

            retained.finish_frame();
        }
    })
    .settings(hegel_settings(100))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn retained_layout_matches_fresh_after_root_constraint_aba_regression(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let frame = GeneratedFrame {
        roots: vec![GeneratedTree::Unmeasured {
            style: GeneratedStyle::Default,
            children: vec![GeneratedTree::Unmeasured {
                style: GeneratedStyle::FixedSize {
                    width: 0,
                    height: 0,
                },
                children: vec![
                    GeneratedTree::PureSize {
                        width: 0,
                        height: 1,
                    },
                    GeneratedTree::Unmeasured {
                        style: GeneratedStyle::FixedWidth(0),
                        children: Vec::new(),
                    },
                ],
            }],
        }],
    };
    let mut retained = LayoutEngine::new();

    for (available_width, available_height) in [(1, 1), (1, 2), (1, 1)] {
        let retained_ids = request_generated_frame(&mut retained, &frame);
        let retained_roots = compute_generated_roots(
            cx,
            &mut retained,
            &retained_ids,
            available_width,
            available_height,
        );
        for root in &retained_ids {
            assert_facts_committed_exactly(&retained, *root);
        }

        let mut fresh = LayoutEngine::new();
        let fresh_ids = request_generated_frame(&mut fresh, &frame);
        let fresh_roots = compute_generated_roots(
            cx,
            &mut fresh,
            &fresh_ids,
            available_width,
            available_height,
        );
        for root in &fresh_ids {
            assert_facts_committed_exactly(&fresh, *root);
        }

        assert_eq!(
            retained_layout_shapes(&retained, &retained_roots),
            retained_layout_shapes(&fresh, &fresh_roots)
        );
        assert_eq!(
            retained_layout_projections(&retained, &retained_roots),
            retained_layout_projections(&fresh, &fresh_roots)
        );
        assert_eq!(
            retained_layout_bounds_trees(&mut retained, &retained_roots, 1.0),
            retained_layout_bounds_trees(&mut fresh, &fresh_roots, 1.0)
        );

        retained.finish_frame();
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn retained_layout_matches_fresh_after_root_constraint_change_with_full_child_regression(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let frame = GeneratedFrame {
        roots: vec![GeneratedTree::Unmeasured {
            style: GeneratedStyle::FlexRow {
                gap: 0,
                wrap: false,
            },
            children: vec![GeneratedTree::Unmeasured {
                style: GeneratedStyle::Default,
                children: vec![
                    GeneratedTree::PureSize {
                        width: 0,
                        height: 1,
                    },
                    GeneratedTree::Unmeasured {
                        style: GeneratedStyle::Full,
                        children: Vec::new(),
                    },
                ],
            }],
        }],
    };
    let mut retained = LayoutEngine::new();

    for (available_width, available_height) in [(1, 1), (1, 2), (1, 1)] {
        let retained_ids = request_generated_frame(&mut retained, &frame);
        let retained_roots = compute_generated_roots(
            cx,
            &mut retained,
            &retained_ids,
            available_width,
            available_height,
        );
        for root in &retained_ids {
            assert_facts_committed_exactly(&retained, *root);
        }

        let mut fresh = LayoutEngine::new();
        let fresh_ids = request_generated_frame(&mut fresh, &frame);
        let fresh_roots = compute_generated_roots(
            cx,
            &mut fresh,
            &fresh_ids,
            available_width,
            available_height,
        );
        for root in &fresh_ids {
            assert_facts_committed_exactly(&fresh, *root);
        }

        assert_eq!(
            retained_layout_shapes(&retained, &retained_roots),
            retained_layout_shapes(&fresh, &fresh_roots)
        );
        assert_eq!(
            retained_layout_projections(&retained, &retained_roots),
            retained_layout_projections(&fresh, &fresh_roots)
        );
        assert_eq!(
            retained_layout_bounds_trees(&mut retained, &retained_roots, 1.0),
            retained_layout_bounds_trees(&mut fresh, &fresh_roots, 1.0)
        );

        retained.finish_frame();
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_canvas_chrome_frame_sequence_matches_fresh_during_sidebar_changes(
    _cx: &mut TestAppContext,
) {
    hegel::Hegel::new(|tc| {
        let mut specs = vec![
            GeneratedCanvasChromeFrameSpec {
                root_width: 2200,
                root_height: 0,
                sidebar_width: 0,
                sidebar_content_height: 0,
                content_gap: 0,
                scale_factor: 1.0,
            },
            GeneratedCanvasChromeFrameSpec {
                root_width: 459,
                root_height: 609,
                sidebar_width: 480,
                sidebar_content_height: 240,
                content_gap: 21,
                scale_factor: 2.0,
            },
            GeneratedCanvasChromeFrameSpec {
                root_width: 2200,
                root_height: 1522,
                sidebar_width: 480,
                sidebar_content_height: 240,
                content_gap: 21,
                scale_factor: 2.0,
            },
        ];
        specs.extend(draw_canvas_chrome_frame_specs(&tc, 2, 6));
        let mut retained = LayoutEngine::new();

        for spec in specs {
            let retained_output = compute_canvas_chrome_frame_output(&mut retained, spec);

            let mut fresh = LayoutEngine::new();
            let fresh_output = compute_canvas_chrome_frame_output(&mut fresh, spec);

            assert_eq!(retained_output, fresh_output);
            if spec.root_width >= 1100 && spec.root_height >= 760 {
                assert_eq!(fresh_output.canvas_is_paintable, (true, true));
            }

            retained.finish_frame();
        }
    })
    .settings(hegel_settings(64))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_canvas_chrome_frame_converges_after_distinct_retained_histories(
    _cx: &mut TestAppContext,
) {
    fn retained_output_after_prefix(
        prefix: &[GeneratedCanvasChromeFrameSpec],
        current: GeneratedCanvasChromeFrameSpec,
    ) -> CanvasChromeFrameOutputForTests {
        let mut engine = LayoutEngine::new();
        for spec in prefix {
            compute_canvas_chrome_frame_output(&mut engine, *spec);
            engine.finish_frame();
        }
        compute_canvas_chrome_frame_output(&mut engine, current)
    }

    hegel::Hegel::new(|tc| {
        let current = draw_canvas_chrome_frame_specs(&tc, 1, 1)[0];
        let zero_height_probe = GeneratedCanvasChromeFrameSpec {
            root_width: 2200,
            root_height: 0,
            sidebar_width: 0,
            sidebar_content_height: 0,
            content_gap: 0,
            scale_factor: 1.0,
        };
        let narrow_scaled_sidebar = GeneratedCanvasChromeFrameSpec {
            root_width: 459,
            root_height: 609,
            sidebar_width: 480,
            sidebar_content_height: 240,
            content_gap: 21,
            scale_factor: 2.0,
        };
        let wide_empty_sidebar = GeneratedCanvasChromeFrameSpec {
            root_width: current.root_width,
            root_height: current.root_height,
            sidebar_width: 0,
            sidebar_content_height: 0,
            content_gap: current.content_gap,
            scale_factor: current.scale_factor,
        };

        let retained_after_probe =
            retained_output_after_prefix(&[zero_height_probe, narrow_scaled_sidebar], current);
        let retained_after_different_sidebar =
            retained_output_after_prefix(&[wide_empty_sidebar], current);
        let mut fresh = LayoutEngine::new();
        let fresh_output = compute_canvas_chrome_frame_output(&mut fresh, current);

        assert_eq!(retained_after_probe, fresh_output);
        assert_eq!(retained_after_different_sidebar, fresh_output);
        assert_eq!(fresh_output.canvas_is_paintable, (true, true));
    })
    .settings(hegel_settings(64))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn taffy_repeated_root_solve_updates_percent_descendant_after_constraint_change() {
    use taffy::prelude::{Dimension as TaffyDimension, FromPercent};
    use taffy::{AvailableSpace as TaffyAvailableSpace, Size as TaffySize, Style as TaffyStyle};

    let mut taffy: taffy::TaffyTree<TaffySize<f32>> = taffy::TaffyTree::new();
    taffy.disable_rounding();

    let measured = taffy
        .new_leaf_with_context(
            TaffyStyle::default(),
            TaffySize {
                width: 0.0,
                height: 1.0,
            },
        )
        .unwrap();
    let full_leaf = taffy
        .new_leaf(TaffyStyle {
            size: TaffySize {
                width: TaffyDimension::from_percent(1.0_f32),
                height: TaffyDimension::from_percent(1.0_f32),
            },
            ..Default::default()
        })
        .unwrap();
    let parent = taffy
        .new_with_children(TaffyStyle::default(), &[measured, full_leaf])
        .unwrap();
    let root = taffy
        .new_with_children(
            TaffyStyle {
                display: taffy::Display::Flex,
                flex_direction: taffy::FlexDirection::Row,
                flex_wrap: taffy::FlexWrap::NoWrap,
                size: TaffySize {
                    width: TaffyDimension::from_percent(1.0_f32),
                    height: TaffyDimension::from_percent(1.0_f32),
                },
                ..Default::default()
            },
            &[parent],
        )
        .unwrap();

    for (available_width, available_height, expected_full_height) in
        [(1.0, 1.0, 1.0), (1.0, 2.0, 2.0), (1.0, 1.0, 1.0)]
    {
        taffy
            .compute_layout_with_measure(
                root,
                TaffySize {
                    width: TaffyAvailableSpace::Definite(available_width),
                    height: TaffyAvailableSpace::Definite(available_height),
                },
                |known_dimensions, available_space, _node_id, node_context, _style| {
                    let fallback = node_context
                        .map(|context| (context.width, context.height))
                        .unwrap_or((0.0, 0.0));
                    TaffySize {
                        width: known_dimensions.width.unwrap_or_else(|| {
                            match available_space.width {
                                TaffyAvailableSpace::Definite(width) => fallback.0.min(width),
                                TaffyAvailableSpace::MinContent
                                | TaffyAvailableSpace::MaxContent => fallback.0,
                            }
                        }),
                        height: known_dimensions.height.unwrap_or_else(|| {
                            match available_space.height {
                                TaffyAvailableSpace::Definite(height) => fallback.1.min(height),
                                TaffyAvailableSpace::MinContent
                                | TaffyAvailableSpace::MaxContent => fallback.1,
                            }
                        }),
                    }
                },
            )
            .unwrap();

        assert_eq!(
            taffy.layout(full_leaf).unwrap().size.height,
            expected_full_height
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn retained_layout_matches_fresh_with_opaque_leaf_after_same_root_constraint_repeat(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let frame = GeneratedFrame {
        roots: vec![GeneratedTree::Unmeasured {
            style: GeneratedStyle::FlexRow { gap: 0, wrap: true },
            children: vec![GeneratedTree::Unmeasured {
                style: GeneratedStyle::Default,
                children: vec![GeneratedTree::Unmeasured {
                    style: GeneratedStyle::FixedWidth(0),
                    children: vec![GeneratedTree::Opaque { width: 0 }],
                }],
            }],
        }],
    };
    let mut retained = LayoutEngine::new();

    for (available_width, available_height) in [(1, 1), (1, 1), (1, 1)] {
        let retained_ids = request_generated_frame(&mut retained, &frame);
        let retained_roots = compute_generated_roots(
            cx,
            &mut retained,
            &retained_ids,
            available_width,
            available_height,
        );
        for root in &retained_ids {
            assert_facts_committed_exactly(&retained, *root);
        }

        let mut fresh = LayoutEngine::new();
        let fresh_ids = request_generated_frame(&mut fresh, &frame);
        let fresh_roots = compute_generated_roots(
            cx,
            &mut fresh,
            &fresh_ids,
            available_width,
            available_height,
        );
        for root in &fresh_ids {
            assert_facts_committed_exactly(&fresh, *root);
        }

        assert_eq!(
            retained_layout_shapes(&retained, &retained_roots),
            retained_layout_shapes(&fresh, &fresh_roots)
        );
        assert_eq!(
            retained_layout_projections(&retained, &retained_roots),
            retained_layout_projections(&fresh, &fresh_roots)
        );
        assert_eq!(
            retained_layout_bounds_trees(&mut retained, &retained_roots, 1.0),
            retained_layout_bounds_trees(&mut fresh, &fresh_roots, 1.0)
        );

        retained.finish_frame();
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_reusable_exact_repeat_emits_no_retained_mutations(cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let mut cx = cx.add_empty_window();
        let frame = draw_generated_frame(&tc, false, 3);
        let available_width = draw_u16(&tc, 1, 360);
        let available_height = draw_u16(&tc, 1, 240);
        let mut engine = LayoutEngine::new();

        let roots = request_generated_frame(&mut engine, &frame);
        compute_generated_roots(cx, &mut engine, &roots, available_width, available_height);
        engine.finish_frame();

        engine.reset_retained_mutation_sample_for_tests();
        let roots = request_generated_frame(&mut engine, &frame);
        compute_generated_roots(cx, &mut engine, &roots, available_width, available_height);
        let stable_sample = engine.finish_frame();

        assert_eq!(
            engine.retained_mutation_sample_for_tests(),
            expected_mutations(&frame.roots, &frame.roots)
        );
        assert_eq!(
            stable_sample.measured_layout_calls, 0,
            "stable generated facts should not force measured callbacks on the repeat frame"
        );
        assert_eq!(
            [
                stable_sample.retained_layout_creates,
                stable_sample.retained_layout_style_updates,
                stable_sample.retained_layout_child_list_updates,
                stable_sample.retained_layout_dirty_marks,
                stable_sample.retained_layout_measured_context_clears,
                stable_sample.retained_layout_removes,
                stable_sample.retained_layout_miss_no_previous,
                stable_sample.retained_layout_miss_style,
                stable_sample.retained_layout_miss_kind,
                stable_sample.retained_layout_miss_measured_kind,
                stable_sample.retained_layout_miss_child_count,
                stable_sample.retained_layout_miss_child_subtree,
                stable_sample.retained_layout_miss_no_exact_child,
            ],
            [0; 13],
            "stable generated facts should not mutate retained layout on the repeat frame"
        );
    })
    .settings(hegel_settings(100))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_rollback_restores_retained_layout_state_for_next_repeat(cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let mut cx = cx.add_empty_window();
        let prefix = draw_generated_frame(&tc, false, 3);
        let real = draw_generated_frame(&tc, false, 3);
        let transient = draw_generated_frame(&tc, true, 3);
        let available_width = draw_u16(&tc, 1, 360);
        let available_height = draw_u16(&tc, 1, 240);

        let mut with_rollback = LayoutEngine::new();
        let prefix_roots = request_generated_frame(&mut with_rollback, &prefix);
        compute_generated_roots(
            cx,
            &mut with_rollback,
            &prefix_roots,
            available_width,
            available_height,
        );
        with_rollback.finish_frame();
        with_rollback.reset_retained_mutation_sample_for_tests();

        let checkpoint = with_rollback.checkpoint();
        let transient_roots = request_generated_frame(&mut with_rollback, &transient);
        compute_generated_roots(
            cx,
            &mut with_rollback,
            &transient_roots,
            available_width,
            available_height,
        );
        with_rollback.rollback_to_checkpoint(checkpoint);

        let real_roots = request_generated_frame(&mut with_rollback, &real);
        let with_rollback_roots = compute_generated_roots(
            cx,
            &mut with_rollback,
            &real_roots,
            available_width,
            available_height,
        );

        let mut skipped = LayoutEngine::new();
        let prefix_roots = request_generated_frame(&mut skipped, &prefix);
        compute_generated_roots(
            cx,
            &mut skipped,
            &prefix_roots,
            available_width,
            available_height,
        );
        skipped.finish_frame();
        skipped.reset_retained_mutation_sample_for_tests();
        let real_roots = request_generated_frame(&mut skipped, &real);
        let skipped_roots = compute_generated_roots(
            cx,
            &mut skipped,
            &real_roots,
            available_width,
            available_height,
        );

        assert_eq!(
            retained_layout_shapes(&with_rollback, &with_rollback_roots),
            retained_layout_shapes(&skipped, &skipped_roots)
        );

        with_rollback.finish_frame();
        skipped.finish_frame();
        assert_eq!(
            with_rollback.retained_mutation_sample_for_tests(),
            skipped.retained_mutation_sample_for_tests()
        );

        with_rollback.reset_retained_mutation_sample_for_tests();
        let repeat_roots = request_generated_frame(&mut with_rollback, &real);
        compute_generated_roots(
            cx,
            &mut with_rollback,
            &repeat_roots,
            available_width,
            available_height,
        );
        with_rollback.finish_frame();

        assert_eq!(
            with_rollback.retained_mutation_sample_for_tests(),
            expected_mutations(&real.roots, &real.roots)
        );
    })
    .settings(hegel_settings(64))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
#[should_panic(expected = "retained layout root should be solved at most once per frame")]
fn same_frame_root_recompute_panics(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let mut engine = LayoutEngine::new();
    let child = request_full_leaf(&mut engine);
    let root = request_full_container(&mut engine, &[child]);

    cx.update(|window, app| {
        engine.compute_layout(
            root,
            size(
                AvailableSpace::Definite(px(100.0)),
                AvailableSpace::Definite(px(80.0)),
            ),
            window,
            app,
        );
        let _ = engine.layout_bounds(child);
        engine.compute_layout(
            root,
            size(
                AvailableSpace::Definite(px(200.0)),
                AvailableSpace::Definite(px(80.0)),
            ),
            window,
            app,
        );
    });
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_text_exact_repeat_hydrates_from_query_cache(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    hegel::Hegel::new(|tc| {
        let key_index = draw_u8(&tc, 0, 3);
        let width = draw_u16(&tc, 1, 240);
        let height = draw_u16(&tc, 1, 120);
        let measure_invocations = Rc::new(Cell::new(0));
        let hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
        let mut engine = LayoutEngine::new();

        let root = request_input_sensitive_text_measured(
            &mut engine,
            0,
            key_index,
            width,
            height,
            measure_invocations.clone(),
            hydrated_artifacts.clone(),
        );
        compute_generated_text_root(cx, &mut engine, root, width);
        engine.finish_frame();
        engine.reset_retained_mutation_sample_for_tests();

        let root = request_input_sensitive_text_measured(
            &mut engine,
            1,
            key_index,
            width,
            height,
            measure_invocations.clone(),
            hydrated_artifacts.clone(),
        );
        compute_generated_text_root(cx, &mut engine, root, width);

        assert_eq!(measure_invocations.get(), 1);
        assert_eq!(engine.layout_work_sample().measured_layout_calls, 0);
        assert_eq!(engine.layout_work_sample().solver_compute_layout_calls, 1);
        assert_eq!(
            hydrated_artifacts.borrow().as_slice(),
            [
                GeneratedTextHydration {
                    label: 0,
                    key_index,
                    width,
                    height,
                },
                GeneratedTextHydration {
                    label: 1,
                    key_index,
                    width,
                    height,
                },
            ]
        );
    })
    .settings(hegel_settings(64))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_text_descendant_hydrates_when_parent_final_layout_cache_hits(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let key_index = 0;
    let fallback_width = 80;
    let height = 18;
    let root_width = 120;
    let measure_invocations = Rc::new(Cell::new(0));
    let hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
    let mut engine = LayoutEngine::new();

    let text = request_input_sensitive_text_measured(
        &mut engine,
        0,
        key_index,
        fallback_width,
        height,
        measure_invocations.clone(),
        hydrated_artifacts.clone(),
    );
    let root = request_container(&mut engine, &[text]);
    compute_generated_text_root(cx, &mut engine, root, root_width);
    engine.finish_frame();
    let first_hydration = hydrated_artifacts.borrow()[0].clone();
    measure_invocations.set(0);
    hydrated_artifacts.borrow_mut().clear();

    let text = request_input_sensitive_text_measured(
        &mut engine,
        1,
        key_index,
        fallback_width,
        height,
        measure_invocations.clone(),
        hydrated_artifacts.clone(),
    );
    let root = request_container(&mut engine, &[text]);
    compute_generated_text_root(cx, &mut engine, root, root_width);

    assert_eq!(
        measure_invocations.get(),
        0,
        "unchanged text descendant should hydrate from the parent's final-layout cache hit"
    );
    assert_eq!(
        hydrated_artifacts.borrow().as_slice(),
        [GeneratedTextHydration {
            label: 1,
            key_index,
            width: first_hydration.width,
            height: first_hydration.height,
        }]
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_text_same_key_new_available_width_remeasures(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    hegel::Hegel::new(|tc| {
        let key_index = draw_u8(&tc, 0, 3);
        let first_width = draw_u16(&tc, 1, 200);
        let width_delta = draw_u16(&tc, 1, 40);
        let second_width = first_width + width_delta;
        let height = draw_u16(&tc, 1, 120);
        let measure_invocations = Rc::new(Cell::new(0));
        let hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
        let mut engine = LayoutEngine::new();

        let root = request_input_sensitive_text_measured(
            &mut engine,
            0,
            key_index,
            first_width,
            height,
            measure_invocations.clone(),
            hydrated_artifacts.clone(),
        );
        compute_generated_text_root(cx, &mut engine, root, first_width);
        engine.finish_frame();
        engine.reset_retained_mutation_sample_for_tests();

        let root = request_input_sensitive_text_measured(
            &mut engine,
            1,
            key_index,
            second_width,
            height,
            measure_invocations.clone(),
            hydrated_artifacts.clone(),
        );
        compute_generated_text_root(cx, &mut engine, root, second_width);

        assert_eq!(measure_invocations.get(), 2);
        assert_eq!(engine.layout_work_sample().measured_layout_calls, 1);
        assert_eq!(engine.layout_work_sample().solver_compute_layout_calls, 1);
        assert_eq!(
            hydrated_artifacts.borrow().as_slice(),
            [
                GeneratedTextHydration {
                    label: 0,
                    key_index,
                    width: first_width,
                    height,
                },
                GeneratedTextHydration {
                    label: 1,
                    key_index,
                    width: second_width,
                    height,
                },
            ]
        );
    })
    .settings(hegel_settings(64))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_text_same_key_new_parent_width_remeasures(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    hegel::Hegel::new(|tc| {
        let key_index = draw_u8(&tc, 0, 3);
        let fallback_width = draw_u16(&tc, 1, 200);
        let first_parent_width = fallback_width + draw_u16(&tc, 1, 40);
        let second_parent_width = first_parent_width + draw_u16(&tc, 1, 40);
        let root_width = second_parent_width + draw_u16(&tc, 1, 40);
        let height = draw_u16(&tc, 1, 120);
        let retained_measure_invocations = Rc::new(Cell::new(0));
        let retained_hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
        let mut engine = LayoutEngine::new();

        let text = request_input_sensitive_text_measured(
            &mut engine,
            0,
            key_index,
            fallback_width,
            height,
            retained_measure_invocations.clone(),
            retained_hydrated_artifacts.clone(),
        );
        let parent = engine.request_layout(
            style_with_width(first_parent_width as f32),
            px(16.0),
            1.0,
            &[text],
        );
        let root = request_container(&mut engine, &[parent]);
        compute_generated_text_root(cx, &mut engine, root, root_width);
        engine.finish_frame();
        retained_measure_invocations.set(0);
        retained_hydrated_artifacts.borrow_mut().clear();

        let text = request_input_sensitive_text_measured(
            &mut engine,
            1,
            key_index,
            fallback_width,
            height,
            retained_measure_invocations.clone(),
            retained_hydrated_artifacts.clone(),
        );
        let parent = engine.request_layout(
            style_with_width(second_parent_width as f32),
            px(16.0),
            1.0,
            &[text],
        );
        let root = request_container(&mut engine, &[parent]);
        compute_generated_text_root(cx, &mut engine, root, root_width);

        let fresh_measure_invocations = Rc::new(Cell::new(0));
        let fresh_hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
        let mut fresh_engine = LayoutEngine::new();
        let fresh_text = request_input_sensitive_text_measured(
            &mut fresh_engine,
            1,
            key_index,
            fallback_width,
            height,
            fresh_measure_invocations.clone(),
            fresh_hydrated_artifacts.clone(),
        );
        let fresh_parent = fresh_engine.request_layout(
            style_with_width(second_parent_width as f32),
            px(16.0),
            1.0,
            &[fresh_text],
        );
        let fresh_root = request_container(&mut fresh_engine, &[fresh_parent]);
        compute_generated_text_root(cx, &mut fresh_engine, fresh_root, root_width);

        assert_ne!(
            fresh_measure_invocations.get(),
            0,
            "fresh layout should observe the current-frame text measurement query"
        );
        assert_eq!(
            retained_hydrated_artifacts.borrow().as_slice(),
            fresh_hydrated_artifacts.borrow().as_slice(),
            "retained layout must hydrate text artifacts equivalent to a fresh current-frame layout"
        );
        assert_eq!(
            engine.layout_work_sample().solver_compute_layout_calls,
            1,
            "retained layout should still perform exactly one legal root solve"
        );
    })
    .settings(hegel_settings(64))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn text_same_key_new_parent_width_minimized_remeasures(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let key_index = 0;
    let fallback_width = 1;
    let first_parent_width = 2;
    let second_parent_width = 3;
    let root_width = 4;
    let height = 1;
    let retained_measure_invocations = Rc::new(Cell::new(0));
    let retained_hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
    let mut engine = LayoutEngine::new();

    let text = request_input_sensitive_text_measured(
        &mut engine,
        0,
        key_index,
        fallback_width,
        height,
        retained_measure_invocations.clone(),
        retained_hydrated_artifacts.clone(),
    );
    let parent = engine.request_layout(
        style_with_width(first_parent_width as f32),
        px(16.0),
        1.0,
        &[text],
    );
    let root = request_container(&mut engine, &[parent]);
    compute_generated_text_root(cx, &mut engine, root, root_width);
    engine.finish_frame();
    retained_measure_invocations.set(0);
    retained_hydrated_artifacts.borrow_mut().clear();

    let text = request_input_sensitive_text_measured(
        &mut engine,
        1,
        key_index,
        fallback_width,
        height,
        retained_measure_invocations.clone(),
        retained_hydrated_artifacts.clone(),
    );
    let parent = engine.request_layout(
        style_with_width(second_parent_width as f32),
        px(16.0),
        1.0,
        &[text],
    );
    let root = request_container(&mut engine, &[parent]);
    compute_generated_text_root(cx, &mut engine, root, root_width);

    assert_eq!(
        retained_hydrated_artifacts.borrow().as_slice(),
        [GeneratedTextHydration {
            label: 1,
            key_index,
            width: 2,
            height,
        }]
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn text_same_key_changed_flex_sibling_width_matches_fresh_hydration(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let key_index = 0;
    let fallback_width = 120;
    let height = 1;
    let root_width = 100;
    let retained_measure_invocations = Rc::new(Cell::new(0));
    let retained_hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
    let mut engine = LayoutEngine::new();

    let text = request_growing_input_sensitive_text_measured(
        &mut engine,
        0,
        key_index,
        fallback_width,
        height,
        retained_measure_invocations.clone(),
        retained_hydrated_artifacts.clone(),
    );
    let sibling = request_leaf(&mut engine, 10.0);
    let root = request_flex_container(&mut engine, &[text, sibling]);
    compute_generated_text_root(cx, &mut engine, root, root_width);
    engine.finish_frame();
    retained_measure_invocations.set(0);
    retained_hydrated_artifacts.borrow_mut().clear();

    let text = request_growing_input_sensitive_text_measured(
        &mut engine,
        1,
        key_index,
        fallback_width,
        height,
        retained_measure_invocations.clone(),
        retained_hydrated_artifacts.clone(),
    );
    let sibling = request_leaf(&mut engine, 30.0);
    let root = request_flex_container(&mut engine, &[text, sibling]);
    compute_generated_text_root(cx, &mut engine, root, root_width);

    let fresh_measure_invocations = Rc::new(Cell::new(0));
    let fresh_hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
    let mut fresh_engine = LayoutEngine::new();
    let fresh_text = request_growing_input_sensitive_text_measured(
        &mut fresh_engine,
        1,
        key_index,
        fallback_width,
        height,
        fresh_measure_invocations.clone(),
        fresh_hydrated_artifacts.clone(),
    );
    let fresh_sibling = request_leaf(&mut fresh_engine, 30.0);
    let fresh_root = request_flex_container(&mut fresh_engine, &[fresh_text, fresh_sibling]);
    compute_generated_text_root(cx, &mut fresh_engine, fresh_root, root_width);

    assert_ne!(
        fresh_measure_invocations.get(),
        0,
        "fresh layout should observe the current-frame text measurement query"
    );
    assert_eq!(
        retained_hydrated_artifacts.borrow().as_slice(),
        fresh_hydrated_artifacts.borrow().as_slice(),
        "retained text hydration should match fresh solver measurement semantics"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_text_does_not_replay_stale_artifact_after_multiple_measure_queries(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    hegel::Hegel::new(|tc| {
        let key_index = draw_u8(&tc, 0, 3);
        let first_width = draw_u16(&tc, 1, 200);
        let width_delta = draw_u16(&tc, 1, 40);
        let second_width = first_width + width_delta;
        let height = draw_u16(&tc, 1, 120);
        let measure_invocations = Rc::new(Cell::new(0));
        let hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
        let mut engine = LayoutEngine::new();

        let root = request_input_sensitive_text_measured(
            &mut engine,
            0,
            key_index,
            first_width,
            height,
            measure_invocations.clone(),
            hydrated_artifacts.clone(),
        );
        compute_generated_text_root(cx, &mut engine, root, first_width);
        engine.finish_frame();

        let root = request_input_sensitive_text_measured(
            &mut engine,
            0,
            key_index,
            first_width,
            height,
            measure_invocations.clone(),
            hydrated_artifacts.clone(),
        );
        compute_generated_text_root(cx, &mut engine, root, second_width);
        engine.finish_frame();

        let root = request_input_sensitive_text_measured(
            &mut engine,
            1,
            key_index,
            first_width,
            height,
            measure_invocations.clone(),
            hydrated_artifacts.clone(),
        );
        compute_generated_text_root(cx, &mut engine, root, first_width);

        assert_eq!(measure_invocations.get(), 3);
        assert_eq!(
            hydrated_artifacts.borrow().as_slice(),
            [
                GeneratedTextHydration {
                    label: 0,
                    key_index,
                    width: first_width,
                    height,
                },
                GeneratedTextHydration {
                    label: 0,
                    key_index,
                    width: second_width,
                    height,
                },
                GeneratedTextHydration {
                    label: 1,
                    key_index,
                    width: first_width,
                    height,
                },
            ]
        );
    })
    .settings(hegel_settings(64))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
#[should_panic(expected = "retained layout root should be solved at most once per frame")]
fn text_same_frame_root_recompute_panics(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let measure_invocations = Rc::new(Cell::new(0));
    let hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
    let mut engine = LayoutEngine::new();
    let root = request_input_sensitive_text_measured(
        &mut engine,
        0,
        1,
        120,
        24,
        measure_invocations,
        hydrated_artifacts,
    );

    cx.update(|window, app| {
        engine.compute_layout(
            root,
            size(
                AvailableSpace::Definite(px(120.0)),
                AvailableSpace::MaxContent,
            ),
            window,
            app,
        );
        engine.compute_layout(
            root,
            size(
                AvailableSpace::Definite(px(180.0)),
                AvailableSpace::MaxContent,
            ),
            window,
            app,
        );
    });
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_text_rollback_discards_transient_measurement_state(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    hegel::Hegel::new(|tc| {
        let key_index = draw_u8(&tc, 0, 3);
        let transient_key_index = (key_index + 1) % 4;
        let width = draw_u16(&tc, 1, 200);
        let transient_width = width + draw_u16(&tc, 1, 40);
        let height = draw_u16(&tc, 1, 120);
        let measure_invocations = Rc::new(Cell::new(0));
        let hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
        let mut engine = LayoutEngine::new();

        let root = request_input_sensitive_text_measured(
            &mut engine,
            0,
            key_index,
            width,
            height,
            measure_invocations.clone(),
            hydrated_artifacts.clone(),
        );
        compute_generated_text_root(cx, &mut engine, root, width);
        engine.finish_frame();
        engine.reset_retained_mutation_sample_for_tests();

        let retry_root = request_input_sensitive_text_measured(
            &mut engine,
            2,
            key_index,
            width,
            height,
            measure_invocations.clone(),
            hydrated_artifacts.clone(),
        );
        let checkpoint = engine.checkpoint();
        let transient_root = request_input_sensitive_text_measured(
            &mut engine,
            1,
            transient_key_index,
            transient_width,
            height,
            measure_invocations.clone(),
            hydrated_artifacts.clone(),
        );
        compute_generated_text_root(cx, &mut engine, transient_root, transient_width);
        engine.rollback_to_checkpoint(checkpoint);
        compute_generated_text_root(cx, &mut engine, retry_root, width);
        engine.finish_frame();
        engine.reset_retained_mutation_sample_for_tests();

        let repeat_root = request_input_sensitive_text_measured(
            &mut engine,
            3,
            key_index,
            width,
            height,
            measure_invocations.clone(),
            hydrated_artifacts.clone(),
        );
        compute_generated_text_root(cx, &mut engine, repeat_root, width);

        assert_eq!(measure_invocations.get(), 2);
        assert_eq!(engine.layout_work_sample().measured_layout_calls, 0);
        assert_eq!(engine.layout_work_sample().solver_compute_layout_calls, 1);
        assert_eq!(
            hydrated_artifacts.borrow().as_slice(),
            [
                GeneratedTextHydration {
                    label: 0,
                    key_index,
                    width,
                    height,
                },
                GeneratedTextHydration {
                    label: 1,
                    key_index: transient_key_index,
                    width: transient_width,
                    height,
                },
                GeneratedTextHydration {
                    label: 2,
                    key_index,
                    width,
                    height,
                },
                GeneratedTextHydration {
                    label: 3,
                    key_index,
                    width,
                    height,
                },
            ]
        );
    })
    .settings(hegel_settings(64))
    .run();
}

#[test]
fn retained_commit_matches_fresh_commit_after_insert_delete_and_reorder() {
    let mut retained = LayoutEngine::new();
    let retained_first_root = request_row(&mut retained, &[10.0, 20.0, 30.0]);
    retained.commit_layout(retained_first_root);
    retained.finish_frame();

    let retained_second_root = request_row(&mut retained, &[30.0, 10.0, 40.0, 20.0]);
    let retained_root_node = retained.commit_layout(retained_second_root);
    assert_facts_committed_exactly(&retained, retained_second_root);

    let mut fresh = LayoutEngine::new();
    let fresh_root = request_row(&mut fresh, &[30.0, 10.0, 40.0, 20.0]);
    let fresh_root_node = fresh.commit_layout(fresh_root);

    assert_eq!(
        retained_layout_shape(&retained, retained_root_node),
        retained_layout_shape(&fresh, fresh_root_node)
    );
}

#[test]
fn unchanged_unmeasured_tree_emits_no_retained_mutations_on_second_frame() {
    let mut engine = LayoutEngine::new();
    let root = request_row(&mut engine, &[10.0, 20.0, 30.0]);
    engine.commit_layout(root);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let root = request_row(&mut engine, &[10.0, 20.0, 30.0]);
    engine.commit_layout(root);
    engine.finish_frame();

    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 4,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn changed_keyed_unmeasured_child_updates_retained_node_and_style() {
    let mut engine = LayoutEngine::new();
    let stable = request_leaf(&mut engine, 10.0);
    let changing = request_keyed_leaf(&mut engine, 21_001, 20.0);
    let first_root = request_container(&mut engine, &[stable, changing]);
    let first_root_node = engine.commit_layout(first_root);
    let first_child_nodes = engine.retained_child_tokens_for_tests(first_root_node);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let stable = request_leaf(&mut engine, 10.0);
    let changing = request_keyed_leaf(&mut engine, 21_001, 30.0);
    let second_root = request_container(&mut engine, &[stable, changing]);
    let second_root_node = engine.commit_layout(second_root);
    assert_facts_committed_exactly(&engine, second_root);
    let second_child_nodes = engine.retained_child_tokens_for_tests(second_root_node);

    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 3,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
    assert_eq!(second_root_node, first_root_node);
    assert_eq!(second_child_nodes[0], first_child_nodes[0]);
    assert_eq!(second_child_nodes[1], first_child_nodes[1]);

    let mut fresh = LayoutEngine::new();
    let fresh_root = request_row(&mut fresh, &[10.0, 30.0]);
    let fresh_root_node = fresh.commit_layout(fresh_root);
    assert_eq!(
        retained_layout_shape(&engine, second_root_node),
        retained_layout_shape(&fresh, fresh_root_node)
    );
}

#[test]
fn changed_unkeyed_same_position_child_reuses_storage_with_full_recommit() {
    let mut engine = LayoutEngine::new();
    let first_child = request_leaf(&mut engine, 10.0);
    let first_root = request_container(&mut engine, &[first_child]);
    let first_root_node = engine.commit_layout(first_root);
    let first_child_nodes = engine.retained_child_tokens_for_tests(first_root_node);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let second_child = request_leaf(&mut engine, 20.0);
    let second_root = request_container(&mut engine, &[second_child]);
    let second_root_node = engine.commit_layout(second_root);
    assert_facts_committed_exactly(&engine, second_root);
    let second_child_nodes = engine.retained_child_tokens_for_tests(second_root_node);

    assert_eq!(second_root_node, first_root_node);
    assert_eq!(
        second_child_nodes[0], first_child_nodes[0],
        "same-position anonymous storage may be reused when current facts are fully recommitted"
    );
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 2,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );

    let mut fresh = LayoutEngine::new();
    let fresh_child = request_leaf(&mut fresh, 20.0);
    let fresh_root = request_container(&mut fresh, &[fresh_child]);
    let fresh_root_node = fresh.commit_layout(fresh_root);
    assert_eq!(
        retained_layout_shape(&engine, second_root_node),
        retained_layout_shape(&fresh, fresh_root_node)
    );
}

#[test]
fn anonymous_wrapper_reuses_storage_while_keyed_descendant_changes() {
    fn request_frame(engine: &mut LayoutEngine, changing_width: f32) -> LayoutId {
        let stable_leaf = request_leaf(engine, 10.0);
        let changing_leaf = request_keyed_leaf(engine, 21_201, changing_width);
        let keyed_panel = request_keyed_layout(
            engine,
            21_202,
            Style::default(),
            &[stable_leaf, changing_leaf],
        );
        let anonymous_wrapper = request_container(engine, &[keyed_panel]);
        request_container(engine, &[anonymous_wrapper])
    }

    let mut engine = LayoutEngine::new();
    let first_root = request_frame(&mut engine, 20.0);
    let first_root_node = engine.commit_layout(first_root);
    let first_wrapper_node = engine.retained_child_tokens_for_tests(first_root_node)[0];
    let first_panel_node = engine.retained_child_tokens_for_tests(first_wrapper_node)[0];
    let first_panel_children = engine.retained_child_tokens_for_tests(first_panel_node);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let second_root = request_frame(&mut engine, 30.0);
    let second_root_node = engine.commit_layout(second_root);
    assert_facts_committed_exactly(&engine, second_root);
    let second_wrapper_node = engine.retained_child_tokens_for_tests(second_root_node)[0];
    let second_panel_node = engine.retained_child_tokens_for_tests(second_wrapper_node)[0];
    let second_panel_children = engine.retained_child_tokens_for_tests(second_panel_node);

    assert_eq!(second_root_node, first_root_node);
    assert_eq!(
        second_wrapper_node, first_wrapper_node,
        "anonymous wrapper storage should survive a descendant-only change"
    );
    assert_eq!(second_panel_node, first_panel_node);
    assert_eq!(second_panel_children, first_panel_children);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 5,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );

    let mut fresh = LayoutEngine::new();
    let fresh_root = request_frame(&mut fresh, 30.0);
    let fresh_root_node = fresh.commit_layout(fresh_root);
    assert_eq!(
        retained_layout_shape(&engine, second_root_node),
        retained_layout_shape(&fresh, fresh_root_node)
    );
}

#[test]
fn unique_global_id_reorder_preserves_semantic_child_node() {
    let mut engine = LayoutEngine::new();
    let first_a = request_keyed_leaf(&mut engine, 1, 10.0);
    let first_b = request_keyed_leaf(&mut engine, 2, 20.0);
    let first_root = request_flex_container(&mut engine, &[first_a, first_b]);
    let first_root_node = engine.commit_layout(first_root);
    let first_child_nodes = engine.retained_child_tokens_for_tests(first_root_node);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let second_b = request_keyed_leaf(&mut engine, 2, 30.0);
    let second_a = request_keyed_leaf(&mut engine, 1, 10.0);
    let second_root = request_flex_container(&mut engine, &[second_b, second_a]);
    let second_root_node = engine.commit_layout(second_root);
    assert_facts_committed_exactly(&engine, second_root);
    let second_child_nodes = engine.retained_child_tokens_for_tests(second_root_node);

    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 3,
            style_updates: 1,
            child_list_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
    assert_eq!(second_root_node, first_root_node);
    assert_eq!(second_child_nodes[0], first_child_nodes[1]);
    assert_eq!(second_child_nodes[1], first_child_nodes[0]);

    let mut fresh = LayoutEngine::new();
    let fresh_b = request_keyed_leaf(&mut fresh, 2, 30.0);
    let fresh_a = request_keyed_leaf(&mut fresh, 1, 10.0);
    let fresh_root = request_flex_container(&mut fresh, &[fresh_b, fresh_a]);
    let fresh_root_node = fresh.commit_layout(fresh_root);
    assert_eq!(
        retained_layout_shape(&engine, second_root_node),
        retained_layout_shape(&fresh, fresh_root_node)
    );
}

#[test]
fn duplicate_global_id_is_not_semantic_identity() {
    let mut engine = LayoutEngine::new();
    let first_a = request_keyed_leaf(&mut engine, 1, 10.0);
    let first_b = request_keyed_leaf(&mut engine, 1, 20.0);
    let first_root = request_flex_container(&mut engine, &[first_a, first_b]);
    let first_root_node = engine.commit_layout(first_root);
    let first_child_nodes = engine.retained_child_tokens_for_tests(first_root_node);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let second = request_keyed_leaf(&mut engine, 1, 20.0);
    let second_root = request_flex_container(&mut engine, &[second]);
    let second_root_node = engine.commit_layout(second_root);
    assert_facts_committed_exactly(&engine, second_root);
    let second_child = engine.retained_node_token_for_tests(second);

    assert_eq!(second_root_node, first_root_node);
    assert_eq!(
        second_child, first_child_nodes[1],
        "duplicate semantic ids are ignored; only the exact previous subtree may be reused"
    );
    assert_ne!(
        second_child, first_child_nodes[0],
        "duplicate semantic ids must not preserve the same-key changed sibling as history"
    );
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 2,
            child_list_updates: 1,
            removes: 1,
            ..RetainedForestMutationSample::default()
        }
    );

    let mut fresh = LayoutEngine::new();
    let fresh_child = request_keyed_leaf(&mut fresh, 1, 20.0);
    let fresh_root = request_flex_container(&mut fresh, &[fresh_child]);
    let fresh_root_node = fresh.commit_layout(fresh_root);
    assert_eq!(
        retained_layout_shape(&engine, second_root_node),
        retained_layout_shape(&fresh, fresh_root_node)
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn generated_duplicate_global_id_uses_exact_current_facts_not_scan_order_identity() {
    hegel::Hegel::new(|tc| {
        let sentinel_width = draw_u16(&tc, 1, 160);
        let first_duplicate_width = draw_u16(&tc, 1, 160);
        let second_duplicate_width_candidate = draw_u16(&tc, 1, 160);
        let second_duplicate_width = if second_duplicate_width_candidate == first_duplicate_width {
            if first_duplicate_width == 160 {
                159
            } else {
                first_duplicate_width + 1
            }
        } else {
            second_duplicate_width_candidate
        };
        let root_width = draw_u16(&tc, 240, 900) as f32;

        let mut retained = LayoutEngine::new();
        let sentinel = request_keyed_leaf(&mut retained, 40_001, sentinel_width as f32);
        let first_duplicate =
            request_keyed_leaf(&mut retained, 40_900, first_duplicate_width as f32);
        let second_duplicate =
            request_keyed_leaf(&mut retained, 40_900, second_duplicate_width as f32);
        let first_root =
            request_flex_container(&mut retained, &[sentinel, first_duplicate, second_duplicate]);
        let first_root_node =
            compute_layout_without_measure(&mut retained, first_root, root_width, 80.0);
        let first_duplicate_node = retained.retained_node_token_for_tests(first_duplicate);
        let second_duplicate_node = retained.retained_node_token_for_tests(second_duplicate);
        retained.finish_frame();

        retained.reset_retained_mutation_sample_for_tests();
        let current_duplicate =
            request_keyed_leaf(&mut retained, 40_900, second_duplicate_width as f32);
        let second_root = request_flex_container(&mut retained, &[current_duplicate]);
        let second_root_node =
            compute_layout_without_measure(&mut retained, second_root, root_width, 80.0);
        assert_facts_committed_exactly(&retained, second_root);

        let current_duplicate_node = retained.retained_node_token_for_tests(current_duplicate);
        assert_eq!(second_root_node, first_root_node);
        assert_eq!(
            current_duplicate_node, second_duplicate_node,
            "duplicate ids are not semantic proof; the exact current facts should select the matching retained node"
        );
        assert_ne!(
            current_duplicate_node, first_duplicate_node,
            "duplicate ids must not preserve the first same-key sibling by scan order"
        );
        assert_eq!(
            retained.retained_child_tokens_for_tests(second_root_node),
            vec![current_duplicate_node]
        );
        assert_eq!(
            retained.retained_parent_token_for_tests(current_duplicate_node),
            Some(second_root_node)
        );
        assert_eq!(
            retained.retained_mutation_sample_for_tests(),
            RetainedForestMutationSample {
                reuses: 2,
                child_list_updates: 1,
                removes: 2,
                ..RetainedForestMutationSample::default()
            }
        );

        let mut fresh = LayoutEngine::new();
        let fresh_duplicate =
            request_keyed_leaf(&mut fresh, 40_900, second_duplicate_width as f32);
        let fresh_root = request_flex_container(&mut fresh, &[fresh_duplicate]);
        let fresh_root = compute_layout_without_measure(&mut fresh, fresh_root, root_width, 80.0);
        assert_eq!(
            retained_layout_projection(&retained, second_root_node),
            retained_layout_projection(&fresh, fresh_root)
        );
        assert_eq!(
            retained_layout_bounds_tree(&mut retained, second_root_node, 1.0),
            retained_layout_bounds_tree(&mut fresh, fresh_root, 1.0)
        );
    })
    .settings(hegel_settings(64))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn generated_duplicate_global_id_reorder_delete_retains_by_exact_facts() {
    hegel::Hegel::new(|tc| {
        let duplicate_count = draw_usize(&tc, 2, 7);
        let base_width = draw_u16(&tc, 1, 40);
        let width_step = draw_u16(&tc, 1, 20);
        let previous_widths = (0..duplicate_count)
            .map(|index| base_width + index as u16 * width_step)
            .collect::<Vec<_>>();
        let start = draw_usize(&tc, 1, duplicate_count - 1);
        let current_len = draw_usize(&tc, 1, duplicate_count - 1);
        let reverse = draw_u8(&tc, 0, 1) == 1;
        let root_width = draw_u16(&tc, 240, 900) as f32;

        let mut current_previous_indices = (0..duplicate_count)
            .cycle()
            .skip(start)
            .take(current_len)
            .collect::<Vec<_>>();
        if reverse {
            current_previous_indices.reverse();
        }

        let mut retained = LayoutEngine::new();
        let previous_duplicates = previous_widths
            .iter()
            .map(|width| request_keyed_leaf(&mut retained, 41_900, *width as f32))
            .collect::<Vec<_>>();
        let first_root = request_flex_container(&mut retained, &previous_duplicates);
        let first_root_node =
            compute_layout_without_measure(&mut retained, first_root, root_width, 80.0);
        let previous_duplicate_nodes = previous_duplicates
            .iter()
            .map(|duplicate| retained.retained_node_token_for_tests(*duplicate))
            .collect::<Vec<_>>();
        retained.finish_frame();

        retained.reset_retained_mutation_sample_for_tests();
        let current_duplicates = current_previous_indices
            .iter()
            .map(|index| request_keyed_leaf(&mut retained, 41_900, previous_widths[*index] as f32))
            .collect::<Vec<_>>();
        let second_root = request_flex_container(&mut retained, &current_duplicates);
        let second_root_node =
            compute_layout_without_measure(&mut retained, second_root, root_width, 80.0);
        assert_facts_committed_exactly(&retained, second_root);

        assert_eq!(second_root_node, first_root_node);
        let current_duplicate_nodes = current_duplicates
            .iter()
            .map(|duplicate| retained.retained_node_token_for_tests(*duplicate))
            .collect::<Vec<_>>();
        let expected_duplicate_nodes = current_previous_indices
            .iter()
            .map(|index| previous_duplicate_nodes[*index])
            .collect::<Vec<_>>();
        assert_eq!(
            current_duplicate_nodes, expected_duplicate_nodes,
            "same global id cannot choose identity; moved duplicate siblings must retain only by exact current facts"
        );
        assert_eq!(
            retained.retained_child_tokens_for_tests(second_root_node),
            current_duplicate_nodes
        );
        assert_eq!(
            retained.retained_mutation_sample_for_tests(),
            RetainedForestMutationSample {
                reuses: current_len as u64 + 1,
                child_list_updates: 1,
                removes: duplicate_count as u64 - current_len as u64,
                ..RetainedForestMutationSample::default()
            }
        );

        let mut fresh = LayoutEngine::new();
        let fresh_duplicates = current_previous_indices
            .iter()
            .map(|index| request_keyed_leaf(&mut fresh, 41_900, previous_widths[*index] as f32))
            .collect::<Vec<_>>();
        let fresh_root = request_flex_container(&mut fresh, &fresh_duplicates);
        let fresh_root = compute_layout_without_measure(&mut fresh, fresh_root, root_width, 80.0);
        assert_eq!(
            retained_layout_projection(&retained, second_root_node),
            retained_layout_projection(&fresh, fresh_root)
        );
        assert_eq!(
            retained_layout_bounds_tree(&mut retained, second_root_node, 1.0),
            retained_layout_bounds_tree(&mut fresh, fresh_root, 1.0)
        );
    })
    .settings(hegel_settings(64))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn generated_semantic_sibling_sequences_match_fresh_across_constraint_aba() {
    hegel::Hegel::new(|tc| {
        let templates = draw_generated_semantic_child_templates(&tc);
        let frames = draw_generated_semantic_frames(&tc, templates.len());
        let first_width = draw_u16(&tc, 160, 900) as f32;
        let first_height = draw_u16(&tc, 48, 240) as f32;
        let second_width = draw_u16(&tc, 160, 900) as f32;
        let second_height = draw_u16(&tc, 48, 240) as f32;
        let constraints = [
            (first_width, first_height),
            (second_width, second_height),
            (first_width, first_height),
        ];
        let mut retained = LayoutEngine::new();

        for (frame_index, frame) in frames.iter().enumerate() {
            let (width, height) = constraints[frame_index % constraints.len()];
            let (retained_layout_id, retained_children) =
                request_generated_semantic_frame(&mut retained, &templates, frame);
            let retained_root =
                compute_layout_without_measure(&mut retained, retained_layout_id, width, height);
            assert_facts_committed_exactly(&retained, retained_layout_id);

            let retained_child_nodes = retained_children
                .iter()
                .map(|child| retained.retained_node_token_for_tests(*child))
                .collect::<Vec<_>>();
            let unique_retained_child_nodes = retained_child_nodes
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(
                unique_retained_child_nodes.len(),
                retained_child_nodes.len(),
                "current-frame layout facts must never alias one retained node"
            );

            let mut fresh = LayoutEngine::new();
            let (fresh_layout_id, _) =
                request_generated_semantic_frame(&mut fresh, &templates, frame);
            let fresh_root =
                compute_layout_without_measure(&mut fresh, fresh_layout_id, width, height);
            assert_facts_committed_exactly(&fresh, fresh_layout_id);

            assert_eq!(
                retained_layout_projection(&retained, retained_root),
                retained_layout_projection(&fresh, fresh_root)
            );
            assert_eq!(
                retained_layout_bounds_tree(&mut retained, retained_root, 1.0),
                retained_layout_bounds_tree(&mut fresh, fresh_root, 1.0)
            );

            retained.finish_frame();
        }
    })
    .settings(hegel_settings(96))
    .run();
}

#[test]
fn changed_measured_child_is_not_an_exact_reordered_match() {
    let mut engine = LayoutEngine::new();
    let first_unmeasured = request_leaf(&mut engine, 0.0);
    let first_measured = request_pure_list_measured(&mut engine, 0.0, 1.0);
    let first_root = request_container(&mut engine, &[first_unmeasured, first_measured]);
    let first_root_node = engine.commit_layout(first_root);
    let first_child_nodes = engine.retained_child_tokens_for_tests(first_root_node);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let second_measured = request_pure_list_measured(&mut engine, 0.0, 0.0);
    let second_root = request_container(&mut engine, &[second_measured]);
    let second_root_node = engine.commit_layout(second_root);
    assert_facts_committed_exactly(&engine, second_root);
    let second_child_nodes = engine.retained_child_tokens_for_tests(second_root_node);

    let mut fresh = LayoutEngine::new();
    let fresh_measured = request_pure_list_measured(&mut fresh, 0.0, 0.0);
    let fresh_root = request_container(&mut fresh, &[fresh_measured]);
    let fresh_root_node = fresh.commit_layout(fresh_root);
    assert_eq!(
        retained_layout_shape(&engine, second_root_node),
        retained_layout_shape(&fresh, fresh_root_node)
    );

    engine.finish_frame();

    assert_eq!(second_root_node, first_root_node);
    assert_ne!(second_child_nodes[0], first_child_nodes[1]);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            creates: 1,
            reuses: 1,
            child_list_updates: 1,
            context_clears: 1,
            removes: 2,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn changed_ancestor_reuses_unmeasured_path_and_updates_changed_leaf() {
    let mut engine = LayoutEngine::new();
    let stable_grandchild = request_leaf(&mut engine, 10.0);
    let changing_grandchild = request_keyed_leaf(&mut engine, 22_001, 20.0);
    let changed_child = request_keyed_layout(
        &mut engine,
        22_002,
        Style::default(),
        &[stable_grandchild, changing_grandchild],
    );
    let stable_sibling = request_leaf(&mut engine, 99.0);
    let first_root = request_container(&mut engine, &[changed_child, stable_sibling]);
    let first_root_node = engine.commit_layout(first_root);
    let first_child_nodes = engine.retained_child_tokens_for_tests(first_root_node);
    let first_changed_child_node = first_child_nodes[0];
    let first_stable_sibling_node = first_child_nodes[1];
    let first_grandchild_nodes = engine.retained_child_tokens_for_tests(first_changed_child_node);
    let first_stable_grandchild_node = first_grandchild_nodes[0];
    let first_changing_grandchild_node = first_grandchild_nodes[1];
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let stable_grandchild = request_leaf(&mut engine, 10.0);
    let changing_grandchild = request_keyed_leaf(&mut engine, 22_001, 30.0);
    let changed_child = request_keyed_layout(
        &mut engine,
        22_002,
        Style::default(),
        &[stable_grandchild, changing_grandchild],
    );
    let stable_sibling = request_leaf(&mut engine, 99.0);
    let second_root = request_container(&mut engine, &[changed_child, stable_sibling]);
    let second_root_node = engine.commit_layout(second_root);
    assert_facts_committed_exactly(&engine, second_root);

    let second_child_nodes = engine.retained_child_tokens_for_tests(second_root_node);
    let second_changed_child_node = second_child_nodes[0];
    let second_stable_sibling_node = second_child_nodes[1];
    let second_grandchild_nodes = engine.retained_child_tokens_for_tests(second_changed_child_node);
    let second_stable_grandchild_node = second_grandchild_nodes[0];
    let second_changing_grandchild_node = second_grandchild_nodes[1];

    assert_eq!(second_root_node, first_root_node);
    assert_eq!(second_changed_child_node, first_changed_child_node);
    assert_eq!(second_stable_sibling_node, first_stable_sibling_node);
    assert_eq!(second_stable_grandchild_node, first_stable_grandchild_node);
    assert_eq!(
        second_changing_grandchild_node,
        first_changing_grandchild_node
    );
    assert_eq!(
        engine.retained_parent_token_for_tests(second_stable_grandchild_node),
        Some(second_changed_child_node)
    );
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 5,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );

    let mut fresh = LayoutEngine::new();
    let stable_grandchild = request_leaf(&mut fresh, 10.0);
    let changing_grandchild = request_leaf(&mut fresh, 30.0);
    let changed_child = request_container(&mut fresh, &[stable_grandchild, changing_grandchild]);
    let stable_sibling = request_leaf(&mut fresh, 99.0);
    let fresh_root = request_container(&mut fresh, &[changed_child, stable_sibling]);
    let fresh_root_node = fresh.commit_layout(fresh_root);
    assert_eq!(
        retained_layout_shape(&engine, second_root_node),
        retained_layout_shape(&fresh, fresh_root_node)
    );

    engine.finish_frame();
    assert_eq!(
        engine.retained_parent_token_for_tests(second_stable_grandchild_node),
        Some(second_changed_child_node)
    );
}

#[test]
fn semantic_reuse_updates_changed_descendant_geometry_without_child_list_mutation() {
    let mut retained = LayoutEngine::new();
    let stable_grandchild = request_leaf(&mut retained, 10.0);
    let changing_grandchild = request_keyed_leaf(&mut retained, 23_001, 20.0);
    let changed_child = request_keyed_layout(
        &mut retained,
        23_002,
        Style::default(),
        &[stable_grandchild, changing_grandchild],
    );
    let first_root = request_flex_container(&mut retained, &[changed_child]);
    compute_layout_without_measure(&mut retained, first_root, 300.0, 100.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let stable_grandchild = request_leaf(&mut retained, 10.0);
    let changing_grandchild = request_keyed_leaf(&mut retained, 23_001, 80.0);
    let changed_child = request_keyed_layout(
        &mut retained,
        23_002,
        Style::default(),
        &[stable_grandchild, changing_grandchild],
    );
    let second_root = request_flex_container(&mut retained, &[changed_child]);
    let retained_root_node =
        compute_layout_without_measure(&mut retained, second_root, 300.0, 100.0);
    assert_facts_committed_exactly(&retained, second_root);

    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 4,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );

    let mut fresh = LayoutEngine::new();
    let stable_grandchild = request_leaf(&mut fresh, 10.0);
    let changing_grandchild = request_leaf(&mut fresh, 80.0);
    let changed_child = request_container(&mut fresh, &[stable_grandchild, changing_grandchild]);
    let fresh_root = request_flex_container(&mut fresh, &[changed_child]);
    let fresh_root_node = compute_layout_without_measure(&mut fresh, fresh_root, 300.0, 100.0);

    assert_eq!(
        retained_layout_projection(&retained, retained_root_node),
        retained_layout_projection(&fresh, fresh_root_node)
    );
    assert_eq!(
        retained_layout_bounds_tree(&mut retained, retained_root_node, 1.0),
        retained_layout_bounds_tree(&mut fresh, fresh_root_node, 1.0)
    );
}

#[test]
fn hovered_swatch_style_change_reuses_appearance_subtree_and_stable_siblings() {
    fn request_appearance_swatch_panel(
        engine: &mut LayoutEngine,
        hovered_swatch_width: f32,
    ) -> (LayoutId, LayoutId, LayoutId, Vec<LayoutId>) {
        let swatches = (0..5)
            .map(|index| {
                let width = if index == 2 {
                    hovered_swatch_width
                } else {
                    24.0
                };
                request_keyed_leaf(engine, 31_100 + index, width)
            })
            .collect::<Vec<_>>();
        let mut swatch_row_style = Style::default();
        swatch_row_style.display = Display::Flex;
        swatch_row_style.flex_direction = FlexDirection::Row;
        let swatch_row = request_keyed_layout(engine, 31_010, swatch_row_style, &swatches);
        let appearance_panel =
            request_keyed_layout(engine, 31_001, Style::default(), &[swatch_row]);
        let root = request_container(engine, &[appearance_panel]);
        (root, appearance_panel, swatch_row, swatches)
    }

    let mut retained = LayoutEngine::new();
    let (first_root, first_panel, first_swatch_row, first_swatches) =
        request_appearance_swatch_panel(&mut retained, 24.0);
    let first_root_node = compute_layout_without_measure(&mut retained, first_root, 320.0, 100.0);
    let first_panel_node = retained.retained_node_token_for_tests(first_panel);
    let first_swatch_row_node = retained.retained_node_token_for_tests(first_swatch_row);
    let first_swatch_nodes = first_swatches
        .iter()
        .map(|swatch| retained.retained_node_token_for_tests(*swatch))
        .collect::<Vec<_>>();
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let (second_root, second_panel, second_swatch_row, second_swatches) =
        request_appearance_swatch_panel(&mut retained, 32.0);
    let retained_root_node =
        compute_layout_without_measure(&mut retained, second_root, 320.0, 100.0);
    assert_facts_committed_exactly(&retained, second_root);

    assert_eq!(retained_root_node, first_root_node);
    assert_eq!(
        retained.retained_node_token_for_tests(second_panel),
        first_panel_node
    );
    assert_eq!(
        retained.retained_node_token_for_tests(second_swatch_row),
        first_swatch_row_node
    );
    assert_eq!(
        second_swatches
            .iter()
            .map(|swatch| retained.retained_node_token_for_tests(*swatch))
            .collect::<Vec<_>>(),
        first_swatch_nodes
    );
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 8,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );

    let mut fresh = LayoutEngine::new();
    let (fresh_root, _, _, _) = request_appearance_swatch_panel(&mut fresh, 32.0);
    let fresh_root_node = compute_layout_without_measure(&mut fresh, fresh_root, 320.0, 100.0);

    assert_eq!(
        retained_layout_projection(&retained, retained_root_node),
        retained_layout_projection(&fresh, fresh_root_node)
    );
    assert_eq!(
        retained_layout_bounds_tree(&mut retained, retained_root_node, 1.0),
        retained_layout_bounds_tree(&mut fresh, fresh_root_node, 1.0)
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn generated_dynamic_sibling_keeps_stable_swatch_panel_local() {
    fn request_panel_with_dynamic_sibling(
        engine: &mut LayoutEngine,
        swatch_widths: &[u16],
        dynamic_width: u16,
    ) -> (LayoutId, LayoutId, LayoutId, Vec<LayoutId>, LayoutId) {
        let swatches = swatch_widths
            .iter()
            .enumerate()
            .map(|(index, width)| request_keyed_leaf(engine, 32_100 + index as u64, *width as f32))
            .collect::<Vec<_>>();

        let mut swatch_row_style = Style::default();
        swatch_row_style.display = Display::Flex;
        swatch_row_style.flex_direction = FlexDirection::Row;
        swatch_row_style.gap = size(definite_px(4.0), definite_px(4.0));
        let swatch_row = request_keyed_layout(engine, 32_010, swatch_row_style, &swatches);
        let panel = request_keyed_layout(engine, 32_001, Style::default(), &[swatch_row]);
        let dynamic = request_keyed_leaf(engine, 32_900, dynamic_width as f32);
        let root = request_flex_container(engine, &[panel, dynamic]);

        (root, panel, swatch_row, swatches, dynamic)
    }

    hegel::Hegel::new(|tc| {
        let swatch_count = draw_usize(&tc, 1, 8);
        let swatch_widths = (0..swatch_count)
            .map(|_| draw_u16(&tc, 1, 72))
            .collect::<Vec<_>>();
        let first_dynamic_width = draw_u16(&tc, 1, 160);
        let second_dynamic_width = if first_dynamic_width == 160 {
            159
        } else {
            first_dynamic_width + 1
        };
        let root_width = draw_u16(&tc, 240, 900) as f32;

        let mut retained = LayoutEngine::new();
        let (first_root, first_panel, first_swatch_row, first_swatches, _) =
            request_panel_with_dynamic_sibling(&mut retained, &swatch_widths, first_dynamic_width);
        compute_layout_without_measure(&mut retained, first_root, root_width, 160.0);
        let first_panel_node = retained.retained_node_token_for_tests(first_panel);
        let first_swatch_row_node = retained.retained_node_token_for_tests(first_swatch_row);
        let first_swatch_nodes = first_swatches
            .iter()
            .map(|swatch| retained.retained_node_token_for_tests(*swatch))
            .collect::<Vec<_>>();
        retained.finish_frame();

        retained.reset_retained_mutation_sample_for_tests();
        let (second_root, second_panel, second_swatch_row, second_swatches, _) =
            request_panel_with_dynamic_sibling(&mut retained, &swatch_widths, second_dynamic_width);
        let retained_root =
            compute_layout_without_measure(&mut retained, second_root, root_width, 160.0);
        assert_facts_committed_exactly(&retained, second_root);

        assert_eq!(
            retained.retained_node_token_for_tests(second_panel),
            first_panel_node
        );
        assert_eq!(
            retained.retained_node_token_for_tests(second_swatch_row),
            first_swatch_row_node
        );
        assert_eq!(
            second_swatches
                .iter()
                .map(|swatch| retained.retained_node_token_for_tests(*swatch))
                .collect::<Vec<_>>(),
            first_swatch_nodes
        );
        assert_eq!(
            retained.retained_mutation_sample_for_tests(),
            RetainedForestMutationSample {
                reuses: swatch_count as u64 + 4,
                style_updates: 1,
                ..RetainedForestMutationSample::default()
            }
        );

        let mut fresh = LayoutEngine::new();
        let (fresh_root, _, _, _, _) =
            request_panel_with_dynamic_sibling(&mut fresh, &swatch_widths, second_dynamic_width);
        let fresh_root = compute_layout_without_measure(&mut fresh, fresh_root, root_width, 160.0);
        assert_eq!(
            retained_layout_projection(&retained, retained_root),
            retained_layout_projection(&fresh, fresh_root)
        );
        assert_eq!(
            retained_layout_bounds_tree(&mut retained, retained_root, 1.0),
            retained_layout_bounds_tree(&mut fresh, fresh_root, 1.0)
        );
    })
    .settings(hegel_settings(64))
    .run();
}

#[test]
fn rollback_discards_failed_transaction_root_slots() {
    let mut engine = LayoutEngine::new();
    let root = request_row(&mut engine, &[10.0, 20.0, 30.0]);
    engine.commit_layout(root);
    engine.finish_frame();

    let checkpoint = engine.checkpoint();
    let transient_root = request_measured(&mut engine, 5.0);
    engine.commit_layout(transient_root);
    engine.rollback_to_checkpoint(checkpoint);

    let root = request_row(&mut engine, &[10.0, 20.0, 30.0]);
    engine.commit_layout(root);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let root = request_row(&mut engine, &[10.0, 20.0, 30.0]);
    engine.commit_layout(root);
    engine.finish_frame();

    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 4,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn rollback_preserves_precheckpoint_measured_producer_for_retry() {
    let mut engine = LayoutEngine::new();
    let drops = Rc::new(Cell::new(0));
    let measured = request_counted_measured(&mut engine, drops.clone());

    let checkpoint = engine.checkpoint();
    let transient_node = engine.commit_layout(measured);
    assert!(engine.retained_node_has_measure_context_for_tests(transient_node));
    engine.rollback_to_checkpoint(checkpoint);

    assert_eq!(drops.get(), 0);
    let retried_node = engine.commit_layout(measured);
    assert!(engine.retained_node_has_measure_context_for_tests(retried_node));
    engine.finish_frame();

    assert_eq!(drops.get(), 1);
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
    let mut retained = LayoutEngine::new();

    for widths in frames {
        let retained_root = request_row(&mut retained, widths);
        let retained_root_node = retained.commit_layout(retained_root);
        assert_facts_committed_exactly(&retained, retained_root);

        let mut fresh = LayoutEngine::new();
        let fresh_root = request_row(&mut fresh, widths);
        let fresh_root_node = fresh.commit_layout(fresh_root);
        assert_facts_committed_exactly(&fresh, fresh_root);

        assert_eq!(
            retained_layout_shape(&retained, retained_root_node),
            retained_layout_shape(&fresh, fresh_root_node)
        );

        retained.finish_frame();
    }
}

#[test]
fn retained_layout_recomputes_after_child_delete_from_content_sized_parent() {
    let mut retained = LayoutEngine::new();
    let root = request_flex_row(&mut retained, &[100.0, 200.0]);
    compute_layout_without_measure_with_available_space(
        &mut retained,
        root,
        AvailableSpace::MaxContent,
        AvailableSpace::Definite(px(100.0)),
        1.0,
    );
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let root = request_flex_row(&mut retained, &[100.0]);
    let retained_root = compute_layout_without_measure_with_available_space(
        &mut retained,
        root,
        AvailableSpace::MaxContent,
        AvailableSpace::Definite(px(100.0)),
        1.0,
    );

    let mut fresh = LayoutEngine::new();
    let fresh_root = request_flex_row(&mut fresh, &[100.0]);
    let fresh_root = compute_layout_without_measure_with_available_space(
        &mut fresh,
        fresh_root,
        AvailableSpace::MaxContent,
        AvailableSpace::Definite(px(100.0)),
        1.0,
    );

    assert_eq!(
        (
            retained_node_size(&retained, retained_root),
            retained_node_size(&fresh, fresh_root),
        ),
        (size(100.0, 0.0), size(100.0, 0.0))
    );
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 2,
            child_list_updates: 1,
            removes: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn unique_global_id_style_change_updates_retained_node_and_matches_fresh_layout() {
    let mut retained = LayoutEngine::new();
    let stable = request_leaf(&mut retained, 10.0);
    let changing = request_keyed_leaf(&mut retained, 23_001, 20.0);
    let first_root = request_flex_container(&mut retained, &[stable, changing]);
    let first_root_node = compute_layout_without_measure(&mut retained, first_root, 240.0, 80.0);
    let first_stable_node = retained.retained_node_token_for_tests(stable);
    let first_changing_node = retained.retained_node_token_for_tests(changing);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let stable = request_leaf(&mut retained, 10.0);
    let changing = request_keyed_leaf(&mut retained, 23_001, 30.0);
    let second_root = request_flex_container(&mut retained, &[stable, changing]);
    let retained_stable = stable;
    let retained_changing = changing;
    let retained_root = compute_layout_without_measure(&mut retained, second_root, 240.0, 80.0);

    let mut fresh = LayoutEngine::new();
    let stable = request_leaf(&mut fresh, 10.0);
    let changing = request_leaf(&mut fresh, 30.0);
    let fresh_root = request_flex_container(&mut fresh, &[stable, changing]);
    let fresh_root = compute_layout_without_measure(&mut fresh, fresh_root, 240.0, 80.0);

    assert_facts_committed_exactly(&retained, second_root);
    assert_eq!(retained_root, first_root_node);
    assert_eq!(
        retained.retained_node_token_for_tests(retained_stable),
        first_stable_node
    );
    assert_eq!(
        retained.retained_node_token_for_tests(retained_changing),
        first_changing_node
    );
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        retained_layout_bounds_trees(&mut retained, &[retained_root], 1.0),
        retained_layout_bounds_trees(&mut fresh, &[fresh_root], 1.0)
    );
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 3,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn same_index_exact_child_is_not_stolen_by_changed_equal_sibling() {
    let mut retained = LayoutEngine::new();
    let changing = request_keyed_leaf(&mut retained, 24_001, 10.0);
    let stable = request_leaf(&mut retained, 99.0);
    let first_root = request_flex_container(&mut retained, &[changing, stable]);
    let first_root_node = compute_layout_without_measure(&mut retained, first_root, 240.0, 80.0);
    let first_changing_node = retained.retained_node_token_for_tests(changing);
    let first_stable_node = retained.retained_node_token_for_tests(stable);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let changing = request_keyed_leaf(&mut retained, 24_001, 99.0);
    let stable = request_leaf(&mut retained, 99.0);
    let second_root = request_flex_container(&mut retained, &[changing, stable]);
    let retained_changing = changing;
    let retained_stable = stable;
    let retained_root = compute_layout_without_measure(&mut retained, second_root, 240.0, 80.0);

    let mut fresh = LayoutEngine::new();
    let changing = request_leaf(&mut fresh, 99.0);
    let stable = request_leaf(&mut fresh, 99.0);
    let fresh_root = request_flex_container(&mut fresh, &[changing, stable]);
    let fresh_root = compute_layout_without_measure(&mut fresh, fresh_root, 240.0, 80.0);

    assert_facts_committed_exactly(&retained, second_root);
    assert_eq!(retained_root, first_root_node);
    assert_eq!(
        retained.retained_node_token_for_tests(retained_changing),
        first_changing_node
    );
    assert_eq!(
        retained.retained_node_token_for_tests(retained_stable),
        first_stable_node
    );
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        retained_layout_bounds_trees(&mut retained, &[retained_root], 1.0),
        retained_layout_bounds_trees(&mut fresh, &[fresh_root], 1.0)
    );
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 3,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn flex_parent_reorder_reuses_exact_children_and_matches_fresh_layout() {
    let mut retained = LayoutEngine::new();
    let a = request_leaf(&mut retained, 10.0);
    let b = request_leaf(&mut retained, 20.0);
    let first_root = request_flex_container(&mut retained, &[a, b]);
    compute_layout_without_measure(&mut retained, first_root, 240.0, 80.0);
    let first_a_node = retained.retained_node_token_for_tests(a);
    let first_b_node = retained.retained_node_token_for_tests(b);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let b = request_leaf(&mut retained, 20.0);
    let c = request_leaf(&mut retained, 30.0);
    let a = request_leaf(&mut retained, 10.0);
    let second_root = request_flex_container(&mut retained, &[b, c, a]);
    let retained_root = compute_layout_without_measure(&mut retained, second_root, 240.0, 80.0);

    let mut fresh = LayoutEngine::new();
    let b = request_leaf(&mut fresh, 20.0);
    let c = request_leaf(&mut fresh, 30.0);
    let a = request_leaf(&mut fresh, 10.0);
    let fresh_root = request_flex_container(&mut fresh, &[b, c, a]);
    let fresh_root = compute_layout_without_measure(&mut fresh, fresh_root, 240.0, 80.0);

    assert_facts_committed_exactly(&retained, second_root);
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(retained.retained_node_token_for_tests(a), first_a_node);
    assert_eq!(retained.retained_node_token_for_tests(b), first_b_node);
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            creates: 1,
            reuses: 3,
            child_list_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn grid_parent_reorder_reuses_exact_children_and_matches_fresh_layout() {
    let mut retained = LayoutEngine::new();
    let a = request_grid_item(&mut retained, 10.0, 1, 1);
    let b = request_grid_item(&mut retained, 20.0, 2, 1);
    let first_root = request_grid_container(&mut retained, &[a, b]);
    compute_layout_without_measure(&mut retained, first_root, 240.0, 80.0);
    let first_a_node = retained.retained_node_token_for_tests(a);
    let first_b_node = retained.retained_node_token_for_tests(b);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let b = request_grid_item(&mut retained, 20.0, 2, 1);
    let c = request_grid_item(&mut retained, 30.0, 3, 1);
    let a = request_grid_item(&mut retained, 10.0, 1, 1);
    let second_root = request_grid_container(&mut retained, &[b, c, a]);
    let retained_root = compute_layout_without_measure(&mut retained, second_root, 240.0, 80.0);

    let mut fresh = LayoutEngine::new();
    let b = request_grid_item(&mut fresh, 20.0, 2, 1);
    let c = request_grid_item(&mut fresh, 30.0, 3, 1);
    let a = request_grid_item(&mut fresh, 10.0, 1, 1);
    let fresh_root = request_grid_container(&mut fresh, &[b, c, a]);
    let fresh_root = compute_layout_without_measure(&mut fresh, fresh_root, 240.0, 80.0);

    assert_facts_committed_exactly(&retained, second_root);
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(retained.retained_node_token_for_tests(a), first_a_node);
    assert_eq!(retained.retained_node_token_for_tests(b), first_b_node);
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            creates: 1,
            reuses: 3,
            child_list_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn duplicate_same_position_exact_children_reuse_on_exact_repeat() {
    let mut engine = LayoutEngine::new();
    let left = request_leaf(&mut engine, 10.0);
    let right = request_leaf(&mut engine, 10.0);
    let first_root = request_container(&mut engine, &[left, right]);
    let first_root_node = engine.commit_layout(first_root);
    let first_child_nodes = engine.retained_child_tokens_for_tests(first_root_node);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let left = request_leaf(&mut engine, 10.0);
    let right = request_leaf(&mut engine, 10.0);
    let second_root = request_container(&mut engine, &[left, right]);
    let second_root_node = engine.commit_layout(second_root);
    assert_facts_committed_exactly(&engine, second_root);
    let second_child_nodes = engine.retained_child_tokens_for_tests(second_root_node);

    assert_eq!(second_root_node, first_root_node);
    assert_eq!(second_child_nodes, first_child_nodes);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 3,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn duplicate_exact_child_reuses_same_position_storage_when_sibling_count_changes() {
    let mut engine = LayoutEngine::new();
    let left = request_leaf(&mut engine, 10.0);
    let right = request_leaf(&mut engine, 10.0);
    let first_root = request_container(&mut engine, &[left, right]);
    let first_root_node = engine.commit_layout(first_root);
    let first_child_nodes = engine.retained_child_tokens_for_tests(first_root_node);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let child = request_leaf(&mut engine, 10.0);
    let second_root = request_container(&mut engine, &[child]);
    let second_root_node = engine.commit_layout(second_root);
    assert_facts_committed_exactly(&engine, second_root);

    let child_node = engine.retained_node_token_for_tests(child);
    assert_eq!(
        child_node, first_child_nodes[0],
        "duplicate anonymous previous siblings are not identity proof, but the same-position node may be reused as fully recommitted storage"
    );
    assert_eq!(
        engine.retained_child_tokens_for_tests(second_root_node),
        vec![child_node]
    );
    assert_eq!(
        engine.retained_parent_token_for_tests(child_node),
        Some(second_root_node)
    );
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 2,
            child_list_updates: 1,
            removes: 1,
            ..RetainedForestMutationSample::default()
        }
    );

    engine.finish_frame();
    assert_eq!(
        engine.retained_parent_token_for_tests(child_node),
        Some(second_root_node)
    );
    assert_eq!(
        engine.retained_child_tokens_for_tests(second_root_node),
        vec![child_node]
    );
}

#[test]
fn moved_duplicate_exact_child_uses_same_position_storage_when_identity_is_ambiguous() {
    let mut retained = LayoutEngine::new();
    let changed_slot = request_leaf(&mut retained, 20.0);
    let duplicate_left = request_leaf(&mut retained, 10.0);
    let duplicate_right = request_leaf(&mut retained, 10.0);
    let first_root = request_container(
        &mut retained,
        &[changed_slot, duplicate_left, duplicate_right],
    );
    let first_root_node = retained.commit_layout(first_root);
    let first_changed_slot_node = retained.retained_node_token_for_tests(changed_slot);
    let first_duplicate_nodes = [
        retained.retained_node_token_for_tests(duplicate_left),
        retained.retained_node_token_for_tests(duplicate_right),
    ];
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let child = request_leaf(&mut retained, 10.0);
    let second_root = request_container(&mut retained, &[child]);
    let second_root_node = retained.commit_layout(second_root);
    assert_facts_committed_exactly(&retained, second_root);
    let child_node = retained.retained_node_token_for_tests(child);

    assert_eq!(second_root_node, first_root_node);
    assert!(
        !first_duplicate_nodes.contains(&child_node),
        "duplicate anonymous exact siblings are not identity proof"
    );
    assert_eq!(
        child_node, first_changed_slot_node,
        "ambiguous anonymous identity is not preserved, but the same-position node may host recommitted current facts"
    );
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 2,
            style_updates: 1,
            child_list_updates: 1,
            removes: 2,
            ..RetainedForestMutationSample::default()
        }
    );

    let mut fresh = LayoutEngine::new();
    let fresh_child = request_leaf(&mut fresh, 10.0);
    let fresh_root = request_container(&mut fresh, &[fresh_child]);
    let fresh_root_node = fresh.commit_layout(fresh_root);
    assert_eq!(
        retained_layout_shape(&retained, second_root_node),
        retained_layout_shape(&fresh, fresh_root_node)
    );
}

#[test]
fn moved_child_has_exactly_the_current_intent_parent() {
    let mut engine = LayoutEngine::new();
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
    assert_facts_committed_exactly(&engine, root);

    let left_node = engine.retained_node_token_for_tests(left);
    let right_node = engine.retained_node_token_for_tests(right);
    let child_node = engine.retained_node_token_for_tests(child);

    assert_eq!(
        engine.retained_child_tokens_for_tests(left_node),
        Vec::<RetainedNodeToken>::new()
    );
    assert_eq!(
        engine.retained_child_tokens_for_tests(right_node),
        vec![child_node]
    );
    assert_eq!(
        engine.retained_parent_token_for_tests(child_node),
        Some(right_node)
    );
}

#[test]
fn child_reparenting_rollback_restores_precheckpoint_retained_state() {
    let mut engine = LayoutEngine::new();
    let a = request_leaf(&mut engine, 10.0);
    let b = request_leaf(&mut engine, 20.0);
    let prefix = request_container(&mut engine, &[a, b]);
    engine.commit_layout(prefix);
    engine.finish_frame();
    engine.reset_retained_mutation_sample_for_tests();

    let checkpoint = engine.checkpoint();
    let a = request_leaf(&mut engine, 10.0);
    let c = request_leaf(&mut engine, 30.0);
    let transient = request_container(&mut engine, &[a, c]);
    engine.commit_layout(transient);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 3,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
    engine.rollback_to_checkpoint(checkpoint);

    let a = request_leaf(&mut engine, 10.0);
    let b = request_leaf(&mut engine, 20.0);
    let retry = request_container(&mut engine, &[a, b]);
    engine.commit_layout(retry);
    engine.finish_frame();

    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 3,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn node_kind_changes_do_not_reuse_stale_measured_context() {
    let mut engine = LayoutEngine::new();
    let measured = request_measured(&mut engine, 10.0);
    engine.commit_layout(measured);
    engine.finish_frame();

    let leaf = request_leaf(&mut engine, 10.0);
    engine.commit_layout(leaf);
    assert_facts_committed_exactly(&engine, leaf);

    let leaf_node = engine.retained_node_token_for_tests(leaf);
    assert!(!engine.retained_node_has_measure_context_for_tests(leaf_node));
    assert_eq!(
        engine.retained_child_tokens_for_tests(leaf_node),
        Vec::<RetainedNodeToken>::new()
    );
}

#[test]
fn measured_producer_drops_at_frame_finish_and_marker_drops_on_removal() {
    let mut engine = LayoutEngine::new();
    let drops = Rc::new(Cell::new(0));

    let measured = request_counted_measured(&mut engine, drops.clone());
    engine.commit_layout(measured);
    let measured_node = engine.retained_node_token_for_tests(measured);
    assert!(engine.retained_node_has_measure_context_for_tests(measured_node));
    assert_eq!(drops.get(), 0);

    engine.finish_frame();
    assert_eq!(drops.get(), 1);
    assert!(engine.retained_node_has_measure_context_for_tests(measured_node));

    let leaf = request_leaf(&mut engine, 10.0);
    engine.commit_layout(leaf);
    assert_facts_committed_exactly(&engine, leaf);
    assert_eq!(drops.get(), 1);
    let leaf_node = engine.retained_node_token_for_tests(leaf);
    assert!(!engine.retained_node_has_measure_context_for_tests(leaf_node));
}

#[test]
fn opaque_measured_node_is_rebuilt_because_closure_semantics_are_opaque() {
    let mut engine = LayoutEngine::new();
    let measured = request_auto_measured(&mut engine, 10.0);
    let first_node = engine.commit_layout(measured);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let measured = request_auto_measured(&mut engine, 10.0);
    let second_node = engine.commit_layout(measured);
    assert_facts_committed_exactly(&engine, measured);

    assert_ne!(second_node, first_node);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            creates: 1,
            removes: 1,
            context_clears: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn unmeasured_subtree_with_opaque_descendant_retains_parent_and_rebuilds_opaque_child() {
    let mut engine = LayoutEngine::new();
    let measured = request_auto_measured(&mut engine, 10.0);
    let root = request_container(&mut engine, &[measured]);
    let first_root_node = engine.commit_layout(root);
    let first_measured_node = engine.retained_node_token_for_tests(measured);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let measured = request_auto_measured(&mut engine, 10.0);
    let root = request_container(&mut engine, &[measured]);
    let second_root_node = engine.commit_layout(root);
    let second_measured_node = engine.retained_node_token_for_tests(measured);
    assert_facts_committed_exactly(&engine, root);

    assert_eq!(second_root_node, first_root_node);
    assert_ne!(second_measured_node, first_measured_node);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            creates: 1,
            reuses: 1,
            child_list_updates: 1,
            removes: 1,
            context_clears: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[gpui::test]
fn retained_measured_node_recomputes_when_measure_result_changes(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let scale_factor = cx.update(|window, _| window.scale_factor());

    let mut retained = LayoutEngine::new();
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
        retained.retained_node_token_for_tests(root)
    });

    let mut fresh = LayoutEngine::new();
    let fresh_root = request_auto_measured(&mut fresh, 100.0);
    let fresh_root = cx.update(|window, app| {
        fresh.compute_layout(
            fresh_root,
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            app,
        );
        fresh.retained_node_token_for_tests(fresh_root)
    });

    let expected_size = size(100.0 * scale_factor, 10.0 * scale_factor);
    assert_eq!(
        (
            retained_node_size(&retained, retained_root),
            retained_node_size(&fresh, fresh_root),
        ),
        (expected_size, expected_size)
    );
}

#[gpui::test]
fn unchanged_pure_size_measure_reuses_solver_cache(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let mut engine = LayoutEngine::new();

    let root = request_pure_list_measured(&mut engine, 10.0, 20.0);
    compute_stable_test_root(
        cx,
        &mut engine,
        root,
        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
    );
    engine.finish_frame();

    let root = request_pure_list_measured(&mut engine, 10.0, 20.0);
    compute_stable_test_root(
        cx,
        &mut engine,
        root,
        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
    );

    assert_eq!(
        engine.layout_work_sample(),
        LayoutWorkSample {
            draw_index: 0,
            layout_node_requests: 0,
            measured_layout_node_requests: 1,
            child_edges: 0,
            compute_layout_calls: 1,
            solver_compute_layout_calls: 1,
            measured_layout_calls: 0,
            retained_layout_commit_duration: engine
                .layout_work_sample()
                .retained_layout_commit_duration,
            solver_observation_setup_duration: engine
                .layout_work_sample()
                .solver_observation_setup_duration,
            solver_layout_duration: engine.layout_work_sample().solver_layout_duration,
            geometry_capture_duration: engine.layout_work_sample().geometry_capture_duration,
            artifact_hydration_duration: engine.layout_work_sample().artifact_hydration_duration,
            fresh_compare_duration: engine.layout_work_sample().fresh_compare_duration,
            retained_layout_finish_frame_duration: engine
                .layout_work_sample()
                .retained_layout_finish_frame_duration,
            compute_layout_duration: engine.layout_work_sample().compute_layout_duration,
            measured_layout_duration: Duration::default(),
            ..LayoutWorkSample::default()
        }
    );
}

#[gpui::test]
fn unchanged_text_measure_hydrates_from_query_cache(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let key = text_measure_key("hello");
    let measure_invocations = Rc::new(Cell::new(0));
    let hydrations = Rc::new(Cell::new(0));
    let mut engine = LayoutEngine::new();

    let root = request_text_measured(
        &mut engine,
        key.clone(),
        size(px(40.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
    );
    compute_stable_test_root(
        cx,
        &mut engine,
        root,
        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
    );
    engine.finish_frame();

    let root = request_text_measured(
        &mut engine,
        key,
        size(px(40.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
    );
    compute_stable_test_root(
        cx,
        &mut engine,
        root,
        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
    );

    assert_eq!((measure_invocations.get(), hydrations.get()), (1, 2));
    assert_eq!(engine.layout_work_sample().measured_layout_calls, 0);
    assert_eq!(engine.layout_work_sample().solver_compute_layout_calls, 1);
}

#[gpui::test]
fn unchanged_nested_text_measure_hydrates_from_query_cache(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let key = text_measure_key("hello");
    let measure_invocations = Rc::new(Cell::new(0));
    let hydrations = Rc::new(Cell::new(0));
    let mut engine = LayoutEngine::new();

    let text = request_text_measured(
        &mut engine,
        key.clone(),
        size(px(40.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
    );
    let root = request_container(&mut engine, &[text]);
    compute_stable_test_root(
        cx,
        &mut engine,
        root,
        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
    );
    engine.finish_frame();

    measure_invocations.set(0);
    hydrations.set(0);
    engine.reset_retained_mutation_sample_for_tests();
    let text = request_text_measured(
        &mut engine,
        key,
        size(px(40.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
    );
    let root = request_container(&mut engine, &[text]);
    compute_stable_test_root(
        cx,
        &mut engine,
        root,
        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
    );

    assert_facts_committed_exactly(&engine, root);
    assert_eq!((measure_invocations.get(), hydrations.get()), (0, 1));
    assert_eq!(engine.layout_work_sample().measured_layout_calls, 0);
    assert_eq!(engine.layout_work_sample().solver_compute_layout_calls, 1);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 2,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[gpui::test]
fn changed_unmeasured_sibling_hydrates_stable_text_without_callback(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let key = text_measure_key("hello");
    let measure_invocations = Rc::new(Cell::new(0));
    let hydrations = Rc::new(Cell::new(0));
    let mut engine = LayoutEngine::new();

    let text = request_text_measured(
        &mut engine,
        key.clone(),
        size(px(40.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
    );
    let changing_sibling = request_leaf(&mut engine, 10.0);
    let root = request_container(&mut engine, &[text, changing_sibling]);
    cx.update(|window, app| {
        engine.compute_layout(
            root,
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            app,
        );
    });
    engine.finish_frame();

    measure_invocations.set(0);
    hydrations.set(0);
    engine.reset_retained_mutation_sample_for_tests();
    let text = request_text_measured(
        &mut engine,
        key,
        size(px(40.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
    );
    let changing_sibling = request_leaf(&mut engine, 20.0);
    let root = request_container(&mut engine, &[text, changing_sibling]);
    cx.update(|window, app| {
        engine.compute_layout(
            root,
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            app,
        );
    });

    assert_eq!((measure_invocations.get(), hydrations.get()), (0, 1));
    assert_eq!(engine.layout_work_sample().measured_layout_calls, 0);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 3,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[gpui::test]
fn changed_unmeasured_sibling_hydrates_nested_stable_text_without_callback(
    cx: &mut TestAppContext,
) {
    let cx = cx.add_empty_window();
    let key = text_measure_key("hello");
    let measure_invocations = Rc::new(Cell::new(0));
    let hydrations = Rc::new(Cell::new(0));
    let mut engine = LayoutEngine::new();

    let text = request_text_measured(
        &mut engine,
        key.clone(),
        size(px(40.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
    );
    let stable_container = request_container(&mut engine, &[text]);
    let changing_sibling = request_leaf(&mut engine, 10.0);
    let root = request_container(&mut engine, &[stable_container, changing_sibling]);
    cx.update(|window, app| {
        engine.compute_layout(
            root,
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            app,
        );
    });
    let first_container_node = engine.retained_node_token_for_tests(stable_container);
    engine.finish_frame();

    measure_invocations.set(0);
    hydrations.set(0);
    engine.reset_retained_mutation_sample_for_tests();
    let text = request_text_measured(
        &mut engine,
        key,
        size(px(40.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
    );
    let stable_container = request_container(&mut engine, &[text]);
    let changing_sibling = request_leaf(&mut engine, 20.0);
    let root = request_container(&mut engine, &[stable_container, changing_sibling]);
    cx.update(|window, app| {
        engine.compute_layout(
            root,
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            app,
        );
    });

    assert_eq!(
        engine.retained_node_token_for_tests(stable_container),
        first_container_node
    );
    assert_eq!((measure_invocations.get(), hydrations.get()), (0, 1));
    assert_eq!(engine.layout_work_sample().measured_layout_calls, 0);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 4,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[gpui::test]
fn changed_text_outside_stable_subtree_does_not_remeasure_stable_text(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let stable_key = text_measure_key("stable");
    let first_dynamic_key = text_measure_key("frame 1");
    let second_dynamic_key = text_measure_key("frame 2");
    let stable_measures = Rc::new(Cell::new(0));
    let stable_hydrations = Rc::new(Cell::new(0));
    let dynamic_measures = Rc::new(Cell::new(0));
    let dynamic_hydrations = Rc::new(Cell::new(0));
    let mut engine = LayoutEngine::new();

    let stable_text = request_text_measured(
        &mut engine,
        stable_key.clone(),
        size(px(40.0), px(20.0)),
        stable_measures.clone(),
        stable_hydrations.clone(),
    );
    let stable_subtree = request_container(&mut engine, &[stable_text]);
    let dynamic_text = request_text_measured(
        &mut engine,
        first_dynamic_key,
        size(px(50.0), px(20.0)),
        dynamic_measures.clone(),
        dynamic_hydrations.clone(),
    );
    let root = request_container(&mut engine, &[stable_subtree, dynamic_text]);
    cx.update(|window, app| {
        engine.compute_layout(
            root,
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            app,
        );
    });
    engine.finish_frame();

    stable_measures.set(0);
    stable_hydrations.set(0);
    dynamic_measures.set(0);
    dynamic_hydrations.set(0);
    engine.reset_retained_mutation_sample_for_tests();
    let stable_text = request_text_measured(
        &mut engine,
        stable_key,
        size(px(40.0), px(20.0)),
        stable_measures.clone(),
        stable_hydrations.clone(),
    );
    let stable_subtree = request_container(&mut engine, &[stable_text]);
    let dynamic_text = request_text_measured(
        &mut engine,
        second_dynamic_key,
        size(px(50.0), px(20.0)),
        dynamic_measures.clone(),
        dynamic_hydrations.clone(),
    );
    let root = request_container(&mut engine, &[stable_subtree, dynamic_text]);
    cx.update(|window, app| {
        engine.compute_layout(
            root,
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            app,
        );
    });

    assert_eq!(
        (
            stable_measures.get(),
            stable_hydrations.get(),
            dynamic_measures.get(),
            dynamic_hydrations.get()
        ),
        (0, 1, 1, 1)
    );
    assert_eq!(engine.layout_work_sample().measured_layout_calls, 1);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 4,
            dirty_marks: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn same_text_measure_key_with_changed_solver_style_reuses_text_node() {
    let key = text_measure_key("hello");
    let mut engine = LayoutEngine::new();
    let text = request_text_measured_with_style(
        &mut engine,
        Style::default(),
        key.clone(),
        size(px(40.0), px(20.0)),
    );
    let first_node = engine.commit_layout(text);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let text = request_text_measured_with_style(
        &mut engine,
        style_with_width(40.0),
        key,
        size(px(40.0), px(20.0)),
    );
    let second_node = engine.commit_layout(text);
    assert_facts_committed_exactly(&engine, text);

    assert_eq!(second_node, first_node);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 1,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[gpui::test]
fn fresh_compare_uses_pure_size_measure_facts(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let mut engine = LayoutEngine::new();
    let root = request_pure_list_measured(&mut engine, 40.0, 20.0);

    cx.update(|window, app| {
        let summary = engine.retained_fresh_layout_comparison_for_tests(
            root,
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            app,
        );
        assert_eq!(summary.checked_nodes, 1);
        assert_eq!(summary.mismatches, 0);
    });
}

#[gpui::test]
fn fresh_compare_uses_text_measure_key_not_retained_layout_size(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let key = text_measure_key("fresh compare text");
    let measure_key = key.clone();
    let mut engine = LayoutEngine::new();
    let root = engine.request_text_measured_layout(
        Style::default(),
        px(16.0),
        1.0,
        key,
        |_| {},
        move |known_dimensions, available_space, measure_cx| {
            measure_key.measure(known_dimensions, available_space, measure_cx)
        },
    );

    cx.update(|window, app| {
        let summary = engine.retained_fresh_layout_comparison_for_tests(
            root,
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            app,
        );
        assert_eq!(summary.checked_nodes, 1);
        assert_eq!(summary.mismatches, 0);
    });
}

#[gpui::test]
fn fresh_compare_skips_opaque_measured_nodes(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let mut engine = LayoutEngine::new();
    let root = engine.request_measured_layout(Style::default(), px(16.0), 1.0, |_, _, _| {
        size(px(40.0), px(20.0))
    });

    cx.update(|window, app| {
        let summary = engine.retained_fresh_layout_comparison_for_tests(
            root,
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            app,
        );
        assert_eq!(summary.checked_nodes, 0);
        assert_eq!(summary.mismatches, 0);
    });
}

#[gpui::test]
fn rollback_resets_text_hydration_state_for_retry(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let key = text_measure_key("hello");
    let measure_invocations = Rc::new(Cell::new(0));
    let hydrations = Rc::new(Cell::new(0));
    let mut engine = LayoutEngine::new();

    let root = request_text_measured(
        &mut engine,
        key.clone(),
        size(px(40.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
    );
    compute_stable_test_root(
        cx,
        &mut engine,
        root,
        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
    );
    engine.finish_frame();

    let root = request_text_measured(
        &mut engine,
        key,
        size(px(40.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
    );
    let checkpoint = engine.checkpoint();
    compute_stable_test_root(
        cx,
        &mut engine,
        root,
        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
    );
    engine.rollback_to_checkpoint(checkpoint);
    compute_stable_test_root(
        cx,
        &mut engine,
        root,
        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
    );

    assert_eq!((measure_invocations.get(), hydrations.get()), (1, 3));
    assert_eq!(engine.layout_work_sample().measured_layout_calls, 0);
    assert_eq!(engine.layout_work_sample().solver_compute_layout_calls, 1);
}

#[gpui::test]
fn changed_text_measure_key_remeasures(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let first_key = text_measure_key("hello");
    let second_key = text_measure_key("world");
    let measure_invocations = Rc::new(Cell::new(0));
    let hydrations = Rc::new(Cell::new(0));
    let hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
    let mut engine = LayoutEngine::new();

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

    engine.reset_retained_mutation_sample_for_tests();
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
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 1,
            dirty_marks: 1,
            ..RetainedForestMutationSample::default()
        }
    );
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
fn fixed_size_text_artifact_change_does_not_dirty_layout(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let first_key = text_measure_key("draw     17");
    let second_key = text_measure_key("draw     18");
    let layout_size = size(px(120.0), px(20.0));
    let measure_invocations = Rc::new(Cell::new(0));
    let hydrations = Rc::new(Cell::new(0));
    let hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
    let mut engine = LayoutEngine::new();

    let root = request_fixed_size_text_measured_with_hydration_log(
        &mut engine,
        first_key.clone(),
        layout_size,
        size(px(40.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
        hydrated_artifacts.clone(),
    );
    compute_stable_test_root(
        cx,
        &mut engine,
        root,
        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
    );
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let root = request_fixed_size_text_measured_with_hydration_log(
        &mut engine,
        second_key.clone(),
        layout_size,
        size(px(50.0), px(20.0)),
        measure_invocations.clone(),
        hydrations.clone(),
        hydrated_artifacts.clone(),
    );
    compute_stable_test_root(
        cx,
        &mut engine,
        root,
        size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
    );

    assert_eq!(engine.layout_work_sample().measured_layout_calls, 0);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 1,
            ..RetainedForestMutationSample::default()
        }
    );
    assert_eq!((measure_invocations.get(), hydrations.get()), (2, 2));
    assert_eq!(
        hydrated_artifacts.borrow().as_slice(),
        [
            HydratedTextArtifact {
                key: first_key,
                size: size(px(40.0), px(20.0)),
            },
            HydratedTextArtifact {
                key: second_key,
                size: size(px(50.0), px(20.0)),
            },
        ]
    );
}

#[gpui::test]
fn changed_text_measure_key_layout_facts_remeasure(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let first_key = text_measure_key("hello");
    let cases = [
        (
            "font-size",
            text_measure_key_with("hello", |_| {}, px(18.0), px(20.0), 1.0, 0),
        ),
        (
            "line-height",
            text_measure_key_with("hello", |_| {}, px(16.0), px(24.0), 1.0, 0),
        ),
        (
            "white-space",
            text_measure_key_with(
                "hello",
                |style| style.white_space = WhiteSpace::Nowrap,
                px(16.0),
                px(20.0),
                1.0,
                0,
            ),
        ),
        (
            "text-overflow",
            text_measure_key_with(
                "hello",
                |style| {
                    style.text_overflow =
                        Some(TextOverflow::Truncate(SharedString::new_static("...")));
                },
                px(16.0),
                px(20.0),
                1.0,
                0,
            ),
        ),
        (
            "line-clamp",
            text_measure_key_with(
                "hello",
                |style| style.line_clamp = Some(1),
                px(16.0),
                px(20.0),
                1.0,
                0,
            ),
        ),
        (
            "scale-factor",
            text_measure_key_with("hello", |_| {}, px(16.0), px(20.0), 2.0, 0),
        ),
        (
            "shaping-epoch",
            text_measure_key_with("hello", |_| {}, px(16.0), px(20.0), 1.0, 1),
        ),
    ];
    let mut actual = Vec::new();

    for (name, second_key) in cases {
        let measure_invocations = Rc::new(Cell::new(0));
        let hydrations = Rc::new(Cell::new(0));
        let hydrated_artifacts = Rc::new(RefCell::new(Vec::new()));
        let mut engine = LayoutEngine::new();

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
            second_key,
            size(px(60.0), px(24.0)),
            measure_invocations.clone(),
            hydrations.clone(),
            Some(hydrated_artifacts),
        );
        cx.update(|window, app| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                app,
            );
        });

        actual.push((
            name,
            measure_invocations.get(),
            hydrations.get(),
            engine.layout_work_sample().measured_layout_calls,
        ));
    }

    assert_eq!(
        actual,
        vec![
            ("font-size", 2, 2, 1),
            ("line-height", 2, 2, 1),
            ("white-space", 2, 2, 1),
            ("text-overflow", 2, 2, 1),
            ("line-clamp", 2, 2, 1),
            ("scale-factor", 2, 2, 1),
            ("shaping-epoch", 2, 2, 1),
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
#[should_panic(expected = "layout facts should appear only once")]
fn duplicate_layout_fact_references_fail_loudly() {
    let mut engine = LayoutEngine::new();
    let child = request_leaf(&mut engine, 10.0);
    let root = request_container(&mut engine, &[child, child]);

    engine.commit_layout(root);
}

#[test]
fn retained_layout_recomputes_when_root_available_space_changes() {
    let mut retained = LayoutEngine::new();
    let child = request_full_leaf(&mut retained);
    let root = request_full_container(&mut retained, &[child]);
    compute_layout_without_measure(&mut retained, root, 0.0, 100.0);
    retained.finish_frame();

    let child = request_full_leaf(&mut retained);
    let root = request_full_container(&mut retained, &[child]);
    let retained_root = compute_layout_without_measure(&mut retained, root, 800.0, 100.0);
    let retained_child = retained.retained_node_token_for_tests(child);

    let mut fresh = LayoutEngine::new();
    let child = request_full_leaf(&mut fresh);
    let root = request_full_container(&mut fresh, &[child]);
    let fresh_root = compute_layout_without_measure(&mut fresh, root, 800.0, 100.0);
    let fresh_child = fresh.retained_node_token_for_tests(child);

    assert_eq!(
        retained_node_size(&retained, retained_root),
        retained_node_size(&fresh, fresh_root)
    );
    assert_eq!(
        retained_node_size(&retained, retained_child),
        retained_node_size(&fresh, fresh_child)
    );
    assert_eq!(
        retained_node_size(&retained, retained_child),
        size(800.0, 100.0)
    );
}

#[test]
fn retained_layout_matches_fresh_after_internal_parent_style_aba() {
    fn request_frame(engine: &mut LayoutEngine, parent_width: Option<f32>) -> (LayoutId, LayoutId) {
        let child = request_full_leaf(engine);
        let mut parent_style = Style::default();
        parent_style.size = Size::full();
        if let Some(width) = parent_width {
            parent_style.size.width = length_px(width);
        }
        let parent = engine.request_layout(parent_style, px(16.0), 1.0, &[child]);
        let root = request_full_container(engine, &[parent]);
        (root, child)
    }

    let mut retained = LayoutEngine::new();
    let (root, _child) = request_frame(&mut retained, None);
    compute_layout_without_measure(&mut retained, root, 800.0, 100.0);
    retained.finish_frame();

    let (root, _child) = request_frame(&mut retained, Some(0.0));
    compute_layout_without_measure(&mut retained, root, 800.0, 100.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let (root, child) = request_frame(&mut retained, None);
    let retained_root = compute_layout_without_measure(&mut retained, root, 800.0, 100.0);
    let retained_child = retained.retained_node_token_for_tests(child);

    let mut fresh = LayoutEngine::new();
    let (root, child) = request_frame(&mut fresh, None);
    let fresh_root = compute_layout_without_measure(&mut fresh, root, 800.0, 100.0);
    let fresh_child = fresh.retained_node_token_for_tests(child);

    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        retained_node_size(&retained, retained_child),
        retained_node_size(&fresh, fresh_child)
    );
    assert_eq!(
        retained_node_size(&retained, retained_child),
        size(800.0, 100.0)
    );
}

#[test]
fn retained_layout_matches_fresh_after_internal_child_list_aba() {
    fn request_frame(engine: &mut LayoutEngine, inserted_sidebar: bool) -> (LayoutId, LayoutId) {
        let canvas = request_full_leaf(engine);
        let mut children = Vec::new();
        if inserted_sidebar {
            children.push(request_leaf(engine, 0.0));
        }
        children.push(canvas);
        let parent = request_flex_container(engine, &children);
        let root = request_full_container(engine, &[parent]);
        (root, canvas)
    }

    let mut retained = LayoutEngine::new();
    let (root, _canvas) = request_frame(&mut retained, false);
    compute_layout_without_measure(&mut retained, root, 800.0, 100.0);
    retained.finish_frame();

    let (root, _canvas) = request_frame(&mut retained, true);
    compute_layout_without_measure(&mut retained, root, 800.0, 100.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let (root, canvas) = request_frame(&mut retained, false);
    let retained_root = compute_layout_without_measure(&mut retained, root, 800.0, 100.0);
    let retained_canvas = retained.retained_node_token_for_tests(canvas);

    let mut fresh = LayoutEngine::new();
    let (root, canvas) = request_frame(&mut fresh, false);
    let fresh_root = compute_layout_without_measure(&mut fresh, root, 800.0, 100.0);
    let fresh_canvas = fresh.retained_node_token_for_tests(canvas);

    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        retained_node_size(&retained, retained_canvas),
        retained_node_size(&fresh, fresh_canvas)
    );
}

#[test]
fn retained_layout_recomputes_when_root_scale_factor_changes() {
    let mut retained = LayoutEngine::new();
    let child = request_full_leaf(&mut retained);
    let root = request_full_container(&mut retained, &[child]);
    compute_layout_without_measure_with_scale(&mut retained, root, 800.0, 100.0, 1.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let child = request_full_leaf(&mut retained);
    let root = request_full_container(&mut retained, &[child]);
    let retained_root =
        compute_layout_without_measure_with_scale(&mut retained, root, 800.0, 100.0, 2.0);
    let retained_child = retained.retained_node_token_for_tests(child);

    let mut fresh = LayoutEngine::new();
    let child = request_full_leaf(&mut fresh);
    let root = request_full_container(&mut fresh, &[child]);
    let fresh_root = compute_layout_without_measure_with_scale(&mut fresh, root, 800.0, 100.0, 2.0);
    let fresh_child = fresh.retained_node_token_for_tests(child);

    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        retained.retained_node_layout_bounds_for_tests(retained_child, 2.0),
        fresh.retained_node_layout_bounds_for_tests(fresh_child, 2.0)
    );
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 2,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn rollback_discards_transient_frame_geometry() {
    let mut with_rollback = LayoutEngine::new();
    let child = request_full_leaf(&mut with_rollback);
    let root = request_full_container(&mut with_rollback, &[child]);
    compute_layout_without_measure(&mut with_rollback, root, 800.0, 100.0);
    with_rollback.finish_frame();
    with_rollback.reset_retained_mutation_sample_for_tests();

    let checkpoint = with_rollback.checkpoint();
    let transient_child = request_full_leaf(&mut with_rollback);
    let transient_root = request_full_container(&mut with_rollback, &[transient_child]);
    compute_layout_without_measure(&mut with_rollback, transient_root, 800.0, 0.0);
    with_rollback.rollback_to_checkpoint(checkpoint);

    let child = request_full_leaf(&mut with_rollback);
    let root = request_full_container(&mut with_rollback, &[child]);
    let with_rollback_root = compute_layout_without_measure(&mut with_rollback, root, 800.0, 100.0);

    let mut skipped = LayoutEngine::new();
    let child = request_full_leaf(&mut skipped);
    let root = request_full_container(&mut skipped, &[child]);
    compute_layout_without_measure(&mut skipped, root, 800.0, 100.0);
    skipped.finish_frame();
    skipped.reset_retained_mutation_sample_for_tests();
    let child = request_full_leaf(&mut skipped);
    let root = request_full_container(&mut skipped, &[child]);
    let skipped_root = compute_layout_without_measure(&mut skipped, root, 800.0, 100.0);

    assert_eq!(
        retained_layout_projection(&with_rollback, with_rollback_root),
        retained_layout_projection(&skipped, skipped_root)
    );
    assert_eq!(
        with_rollback.retained_mutation_sample_for_tests(),
        skipped.retained_mutation_sample_for_tests()
    );
    assert_eq!(
        with_rollback.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 2,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn changed_flex_ancestor_updates_canvas_subtree_and_matches_fresh_layout() {
    let mut retained = LayoutEngine::new();
    let (root, _flex_child, _canvas_host, _canvas) =
        request_canvas_like_flex_frame(&mut retained, 0.0);
    compute_layout_without_measure(&mut retained, root, 838.0, 573.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let (root, flex_child, canvas_host, canvas) =
        request_canvas_like_flex_frame(&mut retained, 24.0);
    let retained_root = compute_layout_without_measure(&mut retained, root, 838.0, 573.0);
    let retained_flex_child = retained.retained_node_token_for_tests(flex_child);
    let retained_canvas_host = retained.retained_node_token_for_tests(canvas_host);
    let retained_canvas = retained.retained_node_token_for_tests(canvas);

    let mut fresh = LayoutEngine::new();
    let (root, flex_child, canvas_host, canvas) = request_canvas_like_flex_frame(&mut fresh, 24.0);
    let fresh_root = compute_layout_without_measure(&mut fresh, root, 838.0, 573.0);
    let fresh_flex_child = fresh.retained_node_token_for_tests(flex_child);
    let fresh_canvas_host = fresh.retained_node_token_for_tests(canvas_host);
    let fresh_canvas = fresh.retained_node_token_for_tests(canvas);

    assert_facts_committed_exactly(&retained, root);
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            retained_node_size(&fresh, fresh_flex_child),
            retained_node_size(&fresh, fresh_canvas_host),
            retained_node_size(&fresh, fresh_canvas),
        )
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (size(836.0, 545.0), size(836.0, 545.0), size(836.0, 545.0))
    );
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 7,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn reused_canvas_panel_under_inserted_sidebar_matches_fresh_layout() {
    let mut retained = LayoutEngine::new();
    let (root, _panel, _flex_child, _canvas_host, _canvas) =
        request_canvas_like_workspace_row(&mut retained, 70.0, None);
    compute_layout_without_measure(&mut retained, root, 2158.0, 1219.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_workspace_row(&mut retained, 70.0, Some(480.0));
    let retained_root = compute_layout_without_measure(&mut retained, root, 2158.0, 1219.0);
    let retained_panel = retained.retained_node_token_for_tests(panel);
    let retained_flex_child = retained.retained_node_token_for_tests(flex_child);
    let retained_canvas_host = retained.retained_node_token_for_tests(canvas_host);
    let retained_canvas = retained.retained_node_token_for_tests(canvas);

    let mut fresh = LayoutEngine::new();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_workspace_row(&mut fresh, 70.0, Some(480.0));
    let fresh_root = compute_layout_without_measure(&mut fresh, root, 2158.0, 1219.0);
    let fresh_panel = fresh.retained_node_token_for_tests(panel);
    let fresh_flex_child = fresh.retained_node_token_for_tests(flex_child);
    let fresh_canvas_host = fresh.retained_node_token_for_tests(canvas_host);
    let fresh_canvas = fresh.retained_node_token_for_tests(canvas);

    assert_facts_committed_exactly(&retained, root);
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            retained_node_size(&fresh, fresh_panel),
            retained_node_size(&fresh, fresh_flex_child),
            retained_node_size(&fresh, fresh_canvas_host),
            retained_node_size(&fresh, fresh_canvas),
        )
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            size(1678.0, 1219.0),
            size(1676.0, 1145.0),
            size(1676.0, 1145.0),
            size(1676.0, 1145.0),
        )
    );
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            creates: 1,
            reuses: 7,
            child_list_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn reused_canvas_panel_inside_chrome_shell_after_sidebar_resize_matches_fresh_layout() {
    let mut retained = LayoutEngine::new();
    let (root, _panel, _flex_child, _canvas_host, _canvas) =
        request_canvas_like_chrome_frame(&mut retained, 0.0);
    compute_layout_without_measure(&mut retained, root, 2200.0, 1522.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame(&mut retained, 480.0);
    let retained_root = compute_layout_without_measure(&mut retained, root, 2200.0, 1522.0);
    let retained_panel = retained.retained_node_token_for_tests(panel);
    let retained_flex_child = retained.retained_node_token_for_tests(flex_child);
    let retained_canvas_host = retained.retained_node_token_for_tests(canvas_host);
    let retained_canvas = retained.retained_node_token_for_tests(canvas);

    let mut fresh = LayoutEngine::new();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame(&mut fresh, 480.0);
    let fresh_root = compute_layout_without_measure(&mut fresh, root, 2200.0, 1522.0);
    let fresh_panel = fresh.retained_node_token_for_tests(panel);
    let fresh_flex_child = fresh.retained_node_token_for_tests(flex_child);
    let fresh_canvas_host = fresh.retained_node_token_for_tests(canvas_host);
    let fresh_canvas = fresh.retained_node_token_for_tests(canvas);

    assert_facts_committed_exactly(&retained, root);
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            retained_node_size(&fresh, fresh_panel),
            retained_node_size(&fresh, fresh_flex_child),
            retained_node_size(&fresh, fresh_canvas_host),
            retained_node_size(&fresh, fresh_canvas),
        )
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            size(1678.0, 1219.0),
            size(1676.0, 1145.0),
            size(1676.0, 1145.0),
            size(1676.0, 1145.0),
        )
    );
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 14,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn reused_canvas_panel_after_zero_height_probe_matches_fresh_layout() {
    let mut retained = LayoutEngine::new();
    let (root, _panel, _flex_child, _canvas_host, _canvas) =
        request_canvas_like_chrome_frame(&mut retained, 0.0);
    compute_layout_without_measure(&mut retained, root, 2200.0, 0.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame(&mut retained, 480.0);
    let retained_root = compute_layout_without_measure(&mut retained, root, 2200.0, 1522.0);
    let retained_panel = retained.retained_node_token_for_tests(panel);
    let retained_flex_child = retained.retained_node_token_for_tests(flex_child);
    let retained_canvas_host = retained.retained_node_token_for_tests(canvas_host);
    let retained_canvas = retained.retained_node_token_for_tests(canvas);

    let mut fresh = LayoutEngine::new();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame(&mut fresh, 480.0);
    let fresh_root = compute_layout_without_measure(&mut fresh, root, 2200.0, 1522.0);
    let fresh_panel = fresh.retained_node_token_for_tests(panel);
    let fresh_flex_child = fresh.retained_node_token_for_tests(flex_child);
    let fresh_canvas_host = fresh.retained_node_token_for_tests(canvas_host);
    let fresh_canvas = fresh.retained_node_token_for_tests(canvas);

    assert_facts_committed_exactly(&retained, root);
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            retained_node_size(&fresh, fresh_panel),
            retained_node_size(&fresh, fresh_flex_child),
            retained_node_size(&fresh, fresh_canvas_host),
            retained_node_size(&fresh, fresh_canvas),
        )
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            size(1678.0, 1219.0),
            size(1676.0, 1145.0),
            size(1676.0, 1145.0),
            size(1676.0, 1145.0),
        )
    );
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 14,
            style_updates: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn reused_canvas_panel_after_scaled_sidebar_resize_matches_fresh_layout() {
    let mut retained = LayoutEngine::new();
    let (root, _panel, _flex_child, _canvas_host, _canvas) =
        request_canvas_like_chrome_frame(&mut retained, 0.0);
    compute_layout_without_measure_with_scale(&mut retained, root, 1100.0, 760.0, 2.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame(&mut retained, 480.0);
    let retained_root =
        compute_layout_without_measure_with_scale(&mut retained, root, 1100.0, 760.0, 2.0);
    let retained_panel = retained.retained_node_token_for_tests(panel);
    let retained_flex_child = retained.retained_node_token_for_tests(flex_child);
    let retained_canvas_host = retained.retained_node_token_for_tests(canvas_host);
    let retained_canvas = retained.retained_node_token_for_tests(canvas);

    let mut fresh = LayoutEngine::new();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame(&mut fresh, 480.0);
    let fresh_root =
        compute_layout_without_measure_with_scale(&mut fresh, root, 1100.0, 760.0, 2.0);
    let fresh_panel = fresh.retained_node_token_for_tests(panel);
    let fresh_flex_child = fresh.retained_node_token_for_tests(flex_child);
    let fresh_canvas_host = fresh.retained_node_token_for_tests(canvas_host);
    let fresh_canvas = fresh.retained_node_token_for_tests(canvas);

    assert_facts_committed_exactly(&retained, root);
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            retained_node_size(&fresh, fresh_panel),
            retained_node_size(&fresh, fresh_flex_child),
            retained_node_size(&fresh, fresh_canvas_host),
            retained_node_size(&fresh, fresh_canvas),
        )
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (size(1676.0, 1143.0), size(1676.0, 1143.0))
    );
}

#[test]
fn reused_canvas_panel_after_scaled_gapped_sidebar_resize_matches_fresh_layout() {
    let mut retained = LayoutEngine::new();
    let (root, _panel, _flex_child, _canvas_host, _canvas) =
        request_canvas_like_chrome_frame_with_sidebar_and_gap(
            &mut retained,
            |engine| request_keyed_fixed_width_sidebar(engine, 12_014, 0.0),
            21.0,
        );
    compute_layout_without_measure_with_scale(&mut retained, root, 1100.0, 760.0, 2.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame_with_sidebar_and_gap(
            &mut retained,
            |engine| request_keyed_fixed_width_sidebar(engine, 12_014, 480.0),
            21.0,
        );
    let retained_root =
        compute_layout_without_measure_with_scale(&mut retained, root, 1100.0, 760.0, 2.0);
    let retained_panel = retained.retained_node_token_for_tests(panel);
    let retained_flex_child = retained.retained_node_token_for_tests(flex_child);
    let retained_canvas_host = retained.retained_node_token_for_tests(canvas_host);
    let retained_canvas = retained.retained_node_token_for_tests(canvas);

    let mut fresh = LayoutEngine::new();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame_with_sidebar_and_gap(
            &mut fresh,
            |engine| request_keyed_fixed_width_sidebar(engine, 12_014, 480.0),
            21.0,
        );
    let fresh_root =
        compute_layout_without_measure_with_scale(&mut fresh, root, 1100.0, 760.0, 2.0);
    let fresh_panel = fresh.retained_node_token_for_tests(panel);
    let fresh_flex_child = fresh.retained_node_token_for_tests(flex_child);
    let fresh_canvas_host = fresh.retained_node_token_for_tests(canvas_host);
    let fresh_canvas = fresh.retained_node_token_for_tests(canvas);

    assert_facts_committed_exactly(&retained, root);
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            retained_node_size(&fresh, fresh_panel),
            retained_node_size(&fresh, fresh_flex_child),
            retained_node_size(&fresh, fresh_canvas_host),
            retained_node_size(&fresh, fresh_canvas),
        )
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (size(1655.0, 1143.0), size(1655.0, 1143.0))
    );
}

#[test]
fn reused_canvas_panel_after_scaled_root_width_resize_matches_fresh_layout() {
    let mut retained = LayoutEngine::new();
    let (root, _panel, _flex_child, _canvas_host, _canvas) =
        request_canvas_like_chrome_frame_with_sidebar_and_gap(
            &mut retained,
            |engine| request_keyed_fixed_width_sidebar(engine, 12_014, 480.0),
            21.0,
        );
    compute_layout_without_measure_with_scale(&mut retained, root, 459.0, 608.5, 2.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame_with_sidebar_and_gap(
            &mut retained,
            |engine| request_keyed_fixed_width_sidebar(engine, 12_014, 480.0),
            21.0,
        );
    let retained_root =
        compute_layout_without_measure_with_scale(&mut retained, root, 1100.0, 760.0, 2.0);
    let retained_panel = retained.retained_node_token_for_tests(panel);
    let retained_flex_child = retained.retained_node_token_for_tests(flex_child);
    let retained_canvas_host = retained.retained_node_token_for_tests(canvas_host);
    let retained_canvas = retained.retained_node_token_for_tests(canvas);

    let mut fresh = LayoutEngine::new();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame_with_sidebar_and_gap(
            &mut fresh,
            |engine| request_keyed_fixed_width_sidebar(engine, 12_014, 480.0),
            21.0,
        );
    let fresh_root =
        compute_layout_without_measure_with_scale(&mut fresh, root, 1100.0, 760.0, 2.0);
    let fresh_panel = fresh.retained_node_token_for_tests(panel);
    let fresh_flex_child = fresh.retained_node_token_for_tests(flex_child);
    let fresh_canvas_host = fresh.retained_node_token_for_tests(canvas_host);
    let fresh_canvas = fresh.retained_node_token_for_tests(canvas);

    assert_facts_committed_exactly(&retained, root);
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            retained_node_size(&fresh, fresh_panel),
            retained_node_size(&fresh, fresh_flex_child),
            retained_node_size(&fresh, fresh_canvas_host),
            retained_node_size(&fresh, fresh_canvas),
        )
    );
}

#[gpui::test]
fn reused_canvas_panel_with_dirty_text_sibling_matches_fresh_layout(cx: &mut TestAppContext) {
    fn request_frame(
        engine: &mut LayoutEngine,
        measure_invocations: Rc<Cell<usize>>,
        hydrations: Rc<Cell<usize>>,
    ) -> (LayoutId, LayoutId, LayoutId, LayoutId, LayoutId) {
        request_canvas_like_chrome_frame_with_sidebar(engine, |engine| {
            let text = request_text_measured(
                engine,
                text_measure_key("debug text"),
                size(px(120.0), px(20.0)),
                measure_invocations,
                hydrations,
            );

            let mut style = Style::default();
            style.display = Display::Block;
            style.size.width =
                Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(480.0))));
            style.size.height = Length::Definite(DefiniteLength::Fraction(1.0));
            engine.request_layout(style, px(16.0), 1.0, &[text])
        })
    }

    let cx = cx.add_empty_window();
    let mut retained = LayoutEngine::new();
    let measure_invocations = Rc::new(Cell::new(0));
    let hydrations = Rc::new(Cell::new(0));
    let (root, _panel, _flex_child, _canvas_host, _canvas) = request_frame(
        &mut retained,
        measure_invocations.clone(),
        hydrations.clone(),
    );
    compute_stable_test_root(
        cx,
        &mut retained,
        root,
        size(
            AvailableSpace::Definite(px(2200.0)),
            AvailableSpace::Definite(px(1520.0)),
        ),
    );
    retained.finish_frame();

    measure_invocations.set(0);
    hydrations.set(0);
    retained.reset_retained_mutation_sample_for_tests();
    let (root, panel, flex_child, canvas_host, canvas) = request_frame(
        &mut retained,
        measure_invocations.clone(),
        hydrations.clone(),
    );
    compute_stable_test_root(
        cx,
        &mut retained,
        root,
        size(
            AvailableSpace::Definite(px(2200.0)),
            AvailableSpace::Definite(px(1520.0)),
        ),
    );
    let retained_root = retained.retained_node_token_for_tests(root);
    let retained_panel = retained.retained_node_token_for_tests(panel);
    let retained_flex_child = retained.retained_node_token_for_tests(flex_child);
    let retained_canvas_host = retained.retained_node_token_for_tests(canvas_host);
    let retained_canvas = retained.retained_node_token_for_tests(canvas);

    let mut fresh = LayoutEngine::new();
    let fresh_measure_invocations = Rc::new(Cell::new(0));
    let fresh_hydrations = Rc::new(Cell::new(0));
    let (root, panel, flex_child, canvas_host, canvas) =
        request_frame(&mut fresh, fresh_measure_invocations, fresh_hydrations);
    compute_stable_test_root(
        cx,
        &mut fresh,
        root,
        size(
            AvailableSpace::Definite(px(2200.0)),
            AvailableSpace::Definite(px(1520.0)),
        ),
    );
    let fresh_root = fresh.retained_node_token_for_tests(root);
    let fresh_panel = fresh.retained_node_token_for_tests(panel);
    let fresh_flex_child = fresh.retained_node_token_for_tests(flex_child);
    let fresh_canvas_host = fresh.retained_node_token_for_tests(canvas_host);
    let fresh_canvas = fresh.retained_node_token_for_tests(canvas);

    assert_facts_committed_exactly(&retained, root);
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            retained_node_size(&fresh, fresh_panel),
            retained_node_size(&fresh, fresh_flex_child),
            retained_node_size(&fresh, fresh_canvas_host),
            retained_node_size(&fresh, fresh_canvas),
        )
    );
    assert!(retained_node_size(&retained, retained_canvas).height > 0.0);
    assert_eq!((measure_invocations.get(), hydrations.get()), (0, 1));
    assert_eq!(
        retained.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 15,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn exact_canvas_sibling_recomputes_after_sidebar_subtree_change() {
    let mut retained = LayoutEngine::new();
    let (root, _panel, _flex_child, _canvas_host, _canvas) =
        request_canvas_like_chrome_frame_with_sidebar_content(&mut retained, 480.0, 0.0);
    compute_layout_without_measure(&mut retained, root, 2200.0, 0.0);
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame_with_sidebar_content(&mut retained, 480.0, 240.0);
    let retained_root = compute_layout_without_measure(&mut retained, root, 2200.0, 1522.0);
    let retained_panel = retained.retained_node_token_for_tests(panel);
    let retained_flex_child = retained.retained_node_token_for_tests(flex_child);
    let retained_canvas_host = retained.retained_node_token_for_tests(canvas_host);
    let retained_canvas = retained.retained_node_token_for_tests(canvas);

    let mut fresh = LayoutEngine::new();
    let (root, panel, flex_child, canvas_host, canvas) =
        request_canvas_like_chrome_frame_with_sidebar_content(&mut fresh, 480.0, 240.0);
    let fresh_root = compute_layout_without_measure(&mut fresh, root, 2200.0, 1522.0);
    let fresh_panel = fresh.retained_node_token_for_tests(panel);
    let fresh_flex_child = fresh.retained_node_token_for_tests(flex_child);
    let fresh_canvas_host = fresh.retained_node_token_for_tests(canvas_host);
    let fresh_canvas = fresh.retained_node_token_for_tests(canvas);

    assert_facts_committed_exactly(&retained, root);
    assert_eq!(
        retained_layout_projection(&retained, retained_root),
        retained_layout_projection(&fresh, fresh_root)
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            retained_node_size(&fresh, fresh_panel),
            retained_node_size(&fresh, fresh_flex_child),
            retained_node_size(&fresh, fresh_canvas_host),
            retained_node_size(&fresh, fresh_canvas),
        )
    );
    assert_eq!(
        (
            retained_node_size(&retained, retained_panel),
            retained_node_size(&retained, retained_flex_child),
            retained_node_size(&retained, retained_canvas_host),
            retained_node_size(&retained, retained_canvas),
        ),
        (
            size(1678.0, 1219.0),
            size(1676.0, 1145.0),
            size(1676.0, 1145.0),
            size(1676.0, 1145.0),
        )
    );
}

#[test]
#[should_panic(expected = "retained layout root should be solved at most once per frame")]
fn repeated_root_layout_recompute_panics() {
    let mut engine = LayoutEngine::new();
    let child = request_full_leaf(&mut engine);
    let root = request_full_container(&mut engine, &[child]);

    compute_layout_without_measure(&mut engine, root, 0.0, 100.0);
    compute_layout_without_measure(&mut engine, root, 800.0, 100.0);
}

#[test]
#[should_panic(expected = "layout root must not already be committed under a parent")]
fn intent_committed_under_parent_cannot_be_computed_as_root() {
    let mut engine = LayoutEngine::new();
    let child = request_full_leaf(&mut engine);
    let root = request_full_container(&mut engine, &[child]);

    compute_layout_without_measure(&mut engine, root, 800.0, 100.0);
    engine.commit_root_layout(child);
}

#[test]
#[should_panic(expected = "retained layout root should be committed at most once per frame")]
fn intent_computed_as_root_cannot_later_be_committed_under_parent() {
    let mut engine = LayoutEngine::new();
    let child = request_full_leaf(&mut engine);

    compute_layout_without_measure(&mut engine, child, 800.0, 100.0);

    let root = request_full_container(&mut engine, &[child]);
    engine.commit_root_layout(root);
}
