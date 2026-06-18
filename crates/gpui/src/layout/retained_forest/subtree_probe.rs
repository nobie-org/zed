//! Subtree-scoped retained-layout proof instrumentation.
//!
//! This module is private to the retained forest because it needs to name
//! Taffy mirror nodes and cache events. Its samples expose only GPUI-facing
//! identity strings, layout ids, node counts, and typed work counters.

use super::super::LayoutId;
use super::work::RetainedWorkDelta;
use crate::GlobalElementId;
use collections::FxHashSet;
use std::sync::OnceLock;
use taffy::tree::{LayoutCacheEvent, NodeId};

/// Work observed for one explicitly identified retained subtree in one frame.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(in crate::layout) struct RetainedSubtreeWorkSample {
    pub(in crate::layout) global_id: String,
    pub(in crate::layout) layout_id: usize,
    pub(in crate::layout) node_count: usize,
    pub(in crate::layout) retained_reuses: u64,
    pub(in crate::layout) retained_misses: u64,
    pub(in crate::layout) mirror_node_creates: u64,
    pub(in crate::layout) mirror_node_removes: u64,
    pub(in crate::layout) mirror_set_style: u64,
    pub(in crate::layout) mirror_set_children: u64,
    pub(in crate::layout) mirror_measured_context_clears: u64,
    pub(in crate::layout) mirror_cache_invalidations: u64,
    pub(in crate::layout) layout_cache_hits: u64,
    pub(in crate::layout) layout_cache_stores: u64,
    pub(in crate::layout) layout_cache_clears: u64,
    pub(in crate::layout) measured_callbacks: u64,
}

impl RetainedSubtreeWorkSample {
    /// Return the observable work that must be zero for a stable subtree.
    pub(in crate::layout) fn no_work_total(&self) -> u64 {
        self.retained_misses
            + self.mirror_node_creates
            + self.mirror_node_removes
            + self.mirror_set_style
            + self.mirror_set_children
            + self.mirror_measured_context_clears
            + self.mirror_cache_invalidations
            + self.layout_cache_stores
            + self.layout_cache_clears
            + self.measured_callbacks
    }
}

#[derive(Clone)]
struct ActiveSubtree {
    sample_index: usize,
    node_ids: FxHashSet<NodeId>,
}

/// Private subtree probe state for the current frame.
#[derive(Clone, Default)]
pub(super) struct SubtreeProbe {
    active_subtrees: Vec<ActiveSubtree>,
    frame_samples: Vec<RetainedSubtreeWorkSample>,
    emitted_samples: usize,
    targets_for_tests: Option<Vec<String>>,
}

/// Transaction checkpoint for subtree proof state.
#[derive(Clone)]
pub(super) struct SubtreeProbeCheckpoint {
    active_subtrees: Vec<ActiveSubtree>,
    frame_samples: Vec<RetainedSubtreeWorkSample>,
    emitted_samples: usize,
    targets_for_tests: Option<Vec<String>>,
}

/// Per-compute recorder that keeps Taffy event attribution out of callers.
pub(super) struct SubtreeProbeComputeRecorder {
    active_subtrees: Vec<ActiveSubtree>,
    sample_deltas: Vec<SubtreeProbeComputeDelta>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SubtreeProbeComputeDelta {
    layout_cache_hits: u64,
    layout_cache_stores: u64,
    layout_cache_clears: u64,
    measured_callbacks: u64,
}

impl SubtreeProbe {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn begin_frame(&mut self) {
        self.active_subtrees.clear();
        self.frame_samples.clear();
    }

    pub(super) fn checkpoint(&self) -> SubtreeProbeCheckpoint {
        SubtreeProbeCheckpoint {
            active_subtrees: self.active_subtrees.clone(),
            frame_samples: self.frame_samples.clone(),
            emitted_samples: self.emitted_samples,
            targets_for_tests: self.targets_for_tests.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: SubtreeProbeCheckpoint) {
        self.active_subtrees = checkpoint.active_subtrees;
        self.frame_samples = checkpoint.frame_samples;
        self.emitted_samples = checkpoint.emitted_samples;
        self.targets_for_tests = checkpoint.targets_for_tests;
    }

    pub(super) fn finish_frame(&mut self) {
        self.emit_samples();
        self.active_subtrees.clear();
        self.frame_samples.clear();
    }

    pub(super) fn matched_global_id(&self, global_id: Option<&GlobalElementId>) -> Option<String> {
        let global_id = global_id?;
        let rendered = global_id.to_string();
        self.targets()
            .iter()
            .any(|target| global_id_matches_target(&rendered, target))
            .then_some(rendered)
    }

    pub(super) fn record_committed_subtree(
        &mut self,
        global_id: String,
        layout_id: LayoutId,
        node_ids: Vec<NodeId>,
        work_delta: RetainedWorkDelta,
    ) {
        if node_ids.is_empty() {
            return;
        }

        let node_count = node_ids.len();
        let node_ids = node_ids.into_iter().collect::<FxHashSet<_>>();
        let sample_index = self.frame_samples.len();
        self.frame_samples.push(RetainedSubtreeWorkSample {
            global_id,
            layout_id: layout_id.0,
            node_count,
            retained_reuses: work_delta.work.reuses,
            retained_misses: work_delta.miss_count(),
            mirror_node_creates: work_delta.work.creates,
            mirror_node_removes: work_delta.work.removes,
            mirror_set_style: work_delta.work.style_updates,
            mirror_set_children: work_delta.work.child_list_updates,
            mirror_measured_context_clears: work_delta.work.measured_context_clears,
            mirror_cache_invalidations: work_delta.work.cache_invalidations,
            ..RetainedSubtreeWorkSample::default()
        });
        self.active_subtrees.push(ActiveSubtree {
            sample_index,
            node_ids,
        });
    }

    pub(super) fn compute_recorder(&self) -> SubtreeProbeComputeRecorder {
        SubtreeProbeComputeRecorder::new(self.active_subtrees.clone(), self.frame_samples.len())
    }

    pub(super) fn record_compute(&mut self, recorder: SubtreeProbeComputeRecorder) {
        for (sample_index, delta) in recorder.into_deltas() {
            if let Some(sample) = self.frame_samples.get_mut(sample_index) {
                sample.layout_cache_hits += delta.layout_cache_hits;
                sample.layout_cache_stores += delta.layout_cache_stores;
                sample.layout_cache_clears += delta.layout_cache_clears;
                sample.measured_callbacks += delta.measured_callbacks;
            }
        }
    }

    fn targets(&self) -> &[String] {
        if let Some(targets) = self.targets_for_tests.as_ref() {
            targets.as_slice()
        } else {
            env_subtree_targets()
        }
    }

    fn emit_samples(&mut self) {
        if env_subtree_targets().is_empty() || self.frame_samples.is_empty() {
            return;
        }

        let limit = env_subtree_sample_limit();
        for sample in &self.frame_samples {
            if self.emitted_samples >= limit {
                break;
            }
            self.emitted_samples += 1;
            eprintln!(
                "gpui retained_layout subtree_sample global_id=\"{}\" layout_id={} nodes={} no_work_total={} retained_reuses={} retained_misses={} creates={} removes={} set_style={} set_children={} context_clears={} cache_invalidations={} cache_hits={} cache_stores={} cache_clears={} measured_callbacks={}",
                sample.global_id,
                sample.layout_id,
                sample.node_count,
                sample.no_work_total(),
                sample.retained_reuses,
                sample.retained_misses,
                sample.mirror_node_creates,
                sample.mirror_node_removes,
                sample.mirror_set_style,
                sample.mirror_set_children,
                sample.mirror_measured_context_clears,
                sample.mirror_cache_invalidations,
                sample.layout_cache_hits,
                sample.layout_cache_stores,
                sample.layout_cache_clears,
                sample.measured_callbacks,
            );
        }
    }

    #[cfg(test)]
    pub(super) fn set_targets_for_tests(&mut self, targets: Vec<String>) {
        self.targets_for_tests = Some(targets);
    }

    #[cfg(test)]
    pub(super) fn samples_for_tests(&self) -> &[RetainedSubtreeWorkSample] {
        &self.frame_samples
    }
}

impl SubtreeProbeComputeRecorder {
    fn new(active_subtrees: Vec<ActiveSubtree>, sample_count: usize) -> Self {
        Self {
            active_subtrees,
            sample_deltas: vec![SubtreeProbeComputeDelta::default(); sample_count],
        }
    }

    pub(super) fn record_measured_callback(&mut self, node_id: NodeId) {
        self.record_node(node_id, |delta| delta.measured_callbacks += 1);
    }

    pub(super) fn record_cache_event(&mut self, event: LayoutCacheEvent) {
        match event {
            LayoutCacheEvent::Hit(entry) => {
                self.record_node(entry.node_id(), |delta| delta.layout_cache_hits += 1);
            }
            LayoutCacheEvent::Stored(entry) => {
                self.record_node(entry.node_id(), |delta| delta.layout_cache_stores += 1);
            }
            LayoutCacheEvent::Cleared(clear) => {
                self.record_node(clear.node_id(), |delta| delta.layout_cache_clears += 1);
            }
            _ => {}
        }
    }

    fn record_node(
        &mut self,
        node_id: NodeId,
        mut record: impl FnMut(&mut SubtreeProbeComputeDelta),
    ) {
        for active in &self.active_subtrees {
            if active.node_ids.contains(&node_id)
                && let Some(delta) = self.sample_deltas.get_mut(active.sample_index)
            {
                record(delta);
            }
        }
    }

    fn into_deltas(self) -> impl Iterator<Item = (usize, SubtreeProbeComputeDelta)> {
        self.sample_deltas
            .into_iter()
            .enumerate()
            .filter(|(_, delta)| *delta != SubtreeProbeComputeDelta::default())
    }
}

fn global_id_matches_target(global_id: &str, target: &str) -> bool {
    global_id == target
        || global_id.ends_with(target)
            && global_id
                .strip_suffix(target)
                .map(|prefix| prefix.is_empty() || prefix.ends_with('.'))
                .unwrap_or(false)
}

fn env_subtree_targets() -> &'static [String] {
    static TARGETS: OnceLock<Vec<String>> = OnceLock::new();
    TARGETS
        .get_or_init(|| {
            std::env::var("GPUI_TRACE_RETAINED_LAYOUT_SUBTREES")
                .ok()
                .into_iter()
                .flat_map(|targets| {
                    targets
                        .split(',')
                        .map(str::trim)
                        .filter(|target| !target.is_empty())
                        .map(ToOwned::to_owned)
                        .collect::<Vec<_>>()
                })
                .collect()
        })
        .as_slice()
}

fn env_subtree_sample_limit() -> usize {
    static LIMIT: OnceLock<usize> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var("GPUI_TRACE_RETAINED_LAYOUT_SUBTREES_LIMIT")
            .ok()
            .and_then(|limit| limit.parse().ok())
            .unwrap_or(120)
    })
}
