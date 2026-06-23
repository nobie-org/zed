//! Retained-layout trace switches.
//!
//! Tracing observes retained layout for debugging. It must not decide retained
//! identity, dirtying, measurement validity, or solve policy.

use std::sync::{
    OnceLock,
    atomic::{AtomicUsize, Ordering},
};

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
