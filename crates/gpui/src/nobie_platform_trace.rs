#![allow(missing_docs)]

use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const TRACE_ENV: &str = "NOBIE_GPUI_TRACE_PLATFORM_PRESENT";

thread_local! {
    static CURRENT_DRAW_ID: Cell<u64> = const { Cell::new(0) };
    static CURRENT_PRESENT_ID: Cell<u64> = const { Cell::new(0) };
    static CURRENT_DRAW_REASON: Cell<&'static str> = const { Cell::new("unspecified") };
}

fn enabled_flag() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os(TRACE_ENV).is_some_and(|value| !value.is_empty()))
}

pub fn enabled() -> bool {
    enabled_flag()
}

fn start_time() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

fn next_id(counter: &AtomicU64) -> u64 {
    if !enabled() {
        return 0;
    }
    counter.fetch_add(1, Ordering::Relaxed).saturating_add(1)
}

pub fn next_draw_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    next_id(&NEXT)
}

pub fn next_present_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    next_id(&NEXT)
}

pub fn next_request_frame_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    next_id(&NEXT)
}

pub fn next_display_link_signal_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    next_id(&NEXT)
}

pub fn next_main_queue_probe_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    next_id(&NEXT)
}

pub fn next_platform_draw_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    next_id(&NEXT)
}

pub fn next_metal_draw_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    next_id(&NEXT)
}

pub fn set_current_draw_id(draw_id: u64) {
    CURRENT_DRAW_ID.with(|current| current.set(draw_id));
}

pub fn clear_current_draw_id() {
    set_current_draw_id(0);
}

pub fn current_draw_id() -> u64 {
    CURRENT_DRAW_ID.with(Cell::get)
}

pub fn set_current_draw_reason(reason: &'static str) {
    CURRENT_DRAW_REASON.with(|current| current.set(reason));
}

pub fn clear_current_draw_reason() {
    set_current_draw_reason("unspecified");
}

pub fn current_draw_reason() -> &'static str {
    CURRENT_DRAW_REASON.with(Cell::get)
}

pub fn set_current_present_id(present_id: u64) {
    CURRENT_PRESENT_ID.with(|current| current.set(present_id));
}

pub fn clear_current_present_id() {
    set_current_present_id(0);
}

pub fn current_present_id() -> u64 {
    CURRENT_PRESENT_ID.with(Cell::get)
}

pub fn trace(event: &'static str, detail: std::fmt::Arguments<'_>) {
    if !enabled() {
        return;
    }

    static SEQ: AtomicU64 = AtomicU64::new(0);
    static LAST_US: AtomicU64 = AtomicU64::new(0);

    let seq = SEQ.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    let now_us = start_time()
        .elapsed()
        .as_micros()
        .try_into()
        .unwrap_or(u64::MAX);
    let prev_us = LAST_US.swap(now_us, Ordering::Relaxed);
    let dt_us = if prev_us == 0 {
        0
    } else {
        now_us.saturating_sub(prev_us)
    };
    let wall_us = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0);

    eprintln!(
        "nobie-gpui platform_present seq={seq} t_us={now_us} dt_us={dt_us} event={event} wall_us={wall_us} {detail}"
    );
}
