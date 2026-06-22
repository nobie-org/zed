//! Current-frame retained-layout facts.
//!
//! These values are temporary GPUI layout facts: what the current render pass
//! asked layout to solve. They are not retained identity and they do not own
//! solver state.

use super::super::LayoutId;
use super::measurement::MeasuredLayoutFacts;
use super::solver::SolverStyle;
use crate::{Display, GlobalElementId, Style};

/// Pure layout request facts produced during the current frame.
///
/// A `CurrentLayoutNodeFacts` is not retained authority. It is a temporary input that
/// says what GPUI wants this frame: a lowered solver style plus either child
/// fact ids or an explicit measured-node kind.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CurrentLayoutNodeFacts {
    pub(super) global_id: Option<GlobalElementId>,
    pub(super) style: SolverStyle,
    pub(super) artifact_policy: LayoutArtifactPolicy,
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

/// Whether a layout fact's subtree can produce GPUI paint artifacts.
///
/// This is a GPUI fact, not solver state. `display: none` subtrees still exist as
/// layout facts, but GPUI skips prepaint/paint for their descendants, so the
/// measurement owner must not require text artifacts from them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LayoutArtifactPolicy {
    CanProduceArtifacts,
    SkipsArtifactSubtree,
}

impl LayoutArtifactPolicy {
    pub(super) fn from_style(style: &Style) -> Self {
        if style.display == Display::None {
            Self::SkipsArtifactSubtree
        } else {
            Self::CanProduceArtifacts
        }
    }

    pub(super) fn can_produce_artifacts(self) -> bool {
        matches!(self, Self::CanProduceArtifacts)
    }
}
