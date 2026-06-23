//! Private layout-solver facade for the retained layout forest.
//!
//! The retained tree owns GPUI layout facts. This module is the only surface the
//! retained tree can use to mutate or solve the downstream layout mirror. The
//! concrete implementation lives in `solver::backend`; callers see only opaque
//! solver handles, styles, measurement queries, and solved layouts.

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
    fn parent(&self, node_id: SolverNodeId) -> Option<SolverNodeId>;
    fn children(&self, node_id: SolverNodeId) -> Vec<SolverNodeId>;
    fn capture_layout_tree(&self, root: SolverNodeId) -> Vec<(SolverNodeId, SolverLayout)>;
    fn style(&self, node_id: SolverNodeId) -> Option<SolverStyle>;
    fn has_measure_context(&self, node_id: SolverNodeId) -> bool;
    fn compute_layout_with_measure(
        &mut self,
        root: SolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        measure: impl FnMut(SolverNodeId, bool, SolverMeasureQuery) -> Size<f32>,
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

    pub(super) fn parent(&self, node_id: SolverNodeId) -> Option<SolverNodeId> {
        self.backend.parent(node_id)
    }

    pub(super) fn children(&self, node_id: SolverNodeId) -> Vec<SolverNodeId> {
        self.backend.children(node_id)
    }

    /// Copy all layout output for a just-solved legal root out of the solver.
    ///
    /// The returned values are a snapshot. Callers do not get per-node access
    /// to the solver's mutable layout slots, so solver history cannot become
    /// GPUI-visible geometry authority.
    pub(super) fn capture_layout_tree(
        &self,
        root: SolverNodeId,
    ) -> Vec<(SolverNodeId, SolverLayout)> {
        self.backend.capture_layout_tree(root)
    }

    pub(super) fn style(&self, node_id: SolverNodeId) -> Option<SolverStyle> {
        self.backend.style(node_id)
    }

    pub(super) fn has_measure_context(&self, node_id: SolverNodeId) -> bool {
        self.backend.has_measure_context(node_id)
    }

    pub(super) fn compute_layout_with_measure(
        &mut self,
        root: SolverNodeId,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        measure: impl FnMut(SolverNodeId, bool, SolverMeasureQuery) -> Size<f32>,
    ) {
        self.backend
            .compute_layout_with_measure(root, available_space, scale_factor, measure);
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
