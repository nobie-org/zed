//! Layout work counters emitted after a completed draw.
//!
//! These counters are intentionally separate from the retained forest so timing
//! and observability do not become part of the layout authority model.

use super::retained_forest::FreshLayoutComparisonSummary;
use super::retained_forest::{RetainedLayoutMissWork, RetainedLayoutWork};
use std::time::Duration;

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
    /// Measured layout callbacks invoked by the retained layout engine.
    pub measured_layout_calls: u64,
    /// Wall time spent computing root layouts.
    pub compute_layout_duration: Duration,
    /// Wall time spent inside measured layout callbacks.
    pub measured_layout_duration: Duration,
    /// Retained layout occurrences allocated while committing current layout intent.
    pub retained_layout_creates: u64,
    /// Retained layout occurrences reused while committing current layout intent.
    pub retained_layout_reuses: u64,
    /// Retained layout occurrence style updates emitted while committing current intent.
    pub retained_layout_style_updates: u64,
    /// Retained layout child-list updates emitted while committing current intent.
    pub retained_layout_child_list_updates: u64,
    /// Retained measured-context clears emitted while removing retained occurrences.
    pub retained_layout_measured_context_clears: u64,
    /// Retained layout occurrences removed while sweeping old subtrees.
    pub retained_layout_removes: u64,
    /// Retained occurrence matches missed because no previous occurrence was available.
    pub retained_layout_miss_no_previous: u64,
    /// Retained occurrence matches missed because style changed.
    pub retained_layout_miss_style: u64,
    /// Retained occurrence matches missed because node kind changed.
    pub retained_layout_miss_kind: u64,
    /// Retained measured occurrence matches missed because the measured key or kind changed.
    pub retained_layout_miss_measured_kind: u64,
    /// Retained occurrence matches missed because child count changed.
    pub retained_layout_miss_child_count: u64,
    /// Retained occurrence matches missed because a descendant subtree changed.
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
        self.retained_layout_measured_context_clears = work.measured_context_clears;
        self.retained_layout_removes = work.removes;
        self.retained_layout_miss_no_previous = misses.no_previous;
        self.retained_layout_miss_style = misses.style;
        self.retained_layout_miss_kind = misses.kind;
        self.retained_layout_miss_measured_kind = misses.measured_kind;
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
