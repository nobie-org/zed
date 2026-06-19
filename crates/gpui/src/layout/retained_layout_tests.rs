use super::*;
use crate::{
    AbsoluteLength, DefiniteLength, Display, Drawable, Edges, Element, ElementId, FlexDirection,
    FlexWrap, GlobalElementId, GridPlacement, GridTemplate, InspectorElementId, IntoElement,
    Length, Overflow, ParentElement as _, Position, SharedString, Styled as _,
    TemplateColumnMinSize, TestAppContext, TextOverflow, TextStyle, VisualTestContext, WhiteSpace,
    div, point, px, size,
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
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
        engine.compute_retained_layout(
            root,
            RetainedLayoutRootId::new(0),
            available_space,
            window,
            app,
        );
    });
}

fn request_full_block_leaf(engine: &mut LayoutEngine) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.size = Size::full();
    engine.request_layout(style, px(16.0), 1.0, &[])
}

fn request_fixed_height_leaf(engine: &mut LayoutEngine, height: f32) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.size.height =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(height))));
    engine.request_layout(style, px(16.0), 1.0, &[])
}

fn request_flex_column_container(engine: &mut LayoutEngine, children: &[LayoutId]) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Flex;
    style.flex_direction = FlexDirection::Column;
    style.size = Size::full();
    engine.request_layout(style, px(16.0), 1.0, children)
}

fn request_growing_block_container(engine: &mut LayoutEngine, children: &[LayoutId]) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.min_size.height =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(0.0))));
    style.flex_grow = 1.0;
    style.flex_shrink = 1.0;
    engine.request_layout(style, px(16.0), 1.0, children)
}

fn request_live_panel_container(engine: &mut LayoutEngine, children: &[LayoutId]) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Flex;
    style.flex_direction = FlexDirection::Column;
    style.overflow = point(Overflow::Hidden, Overflow::Hidden);
    style.border_widths.left = AbsoluteLength::Pixels(px(2.0));
    style.border_widths.top = AbsoluteLength::Pixels(px(2.0));
    style.border_widths.bottom = AbsoluteLength::Pixels(px(2.0));
    style.flex_grow = 1.0;
    style.flex_shrink = 1.0;
    engine.request_layout(style, px(16.0), 1.0, children)
}

fn request_full_growing_hidden_block_container(
    engine: &mut LayoutEngine,
    children: &[LayoutId],
) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.size = Size::full();
    style.flex_grow = 1.0;
    style.flex_shrink = 1.0;
    style.overflow = point(Overflow::Hidden, Overflow::Hidden);
    engine.request_layout(style, px(16.0), 1.0, children)
}

fn request_absolute_leaf(engine: &mut LayoutEngine) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.position = Position::Absolute;
    engine.request_layout(style, px(16.0), 1.0, &[])
}

fn request_canvas_like_flex_frame(
    engine: &mut LayoutEngine,
    header_height: f32,
) -> (LayoutId, LayoutId, LayoutId, LayoutId) {
    let canvas = request_full_block_leaf(engine);
    let overlay = request_absolute_leaf(engine);
    let canvas_host = request_full_growing_hidden_block_container(engine, &[canvas, overlay]);
    let flex_child = request_growing_block_container(engine, &[canvas_host]);
    let header = request_fixed_height_leaf(engine, header_height);
    let panel = request_live_panel_container(engine, &[header, flex_child]);
    let root = request_flex_column_container(engine, &[panel]);
    (root, flex_child, canvas_host, canvas)
}

fn request_fixed_width_sidebar(engine: &mut LayoutEngine, width: f32) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Block;
    style.size.width =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(width))));
    style.size.height = Length::Definite(DefiniteLength::Fraction(1.0));
    engine.request_layout(style, px(16.0), 1.0, &[])
}

fn request_fixed_width_sidebar_with_content(
    engine: &mut LayoutEngine,
    width: f32,
    content_height: f32,
) -> LayoutId {
    let content = request_fixed_height_leaf(engine, content_height);
    let mut style = Style::default();
    style.display = Display::Block;
    style.size.width =
        Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(px(width))));
    style.size.height = Length::Definite(DefiniteLength::Fraction(1.0));
    engine.request_layout(style, px(16.0), 1.0, &[content])
}

fn request_flex_row_root(engine: &mut LayoutEngine, children: &[LayoutId]) -> LayoutId {
    let mut style = Style::default();
    style.display = Display::Flex;
    style.flex_direction = FlexDirection::Row;
    style.size = Size::full();
    engine.request_layout(style, px(16.0), 1.0, children)
}

fn request_canvas_like_workspace_row(
    engine: &mut LayoutEngine,
    header_height: f32,
    sidebar_width: Option<f32>,
) -> (LayoutId, LayoutId, LayoutId, LayoutId, LayoutId) {
    let canvas = request_full_block_leaf(engine);
    let overlay = request_absolute_leaf(engine);
    let canvas_host = request_full_growing_hidden_block_container(engine, &[canvas, overlay]);
    let flex_child = request_growing_block_container(engine, &[canvas_host]);
    let header = request_fixed_height_leaf(engine, header_height);
    let panel = request_live_panel_container(engine, &[header, flex_child]);
    let root = match sidebar_width {
        Some(sidebar_width) => {
            let sidebar = request_fixed_width_sidebar(engine, sidebar_width);
            request_flex_row_root(engine, &[panel, sidebar])
        }
        None => request_flex_row_root(engine, &[panel]),
    };
    (root, panel, flex_child, canvas_host, canvas)
}

fn request_growing_flex_row_container(
    engine: &mut LayoutEngine,
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
    engine.request_layout(style, px(16.0), 1.0, children)
}

fn request_growing_flex_column_container(
    engine: &mut LayoutEngine,
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
    engine.request_layout(style, px(16.0), 1.0, children)
}

fn request_padded_growing_flex_row_container(
    engine: &mut LayoutEngine,
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
    engine.request_layout(style, px(16.0), 1.0, children)
}

fn request_canvas_like_chrome_frame(
    engine: &mut LayoutEngine,
    sidebar_width: f32,
) -> (LayoutId, LayoutId, LayoutId, LayoutId, LayoutId) {
    request_canvas_like_chrome_frame_with_sidebar(engine, |engine| {
        request_fixed_width_sidebar(engine, sidebar_width)
    })
}

fn request_canvas_like_chrome_frame_with_sidebar_content(
    engine: &mut LayoutEngine,
    sidebar_width: f32,
    sidebar_content_height: f32,
) -> (LayoutId, LayoutId, LayoutId, LayoutId, LayoutId) {
    request_canvas_like_chrome_frame_with_sidebar(engine, |engine| {
        request_fixed_width_sidebar_with_content(engine, sidebar_width, sidebar_content_height)
    })
}

fn request_canvas_like_chrome_frame_with_sidebar(
    engine: &mut LayoutEngine,
    request_sidebar: impl FnOnce(&mut LayoutEngine) -> LayoutId,
) -> (LayoutId, LayoutId, LayoutId, LayoutId, LayoutId) {
    let canvas = request_full_block_leaf(engine);
    let overlay = request_absolute_leaf(engine);
    let canvas_host = request_full_growing_hidden_block_container(engine, &[canvas, overlay]);
    let flex_child = request_growing_block_container(engine, &[canvas_host]);
    let header = request_fixed_height_leaf(engine, 70.0);
    let panel = request_live_panel_container(engine, &[header, flex_child]);
    let sidebar = request_sidebar(engine);
    let content_row = request_growing_flex_row_container(engine, &[panel, sidebar]);
    let padded_row = request_padded_growing_flex_row_container(engine, &[content_row]);
    let footer = request_fixed_height_leaf(engine, 98.0);
    let lower_column = request_growing_flex_column_container(engine, &[padded_row, footer]);
    let main_row = request_growing_flex_row_container(engine, &[lower_column]);
    let top_chrome = request_fixed_height_leaf(engine, 156.0);
    let root = request_flex_column_container(engine, &[top_chrome, main_row]);
    (root, panel, flex_child, canvas_host, canvas)
}

fn request_measured(engine: &mut LayoutEngine, width: f32) -> LayoutId {
    engine.request_measured_layout(style_with_width(width), px(16.0), 1.0, move |_, _, _, _| {
        size(px(width), px(10.0))
    })
}

fn request_auto_measured(engine: &mut LayoutEngine, width: f32) -> LayoutId {
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

fn request_counted_measured(engine: &mut LayoutEngine, drops: Rc<Cell<usize>>) -> LayoutId {
    let counter = DropCounter(drops);
    engine.request_measured_layout(Style::default(), px(16.0), 1.0, move |_, _, _, _| {
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
    let artifact = TextLayoutArtifact::for_tests(key.clone(), size);
    engine.request_text_measured_layout(
        style,
        px(16.0),
        1.0,
        key,
        |_| {},
        move |_, _, _, _| artifact.clone(),
    )
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

    fn text_count(&self) -> u64 {
        match self {
            Self::Unmeasured { children, .. } => children.iter().map(Self::text_count).sum(),
            Self::Text { .. } => 1,
            Self::PureSize { .. } | Self::Opaque { .. } => 0,
        }
    }

    fn contains_opaque(&self) -> bool {
        match self {
            Self::Unmeasured { children, .. } => children.iter().any(Self::contains_opaque),
            Self::PureSize { .. } | Self::Text { .. } => false,
            Self::Opaque { .. } => true,
        }
    }

    fn retained_occurrence_matches_intent(&self, current: &Self) -> bool {
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
                            previous.retained_occurrence_matches_intent(current)
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

    fn retained_occurrence_can_update_intent(&self, current: &Self) -> bool {
        matches!(
            (self, current),
            (Self::Unmeasured { .. }, Self::Unmeasured { .. })
                | (Self::PureSize { .. }, Self::PureSize { .. })
                | (Self::Text { .. }, Self::Text { .. })
        )
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
            let artifact = TextLayoutArtifact::for_tests(
                key.clone(),
                size(px(*width as f32), px(*height as f32)),
            );
            engine.request_text_measured_layout(
                Style::default(),
                px(16.0),
                1.0,
                key,
                |_| {},
                move |_, _, _, _| artifact.clone(),
            )
        }
        GeneratedTree::Opaque { width } => {
            let width = *width;
            engine.request_measured_layout(
                style_with_width(width as f32),
                px(16.0),
                1.0,
                move |_, _, _, _| size(px(width as f32), px(10.0)),
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
fn commit_generated_roots(engine: &mut LayoutEngine, roots: &[LayoutId]) -> Vec<RetainedNodeToken> {
    roots
        .iter()
        .enumerate()
        .map(|(index, root)| {
            let node = engine.commit_layout_in_test_root(index as u64, *root);
            assert_intent_committed_exactly(engine, *root);
            node
        })
        .collect()
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
                engine.compute_retained_layout(
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
fn add_expected_snapshot_work(
    expected: &mut RetainedForestMutationSample,
    previous: &[GeneratedTree],
    current: &[GeneratedTree],
) {
    for index in 0..current.len() {
        match (previous.get(index), current.get(index)) {
            (Some(previous), Some(current))
                if previous.retained_occurrence_matches_intent(current)
                    && !current.contains_opaque() =>
            {
                expected.snapshot_hits += 1;
                expected.snapshot_text_artifact_replays += current.text_count();
            }
            _ => {
                expected.snapshot_misses += 1;
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
trait ExpectedMutationCountsExt {
    fn add_commit(&mut self, previous: &GeneratedTree, current: &GeneratedTree) -> bool;
    fn add_fresh_tree(&mut self, current: &GeneratedTree);
    fn add_remove_tree(&mut self, previous: &GeneratedTree);
    fn add_reused_tree(&mut self, current: &GeneratedTree);
}

#[cfg(not(target_arch = "wasm32"))]
impl ExpectedMutationCountsExt for RetainedForestMutationSample {
    fn add_commit(&mut self, previous: &GeneratedTree, current: &GeneratedTree) -> bool {
        if previous.retained_occurrence_matches_intent(current) {
            self.add_reused_tree(current);
            return true;
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
                let mut assigned_previous_indices = current_children
                    .iter()
                    .map(|current_child| {
                        let matching_previous_index = previous_children
                            .iter()
                            .enumerate()
                            .position(|(index, previous_child)| {
                                !previous_used[index]
                                    && previous_child
                                        .retained_occurrence_matches_intent(current_child)
                            });
                        if let Some(index) = matching_previous_index {
                            previous_used[index] = true;
                        }
                        matching_previous_index
                    })
                    .collect::<Vec<_>>();

                for (current_index, current_child) in current_children.iter().enumerate() {
                    if assigned_previous_indices[current_index].is_some() {
                        continue;
                    }
                    if let Some(previous_child) = previous_children.get(current_index) {
                        if !previous_used[current_index]
                            && previous_child.retained_occurrence_can_update_intent(current_child)
                        {
                            previous_used[current_index] = true;
                            assigned_previous_indices[current_index] = Some(current_index);
                        }
                    }
                }

                let mut child_list_changed = previous_children.len() != current_children.len();
                for (current_index, current_child) in current_children.iter().enumerate() {
                    let matching_previous_index = assigned_previous_indices[current_index];
                    if let Some(index) = matching_previous_index {
                        let reused_existing_node =
                            self.add_commit(&previous_children[index], current_child);
                        if !reused_existing_node || index != current_index {
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
                true
            }
            _ => {
                self.add_remove_tree(previous);
                self.add_fresh_tree(current);
                false
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

    fn add_reused_tree(&mut self, current: &GeneratedTree) {
        self.reuses += current.node_count();
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
        move |_, available_space, _, _| {
            measure_invocations.set(measure_invocations.get() + 1);
            let measured_width = match available_space.width {
                AvailableSpace::Definite(width) => width,
                AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                    px(fallback_width as f32)
                }
            };
            TextLayoutArtifact::for_tests(
                measure_key.clone(),
                size(measured_width, px(height as f32)),
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
    compute_layout_without_measure_with_available_space(
        engine,
        root,
        AvailableSpace::Definite(px(width)),
        AvailableSpace::Definite(px(height)),
    )
}

fn compute_layout_without_measure_with_available_space(
    engine: &mut LayoutEngine,
    root: LayoutId,
    width: AvailableSpace,
    height: AvailableSpace,
) -> RetainedNodeToken {
    engine.compute_unmeasured_layout_for_tests(root, size(width, height))
}

fn retained_node_size(engine: &LayoutEngine, node_id: RetainedNodeToken) -> Size<f32> {
    engine.retained_node_size_for_tests(node_id)
}

fn assert_intent_committed_exactly(engine: &LayoutEngine, id: LayoutId) {
    engine.assert_intent_committed_exactly_for_tests(id);
}

#[cfg(not(target_arch = "wasm32"))]
#[gpui::test]
fn generated_retained_commit_matches_fresh_and_expected_mutation_counts(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    hegel::Hegel::new(|tc| {
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
                assert_intent_committed_exactly(&retained, *root);
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
                assert_intent_committed_exactly(&fresh, *root);
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

            let mut expected = expected_mutations(&previous_roots, &frame.roots);
            add_expected_snapshot_work(&mut expected, &previous_roots, &frame.roots);
            retained.finish_frame();
            assert_eq!(retained.retained_mutation_sample_for_tests(), expected);

            previous_roots = frame.roots;
        }
    })
    .settings(hegel_settings(100))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn generated_reusable_exact_repeat_emits_no_retained_mutations() {
    hegel::Hegel::new(|tc| {
        let frame = draw_generated_frame(&tc, false, 3);
        let mut engine = LayoutEngine::new();

        let roots = request_generated_frame(&mut engine, &frame);
        commit_generated_roots(&mut engine, &roots);
        engine.finish_frame();

        engine.reset_retained_mutation_sample_for_tests();
        let roots = request_generated_frame(&mut engine, &frame);
        commit_generated_roots(&mut engine, &roots);
        engine.finish_frame();

        assert_eq!(
            engine.retained_mutation_sample_for_tests(),
            expected_mutations(&frame.roots, &frame.roots)
        );
    })
    .settings(hegel_settings(100))
    .run();
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn generated_rollback_restores_retained_layout_state_for_next_repeat() {
    hegel::Hegel::new(|tc| {
        let prefix = draw_generated_frame(&tc, false, 3);
        let real = draw_generated_frame(&tc, false, 3);
        let transient = draw_generated_frame(&tc, true, 3);

        let mut with_rollback = LayoutEngine::new();
        let prefix_roots = request_generated_frame(&mut with_rollback, &prefix);
        commit_generated_roots(&mut with_rollback, &prefix_roots);
        with_rollback.finish_frame();
        with_rollback.reset_retained_mutation_sample_for_tests();

        let checkpoint = with_rollback.checkpoint();
        let transient_roots = request_generated_frame(&mut with_rollback, &transient);
        commit_generated_roots(&mut with_rollback, &transient_roots);
        with_rollback.rollback_to_checkpoint(checkpoint);

        let real_roots = request_generated_frame(&mut with_rollback, &real);
        let with_rollback_roots = commit_generated_roots(&mut with_rollback, &real_roots);
        let expected_real_mutations = expected_mutations(&prefix.roots, &real.roots);

        let mut skipped = LayoutEngine::new();
        let prefix_roots = request_generated_frame(&mut skipped, &prefix);
        commit_generated_roots(&mut skipped, &prefix_roots);
        skipped.finish_frame();
        skipped.reset_retained_mutation_sample_for_tests();
        let real_roots = request_generated_frame(&mut skipped, &real);
        let skipped_roots = commit_generated_roots(&mut skipped, &real_roots);

        assert_eq!(
            retained_layout_shapes(&with_rollback, &with_rollback_roots),
            retained_layout_shapes(&skipped, &skipped_roots)
        );

        with_rollback.finish_frame();
        skipped.finish_frame();
        assert_eq!(
            with_rollback.retained_mutation_sample_for_tests(),
            expected_real_mutations
        );
        assert_eq!(
            skipped.retained_mutation_sample_for_tests(),
            expected_real_mutations
        );

        with_rollback.reset_retained_mutation_sample_for_tests();
        let repeat_roots = request_generated_frame(&mut with_rollback, &real);
        commit_generated_roots(&mut with_rollback, &repeat_roots);
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
        let _ = engine.layout_bounds(child, window.scale_factor());
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
fn generated_text_exact_repeat_hydrates_from_root_snapshot(cx: &mut TestAppContext) {
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
        assert_eq!(engine.layout_work_sample().solver_compute_layout_calls, 0);
        assert_eq!(engine.retained_mutation_sample_for_tests().snapshot_hits, 1);
        assert_eq!(
            engine
                .retained_mutation_sample_for_tests()
                .snapshot_text_artifact_replays,
            1
        );
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
            engine.retained_mutation_sample_for_tests().snapshot_misses,
            1
        );
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
        assert_eq!(engine.layout_work_sample().solver_compute_layout_calls, 0);
        assert_eq!(engine.retained_mutation_sample_for_tests().snapshot_hits, 1);
        assert_eq!(
            engine
                .retained_mutation_sample_for_tests()
                .snapshot_text_artifact_replays,
            1
        );
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
    assert_intent_committed_exactly(&retained, retained_second_root);

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
fn changed_unmeasured_child_reuses_position_and_updates_style() {
    let mut engine = LayoutEngine::new();
    let first_root = request_row(&mut engine, &[10.0, 20.0]);
    let first_root_node = engine.commit_layout(first_root);
    let first_child_nodes = engine.retained_child_tokens_for_tests(first_root_node);
    engine.finish_frame();

    engine.reset_retained_mutation_sample_for_tests();
    let second_root = request_row(&mut engine, &[10.0, 30.0]);
    let second_root_node = engine.commit_layout(second_root);
    assert_intent_committed_exactly(&engine, second_root);
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
fn changed_ancestor_reuses_unmeasured_path_and_updates_changed_leaf() {
    let mut engine = LayoutEngine::new();
    let stable_grandchild = request_leaf(&mut engine, 10.0);
    let changing_grandchild = request_leaf(&mut engine, 20.0);
    let changed_child = request_container(&mut engine, &[stable_grandchild, changing_grandchild]);
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
    let changing_grandchild = request_leaf(&mut engine, 30.0);
    let changed_child = request_container(&mut engine, &[stable_grandchild, changing_grandchild]);
    let stable_sibling = request_leaf(&mut engine, 99.0);
    let second_root = request_container(&mut engine, &[changed_child, stable_sibling]);
    let second_root_node = engine.commit_layout(second_root);
    assert_intent_committed_exactly(&engine, second_root);

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
        assert_intent_committed_exactly(&retained, retained_root);

        let mut fresh = LayoutEngine::new();
        let fresh_root = request_row(&mut fresh, widths);
        let fresh_root_node = fresh.commit_layout(fresh_root);
        assert_intent_committed_exactly(&fresh, fresh_root);

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
    );
    retained.finish_frame();

    retained.reset_retained_mutation_sample_for_tests();
    let root = request_flex_row(&mut retained, &[100.0]);
    let retained_root = compute_layout_without_measure_with_available_space(
        &mut retained,
        root,
        AvailableSpace::MaxContent,
        AvailableSpace::Definite(px(100.0)),
    );

    let mut fresh = LayoutEngine::new();
    let fresh_root = request_flex_row(&mut fresh, &[100.0]);
    let fresh_root = compute_layout_without_measure_with_available_space(
        &mut fresh,
        fresh_root,
        AvailableSpace::MaxContent,
        AvailableSpace::Definite(px(100.0)),
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

    assert_intent_committed_exactly(&retained, second_root);
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

    assert_intent_committed_exactly(&retained, second_root);
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
fn duplicate_exact_children_reuse_at_most_one_previous_node() {
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
    assert_intent_committed_exactly(&engine, second_root);

    let child_node = engine.retained_node_token_for_tests(child);
    assert!(first_child_nodes.contains(&child_node));
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
    assert_intent_committed_exactly(&engine, root);

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
    assert_intent_committed_exactly(&engine, leaf);

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
    assert_intent_committed_exactly(&engine, leaf);
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
    assert_intent_committed_exactly(&engine, measured);

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
    assert_intent_committed_exactly(&engine, root);

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
fn unchanged_pure_size_measure_reuses_taffy_cache(cx: &mut TestAppContext) {
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
            solver_compute_layout_calls: 0,
            measured_layout_calls: 0,
            compute_layout_duration: engine.layout_work_sample().compute_layout_duration,
            measured_layout_duration: Duration::default(),
            ..LayoutWorkSample::default()
        }
    );
    assert_eq!(engine.retained_mutation_sample_for_tests().snapshot_hits, 1);
}

#[gpui::test]
fn unchanged_text_measure_replays_root_snapshot(cx: &mut TestAppContext) {
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
    assert_eq!(engine.layout_work_sample().solver_compute_layout_calls, 0);
    assert_eq!(engine.retained_mutation_sample_for_tests().snapshot_hits, 1);
    assert_eq!(
        engine
            .retained_mutation_sample_for_tests()
            .snapshot_text_artifact_replays,
        1
    );
}

#[gpui::test]
fn unchanged_nested_text_measure_replays_root_snapshot(cx: &mut TestAppContext) {
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

    assert_intent_committed_exactly(&engine, root);
    assert_eq!((measure_invocations.get(), hydrations.get()), (0, 1));
    assert_eq!(engine.layout_work_sample().measured_layout_calls, 0);
    assert_eq!(engine.layout_work_sample().solver_compute_layout_calls, 0);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 2,
            snapshot_hits: 1,
            snapshot_text_artifact_replays: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[gpui::test]
fn changed_unmeasured_sibling_remeasures_text_child(cx: &mut TestAppContext) {
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

    assert_eq!((measure_invocations.get(), hydrations.get()), (1, 1));
    assert_eq!(engine.layout_work_sample().measured_layout_calls, 1);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 3,
            style_updates: 1,
            snapshot_misses: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[gpui::test]
fn changed_unmeasured_sibling_remeasures_nested_text_child(cx: &mut TestAppContext) {
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
    assert_eq!((measure_invocations.get(), hydrations.get()), (1, 1));
    assert_eq!(engine.layout_work_sample().measured_layout_calls, 1);
    assert_eq!(
        engine.retained_mutation_sample_for_tests(),
        RetainedForestMutationSample {
            reuses: 4,
            style_updates: 1,
            snapshot_misses: 1,
            ..RetainedForestMutationSample::default()
        }
    );
}

#[test]
fn same_text_measure_key_with_changed_taffy_style_reuses_text_node() {
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
    assert_intent_committed_exactly(&engine, text);

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
        move |known_dimensions, available_space, window, app| {
            measure_key.measure(known_dimensions, available_space, window, app)
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
    let root = engine.request_measured_layout(Style::default(), px(16.0), 1.0, |_, _, _, _| {
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
    assert_eq!(engine.layout_work_sample().solver_compute_layout_calls, 0);
    assert_eq!(engine.retained_mutation_sample_for_tests().snapshot_hits, 1);
    assert_eq!(
        engine
            .retained_mutation_sample_for_tests()
            .snapshot_text_artifact_replays,
        1
    );
}

#[gpui::test]
fn window_transact_discards_failed_text_prepaint_state(cx: &mut TestAppContext) {
    #[derive(Clone)]
    struct TextTransactionElement {
        prepaint_log: Rc<RefCell<Vec<&'static str>>>,
        paint_log: Rc<RefCell<Vec<&'static str>>>,
    }

    impl IntoElement for TextTransactionElement {
        type Element = Self;

        fn into_element(self) -> Self::Element {
            self
        }
    }

    impl Element for TextTransactionElement {
        type RequestLayoutState = ();
        type PrepaintState = (Drawable<SharedString>, &'static str);

        fn id(&self) -> Option<ElementId> {
            None
        }

        fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
            None
        }

        fn request_layout(
            &mut self,
            _id: Option<&GlobalElementId>,
            _inspector_id: Option<&InspectorElementId>,
            window: &mut Window,
            cx: &mut App,
        ) -> (LayoutId, Self::RequestLayoutState) {
            (window.request_layout(Style::default(), [], cx), ())
        }

        fn prepaint(
            &mut self,
            _id: Option<&GlobalElementId>,
            _inspector_id: Option<&InspectorElementId>,
            bounds: Bounds<Pixels>,
            _request_layout: &mut Self::RequestLayoutState,
            window: &mut Window,
            cx: &mut App,
        ) -> Self::PrepaintState {
            let transient: Result<(), ()> = window.transact(|window| {
                let mut transient = Drawable::new(SharedString::from("transient"));
                transient.layout_as_root(
                    bounds.size.into(),
                    RetainedLayoutRootSite::caller(core::panic::Location::caller()),
                    window,
                    cx,
                );
                window.with_absolute_element_offset(bounds.origin, |window| {
                    transient.prepaint(window, cx)
                });
                self.prepaint_log.borrow_mut().push("transient");
                Err(())
            });
            assert_eq!(transient, Err(()));

            let mut committed = Drawable::new(SharedString::from("committed"));
            committed.layout_as_root(
                bounds.size.into(),
                RetainedLayoutRootSite::caller(core::panic::Location::caller()),
                window,
                cx,
            );
            window.with_absolute_element_offset(bounds.origin, |window| {
                committed.prepaint(window, cx)
            });
            self.prepaint_log.borrow_mut().push("committed");
            (committed, "committed")
        }

        fn paint(
            &mut self,
            _id: Option<&GlobalElementId>,
            _inspector_id: Option<&InspectorElementId>,
            _bounds: Bounds<Pixels>,
            _request_layout: &mut Self::RequestLayoutState,
            prepaint: &mut Self::PrepaintState,
            window: &mut Window,
            cx: &mut App,
        ) {
            self.paint_log.borrow_mut().push(prepaint.1);
            prepaint.0.paint(window, cx);
        }
    }

    let prepaint_log = Rc::new(RefCell::new(Vec::new()));
    let paint_log = Rc::new(RefCell::new(Vec::new()));
    let cx = cx.add_empty_window();
    let _ = cx.draw(
        point(px(0.0), px(0.0)),
        size(
            AvailableSpace::Definite(px(240.0)),
            AvailableSpace::MaxContent,
        ),
        |_, _| TextTransactionElement {
            prepaint_log: prepaint_log.clone(),
            paint_log: paint_log.clone(),
        },
    );

    assert_eq!(&*prepaint_log.borrow(), &["transient", "committed"]);
    assert_eq!(&*paint_log.borrow(), &["committed"]);
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
            creates: 1,
            context_clears: 1,
            removes: 1,
            snapshot_misses: 1,
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
#[should_panic(expected = "layout intent should appear only once")]
fn duplicate_intent_references_fail_loudly() {
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

    assert_intent_committed_exactly(&retained, root);
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

    assert_intent_committed_exactly(&retained, root);
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

    assert_intent_committed_exactly(&retained, root);
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

    assert_intent_committed_exactly(&retained, root);
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

    assert_intent_committed_exactly(&retained, root);
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
