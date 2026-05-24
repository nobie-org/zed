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
    static CURRENT_REQUEST_FRAME_ID: Cell<u64> = const { Cell::new(0) };
    static CURRENT_DISPLAY_LINK_SIGNAL_ID: Cell<u64> = const { Cell::new(0) };
    static CURRENT_DISPLAY_LINK_COALESCED_COUNT: Cell<u64> = const { Cell::new(0) };
    static CURRENT_DISPLAY_LINK_CALLBACK_WALL_US: Cell<u64> = const { Cell::new(0) };
    // CoreAnimation media times are stored as raw f64 bits so the cell can
    // remain `const`-initializable and stay on the read fast path with no
    // allocation. Convert at the boundary via `f64::from_bits`.
    static CURRENT_DISPLAY_LINK_CALLBACK_CA_TIME_BITS: Cell<u64> = const { Cell::new(0) };
    static CURRENT_DISPLAY_LINK_OUTPUT_CA_TIME_BITS: Cell<u64> = const { Cell::new(0) };
}

// Latest CVDisplayLink callback observation, written on the CV background
// thread and read by the main-thread `step` callback. Atomics are required
// because the writer and reader live on different threads; the dispatch
// source `merge_data` coalescing is what makes this readable as
// "what was the most recent CV fire when this step ran." The difference
// between two consecutive observations of `LATEST_DISPLAY_LINK_SIGNAL_ID`
// is the number of CV fires that coalesced into one main-thread step --
// directly answering "is CVDisplayLink throttled, or is the main queue
// coalescing?".
static LATEST_DISPLAY_LINK_SIGNAL_ID: AtomicU64 = AtomicU64::new(0);
static LATEST_DISPLAY_LINK_CALLBACK_WALL_US: AtomicU64 = AtomicU64::new(0);
static LATEST_DISPLAY_LINK_CALLBACK_CA_TIME_BITS: AtomicU64 = AtomicU64::new(0);
static LATEST_DISPLAY_LINK_OUTPUT_CA_TIME_BITS: AtomicU64 = AtomicU64::new(0);

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

pub fn set_current_request_frame_id(request_frame_id: u64) {
    CURRENT_REQUEST_FRAME_ID.with(|current| current.set(request_frame_id));
}

pub fn clear_current_request_frame_id() {
    set_current_request_frame_id(0);
}

pub fn current_request_frame_id() -> u64 {
    CURRENT_REQUEST_FRAME_ID.with(Cell::get)
}

// Called from the CVDisplayLink background-thread callback. Mints a fresh
// signal id when tracing is enabled and stores the most recent timing
// snapshot in cross-thread atomics. The returned id is the monotonic count
// of CV fires; the main-thread step() reads it via `latest_display_link_*`
// to compute how many fires coalesced into one step.
pub fn record_display_link_callback(
    callback_wall_us: u64,
    callback_ca_time: f64,
    output_ca_time: f64,
) -> u64 {
    let id = next_display_link_signal_id();
    if id == 0 {
        return 0;
    }
    LATEST_DISPLAY_LINK_CALLBACK_WALL_US.store(callback_wall_us, Ordering::Relaxed);
    LATEST_DISPLAY_LINK_CALLBACK_CA_TIME_BITS.store(callback_ca_time.to_bits(), Ordering::Relaxed);
    LATEST_DISPLAY_LINK_OUTPUT_CA_TIME_BITS.store(output_ca_time.to_bits(), Ordering::Relaxed);
    LATEST_DISPLAY_LINK_SIGNAL_ID.store(id, Ordering::Release);
    id
}

pub fn latest_display_link_signal_id() -> u64 {
    LATEST_DISPLAY_LINK_SIGNAL_ID.load(Ordering::Acquire)
}

pub fn latest_display_link_callback_wall_us() -> u64 {
    LATEST_DISPLAY_LINK_CALLBACK_WALL_US.load(Ordering::Relaxed)
}

pub fn latest_display_link_callback_ca_time() -> f64 {
    f64::from_bits(LATEST_DISPLAY_LINK_CALLBACK_CA_TIME_BITS.load(Ordering::Relaxed))
}

pub fn latest_display_link_output_ca_time() -> f64 {
    f64::from_bits(LATEST_DISPLAY_LINK_OUTPUT_CA_TIME_BITS.load(Ordering::Relaxed))
}

pub fn set_current_display_link_signal_id(value: u64) {
    CURRENT_DISPLAY_LINK_SIGNAL_ID.with(|cell| cell.set(value));
}

pub fn set_current_display_link_coalesced_count(value: u64) {
    CURRENT_DISPLAY_LINK_COALESCED_COUNT.with(|cell| cell.set(value));
}

pub fn set_current_display_link_callback_wall_us(value: u64) {
    CURRENT_DISPLAY_LINK_CALLBACK_WALL_US.with(|cell| cell.set(value));
}

pub fn set_current_display_link_callback_ca_time(value: f64) {
    CURRENT_DISPLAY_LINK_CALLBACK_CA_TIME_BITS.with(|cell| cell.set(value.to_bits()));
}

pub fn set_current_display_link_output_ca_time(value: f64) {
    CURRENT_DISPLAY_LINK_OUTPUT_CA_TIME_BITS.with(|cell| cell.set(value.to_bits()));
}

pub fn clear_current_display_link_observation() {
    set_current_display_link_signal_id(0);
    set_current_display_link_coalesced_count(0);
    set_current_display_link_callback_wall_us(0);
    set_current_display_link_callback_ca_time(0.0);
    set_current_display_link_output_ca_time(0.0);
}

pub fn current_display_link_signal_id() -> u64 {
    CURRENT_DISPLAY_LINK_SIGNAL_ID.with(Cell::get)
}

pub fn current_display_link_coalesced_count() -> u64 {
    CURRENT_DISPLAY_LINK_COALESCED_COUNT.with(Cell::get)
}

pub fn current_display_link_callback_wall_us() -> u64 {
    CURRENT_DISPLAY_LINK_CALLBACK_WALL_US.with(Cell::get)
}

pub fn current_display_link_callback_ca_time() -> f64 {
    f64::from_bits(CURRENT_DISPLAY_LINK_CALLBACK_CA_TIME_BITS.with(Cell::get))
}

pub fn current_display_link_output_ca_time() -> f64 {
    f64::from_bits(CURRENT_DISPLAY_LINK_OUTPUT_CA_TIME_BITS.with(Cell::get))
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
