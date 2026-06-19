//! Retained GPUI layout forest and private Taffy mirror.
//!
//! The forest is the authority for retained layout occurrences. Taffy is kept
//! inside this module as a downstream mutable mirror used for layout execution
//! and caching; callers may request layout, compute roots, and read bounds, but
//! cannot see or mutate Taffy nodes directly.

use super::{
    AvailableSpace, EXPECT_MESSAGE, LayoutId, RetainedLayoutRootId, RetainedLayoutRootSite,
};
mod bounds_cache;
mod committed;
mod frame;
mod geometry;
mod measurement;
mod root_slots;
mod roots;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod state_tests;
mod subtree_probe;
mod work;
use crate::{
    AbsoluteLength, App, Bounds, DefiniteLength, Edges, ElementId, GlobalElementId, GridTemplate,
    Length, Pixels, Point, Size, Style, TextLayoutArtifact, TextMeasureKey, Window, size,
    util::{
        ceil_to_device_pixel, round_half_toward_zero, round_stroke_to_device_pixel,
        round_to_device_pixel,
    },
};
use bounds_cache::{BoundsCache, BoundsCacheCheckpoint};
use collections::FxHashSet;
use committed::{CommittedLayoutCheckpoint, CommittedLayoutState};
use frame::{FrameIntents, FrameIntentsCheckpoint};
use geometry::{GeometryStore, GeometryStoreCheckpoint};
pub(crate) use measurement::PureSizeMeasure;
use measurement::{
    CurrentMeasurement, LayoutMeasureContext, MeasuredLayoutKind, MeasuredLayoutResult,
    MeasurementStore, MeasurementStoreCheckpoint, NodeContext,
};
use root_slots::{RootSlots, RootSlotsCheckpoint};
use roots::{RootRegistry, RootRegistryCheckpoint};
use stacksafe::StackSafe;
use std::{
    collections::hash_map::DefaultHasher,
    fmt::Debug,
    hash::{Hash, Hasher},
    ops::Range,
    rc::Rc,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[cfg(test)]
pub(super) use subtree_probe::RetainedSubtreeWorkSample;
use subtree_probe::{SubtreeProbe, SubtreeProbeCheckpoint, SubtreeProbeComputeRecorder};
use taffy::{
    TaffyTree,
    geometry::{Point as TaffyPoint, Rect as TaffyRect, Size as TaffySize},
    prelude::{TaffyGridLine, TaffyGridSpan, max_content, min_content},
    style::AvailableSpace as TaffyAvailableSpace,
    tree::{Layout, NodeId},
};
#[cfg(test)]
pub(super) use work::RetainedForestMutationSample;
pub(super) use work::{RetainedLayoutMissWork, RetainedLayoutWork};
use work::{RetainedWorkCheckpoint, RetainedWorkState};

/// Pure layout request facts produced during the current frame.
///
/// A `LayoutIntent` is not retained authority. It is a temporary input that
/// says what GPUI wants this frame: a lowered Taffy style plus either child
/// intent ids or an explicit measured-node kind.
#[derive(Clone, Debug, PartialEq)]
struct LayoutIntent {
    global_id: Option<GlobalElementId>,
    style: taffy::style::Style,
    kind: LayoutIntentKind,
}

/// Current-frame layout node shape.
///
/// Measured nodes keep their executable producer separately from the comparable
/// `MeasuredLayoutKind`, so closure identity cannot leak into retention.
#[derive(Clone, Debug, PartialEq)]
enum LayoutIntentKind {
    Unmeasured {
        children: Vec<LayoutId>,
    },
    Measured {
        measure: Option<usize>,
        measured_kind: MeasuredLayoutKind,
    },
}

/// Retained node category used for exact occurrence compatibility.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetainedLayoutKind {
    Unmeasured,
    Measured,
}

/// Layout-visible facts retained with an occurrence.
///
/// These facts are the shallow comparison authority for deciding whether a
/// retained node's Taffy history is still valid for a current intent.
#[derive(Clone, Debug, PartialEq)]
struct RetainedLayoutFacts {
    style: taffy::style::Style,
    kind: RetainedLayoutKind,
    measured_kind: Option<MeasuredLayoutKind>,
}

/// Cross-frame GPUI layout occurrence.
///
/// Each occurrence owns exactly one Taffy `NodeId` plus the retained facts and
/// child occurrences that make that mirror node meaningful. Outside this module
/// the `NodeId` is never exposed.
#[derive(Clone, Debug, PartialEq)]
struct RetainedLayoutOccurrence {
    node_id: NodeId,
    facts: RetainedLayoutFacts,
    children: Vec<RetainedLayoutOccurrence>,
}

/// Result of committing one current-frame intent into the retained forest.
///
/// `layout_context_changed` reports that this occurrence or a descendant needs
/// the enclosing retained mirror path invalidated before the next legal solve.
/// Direct mirror mutations (`set_style`, `set_children`, create/remove) still
/// own their own Taffy dirtying; this flag exists for reused occurrence slots
/// whose retained history is no longer proven valid by exact equality alone.
struct RetainedLayoutCommit {
    occurrence: RetainedLayoutOccurrence,
    layout_context_changed: bool,
}

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
/// Taffy's `NodeId` type or making it part of GPUI's public layout model.
#[cfg(test)]
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(super) struct RetainedNodeToken(NodeId);

#[cfg(test)]
impl Debug for RetainedNodeToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RetainedNodeToken(..)")
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct RetainedStyleForTests(taffy::style::Style);

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
    location: Point<f32>,
    size: Size<f32>,
    children: Vec<RetainedLayoutProjectionForTests>,
}

/// Owner of retained GPUI layout occurrences and their private Taffy mirror.
///
/// All mirror mutations are encapsulated here. Methods such as
/// `request_layout`, `compute_layout`, and `finish_frame` move retained facts,
/// measurement artifacts, bounds caches, and Taffy state together so the forest
/// stays observationally equivalent to a freshly built layout tree.
pub(super) struct RetainedLayoutForest {
    taffy: TaffyTree<NodeContext>,
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
    taffy: TaffyTree<NodeContext>,
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
            overflow: self.overflow.map(overflow_to_taffy).into(),
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

fn retained_layout_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("GPUI_TRACE_RETAINED_LAYOUT").is_some())
}

fn retained_layout_miss_trace_limit() -> Option<usize> {
    static LIMIT: OnceLock<Option<usize>> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        let value = std::env::var("GPUI_TRACE_RETAINED_LAYOUT_MISSES").ok()?;
        if value.is_empty() {
            return Some(64);
        }
        value.parse::<usize>().ok().or(Some(64))
    })
}

fn retained_layout_fresh_compare_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("GPUI_TRACE_RETAINED_LAYOUT_FRESH_COMPARE").is_some())
}

fn retained_layout_zero_bounds_trace_limit() -> Option<usize> {
    static LIMIT: OnceLock<Option<usize>> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        let value = std::env::var("GPUI_TRACE_RETAINED_LAYOUT_ZERO_BOUNDS").ok()?;
        if value.is_empty() {
            return Some(128);
        }
        value.parse::<usize>().ok().or(Some(128))
    })
}

fn should_trace_retained_zero_bounds() -> bool {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let Some(limit) = retained_layout_zero_bounds_trace_limit() else {
        return false;
    };
    COUNT.fetch_add(1, Ordering::Relaxed) < limit
}

fn retained_layout_trace_layout_ids() -> Option<&'static Vec<usize>> {
    static LAYOUT_IDS: OnceLock<Option<Vec<usize>>> = OnceLock::new();
    LAYOUT_IDS
        .get_or_init(|| {
            let layout_ids = std::env::var("GPUI_TRACE_RETAINED_LAYOUT_IDS").ok()?;
            Some(
                layout_ids
                    .split(',')
                    .filter_map(|layout_id| layout_id.trim().parse().ok())
                    .collect(),
            )
        })
        .as_ref()
}

fn retained_layout_detail_trace_enabled() -> bool {
    retained_layout_trace_enabled() && retained_layout_trace_layout_ids().is_some()
}

fn trace_layout_id_is_targeted(layout_id: Option<usize>) -> bool {
    retained_layout_trace_layout_ids()
        .map(|target_layout_ids| {
            layout_id
                .map(|layout_id| target_layout_ids.contains(&layout_id))
                .unwrap_or(false)
        })
        .unwrap_or(true)
}

impl RetainedLayoutForest {
    /// Create an empty retained forest and configure the mirror for GPUI snapping.
    pub(super) fn new() -> Self {
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        Self {
            taffy,
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
        self.roots.begin_frame();
        self.measurements.begin_frame();
        self.geometry.begin_frame();
        self.subtree_probe.begin_frame();
        self.work.begin_frame();
    }

    /// Snapshot every retained and mirror field affected by speculative layout.
    pub(super) fn checkpoint(&self) -> RetainedLayoutForestCheckpoint {
        RetainedLayoutForestCheckpoint {
            taffy: self.taffy.clone(),
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
        self.taffy = checkpoint.taffy;
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
        element_id_stack: &[ElementId],
    ) -> RetainedLayoutRootId {
        self.roots
            .retained_root_id(root_site, global_id, element_id_stack)
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
        if retained_layout_trace_enabled() {
            if misses != RetainedLayoutMissWork::default() {
                eprintln!(
                    "gpui retained_layout match_miss_summary no_previous={} style={} kind={} measured_kind={} child_count={} child_subtree={} no_exact_child={}",
                    misses.no_previous,
                    misses.style,
                    misses.kind,
                    misses.measured_kind,
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
            style: style.to_taffy(rem_size, scale_factor),
            kind: LayoutIntentKind::Unmeasured {
                children: children.to_vec(),
            },
        })
    }

    /// Store a conservative measured intent backed by a current-frame producer.
    ///
    /// The producer is executable state, not retained meaning. Opaque measured
    /// intents therefore keep their callback current without letting Taffy skip
    /// work based on closure identity.
    pub(super) fn request_opaque_measured_layout(
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
        self.request_measured_layout(
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

    /// Store a measured intent whose size function is explicit comparable data.
    pub(super) fn request_pure_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measure: PureSizeMeasure,
    ) -> LayoutId {
        self.request_measured_layout(
            style,
            rem_size,
            scale_factor,
            MeasuredLayoutKind::PureSize(measure),
            None,
        )
    }

    /// Store a text measured intent with an artifact hydrator.
    ///
    /// The measure key is the comparable layout fact. The hydrator is a
    /// current-frame side effect used to install a cached or newly produced text
    /// artifact into the element's `TextLayout`.
    pub(super) fn request_text_measured_layout(
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
        self.request_measured_layout(
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

    /// Shared measured-intent constructor.
    ///
    /// `measure_context` is optional because pure-size measured nodes are
    /// executable from data alone; opaque and text nodes register a producer for
    /// this frame.
    fn request_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measured_kind: MeasuredLayoutKind,
        measure_context: Option<LayoutMeasureContext>,
    ) -> LayoutId {
        let measure = measure_context
            .map(|measure_context| self.measurements.push_producer_context(measure_context));

        self.push_intent(LayoutIntent {
            global_id: None,
            style: style.to_taffy(rem_size, scale_factor),
            kind: LayoutIntentKind::Measured {
                measure,
                measured_kind,
            },
        })
    }

    /// Allocate the next current-frame `LayoutId`.
    fn push_intent(&mut self, intent: LayoutIntent) -> LayoutId {
        self.frame.push_intent(intent)
    }

    #[cfg(test)]
    pub(super) fn set_retained_subtree_probe_targets_for_tests(&mut self, targets: Vec<String>) {
        self.subtree_probe.set_targets_for_tests(targets);
    }

    #[cfg(test)]
    pub(super) fn retained_subtree_work_samples_for_tests(&self) -> &[RetainedSubtreeWorkSample] {
        self.subtree_probe.samples_for_tests()
    }

    fn committed_node(&self, id: LayoutId) -> NodeId {
        self.committed.node(id)
    }

    fn committed_layout_id_for_node(&self, node_id: NodeId) -> Option<LayoutId> {
        self.committed.layout_id_for_node(node_id)
    }

    fn children(&self, node_id: NodeId) -> Vec<NodeId> {
        self.taffy.children(node_id).expect(EXPECT_MESSAGE)
    }

    fn parent(&self, node_id: NodeId) -> Option<NodeId> {
        self.taffy.parent(node_id)
    }

    fn geometry_layout(&self, node_id: NodeId) -> Layout {
        self.geometry.layout(node_id).unwrap_or_else(|| {
            panic!(
                "retained layout geometry should be captured before reading node {:?}",
                node_id
            )
        })
    }

    fn try_geometry_layout(&self, node_id: NodeId) -> Option<Layout> {
        self.geometry.layout(node_id)
    }

    fn try_solver_layout(&self, node_id: NodeId) -> Option<Layout> {
        self.taffy.layout(node_id).ok().cloned()
    }

    fn try_style(&self, node_id: NodeId) -> Option<taffy::style::Style> {
        self.taffy.style(node_id).ok().cloned()
    }

    /// Commit a root intent, compute the mirror, and hydrate measurement artifacts.
    pub(super) fn compute_layout(
        &mut self,
        root_id: RetainedLayoutRootId,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> ComputeLayoutWork {
        let root_layout_context_changed = self
            .root_slots
            .retained_root_node_id(root_id)
            .is_some_and(|root_node| {
                self.geometry.retained_root_solve_context_changed(
                    root_id,
                    root_node,
                    available_space,
                    scale_factor,
                )
            });
        let node_id = self.commit_root_layout(root_id, id, root_layout_context_changed);
        assert!(
            !self.geometry.has_solved_root(root_id),
            "retained layout root should be solved at most once per frame"
        );
        let repeated_root = !self.bounds.mark_computed(node_id);
        assert!(
            !repeated_root,
            "retained layout node should not be computed through multiple roots in one frame"
        );

        let taffy_available_space = scale_available_space_for_taffy(available_space, scale_factor);

        if retained_layout_detail_trace_enabled() && trace_layout_id_is_targeted(Some(id.0)) {
            eprintln!(
                "gpui retained_layout compute_start layout_id={} node_id={:?} repeated_root={} available_space={:?}",
                id.0, node_id, repeated_root, available_space
            );
        }

        let compute_start = std::time::Instant::now();
        let mut subtree_compute_recorder = self.subtree_probe.compute_recorder();
        let (measured_layout_calls, measured_layout_duration) = self.compute_layout_with_measure(
            node_id,
            taffy_available_space,
            scale_factor,
            window,
            cx,
            &mut subtree_compute_recorder,
        );
        let compute_layout_duration = compute_start.elapsed();
        self.subtree_probe.record_compute(subtree_compute_recorder);

        {
            let Self {
                taffy, geometry, ..
            } = self;
            geometry.capture_from_solver(
                root_id,
                node_id,
                available_space,
                scale_factor,
                |node_id| taffy.layout(node_id).expect(EXPECT_MESSAGE).clone(),
                |node_id| taffy.children(node_id).expect(EXPECT_MESSAGE),
            );
        }

        if retained_layout_detail_trace_enabled() && trace_layout_id_is_targeted(Some(id.0)) {
            let layout = self.geometry_layout(node_id);
            let solver_layout = self.try_solver_layout(node_id);
            eprintln!(
                "gpui retained_layout compute_finish layout_id={} node_id={:?} repeated_root={} root_layout={:?} solver_layout={:?}",
                id.0, node_id, repeated_root, layout, solver_layout
            );
        }

        let work = ComputeLayoutWork {
            solver_compute_layout_calls: 1,
            measured_layout_calls,
            compute_layout_duration,
            measured_layout_duration,
            fresh_layout_comparison: if retained_layout_fresh_compare_enabled() {
                let target_layout_ids = retained_layout_trace_layout_ids().map(Vec::as_slice);
                Some(self.trace_retained_fresh_layout_comparison(
                    id,
                    node_id,
                    taffy_available_space,
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
            retained_layout_detail_trace_enabled() && trace_layout_id_is_targeted(Some(id.0));
        if has_zero_size && (trace_targeted_zero_bounds || should_trace_retained_zero_bounds()) {
            let layout = self.try_geometry_layout(node_id);
            let solver_layout = self.try_solver_layout(node_id);
            let parent = self.parent(node_id);
            let parent_layout = parent.and_then(|parent| self.try_geometry_layout(parent));
            let children = self.children(node_id);
            let style = self.try_style(node_id);
            eprintln!(
                "gpui retained_layout zero_bounds layout_id={} node_id={:?} bounds={:?} geometry_layout={:?} solver_layout={:?} parent={:?} parent_geometry_layout={:?} children={:?} style={:?}",
                id.0,
                node_id,
                bounds,
                layout,
                solver_layout,
                parent,
                parent_layout,
                children,
                style
            );
            let mut depth = 0;
            let mut ancestor = Some(node_id);
            while let Some(ancestor_node_id) = ancestor {
                let ancestor_layout_id = self
                    .committed_layout_id_for_node(ancestor_node_id)
                    .map(|layout_id| layout_id.0);
                let ancestor_layout = self.try_geometry_layout(ancestor_node_id);
                let ancestor_solver_layout = self.try_solver_layout(ancestor_node_id);
                let ancestor_parent = self.parent(ancestor_node_id);
                let ancestor_children = self.children(ancestor_node_id);
                let ancestor_style = self.try_style(ancestor_node_id);
                eprintln!(
                    "gpui retained_layout zero_bounds_ancestor requested_layout_id={} depth={} layout_id={:?} node_id={:?} parent={:?} geometry_layout={:?} solver_layout={:?} children={:?} style={:?}",
                    id.0,
                    depth,
                    ancestor_layout_id,
                    ancestor_node_id,
                    ancestor_parent,
                    ancestor_layout,
                    ancestor_solver_layout,
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

    fn layout_bounds_for_node(&mut self, node_id: NodeId, scale_factor: f32) -> Bounds<Pixels> {
        let Self {
            taffy,
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
            |node_id| taffy.parent(node_id),
        )
    }

    fn compute_layout_with_measure(
        &mut self,
        node_id: NodeId,
        available_space: TaffySize<TaffyAvailableSpace>,
        scale_factor: f32,
        window: &mut Window,
        cx: &mut App,
        subtree_compute_recorder: &mut SubtreeProbeComputeRecorder,
    ) -> (u64, std::time::Duration) {
        let mut measured_layout_calls = 0;
        let mut measured_layout_duration = std::time::Duration::default();

        let Self {
            taffy,
            measurements,
            ..
        } = self;

        let compute_measurements = std::cell::RefCell::new(measurements.compute_state());
        let subtree_compute_recorder = std::cell::RefCell::new(subtree_compute_recorder);

        taffy
            .compute_layout_with_measure(
                node_id,
                available_space,
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
                    let callback_kind = compute_measurements
                        .borrow()
                        .callback_kind(node_id)
                        .expect("measured Taffy node should have a current measurement");
                    subtree_compute_recorder
                        .borrow_mut()
                        .record_measured_callback(node_id, callback_kind);
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
            )
            .expect(EXPECT_MESSAGE);

        (measured_layout_calls, measured_layout_duration)
    }

    fn trace_retained_layout_miss(
        &mut self,
        reason: &'static str,
        id: LayoutId,
        previous: Option<&RetainedLayoutOccurrence>,
        detail: impl FnOnce() -> String,
    ) {
        let Some(limit) = retained_layout_miss_trace_limit() else {
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
            LayoutIntentKind::Measured {
                measure,
                measured_kind,
            } => {
                format!(
                    "measured measure_slot={} measured_kind={}",
                    measure
                        .map(|measure| measure.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                    Self::debug_fingerprint(measured_kind)
                )
            }
        }
    }

    fn retained_node_summary(node: &RetainedLayoutOccurrence) -> String {
        format!(
            "{{node_id={:?}, kind={:?}, measured_kind={}, children={}, style={}}}",
            node.node_id,
            node.facts.kind,
            node.facts
                .measured_kind
                .as_ref()
                .map(Self::debug_fingerprint)
                .unwrap_or_else(|| "none".to_string()),
            node.children.len(),
            Self::debug_fingerprint(&node.facts.style)
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
    fn retained_layout_shape_for_node(&self, node_id: NodeId) -> RetainedLayoutShapeForTests {
        RetainedLayoutShapeForTests {
            style: RetainedStyleForTests(self.taffy.style(node_id).expect(EXPECT_MESSAGE).clone()),
            has_measure_context: self.taffy.get_node_context(node_id).is_some(),
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
        node_id: NodeId,
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
        self.taffy.get_node_context(token.0).is_some()
    }

    #[cfg(test)]
    pub(super) fn compute_unmeasured_layout_for_tests(
        &mut self,
        root_id: RetainedLayoutRootId,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
    ) -> RetainedNodeToken {
        let root_layout_context_changed = self
            .root_slots
            .retained_root_node_id(root_id)
            .is_some_and(|root_node| {
                self.geometry.retained_root_solve_context_changed(
                    root_id,
                    root_node,
                    available_space,
                    scale_factor,
                )
            });
        let root_node = self.commit_root_layout(root_id, id, root_layout_context_changed);
        assert!(
            !self.geometry.has_solved_root(root_id),
            "retained layout root should be solved at most once per frame"
        );
        assert!(
            self.bounds.mark_computed(root_node),
            "retained layout node should not be computed through multiple roots in one frame"
        );
        self.taffy
            .compute_layout_with_measure(
                root_node,
                scale_available_space_for_taffy(available_space, scale_factor),
                |_known_dimensions, _available_space, _id, _node_context, _style| TaffySize::ZERO,
            )
            .expect(EXPECT_MESSAGE);
        {
            let Self {
                taffy, geometry, ..
            } = self;
            geometry.capture_from_solver(
                root_id,
                root_node,
                available_space,
                scale_factor,
                |node_id| taffy.layout(node_id).expect(EXPECT_MESSAGE).clone(),
                |node_id| taffy.children(node_id).expect(EXPECT_MESSAGE),
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
            scale_available_space_for_taffy(available_space, scale_factor),
            scale_factor,
            window,
            cx,
            None,
        )
    }

    pub(super) fn trace_retained_fresh_layout_comparison(
        &self,
        root_layout_id: LayoutId,
        retained_root_node_id: NodeId,
        available_space: TaffySize<TaffyAvailableSpace>,
        scale_factor: f32,
        window: &mut Window,
        cx: &mut App,
        target_layout_ids: Option<&[usize]>,
    ) -> FreshLayoutComparisonSummary {
        if self.intent_subtree_contains_opaque_measurement(root_layout_id) {
            eprintln!(
                "gpui retained_layout fresh_compare_skipped root_layout_id={} retained_root_node_id={:?} available_space={:?} reason=opaque_measured_node",
                root_layout_id.0, retained_root_node_id, available_space
            );
            return FreshLayoutComparisonSummary::default();
        }

        let mut fresh_taffy = TaffyTree::<FreshLayoutCompareNodeContext>::new();
        fresh_taffy.disable_rounding();
        let fresh_root_node_id =
            self.build_fresh_layout_compare_tree(&mut fresh_taffy, root_layout_id);

        fresh_taffy
            .compute_layout_with_measure(
                fresh_root_node_id,
                available_space,
                |known_dimensions, available_space, _fresh_node_id, node_context, _style| {
                    let Some(node_context) = node_context else {
                        return TaffySize::ZERO;
                    };
                    let LayoutIntentKind::Measured { measured_kind, .. } =
                        &self.intent(node_context.layout_id).kind
                    else {
                        return TaffySize::ZERO;
                    };

                    let known_dimensions = Size {
                        width: known_dimensions
                            .width
                            .map(|value| Pixels(value / scale_factor)),
                        height: known_dimensions
                            .height
                            .map(|value| Pixels(value / scale_factor)),
                    };
                    let available_space: Size<AvailableSpace> = available_space.into();
                    let untransform = |space: AvailableSpace| match space {
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

                    let measured_size = match measured_kind {
                        MeasuredLayoutKind::Opaque => {
                            unreachable!("opaque measured nodes skip fresh comparison")
                        }
                        MeasuredLayoutKind::PureSize(measure) => {
                            measure.measure(known_dimensions, available_space)
                        }
                        MeasuredLayoutKind::Text(key) => key
                            .measure(known_dimensions, available_space, window, cx)
                            .size(),
                    };
                    snap_measured_size_to_device_pixels(measured_size, scale_factor).into()
                },
            )
            .expect(EXPECT_MESSAGE);

        let mut comparison = FreshLayoutComparison::default();
        self.observe_retained_fresh_layout_comparison(
            &fresh_taffy,
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
        fresh_taffy: &mut TaffyTree<FreshLayoutCompareNodeContext>,
        id: LayoutId,
    ) -> NodeId {
        let intent = self.intent(id);
        match &intent.kind {
            LayoutIntentKind::Unmeasured { children } => {
                let child_node_ids = children
                    .iter()
                    .map(|child| self.build_fresh_layout_compare_tree(fresh_taffy, *child))
                    .collect::<Vec<_>>();
                if child_node_ids.is_empty() {
                    fresh_taffy
                        .new_leaf(intent.style.clone())
                        .expect(EXPECT_MESSAGE)
                } else {
                    fresh_taffy
                        .new_with_children(intent.style.clone(), &child_node_ids)
                        .expect(EXPECT_MESSAGE)
                }
            }
            LayoutIntentKind::Measured { .. } => fresh_taffy
                .new_leaf_with_context(
                    intent.style.clone(),
                    FreshLayoutCompareNodeContext { layout_id: id },
                )
                .expect(EXPECT_MESSAGE),
        }
    }

    fn intent_subtree_contains_opaque_measurement(&self, id: LayoutId) -> bool {
        match &self.intent(id).kind {
            LayoutIntentKind::Unmeasured { children } => children
                .iter()
                .any(|child| self.intent_subtree_contains_opaque_measurement(*child)),
            LayoutIntentKind::Measured { measured_kind, .. } => {
                matches!(measured_kind, MeasuredLayoutKind::Opaque)
            }
        }
    }

    fn observe_retained_fresh_layout_comparison(
        &self,
        fresh_taffy: &TaffyTree<FreshLayoutCompareNodeContext>,
        id: LayoutId,
        fresh_node_id: NodeId,
        path: &mut Vec<LayoutId>,
        comparison: &mut FreshLayoutComparison,
        target_layout_ids: Option<&[usize]>,
    ) {
        let retained_node_id = self.committed_node(id);
        let retained_layout = self.geometry_layout(retained_node_id);
        let fresh_layout = fresh_taffy.layout(fresh_node_id).expect(EXPECT_MESSAGE);
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
                    fresh_layout,
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
                fresh_layout,
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
                    fresh_layout,
                    path,
                ));
            }
        }

        if let LayoutIntentKind::Unmeasured { children } = &self.intent(id).kind {
            let fresh_child_node_ids = fresh_taffy.children(fresh_node_id).expect(EXPECT_MESSAGE);
            assert_eq!(
                fresh_child_node_ids.len(),
                children.len(),
                "fresh compare tree should mirror intent child count"
            );
            for (child, fresh_child_node_id) in children.iter().zip(fresh_child_node_ids) {
                path.push(*child);
                self.observe_retained_fresh_layout_comparison(
                    fresh_taffy,
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
        retained_node_id: NodeId,
        fresh_node_id: NodeId,
        retained_layout: &Layout,
        fresh_layout: &Layout,
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

    fn layout_has_zero_size(layout: &Layout) -> bool {
        layout.size.width <= 0.0 || layout.size.height <= 0.0
    }
}

impl RetainedLayoutForest {
    /// Commit a current-frame intent into a retained root slot.
    ///
    /// This method is the root of the retained occurrence update. It may reuse a
    /// previous occurrence, build fresh mirror nodes, or detach obsolete
    /// subtrees, but all resulting Taffy mutations stay inside the forest.
    fn commit_layout(
        &mut self,
        root_id: RetainedLayoutRootId,
        id: LayoutId,
        root_layout_context_changed: bool,
    ) -> NodeId {
        if let Some(node_id) = self.committed.try_node(id) {
            return node_id;
        }

        assert!(
            !self.root_slots.has_current_root(root_id),
            "retained layout root should be committed at most once per frame"
        );
        let retained_root = self.root_slots.take_retained_root(root_id);
        if retained_layout_detail_trace_enabled() && trace_layout_id_is_targeted(Some(id.0)) {
            eprintln!(
                "gpui retained_layout commit_root_candidate root_id={:?} layout_id={} retained_root={}",
                root_id,
                id.0,
                retained_root.is_some()
            );
        }

        let retained_node = self
            .commit_intent(id, retained_root, root_layout_context_changed)
            .occurrence;
        let node_id = retained_node.node_id;
        self.flush_detached_subtree_removals();
        self.root_slots.insert_current_root(root_id, retained_node);
        if retained_layout_detail_trace_enabled() && trace_layout_id_is_targeted(Some(id.0)) {
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
    fn commit_root_layout(
        &mut self,
        root_id: RetainedLayoutRootId,
        id: LayoutId,
        root_layout_context_changed: bool,
    ) -> NodeId {
        let node_id = self.commit_layout(root_id, id, root_layout_context_changed);
        assert!(
            self.taffy.parent(node_id).is_none(),
            "layout root must not already be committed under a parent"
        );
        node_id
    }

    /// Test-only retained commit probe that preserves Taffy privacy.
    #[cfg(test)]
    pub(super) fn commit_layout_for_tests(
        &mut self,
        root_id: RetainedLayoutRootId,
        id: LayoutId,
    ) -> RetainedNodeToken {
        RetainedNodeToken(self.commit_layout(root_id, id, false))
    }

    /// Test-only root commit probe that preserves Taffy privacy.
    #[cfg(test)]
    pub(super) fn commit_root_layout_for_tests(
        &mut self,
        root_id: RetainedLayoutRootId,
        id: LayoutId,
    ) -> RetainedNodeToken {
        RetainedNodeToken(self.commit_root_layout(root_id, id, false))
    }

    /// Commit one intent against an optional previous retained occurrence.
    fn commit_intent(
        &mut self,
        id: LayoutId,
        previous: Option<RetainedLayoutOccurrence>,
        parent_layout_context_changed: bool,
    ) -> RetainedLayoutCommit {
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
            LayoutIntentKind::Unmeasured { children } => self.commit_unmeasured_intent(
                id,
                intent.style.clone(),
                children,
                previous,
                parent_layout_context_changed,
            ),
            LayoutIntentKind::Measured {
                measure,
                measured_kind,
            } => self.commit_measured_intent(
                id,
                intent.style.clone(),
                measure,
                measured_kind,
                previous,
                parent_layout_context_changed,
            ),
        };

        if let (Some(global_id), Some(work_snapshot)) = (probe_global_id, work_snapshot) {
            let work_delta = self.work.delta_since(work_snapshot);
            let node_ids = Self::retained_occurrence_node_ids(&retained_node.occurrence);
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
            LayoutIntentKind::Measured {
                measure,
                measured_kind,
            } => self.build_fresh_measured_occurrence(
                id,
                self.intent(id).style.clone(),
                measure,
                measured_kind,
            ),
        }
    }

    /// Build an unmeasured retained occurrence and mirror subtree from scratch.
    fn build_fresh_unmeasured_occurrence(
        &mut self,
        id: LayoutId,
        style: taffy::style::Style,
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
            self.taffy.new_leaf(style.clone()).expect(EXPECT_MESSAGE)
        } else {
            self.taffy
                .new_with_children(style.clone(), &child_node_ids)
                .expect(EXPECT_MESSAGE)
        };
        self.work.record_create();
        self.mark_taffy_node_committed(node_id);
        self.committed.insert(id, node_id);
        RetainedLayoutOccurrence {
            node_id,
            facts: RetainedLayoutFacts {
                style,
                kind: RetainedLayoutKind::Unmeasured,
                measured_kind: None,
            },
            children: retained_children,
        }
    }

    /// Build a measured retained occurrence and mirror node from scratch.
    fn build_fresh_measured_occurrence(
        &mut self,
        id: LayoutId,
        style: taffy::style::Style,
        measure: Option<usize>,
        measured_kind: MeasuredLayoutKind,
    ) -> RetainedLayoutOccurrence {
        assert!(
            !self.committed.contains_layout(id),
            "layout intent should appear only once in a committed layout tree"
        );

        let node_id = self
            .taffy
            .new_leaf_with_context(style.clone(), NodeContext)
            .expect(EXPECT_MESSAGE);
        self.work.record_create();
        self.mark_taffy_node_committed(node_id);
        self.measurements.insert_current_measurement(
            node_id,
            Self::current_measurement(measured_kind.clone(), measure),
        );
        self.committed.insert(id, node_id);
        RetainedLayoutOccurrence {
            node_id,
            facts: RetainedLayoutFacts {
                style,
                kind: RetainedLayoutKind::Measured,
                measured_kind: Some(measured_kind),
            },
            children: Vec::new(),
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
        style: taffy::style::Style,
        children: Vec<LayoutId>,
        previous: Option<RetainedLayoutOccurrence>,
        parent_layout_context_changed: bool,
    ) -> RetainedLayoutCommit {
        assert!(
            !self.committed.contains_layout(id),
            "layout intent should appear only once in a committed layout tree"
        );

        let Some(previous) = previous else {
            self.work.record_no_previous_miss();
            self.trace_retained_layout_miss("no_previous", id, None, || String::new());
            return RetainedLayoutCommit {
                occurrence: self.build_fresh_unmeasured_occurrence(id, style, children),
                layout_context_changed: true,
            };
        };

        self.update_unmeasured_retained_occurrence(
            id,
            style,
            children,
            previous,
            parent_layout_context_changed,
        )
    }

    /// Update an unmeasured occurrence and mirror node to match the current intent.
    fn update_unmeasured_retained_occurrence(
        &mut self,
        id: LayoutId,
        style: taffy::style::Style,
        children: Vec<LayoutId>,
        previous: RetainedLayoutOccurrence,
        parent_layout_context_changed: bool,
    ) -> RetainedLayoutCommit {
        assert!(
            !self.committed.contains_layout(id),
            "layout intent should appear only once in a committed layout tree"
        );

        if previous.facts.kind != RetainedLayoutKind::Unmeasured {
            let fresh = self.build_fresh_unmeasured_occurrence(id, style, children);
            self.root_slots.detach_subtree(previous);
            return RetainedLayoutCommit {
                occurrence: fresh,
                layout_context_changed: true,
            };
        }

        let previous_facts = previous.facts;
        let previous_child_node_ids = previous
            .children
            .iter()
            .map(|child| child.node_id)
            .collect::<Vec<_>>();
        let mut previous_children = previous.children.into_iter().map(Some).collect::<Vec<_>>();
        let node_id = previous.node_id;
        let style_changed = previous_facts.style != style;
        self.work.record_reuse();
        self.mark_taffy_node_committed(node_id);
        self.committed.insert(id, node_id);

        let mut exact_previous_children =
            self.assign_matching_previous_children(&children, previous_children.as_mut_slice());
        let assigned_previous_child_node_ids = exact_previous_children
            .iter()
            .map(|child| child.as_ref().map(|child| child.node_id))
            .collect::<Vec<_>>();
        let child_list_will_change = previous_child_node_ids.len() != children.len()
            || assigned_previous_child_node_ids
                .iter()
                .enumerate()
                .any(|(index, child_node_id)| {
                    *child_node_id != previous_child_node_ids.get(index).copied()
                });
        let child_layout_context_changed =
            parent_layout_context_changed || style_changed || child_list_will_change;

        let mut retained_children = Vec::with_capacity(children.len());
        let mut child_node_ids = Vec::with_capacity(children.len());
        let mut any_child_layout_context_changed = false;
        for (index, child) in children.into_iter().enumerate() {
            let previous_child = exact_previous_children[index].take();
            let retained_child =
                self.commit_intent(child, previous_child, child_layout_context_changed);
            any_child_layout_context_changed |= retained_child.layout_context_changed;
            let retained_child = retained_child.occurrence;
            child_node_ids.push(retained_child.node_id);
            retained_children.push(retained_child);
        }

        if style_changed {
            self.taffy
                .set_style(node_id, style.clone())
                .expect(EXPECT_MESSAGE);
            self.work.record_style_update();
        }

        let children_changed = previous_child_node_ids != child_node_ids;
        if children_changed {
            self.taffy
                .set_children(node_id, &child_node_ids)
                .expect(EXPECT_MESSAGE);
            self.work.record_child_list_update();
        }

        for previous_child in previous_children.into_iter().flatten() {
            self.root_slots.detach_subtree(previous_child);
        }

        let retained_node = RetainedLayoutOccurrence {
            node_id,
            facts: RetainedLayoutFacts {
                style,
                kind: RetainedLayoutKind::Unmeasured,
                measured_kind: None,
            },
            children: retained_children,
        };
        let layout_context_changed = parent_layout_context_changed
            || style_changed
            || children_changed
            || any_child_layout_context_changed;
        if parent_layout_context_changed
            || (any_child_layout_context_changed && !style_changed && !children_changed)
        {
            self.mark_taffy_node_dirty(node_id);
        }

        RetainedLayoutCommit {
            occurrence: retained_node,
            layout_context_changed,
        }
    }

    /// Find previous child occurrences that exactly match current child intents.
    ///
    /// This preserves retention across insert/delete/reorder only when a
    /// previous child subtree is already compatible with a current child. If no
    /// exact match exists, the child at the same position may still be updated
    /// in place when the retained node kind can host the current intent. This
    /// keeps conservative measured leaves from forcing every ancestor to churn.
    fn assign_matching_previous_children(
        &self,
        children: &[LayoutId],
        previous_children: &mut [Option<RetainedLayoutOccurrence>],
    ) -> Vec<Option<RetainedLayoutOccurrence>> {
        let mut assigned = children
            .iter()
            .map(|child| self.take_matching_previous_child(*child, previous_children))
            .collect::<Vec<_>>();

        for (index, child) in children.iter().enumerate() {
            if assigned[index].is_some() {
                continue;
            }

            let Some(previous_child) = previous_children.get_mut(index) else {
                continue;
            };
            let Some(candidate) = previous_child.as_ref() else {
                continue;
            };
            if self.retained_occurrence_can_update_intent(*child, candidate) {
                assigned[index] = previous_child.take();
            }
        }

        assigned
    }

    /// Remove one exact previous child match from the available sibling set.
    fn take_matching_previous_child(
        &self,
        child: LayoutId,
        previous_children: &mut [Option<RetainedLayoutOccurrence>],
    ) -> Option<RetainedLayoutOccurrence> {
        for previous_child in previous_children {
            let Some(candidate) = previous_child.as_ref() else {
                continue;
            };
            if self.retained_occurrence_matches_intent(child, candidate) {
                return previous_child.take();
            }
        }
        None
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
        style: taffy::style::Style,
        measure: Option<usize>,
        measured_kind: MeasuredLayoutKind,
        previous: Option<RetainedLayoutOccurrence>,
        parent_layout_context_changed: bool,
    ) -> RetainedLayoutCommit {
        assert!(
            !self.committed.contains_layout(id),
            "layout intent should appear only once in a committed layout tree"
        );

        let Some(previous) = previous else {
            self.work.record_no_previous_miss();
            self.trace_retained_layout_miss("no_previous", id, None, || String::new());
            return RetainedLayoutCommit {
                occurrence: self.build_fresh_measured_occurrence(id, style, measure, measured_kind),
                layout_context_changed: true,
            };
        };

        let previous_measured_kind = previous.facts.measured_kind.clone();
        let compatible = previous.facts.kind == RetainedLayoutKind::Measured
            && previous.children.is_empty()
            && Self::measured_kinds_compatible(previous_measured_kind.as_ref(), &measured_kind);
        if !compatible {
            self.work.record_measured_kind_miss();
            self.trace_retained_layout_miss("measured_kind", id, Some(&previous), || {
                format!(
                    "previous_measured_kind={} current_measured_kind={}",
                    previous
                        .facts
                        .measured_kind
                        .as_ref()
                        .map(Self::debug_fingerprint)
                        .unwrap_or_else(|| "none".to_string()),
                    Self::debug_fingerprint(&measured_kind)
                )
            });
            let layout_context_changed = parent_layout_context_changed
                || !self.retained_occurrence_has_same_layout_context(id, &previous);
            let fresh = self.build_fresh_measured_occurrence(id, style, measure, measured_kind);
            self.root_slots.detach_subtree(previous);
            return RetainedLayoutCommit {
                occurrence: fresh,
                layout_context_changed,
            };
        }

        let node_id = previous.node_id;
        let previous_style = previous.facts.style;
        self.work.record_reuse();
        self.mark_taffy_node_committed(node_id);
        self.committed.insert(id, node_id);

        let style_changed = previous_style != style;
        let measured_kind_changed = previous_measured_kind.as_ref() != Some(&measured_kind);
        let text_measurement_needs_hydration =
            matches!(&measured_kind, MeasuredLayoutKind::Text(_));
        if style_changed {
            self.taffy
                .set_style(node_id, style.clone())
                .expect(EXPECT_MESSAGE);
            self.work.record_style_update();
        }
        if measured_kind_changed
            || text_measurement_needs_hydration
            || parent_layout_context_changed
        {
            self.mark_taffy_node_dirty(node_id);
        }

        self.measurements.insert_current_measurement(
            node_id,
            Self::current_measurement(measured_kind.clone(), measure),
        );

        let retained_node = RetainedLayoutOccurrence {
            node_id,
            facts: RetainedLayoutFacts {
                style,
                kind: RetainedLayoutKind::Measured,
                measured_kind: Some(measured_kind),
            },
            children: Vec::new(),
        };
        let layout_context_changed =
            parent_layout_context_changed || style_changed || measured_kind_changed;

        RetainedLayoutCommit {
            occurrence: retained_node,
            layout_context_changed,
        }
    }

    /// Return whether a retained measured node can preserve mirror identity.
    fn measured_kinds_compatible(
        previous: Option<&MeasuredLayoutKind>,
        current: &MeasuredLayoutKind,
    ) -> bool {
        match (previous, current) {
            (Some(MeasuredLayoutKind::PureSize(_)), MeasuredLayoutKind::PureSize(_)) => true,
            (Some(MeasuredLayoutKind::Text(_)), MeasuredLayoutKind::Text(_)) => true,
            _ => false,
        }
    }

    /// Check whether an existing occurrence subtree is an exact semantic match.
    ///
    /// This is a proof rule, not a heuristic. A `true` result means the
    /// occurrence's retained facts and child shape already match the current
    /// intent tree, so its Taffy cache can remain meaningful.
    fn retained_occurrence_matches_intent(
        &self,
        id: LayoutId,
        previous: &RetainedLayoutOccurrence,
    ) -> bool {
        let intent = self.intent(id);
        if previous.facts.style != intent.style {
            return false;
        }

        match &intent.kind {
            LayoutIntentKind::Unmeasured { children } => {
                previous.facts.kind == RetainedLayoutKind::Unmeasured
                    && previous.facts.measured_kind.is_none()
                    && previous.children.len() == children.len()
                    && children
                        .iter()
                        .zip(&previous.children)
                        .all(|(child, previous_child)| {
                            self.retained_occurrence_matches_intent(*child, previous_child)
                        })
            }
            LayoutIntentKind::Measured { measured_kind, .. } => {
                previous.facts.kind == RetainedLayoutKind::Measured
                    && previous.children.is_empty()
                    && previous.facts.measured_kind.as_ref() == Some(measured_kind)
            }
        }
    }

    /// Return whether a retained occurrence may be updated to host an intent.
    ///
    /// This is a shallow compatibility check. It does not claim the retained
    /// subtree is still valid; it only says that committing the current intent
    /// through this occurrence can update all retained facts and mirror edges
    /// without changing the node category.
    fn retained_occurrence_can_update_intent(
        &self,
        id: LayoutId,
        previous: &RetainedLayoutOccurrence,
    ) -> bool {
        match &self.intent(id).kind {
            LayoutIntentKind::Unmeasured { .. } => {
                previous.facts.kind == RetainedLayoutKind::Unmeasured
                    && previous.facts.measured_kind.is_none()
            }
            LayoutIntentKind::Measured { measured_kind, .. } => {
                previous.facts.kind == RetainedLayoutKind::Measured
                    && previous.children.is_empty()
                    && Self::measured_kinds_compatible(
                        previous.facts.measured_kind.as_ref(),
                        measured_kind,
                    )
            }
        }
    }

    /// Return whether a previous occurrence has the same shallow layout meaning.
    ///
    /// This is weaker than retained-node compatibility. Text nodes with the same
    /// explicit measure key are layout-equivalent, but still not node-reusable
    /// under stock Taffy because their paint artifact must be hydrated from a
    /// current callback.
    fn retained_occurrence_has_same_layout_context(
        &self,
        id: LayoutId,
        previous: &RetainedLayoutOccurrence,
    ) -> bool {
        let intent = self.intent(id);
        if previous.facts.style != intent.style {
            return false;
        }

        match &intent.kind {
            LayoutIntentKind::Unmeasured { .. } => {
                previous.facts.kind == RetainedLayoutKind::Unmeasured
                    && previous.facts.measured_kind.is_none()
            }
            LayoutIntentKind::Measured { measured_kind, .. } => {
                previous.facts.kind == RetainedLayoutKind::Measured
                    && previous.children.is_empty()
                    && match (previous.facts.measured_kind.as_ref(), measured_kind) {
                        (
                            Some(MeasuredLayoutKind::PureSize(previous)),
                            MeasuredLayoutKind::PureSize(current),
                        ) => previous == current,
                        (
                            Some(MeasuredLayoutKind::Text(previous)),
                            MeasuredLayoutKind::Text(current),
                        ) => previous == current,
                        _ => false,
                    }
            }
        }
    }

    /// Mark a mirror node as used at one current-frame position.
    fn mark_taffy_node_committed(&mut self, node_id: NodeId) {
        self.committed.mark_taffy_node_committed(node_id);
    }

    /// Mark a private mirror node dirty at most once in the current frame.
    fn mark_taffy_node_dirty(&mut self, node_id: NodeId) {
        if self.committed.mark_taffy_node_dirty(node_id) {
            self.taffy.mark_dirty(node_id).expect(EXPECT_MESSAGE);
            self.work.record_dirty_mark();
        }
    }

    fn retained_occurrence_node_ids(retained_node: &RetainedLayoutOccurrence) -> Vec<NodeId> {
        let mut node_ids = vec![retained_node.node_id];
        for child in &retained_node.children {
            node_ids.extend(Self::retained_occurrence_node_ids(child));
        }
        node_ids
    }

    #[cfg(any(test, debug_assertions))]
    /// Assert that the private mirror is equivalent to the current intent tree.
    fn debug_assert_committed_intent_matches(&mut self, id: LayoutId, node_id: NodeId) {
        let mut seen = FxHashSet::default();
        self.debug_assert_intent_node_matches(id, node_id, None, &mut seen);
    }

    #[cfg(any(test, debug_assertions))]
    fn debug_assert_intent_node_matches(
        &self,
        id: LayoutId,
        node_id: NodeId,
        expected_parent: Option<NodeId>,
        seen: &mut FxHashSet<NodeId>,
    ) {
        assert!(
            seen.insert(node_id),
            "committed Taffy node should appear at only one current intent position"
        );
        assert_eq!(self.taffy.parent(node_id), expected_parent);

        let intent = self.intent(id);
        assert_eq!(
            self.taffy.style(node_id).expect(EXPECT_MESSAGE),
            &intent.style
        );

        match &intent.kind {
            LayoutIntentKind::Unmeasured { children } => {
                assert!(
                    !self.measurements.has_current_measurement(node_id),
                    "unmeasured intent should not have current measurement state"
                );
                assert!(self.taffy.get_node_context(node_id).is_none());
                let child_node_ids = children
                    .iter()
                    .map(|child| self.committed.node(*child))
                    .collect::<Vec<_>>();
                assert_eq!(
                    self.taffy.children(node_id).expect(EXPECT_MESSAGE),
                    child_node_ids
                );
                for (child, child_node_id) in children.iter().zip(child_node_ids) {
                    self.debug_assert_intent_node_matches(
                        *child,
                        child_node_id,
                        Some(node_id),
                        seen,
                    );
                }
            }
            LayoutIntentKind::Measured { measured_kind, .. } => {
                assert!(self.taffy.get_node_context(node_id).is_some());
                assert_eq!(
                    self.taffy.children(node_id).expect(EXPECT_MESSAGE),
                    Vec::<NodeId>::new()
                );
                match (
                    measured_kind,
                    self.measurements.current_measurement(node_id),
                ) {
                    (MeasuredLayoutKind::Opaque, Some(CurrentMeasurement::Opaque(_))) => {}
                    (
                        MeasuredLayoutKind::PureSize(expected),
                        Some(CurrentMeasurement::PureSize(actual)),
                    ) => assert_eq!(actual, expected),
                    (
                        MeasuredLayoutKind::Text(expected),
                        Some(CurrentMeasurement::Text { key: actual, .. }),
                    ) => assert_eq!(actual, expected),
                    _ => {
                        panic!("measured intent should have matching current measurement state")
                    }
                }
            }
        }
    }

    fn current_measurement(
        measured_kind: MeasuredLayoutKind,
        measure: Option<usize>,
    ) -> CurrentMeasurement {
        match measured_kind {
            MeasuredLayoutKind::Opaque => CurrentMeasurement::Opaque(
                measure.expect("opaque measured layout should have a current producer"),
            ),
            MeasuredLayoutKind::PureSize(measure) => CurrentMeasurement::PureSize(measure),
            MeasuredLayoutKind::Text(key) => CurrentMeasurement::Text {
                key,
                measure: measure
                    .expect("text measured layout should have a current producer and hydrator"),
            },
        }
    }

    fn flush_detached_subtree_removals(&mut self) {
        for retained_node in self.root_slots.take_detached_subtree_removals() {
            self.remove_retained_subtree(retained_node);
        }
    }

    fn remove_retained_subtree(&mut self, retained_node: RetainedLayoutOccurrence) {
        for child in retained_node.children {
            self.remove_retained_subtree(child);
        }
        if retained_node.facts.kind == RetainedLayoutKind::Measured {
            self.taffy
                .set_node_context(retained_node.node_id, None)
                .expect(EXPECT_MESSAGE);
            self.work.record_measured_context_clear();
        }
        self.taffy
            .remove(retained_node.node_id)
            .expect(EXPECT_MESSAGE);
        self.work.record_remove();
    }
}
