//! Retained GPUI layout node tree.
//!
//! Each retained node is the retained-tree node owned by GPUI. It stores only
//! layout facts and one private solver handle. Artifact policy, concrete solver
//! behavior, and measurement producers are deliberately outside this type.

use super::measurement::MeasuredLayoutFacts;
use super::solver::{SolverNodeId, SolverStyle};
use crate::GlobalElementId;

/// Layout-visible facts retained with a retained node.
///
/// These facts describe the retained node and private mirror node. They
/// may justify preserving the node, but GPUI-visible geometry and
/// measurement outputs still require current-frame solve output or explicit
/// current observations.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum RetainedLayoutNodeKind {
    Unmeasured { children: Vec<RetainedLayoutNode> },
    Measured { measured_facts: MeasuredLayoutFacts },
}

/// Cross-frame GPUI layout node.
///
/// Each retained node owns exactly one solver node plus the retained facts and
/// child nodes that make that mirror node meaningful. Outside the
/// retained-forest module tree, the concrete solver node is never exposed.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RetainedLayoutNode {
    pub(super) node_id: SolverNodeId,
    pub(super) identity: Option<GlobalElementId>,
    pub(super) style: SolverStyle,
    pub(super) kind: RetainedLayoutNodeKind,
}

impl RetainedLayoutNode {
    pub(super) fn children(&self) -> &[RetainedLayoutNode] {
        match &self.kind {
            RetainedLayoutNodeKind::Unmeasured { children } => children,
            RetainedLayoutNodeKind::Measured { .. } => &[],
        }
    }

    pub(super) fn into_children(self) -> Vec<RetainedLayoutNode> {
        match self.kind {
            RetainedLayoutNodeKind::Unmeasured { children } => children,
            RetainedLayoutNodeKind::Measured { .. } => Vec::new(),
        }
    }

    pub(super) fn measured_facts(&self) -> Option<&MeasuredLayoutFacts> {
        match &self.kind {
            RetainedLayoutNodeKind::Unmeasured { .. } => None,
            RetainedLayoutNodeKind::Measured { measured_facts } => Some(measured_facts),
        }
    }

    pub(super) fn kind_name(&self) -> &'static str {
        match &self.kind {
            RetainedLayoutNodeKind::Unmeasured { .. } => "unmeasured",
            RetainedLayoutNodeKind::Measured { .. } => "measured",
        }
    }
}
