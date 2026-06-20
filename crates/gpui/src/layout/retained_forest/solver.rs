//! Private layout-solver facade for the retained layout forest.
//!
//! The retained tree owns GPUI layout facts. This module is the only surface the
//! retained tree can use to mutate or solve the downstream layout mirror. The
//! concrete implementation lives in `solver::backend`; callers see
//! only opaque solver handles, styles, cache observations, and layouts.

mod backend;

use super::super::AvailableSpace;
use crate::{Pixels, Point, Size, Style};
use backend::{FreshSolverBackendImpl, SolverBackendImpl};
use std::fmt::{self, Debug};

/// Solver-owned node handle.
///
/// The private field prevents retained-tree code from manufacturing backend
/// node ids or calling backend APIs directly. A `SolverNodeId` is meaningful
/// only to `LayoutSolver`.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(super) struct SolverNodeId(backend::BackendNodeId);

impl Debug for SolverNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SolverNodeId").field(&self.0).finish()
    }
}

/// Solver-owned style value.
///
/// GPUI may compare and clone solver styles as layout facts, but only the
/// backend can translate them into solver-native styles.
#[derive(Clone, PartialEq)]
pub(super) struct SolverStyle(backend::BackendStyle);

impl Debug for SolverStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl SolverStyle {
    pub(super) fn from_gpui_style(style: &Style, rem_size: Pixels, scale_factor: f32) -> Self {
        Self(backend::BackendStyle::from_gpui_style(
            style,
            rem_size,
            scale_factor,
        ))
    }

    #[cfg(test)]
    pub(super) fn test_with_size(width: f32, height: f32) -> Self {
        Self(backend::BackendStyle::test_with_size(width, height))
    }
}

impl Default for SolverStyle {
    fn default() -> Self {
        Self(backend::BackendStyle::default())
    }
}

/// Layout result copied out of the solver after the legal root solve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SolverLayout {
    pub(super) location: Point<f32>,
    pub(super) size: Size<f32>,
}

/// Opaque identity of one solver cache entry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct SolverCacheEntryId(backend::BackendCacheEntryId);

/// Passive observation of solver cache activity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SolverCacheEvent {
    Hit(SolverCacheEntry),
    Stored(SolverCacheEntry),
    Cleared(SolverCacheClear),
}

/// Passive observation that the solver cleared one node's cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SolverCacheClear(backend::BackendCacheClear);

impl SolverCacheClear {
    pub(super) fn node_id(&self) -> SolverNodeId {
        self.0.node_id()
    }
}

/// Passive observation of one cache entry selected or stored by the solver.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SolverCacheEntry(backend::BackendCacheEntry);

impl SolverCacheEntry {
    pub(super) fn node_id(&self) -> SolverNodeId {
        self.0.node_id()
    }

    pub(super) fn entry_id(&self) -> SolverCacheEntryId {
        self.0.entry_id()
    }

    pub(super) fn is_compute_size(&self) -> bool {
        self.0.is_compute_size()
    }

    pub(super) fn is_perform_layout(&self) -> bool {
        self.0.is_perform_layout()
    }

    pub(super) fn known_dimensions(&self, scale_factor: f32) -> Size<Option<Pixels>> {
        self.0.known_dimensions(scale_factor)
    }

    pub(super) fn available_space(&self, scale_factor: f32) -> Size<AvailableSpace> {
        self.0.available_space(scale_factor)
    }

    pub(super) fn trace_details(&self) -> SolverCacheEntryTraceDetails {
        self.0.trace_details()
    }
}

/// Debug-only cache entry facts used by retained-layout tracing.
pub(super) struct SolverCacheEntryTraceDetails {
    pub(super) entry_id: SolverCacheEntryId,
    pub(super) run_mode: String,
    pub(super) sizing_mode: String,
    pub(super) axis: String,
    pub(super) known_dimensions: String,
    pub(super) parent_size: String,
    pub(super) available_space: String,
    pub(super) output_size: String,
    pub(super) has_zero_output: bool,
    pub(super) has_zero_known_dimension: bool,
    pub(super) has_zero_parent_dimension: bool,
    pub(super) has_zero_available_space: bool,
}

/// Exact measurement query the solver asked GPUI to answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SolverMeasureQuery {
    pub(super) known_dimensions: Size<Option<Pixels>>,
    pub(super) available_space: Size<AvailableSpace>,
}

/// Private layout solver facade used by the retained forest.
///
/// The facade is intentionally concrete: callers do not choose a solver
/// implementation. The private backend owns the actual solver API and is the
/// only code allowed to name concrete solver internals.
#[derive(Clone)]
pub(super) struct LayoutSolver {
    backend: SolverBackendImpl,
}

trait SolverBackend: Clone {
    fn new() -> Self;
    fn new_leaf(&mut self, style: SolverStyle) -> SolverNodeId;
    fn new_with_children(&mut self, style: SolverStyle, children: &[SolverNodeId]) -> SolverNodeId;
    fn new_measured(&mut self, style: SolverStyle) -> SolverNodeId;
    fn set_style(&mut self, node_id: SolverNodeId, style: SolverStyle);
    fn set_children(&mut self, node_id: SolverNodeId, children: &[SolverNodeId]);
    fn clear_measure_context(&mut self, node_id: SolverNodeId);
    fn remove(&mut self, node_id: SolverNodeId);
    fn mark_dirty(&mut self, node_id: SolverNodeId);
    fn parent(&self, node_id: SolverNodeId) -> Option<SolverNodeId>;
    fn children(&self, node_id: SolverNodeId) -> Vec<SolverNodeId>;
    fn layout(&self, node_id: SolverNodeId) -> Option<SolverLayout>;
    fn style(&self, node_id: SolverNodeId) -> Option<SolverStyle>;
    fn has_measure_context(&self, node_id: SolverNodeId) -> bool;
    fn compute_layout_with_measure_and_cache_events(
        &mut self,
        root: SolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        measure: impl FnMut(SolverNodeId, bool, SolverMeasureQuery) -> Size<f32>,
        handle_cache_event: impl FnMut(SolverCacheEvent),
    );
}

impl LayoutSolver {
    pub(super) fn new() -> Self {
        Self {
            backend: SolverBackendImpl::new(),
        }
    }

    pub(super) fn new_leaf(&mut self, style: SolverStyle) -> SolverNodeId {
        self.backend.new_leaf(style)
    }

    pub(super) fn new_with_children(
        &mut self,
        style: SolverStyle,
        children: &[SolverNodeId],
    ) -> SolverNodeId {
        self.backend.new_with_children(style, children)
    }

    pub(super) fn new_measured(&mut self, style: SolverStyle) -> SolverNodeId {
        self.backend.new_measured(style)
    }

    pub(super) fn set_style(&mut self, node_id: SolverNodeId, style: SolverStyle) {
        self.backend.set_style(node_id, style);
    }

    pub(super) fn set_children(&mut self, node_id: SolverNodeId, children: &[SolverNodeId]) {
        self.backend.set_children(node_id, children);
    }

    pub(super) fn clear_measure_context(&mut self, node_id: SolverNodeId) {
        self.backend.clear_measure_context(node_id);
    }

    pub(super) fn remove(&mut self, node_id: SolverNodeId) {
        self.backend.remove(node_id);
    }

    pub(super) fn mark_dirty(&mut self, node_id: SolverNodeId) {
        self.backend.mark_dirty(node_id);
    }

    pub(super) fn parent(&self, node_id: SolverNodeId) -> Option<SolverNodeId> {
        self.backend.parent(node_id)
    }

    pub(super) fn children(&self, node_id: SolverNodeId) -> Vec<SolverNodeId> {
        self.backend.children(node_id)
    }

    pub(super) fn layout(&self, node_id: SolverNodeId) -> Option<SolverLayout> {
        self.backend.layout(node_id)
    }

    pub(super) fn style(&self, node_id: SolverNodeId) -> Option<SolverStyle> {
        self.backend.style(node_id)
    }

    pub(super) fn has_measure_context(&self, node_id: SolverNodeId) -> bool {
        self.backend.has_measure_context(node_id)
    }

    pub(super) fn compute_layout_with_measure_and_cache_events(
        &mut self,
        root: SolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        measure: impl FnMut(SolverNodeId, bool, SolverMeasureQuery) -> Size<f32>,
        handle_cache_event: impl FnMut(SolverCacheEvent),
    ) {
        self.backend.compute_layout_with_measure_and_cache_events(
            root,
            available_space,
            scale_factor,
            measure,
            handle_cache_event,
        );
    }
}

/// Fresh one-shot solver used only as a correctness oracle.
pub(super) struct FreshLayoutSolver<C> {
    backend: FreshSolverBackendImpl<C>,
}

/// Solver node id for fresh oracle trees.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct FreshSolverNodeId(backend::FreshBackendNodeId);

impl<C> FreshLayoutSolver<C> {
    pub(super) fn new() -> Self {
        Self {
            backend: FreshSolverBackendImpl::new(),
        }
    }

    pub(super) fn new_leaf(&mut self, style: SolverStyle) -> FreshSolverNodeId {
        self.backend.new_leaf(style)
    }

    pub(super) fn new_with_children(
        &mut self,
        style: SolverStyle,
        children: &[FreshSolverNodeId],
    ) -> FreshSolverNodeId {
        self.backend.new_with_children(style, children)
    }

    pub(super) fn new_measured(&mut self, style: SolverStyle, context: C) -> FreshSolverNodeId {
        self.backend.new_measured(style, context)
    }

    pub(super) fn compute_layout_with_measure(
        &mut self,
        root: FreshSolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        measure: impl FnMut(FreshSolverNodeId, Option<&C>, SolverMeasureQuery) -> Size<f32>,
    ) {
        self.backend
            .compute_layout_with_measure(root, available_space, scale_factor, measure);
    }

    pub(super) fn layout(&self, node_id: FreshSolverNodeId) -> SolverLayout {
        self.backend.layout(node_id)
    }

    pub(super) fn children(&self, node_id: FreshSolverNodeId) -> Vec<FreshSolverNodeId> {
        self.backend.children(node_id)
    }
}
