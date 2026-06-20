//! Retained GPUI layout occurrence tree.
//!
//! Each occurrence is the retained-tree node owned by GPUI. It stores only
//! layout facts and one private solver handle. Artifact policy, concrete solver
//! behavior, and measurement producers are deliberately outside this type.

use super::measurement::MeasuredLayoutFacts;
use super::solver::{SolverNodeId, SolverStyle};
use crate::GlobalElementId;

/// Layout-visible facts retained with an occurrence.
///
/// These facts describe the retained occurrence and private mirror node. They
/// may justify preserving the occurrence, but GPUI-visible geometry and
/// measurement outputs still require current-frame solve output or explicit
/// current observations.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum RetainedLayoutOccurrenceKind {
    Unmeasured {
        children: Vec<RetainedLayoutOccurrence>,
    },
    Measured {
        measured_facts: MeasuredLayoutFacts,
    },
}

/// Cross-frame GPUI layout occurrence.
///
/// Each occurrence owns exactly one solver node plus the retained facts and
/// child occurrences that make that mirror node meaningful. Outside the
/// retained-forest module tree, the concrete solver node is never exposed.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RetainedLayoutOccurrence {
    pub(super) node_id: SolverNodeId,
    pub(super) identity: Option<GlobalElementId>,
    pub(super) style: SolverStyle,
    pub(super) kind: RetainedLayoutOccurrenceKind,
}

impl RetainedLayoutOccurrence {
    pub(super) fn children(&self) -> &[RetainedLayoutOccurrence] {
        match &self.kind {
            RetainedLayoutOccurrenceKind::Unmeasured { children } => children,
            RetainedLayoutOccurrenceKind::Measured { .. } => &[],
        }
    }

    pub(super) fn into_children(self) -> Vec<RetainedLayoutOccurrence> {
        match self.kind {
            RetainedLayoutOccurrenceKind::Unmeasured { children } => children,
            RetainedLayoutOccurrenceKind::Measured { .. } => Vec::new(),
        }
    }

    pub(super) fn measured_facts(&self) -> Option<&MeasuredLayoutFacts> {
        match &self.kind {
            RetainedLayoutOccurrenceKind::Unmeasured { .. } => None,
            RetainedLayoutOccurrenceKind::Measured { measured_facts } => Some(measured_facts),
        }
    }

    pub(super) fn kind_name(&self) -> &'static str {
        match &self.kind {
            RetainedLayoutOccurrenceKind::Unmeasured { .. } => "unmeasured",
            RetainedLayoutOccurrenceKind::Measured { .. } => "measured",
        }
    }
}
