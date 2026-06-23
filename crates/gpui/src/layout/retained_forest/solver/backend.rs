//! Taffy implementation of the retained-forest solver facade.
//!
//! This module is the only retained-layout code that may name Taffy. It
//! translates GPUI layout facts into Taffy styles, mirrors retained-tree
//! mutations into a `TaffyTree`, and runs the legal root solve.

use super::super::super::{AvailableSpace, EXPECT_MESSAGE};
use super::super::measurement::NodeContext;
use super::{
    FreshSolverNodeId, SolverBackend, SolverLayout, SolverMeasureQuery, SolverNodeId, SolverStyle,
};
use crate::{
    AbsoluteLength, DefiniteLength, Edges, GridTemplate, Length, Pixels, Point, Size, Style, point,
    size,
    util::{round_stroke_to_device_pixel, round_to_device_pixel},
};
use std::fmt::{self, Debug};
use std::ops::Range;
use taffy::{
    TaffyTree,
    geometry::{Point as TaffyPoint, Rect as TaffyRect, Size as TaffySize},
    prelude::{TaffyGridLine, TaffyGridSpan, max_content, min_content},
    style::AvailableSpace as TaffyAvailableSpace,
    tree::{Layout as TaffyLayout, NodeId},
};

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(super) struct BackendNodeId(NodeId);

impl Debug for BackendNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("BackendNodeId").field(&self.0).finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct FreshBackendNodeId(NodeId);

#[derive(Clone, PartialEq)]
pub(super) struct BackendStyle(taffy::style::Style);

impl Debug for BackendStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl BackendStyle {
    pub(super) fn from_gpui_style(style: &Style, rem_size: Pixels, scale_factor: f32) -> Self {
        Self(style.to_taffy(rem_size, scale_factor))
    }

    #[cfg(test)]
    pub(super) fn test_with_size(width: f32, height: f32) -> Self {
        Self(taffy::style::Style {
            size: TaffySize {
                width: taffy::style::Dimension::length(width),
                height: taffy::style::Dimension::length(height),
            },
            ..taffy::style::Style::default()
        })
    }
}

impl Default for BackendStyle {
    fn default() -> Self {
        Self(taffy::style::Style::default())
    }
}

impl From<TaffyLayout> for SolverLayout {
    fn from(layout: TaffyLayout) -> Self {
        Self {
            location: point(layout.location.x, layout.location.y),
            size: size(layout.size.width, layout.size.height),
        }
    }
}

#[derive(Clone)]
pub(super) struct SolverBackendImpl {
    taffy: TaffyTree<NodeContext>,
}

impl SolverBackend for SolverBackendImpl {
    fn new() -> Self {
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        Self { taffy }
    }

    fn new_leaf(&mut self, style: SolverStyle) -> SolverNodeId {
        SolverNodeId(BackendNodeId(
            self.taffy.new_leaf(style.0.0).expect(EXPECT_MESSAGE),
        ))
    }

    fn new_with_children(&mut self, style: SolverStyle, children: &[SolverNodeId]) -> SolverNodeId {
        let children = children.iter().map(|child| child.0.0).collect::<Vec<_>>();
        SolverNodeId(BackendNodeId(
            self.taffy
                .new_with_children(style.0.0, &children)
                .expect(EXPECT_MESSAGE),
        ))
    }

    fn new_measured(&mut self, style: SolverStyle) -> SolverNodeId {
        SolverNodeId(BackendNodeId(
            self.taffy
                .new_leaf_with_context(style.0.0, NodeContext)
                .expect(EXPECT_MESSAGE),
        ))
    }

    fn set_style(&mut self, node_id: SolverNodeId, style: SolverStyle) {
        self.taffy
            .set_style(node_id.0.0, style.0.0)
            .expect(EXPECT_MESSAGE);
    }

    fn set_children(&mut self, node_id: SolverNodeId, children: &[SolverNodeId]) {
        let children = children.iter().map(|child| child.0.0).collect::<Vec<_>>();
        self.taffy
            .set_children(node_id.0.0, &children)
            .expect(EXPECT_MESSAGE);
    }

    fn clear_measure_context(&mut self, node_id: SolverNodeId) {
        self.taffy
            .set_node_context(node_id.0.0, None)
            .expect(EXPECT_MESSAGE);
    }

    fn remove(&mut self, node_id: SolverNodeId) {
        self.taffy.remove(node_id.0.0).expect(EXPECT_MESSAGE);
    }

    fn mark_dirty(&mut self, node_id: SolverNodeId) {
        self.taffy.mark_dirty(node_id.0.0).expect(EXPECT_MESSAGE);
    }

    fn parent(&self, node_id: SolverNodeId) -> Option<SolverNodeId> {
        self.taffy
            .parent(node_id.0.0)
            .map(|node_id| SolverNodeId(BackendNodeId(node_id)))
    }

    fn children(&self, node_id: SolverNodeId) -> Vec<SolverNodeId> {
        self.taffy
            .children(node_id.0.0)
            .expect(EXPECT_MESSAGE)
            .into_iter()
            .map(|node_id| SolverNodeId(BackendNodeId(node_id)))
            .collect()
    }

    fn capture_layout_tree(&self, root: SolverNodeId) -> Vec<(SolverNodeId, SolverLayout)> {
        let mut layouts = Vec::new();
        let mut stack = vec![root];

        while let Some(node_id) = stack.pop() {
            let layout = self
                .taffy
                .layout(node_id.0.0)
                .expect(EXPECT_MESSAGE)
                .to_owned()
                .into();
            layouts.push((node_id, layout));
            stack.extend(
                self.taffy
                    .children(node_id.0.0)
                    .expect(EXPECT_MESSAGE)
                    .into_iter()
                    .map(|child| SolverNodeId(BackendNodeId(child))),
            );
        }

        layouts
    }

    fn style(&self, node_id: SolverNodeId) -> Option<SolverStyle> {
        self.taffy
            .style(node_id.0.0)
            .ok()
            .cloned()
            .map(|style| SolverStyle(BackendStyle(style)))
    }

    fn has_measure_context(&self, node_id: SolverNodeId) -> bool {
        self.taffy.get_node_context(node_id.0.0).is_some()
    }

    fn compute_layout_with_measure(
        &mut self,
        root: SolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        mut measure: impl FnMut(SolverNodeId, bool, SolverMeasureQuery) -> Size<f32>,
    ) {
        let taffy_available_space = scale_available_space_for_taffy(available_space, scale_factor);
        self.taffy
            .compute_layout_with_measure(
                root.0.0,
                taffy_available_space,
                |known_dimensions, available_space, node_id, node_context, _style| {
                    let known_dimensions = size(
                        known_dimensions.width.map(|e| Pixels(e / scale_factor)),
                        known_dimensions.height.map(|e| Pixels(e / scale_factor)),
                    );
                    let available_space = size(
                        available_space_from_taffy(available_space.width, scale_factor),
                        available_space_from_taffy(available_space.height, scale_factor),
                    );
                    measure(
                        SolverNodeId(BackendNodeId(node_id)),
                        node_context.is_some(),
                        SolverMeasureQuery {
                            known_dimensions,
                            available_space,
                        },
                    )
                    .into()
                },
            )
            .expect(EXPECT_MESSAGE);
    }
}

pub(super) struct FreshSolverBackendImpl<C> {
    taffy: TaffyTree<C>,
}

impl<C> FreshSolverBackendImpl<C> {
    pub(super) fn new() -> Self {
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        Self { taffy }
    }

    pub(super) fn new_leaf(&mut self, style: SolverStyle) -> FreshSolverNodeId {
        FreshSolverNodeId(FreshBackendNodeId(
            self.taffy.new_leaf(style.0.0).expect(EXPECT_MESSAGE),
        ))
    }

    pub(super) fn new_with_children(
        &mut self,
        style: SolverStyle,
        children: &[FreshSolverNodeId],
    ) -> FreshSolverNodeId {
        let children = children.iter().map(|child| child.0.0).collect::<Vec<_>>();
        FreshSolverNodeId(FreshBackendNodeId(
            self.taffy
                .new_with_children(style.0.0, &children)
                .expect(EXPECT_MESSAGE),
        ))
    }

    pub(super) fn new_measured(&mut self, style: SolverStyle, context: C) -> FreshSolverNodeId {
        FreshSolverNodeId(FreshBackendNodeId(
            self.taffy
                .new_leaf_with_context(style.0.0, context)
                .expect(EXPECT_MESSAGE),
        ))
    }

    pub(super) fn compute_layout_with_measure(
        &mut self,
        root: FreshSolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        mut measure: impl FnMut(FreshSolverNodeId, Option<&C>, SolverMeasureQuery) -> Size<f32>,
    ) {
        let taffy_available_space = scale_available_space_for_taffy(available_space, scale_factor);
        self.taffy
            .compute_layout_with_measure(
                root.0.0,
                taffy_available_space,
                |known_dimensions, available_space, node_id, node_context, _style| {
                    let known_dimensions = size(
                        known_dimensions.width.map(|e| Pixels(e / scale_factor)),
                        known_dimensions.height.map(|e| Pixels(e / scale_factor)),
                    );
                    let available_space = size(
                        available_space_from_taffy(available_space.width, scale_factor),
                        available_space_from_taffy(available_space.height, scale_factor),
                    );
                    measure(
                        FreshSolverNodeId(FreshBackendNodeId(node_id)),
                        node_context.as_deref(),
                        SolverMeasureQuery {
                            known_dimensions,
                            available_space,
                        },
                    )
                    .into()
                },
            )
            .expect(EXPECT_MESSAGE);
    }

    pub(super) fn layout(&self, node_id: FreshSolverNodeId) -> SolverLayout {
        self.taffy
            .layout(node_id.0.0)
            .expect(EXPECT_MESSAGE)
            .to_owned()
            .into()
    }

    pub(super) fn children(&self, node_id: FreshSolverNodeId) -> Vec<FreshSolverNodeId> {
        self.taffy
            .children(node_id.0.0)
            .expect(EXPECT_MESSAGE)
            .into_iter()
            .map(|node_id| FreshSolverNodeId(FreshBackendNodeId(node_id)))
            .collect()
    }
}

fn scale_available_space_for_taffy(
    available_space: Size<AvailableSpace>,
    scale_factor: f32,
) -> TaffySize<TaffyAvailableSpace> {
    let transform = |space: AvailableSpace| match space {
        AvailableSpace::Definite(pixels) => {
            AvailableSpace::Definite(Pixels(pixels.0 * scale_factor))
        }
        AvailableSpace::MinContent => AvailableSpace::MinContent,
        AvailableSpace::MaxContent => AvailableSpace::MaxContent,
    };
    size(
        transform(available_space.width),
        transform(available_space.height),
    )
    .into()
}

fn available_space_from_taffy(value: TaffyAvailableSpace, scale_factor: f32) -> AvailableSpace {
    match value {
        TaffyAvailableSpace::Definite(value) => {
            AvailableSpace::Definite(Pixels(value / scale_factor))
        }
        TaffyAvailableSpace::MinContent => AvailableSpace::MinContent,
        TaffyAvailableSpace::MaxContent => AvailableSpace::MaxContent,
    }
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

fn align_items_to_taffy(value: crate::AlignItems) -> taffy::style::AlignItems {
    match value {
        crate::AlignItems::Start => taffy::style::AlignItems::START,
        crate::AlignItems::End => taffy::style::AlignItems::END,
        crate::AlignItems::FlexStart => taffy::style::AlignItems::FLEX_START,
        crate::AlignItems::FlexEnd => taffy::style::AlignItems::FLEX_END,
        crate::AlignItems::Center => taffy::style::AlignItems::CENTER,
        crate::AlignItems::Baseline => taffy::style::AlignItems::BASELINE,
        crate::AlignItems::Stretch => taffy::style::AlignItems::STRETCH,
    }
}

fn align_content_to_taffy(value: crate::AlignContent) -> taffy::style::AlignContent {
    match value {
        crate::AlignContent::Start => taffy::style::AlignContent::START,
        crate::AlignContent::End => taffy::style::AlignContent::END,
        crate::AlignContent::FlexStart => taffy::style::AlignContent::FLEX_START,
        crate::AlignContent::FlexEnd => taffy::style::AlignContent::FLEX_END,
        crate::AlignContent::Center => taffy::style::AlignContent::CENTER,
        crate::AlignContent::Stretch => taffy::style::AlignContent::STRETCH,
        crate::AlignContent::SpaceBetween => taffy::style::AlignContent::SPACE_BETWEEN,
        crate::AlignContent::SpaceEvenly => taffy::style::AlignContent::SPACE_EVENLY,
        crate::AlignContent::SpaceAround => taffy::style::AlignContent::SPACE_AROUND,
    }
}

fn display_to_taffy(value: crate::Display) -> taffy::style::Display {
    match value {
        crate::Display::Block => taffy::style::Display::Block,
        crate::Display::Flex => taffy::style::Display::Flex,
        crate::Display::Grid => taffy::style::Display::Grid,
        crate::Display::None => taffy::style::Display::None,
    }
}

fn flex_wrap_to_taffy(value: crate::FlexWrap) -> taffy::style::FlexWrap {
    match value {
        crate::FlexWrap::NoWrap => taffy::style::FlexWrap::NoWrap,
        crate::FlexWrap::Wrap => taffy::style::FlexWrap::Wrap,
        crate::FlexWrap::WrapReverse => taffy::style::FlexWrap::WrapReverse,
    }
}

fn flex_direction_to_taffy(value: crate::FlexDirection) -> taffy::style::FlexDirection {
    match value {
        crate::FlexDirection::Row => taffy::style::FlexDirection::Row,
        crate::FlexDirection::Column => taffy::style::FlexDirection::Column,
        crate::FlexDirection::RowReverse => taffy::style::FlexDirection::RowReverse,
        crate::FlexDirection::ColumnReverse => taffy::style::FlexDirection::ColumnReverse,
    }
}

fn overflow_to_taffy(value: crate::Overflow) -> taffy::style::Overflow {
    match value {
        crate::Overflow::Visible => taffy::style::Overflow::Visible,
        crate::Overflow::Clip => taffy::style::Overflow::Clip,
        crate::Overflow::Hidden => taffy::style::Overflow::Hidden,
        crate::Overflow::Scroll => taffy::style::Overflow::Scroll,
    }
}

fn position_to_taffy(value: crate::Position) -> taffy::style::Position {
    match value {
        crate::Position::Relative => taffy::style::Position::Relative,
        crate::Position::Absolute => taffy::style::Position::Absolute,
    }
}

fn grid_placement_to_taffy(placement: crate::GridPlacement) -> taffy::GridPlacement {
    match placement {
        crate::GridPlacement::Line(index) => taffy::GridPlacement::from_line_index(index),
        crate::GridPlacement::Span(span) => taffy::GridPlacement::from_span(span),
        crate::GridPlacement::Auto => taffy::GridPlacement::Auto,
    }
}

impl ToTaffy<taffy::style::Style> for Style {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Style {
        use taffy::style_helpers::{fr, length, minmax, repeat};

        fn to_grid_line(
            placement: &Range<crate::GridPlacement>,
        ) -> taffy::Line<taffy::GridPlacement> {
            taffy::Line {
                start: grid_placement_to_taffy(placement.start),
                end: grid_placement_to_taffy(placement.end),
            }
        }

        fn to_grid_repeat<T: taffy::style::CheapCloneStr>(
            unit: &Option<GridTemplate>,
        ) -> Vec<taffy::GridTemplateComponent<T>> {
            unit.map(|template| match template.min_size {
                crate::TemplateColumnMinSize::Zero => {
                    vec![repeat(
                        template.repeat,
                        vec![minmax(length(0.0_f32), fr(1.0_f32))],
                    )]
                }
                crate::TemplateColumnMinSize::MinContent => {
                    vec![repeat(
                        template.repeat,
                        vec![minmax(min_content(), fr(1.0_f32))],
                    )]
                }
                crate::TemplateColumnMinSize::MaxContent => {
                    vec![repeat(
                        template.repeat,
                        vec![minmax(length(0.0_f32), max_content())],
                    )]
                }
            })
            .unwrap_or_default()
        }

        taffy::style::Style {
            display: display_to_taffy(self.display),
            overflow: TaffyPoint {
                x: overflow_to_taffy(self.overflow.x),
                y: overflow_to_taffy(self.overflow.y),
            },
            scrollbar_width: self.scrollbar_width.to_taffy(rem_size, scale_factor),
            position: position_to_taffy(self.position),
            inset: self.inset.to_taffy(rem_size, scale_factor),
            size: self.size.to_taffy(rem_size, scale_factor),
            min_size: self.min_size.to_taffy(rem_size, scale_factor),
            max_size: self.max_size.to_taffy(rem_size, scale_factor),
            aspect_ratio: self.aspect_ratio,
            margin: self.margin.to_taffy(rem_size, scale_factor),
            padding: self.padding.to_taffy(rem_size, scale_factor),
            border: border_widths_to_taffy(&self.border_widths, rem_size, scale_factor),
            align_items: self.align_items.map(align_items_to_taffy),
            align_self: self.align_self.map(align_items_to_taffy),
            align_content: self.align_content.map(align_content_to_taffy),
            justify_content: self.justify_content.map(align_content_to_taffy),
            gap: self.gap.to_taffy(rem_size, scale_factor),
            flex_direction: flex_direction_to_taffy(self.flex_direction),
            flex_wrap: flex_wrap_to_taffy(self.flex_wrap),
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
