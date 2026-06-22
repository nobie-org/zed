//! Layout work counters emitted after a completed draw.
//!
//! These counters are intentionally separate from the retained forest so timing
//! and observability do not become part of the layout authority model.

use super::retained_forest::FreshLayoutComparisonSummary;
use super::retained_forest::{RetainedLayoutMissWork, RetainedLayoutWork};
use std::time::Duration;

/// Retained-layout work observed for one explicitly identified subtree.
///
/// This is a GPUI-facing diagnostic sample. The retained forest may use private
/// solver mirror nodes to attribute work internally, but exported samples expose
/// only element identity strings, layout ids, node counts, and typed retained
/// work counters.
#[non_exhaustive]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RetainedSubtreeWorkSample {
    /// The matched [`GlobalElementId`](crate::GlobalElementId) rendered as a string.
    pub global_id: String,
    /// The current-frame [`LayoutId`](super::LayoutId) at the subtree root.
    pub layout_id: usize,
    /// Number of retained mirror nodes inside the observed subtree.
    pub node_count: usize,
    /// Retained nodes reused while committing this subtree.
    pub retained_reuses: u64,
    /// Retained node misses while committing this subtree.
    pub retained_misses: u64,
    /// Mirror nodes created inside this subtree.
    pub mirror_node_creates: u64,
    /// Mirror nodes removed inside this subtree.
    pub mirror_node_removes: u64,
    /// Mirror `set_style` operations inside this subtree.
    pub mirror_set_style: u64,
    /// Mirror `set_children` operations inside this subtree.
    pub mirror_set_children: u64,
    /// Measured-context clears caused by subtree removal.
    pub mirror_measured_context_clears: u64,
    /// Measured callbacks attributed to nodes inside this subtree.
    pub measured_callbacks: u64,
    /// Measured callbacks that the measurement owner excludes from `no_work_total`.
    ///
    /// The field keeps its historical name for existing diagnostics. The
    /// retained subtree probe no longer decides this from text identity.
    pub conservative_text_measured_callbacks: u64,
}

impl RetainedSubtreeWorkSample {
    /// Return the observable work that must be zero for a stable subtree.
    pub fn no_work_total(&self) -> u64 {
        let hard_measured_callbacks = self
            .measured_callbacks
            .saturating_sub(self.conservative_text_measured_callbacks);
        self.retained_misses
            + self.mirror_node_creates
            + self.mirror_node_removes
            + self.mirror_set_style
            + self.mirror_set_children
            + self.mirror_measured_context_clears
            + hard_measured_callbacks
    }
}

/// Layout work performed by GPUI for one completed window draw.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LayoutWorkSample {
    /// Monotonic window-local draw index that produced this sample.
    pub draw_index: u64,
    /// Non-measured nodes requested through [`Window::request_layout`](crate::Window::request_layout).
    pub layout_node_requests: u64,
    /// Measured nodes requested through [`Window::request_measured_layout`](crate::Window::request_measured_layout).
    pub measured_layout_node_requests: u64,
    /// Parent-to-child layout edges requested by GPUI.
    pub child_edges: u64,
    /// Root layout computations requested for the draw.
    pub compute_layout_calls: u64,
    /// Root layout computations that invoked the private solver.
    pub solver_compute_layout_calls: u64,
    /// Measured layout queries that required GPUI measurement work.
    ///
    /// Exact retained artifact/cache answers are excluded. The private solver may
    /// still ask a measured query during its legal solve; this counter tracks the
    /// GPUI producer work that was not avoided.
    pub measured_layout_calls: u64,
    /// Wall time spent computing root layouts.
    pub compute_layout_duration: Duration,
    /// Wall time spent inside measured layout work counted by
    /// [`LayoutWorkSample::measured_layout_calls`].
    pub measured_layout_duration: Duration,
    /// Retained layout nodes allocated while committing current layout facts.
    pub retained_layout_creates: u64,
    /// Retained layout nodes reused while committing current layout facts.
    pub retained_layout_reuses: u64,
    /// Retained layout node style updates emitted while committing current facts.
    pub retained_layout_style_updates: u64,
    /// Retained layout child-list updates emitted while committing current facts.
    pub retained_layout_child_list_updates: u64,
    /// Explicit retained mirror dirty marks emitted while committing current facts.
    pub retained_layout_dirty_marks: u64,
    /// Retained measured-context clears emitted while removing retained nodes.
    pub retained_layout_measured_context_clears: u64,
    /// Retained layout nodes removed while sweeping old retained subtrees.
    pub retained_layout_removes: u64,
    /// Retained node matches missed because no previous node was available.
    pub retained_layout_miss_no_previous: u64,
    /// Retained node matches missed because style changed.
    pub retained_layout_miss_style: u64,
    /// Retained node matches missed because node kind changed.
    pub retained_layout_miss_kind: u64,
    /// Retained measured node matches missed because measured facts changed.
    ///
    /// This field keeps the historical telemetry name. The retained forest
    /// records the miss as a measured-facts miss internally, but callers should
    /// not need to track that implementation wording.
    pub retained_layout_miss_measured_kind: u64,
    /// Retained node matches missed because child count changed.
    pub retained_layout_miss_child_count: u64,
    /// Retained node matches missed because a descendant subtree changed.
    pub retained_layout_miss_child_subtree: u64,
    /// Retained child matches missed because no exact previous sibling subtree was available.
    pub retained_layout_miss_no_exact_child: u64,
    /// Retained-vs-fresh layout comparison nodes checked for the draw.
    pub retained_layout_fresh_compare_nodes: u64,
    /// Retained-vs-fresh layout comparison nodes whose layout differed.
    pub retained_layout_fresh_compare_mismatches: u64,
    /// Retained-vs-fresh layout comparison nodes that matched with a zero width or height.
    pub retained_layout_fresh_compare_equal_zero_nodes: u64,
    /// Targeted retained-vs-fresh layout comparison nodes checked for the draw.
    pub retained_layout_fresh_compare_target_nodes: u64,
    /// Targeted retained-vs-fresh layout comparison nodes whose layout differed.
    pub retained_layout_fresh_compare_target_mismatches: u64,
}

impl LayoutWorkSample {
    pub(super) fn record_retained_layout_work(
        &mut self,
        work: RetainedLayoutWork,
        misses: RetainedLayoutMissWork,
    ) {
        self.retained_layout_creates = work.creates;
        self.retained_layout_reuses = work.reuses;
        self.retained_layout_style_updates = work.style_updates;
        self.retained_layout_child_list_updates = work.child_list_updates;
        self.retained_layout_dirty_marks = work.dirty_marks;
        self.retained_layout_measured_context_clears = work.measured_context_clears;
        self.retained_layout_removes = work.removes;
        self.retained_layout_miss_no_previous = misses.no_previous;
        self.retained_layout_miss_style = misses.style;
        self.retained_layout_miss_kind = misses.kind;
        self.retained_layout_miss_measured_kind = misses.measured_facts;
        self.retained_layout_miss_child_count = misses.child_count;
        self.retained_layout_miss_child_subtree = misses.child_subtree;
        self.retained_layout_miss_no_exact_child = misses.no_exact_child;
    }

    pub(super) fn record_retained_fresh_layout_comparison(
        &mut self,
        summary: FreshLayoutComparisonSummary,
    ) {
        self.retained_layout_fresh_compare_nodes += summary.checked_nodes;
        self.retained_layout_fresh_compare_mismatches += summary.mismatches;
        self.retained_layout_fresh_compare_equal_zero_nodes += summary.equal_zero_nodes;
        self.retained_layout_fresh_compare_target_nodes += summary.target_nodes;
        self.retained_layout_fresh_compare_target_mismatches += summary.target_mismatches;
    }
}
