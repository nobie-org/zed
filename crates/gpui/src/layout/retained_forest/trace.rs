//! Retained-layout trace sinks.
//!
//! Tracing observes retained layout and solver events for debugging. It must not
//! decide retained identity, dirtying, measurement validity, or solve policy.

use super::solver::{SolverCacheEntry, SolverCacheEvent, SolverCacheMiss, SolverNodeId};
use crate::layout::LayoutId;
use std::sync::{
    OnceLock,
    atomic::{AtomicUsize, Ordering},
};

/// Cache-event trace sink for one legal root solve.
pub(super) struct CacheEventTracer {
    node_layout_ids: Vec<(SolverNodeId, LayoutId)>,
}

impl CacheEventTracer {
    pub(super) fn new(node_layout_ids: Vec<(SolverNodeId, LayoutId)>) -> Self {
        Self { node_layout_ids }
    }

    pub(super) fn record(&mut self, event: SolverCacheEvent) {
        if trace_layout_ids().is_none() {
            return;
        }

        match event {
            SolverCacheEvent::Hit(entry) => {
                self.trace_layout_cache_entry("hit", entry);
            }
            SolverCacheEvent::Stored(entry) => {
                self.trace_layout_cache_entry("stored", entry);
            }
            SolverCacheEvent::Miss(miss) => {
                self.trace_layout_cache_miss(miss);
            }
            SolverCacheEvent::Cleared(clear) => {
                let node_id = clear.node_id();
                let layout_id = self.layout_id_for_node(node_id);
                if layout_id_is_targeted(layout_id) {
                    eprintln!(
                        "gpui retained_layout cache_event kind=cleared layout_id={:?} node_id={:?}",
                        layout_id, node_id
                    );
                }
            }
            SolverCacheEvent::Measure(_) => {}
        }
    }

    fn trace_layout_cache_miss(&self, miss: SolverCacheMiss) {
        let node_id = miss.node_id();
        let layout_id = self.layout_id_for_node(node_id);

        if !layout_id_is_targeted(layout_id) || !should_trace_all_cache_events() {
            return;
        }

        let details = miss.trace_details();
        eprintln!(
            "gpui retained_layout cache_event kind=miss layout_id={:?} node_id={:?} reason={:?} requested_run_mode={:?} cache_run_mode={:?} cache_sizing_mode={:?} cache_axis={:?} requested_known_dimensions={:?} cache_known_dimensions={:?} requested_parent_size={:?} cache_parent_size={:?} requested_available_space={:?} cache_available_space={:?} descendant_layout_generation={} cached_descendant_layout_generation={:?}",
            layout_id,
            node_id,
            details.reason,
            details.requested_run_mode,
            details.cache_run_mode,
            details.cache_sizing_mode,
            details.cache_axis,
            details.requested_known_dimensions,
            details.cache_known_dimensions,
            details.requested_parent_size,
            details.cache_parent_size,
            details.requested_available_space,
            details.cache_available_space,
            details.descendant_layout_generation,
            details.cached_descendant_layout_generation
        );
    }

    fn trace_layout_cache_entry(&self, kind: &'static str, entry: SolverCacheEntry) {
        let node_id = entry.node_id();
        let layout_id = self.layout_id_for_node(node_id);

        if !layout_id_is_targeted(layout_id) {
            return;
        }

        let details = entry.trace_details();
        if !should_trace_all_cache_events()
            && !(details.has_zero_output
                || details.has_zero_known_dimension
                || details.has_zero_parent_dimension
                || details.has_zero_available_space)
        {
            return;
        }

        eprintln!(
            "gpui retained_layout cache_event kind={} layout_id={:?} node_id={:?} entry_id={:?} run_mode={:?} sizing_mode={:?} axis={:?} known_dimensions={:?} parent_size={:?} available_space={:?} output_size={:?}",
            kind,
            layout_id,
            node_id,
            details.entry_id,
            details.run_mode,
            details.sizing_mode,
            details.axis,
            details.known_dimensions,
            details.parent_size,
            details.available_space,
            details.output_size
        );
    }

    fn layout_id_for_node(&self, node_id: SolverNodeId) -> Option<usize> {
        self.node_layout_ids
            .iter()
            .find_map(|(candidate_node_id, layout_id)| {
                (*candidate_node_id == node_id).then_some(layout_id.0)
            })
    }
}

pub(super) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("GPUI_TRACE_RETAINED_LAYOUT").is_some())
}

pub(super) fn miss_trace_limit() -> Option<usize> {
    static LIMIT: OnceLock<Option<usize>> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        let value = std::env::var("GPUI_TRACE_RETAINED_LAYOUT_MISSES").ok()?;
        if value.is_empty() {
            return Some(64);
        }
        value.parse::<usize>().ok().or(Some(64))
    })
}

pub(super) fn detail_enabled() -> bool {
    enabled() && trace_layout_ids().is_some()
}

pub(super) fn layout_id_is_targeted(layout_id: Option<usize>) -> bool {
    trace_layout_ids()
        .map(|target_layout_ids| {
            layout_id
                .map(|layout_id| target_layout_ids.contains(&layout_id))
                .unwrap_or(false)
        })
        .unwrap_or(true)
}

pub(super) fn target_layout_ids() -> Option<&'static [usize]> {
    trace_layout_ids().map(Vec::as_slice)
}

pub(super) fn should_trace_mutation() -> bool {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let Some(limit) = mutation_trace_limit() else {
        return false;
    };
    COUNT.fetch_add(1, Ordering::Relaxed) < limit
}

pub(super) fn should_trace_zero_bounds() -> bool {
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let Some(limit) = zero_bounds_trace_limit() else {
        return false;
    };
    COUNT.fetch_add(1, Ordering::Relaxed) < limit
}

fn trace_layout_ids() -> Option<&'static Vec<usize>> {
    static LAYOUT_IDS: OnceLock<Option<Vec<usize>>> = OnceLock::new();
    LAYOUT_IDS
        .get_or_init(|| parse_layout_ids_env("GPUI_TRACE_RETAINED_LAYOUT_IDS"))
        .as_ref()
}

fn parse_layout_ids_env(name: &str) -> Option<Vec<usize>> {
    let layout_ids = std::env::var(name).ok()?;
    Some(
        layout_ids
            .split(',')
            .filter_map(|layout_id| layout_id.trim().parse().ok())
            .collect(),
    )
}

fn zero_bounds_trace_limit() -> Option<usize> {
    static LIMIT: OnceLock<Option<usize>> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        let value = std::env::var("GPUI_TRACE_RETAINED_LAYOUT_ZERO_BOUNDS").ok()?;
        if value.is_empty() {
            return Some(128);
        }
        value.parse::<usize>().ok().or(Some(128))
    })
}

fn mutation_trace_limit() -> Option<usize> {
    static LIMIT: OnceLock<Option<usize>> = OnceLock::new();
    *LIMIT.get_or_init(|| {
        let value = std::env::var("GPUI_TRACE_RETAINED_LAYOUT_MUTATIONS").ok()?;
        if value.is_empty() {
            return Some(256);
        }
        value.parse::<usize>().ok().or(Some(256))
    })
}

fn should_trace_all_cache_events() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED
        .get_or_init(|| std::env::var_os("GPUI_TRACE_RETAINED_LAYOUT_CACHE_EVENTS_ALL").is_some())
}
