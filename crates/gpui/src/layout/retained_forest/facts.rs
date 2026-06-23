//! Current-frame retained-layout facts.
//!
//! These values are temporary GPUI layout facts: what the current render pass
//! asked layout to solve. They are not retained identity and they do not own
//! solver state.

use super::super::LayoutId;
use super::measurement::MeasuredLayoutFacts;
use super::solver::SolverStyle;
use crate::GlobalElementId;

/// Pure layout request facts produced during the current frame.
///
/// A `CurrentLayoutNodeFacts` is not retained authority. It is a temporary input that
/// says what GPUI wants this frame: a lowered solver style plus either child
/// fact ids or an explicit measured-node kind.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CurrentLayoutNodeFacts {
    pub(super) global_id: Option<GlobalElementId>,
    pub(super) style: SolverStyle,
    pub(super) kind: CurrentLayoutNodeKind,
}

/// Current-frame layout node shape.
///
/// Measured nodes store only comparable measurement facts. Executable producers
/// and paint/hit-test artifacts stay in the measurement owner.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum CurrentLayoutNodeKind {
    Unmeasured { children: Vec<LayoutId> },
    Measured(MeasuredLayoutFacts),
}
