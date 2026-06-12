pub fn enabled() -> bool {
    false
}

#[must_use]
pub struct InputBoundaryTraceGuard;

impl InputBoundaryTraceGuard {
    pub fn id(&self) -> u64 {
        0
    }
}

pub fn begin_input_boundary() -> InputBoundaryTraceGuard {
    InputBoundaryTraceGuard
}

pub fn set_current_input_boundary_id(_input_boundary_id: u64) {}

pub fn clear_current_input_boundary_id() {}

pub fn current_input_boundary_id() -> u64 {
    0
}

pub fn set_last_input_boundary_id(_input_boundary_id: u64) {}

pub fn clear_last_input_boundary_id() {}

pub fn last_input_boundary_id() -> u64 {
    0
}

pub fn next_draw_id() -> u64 {
    0
}

pub fn next_present_id() -> u64 {
    0
}

pub fn next_request_frame_id() -> u64 {
    0
}

pub fn next_display_link_signal_id() -> u64 {
    0
}

pub fn next_main_queue_probe_id() -> u64 {
    0
}

pub fn next_platform_draw_id() -> u64 {
    0
}

pub fn next_metal_draw_id() -> u64 {
    0
}

pub fn set_current_draw_id(_draw_id: u64) {}

pub fn clear_current_draw_id() {}

pub fn current_draw_id() -> u64 {
    0
}

pub fn set_current_draw_reason(_reason: &'static str) {}

pub fn clear_current_draw_reason() {}

pub fn current_draw_reason() -> &'static str {
    "unspecified"
}

pub fn set_current_present_id(_present_id: u64) {}

pub fn clear_current_present_id() {}

pub fn current_present_id() -> u64 {
    0
}

pub fn set_current_request_frame_id(_request_frame_id: u64) {}

pub fn clear_current_request_frame_id() {}

pub fn current_request_frame_id() -> u64 {
    0
}

pub fn record_display_link_callback(
    _callback_wall_us: u64,
    _callback_ca_time: f64,
    _output_ca_time: f64,
) -> u64 {
    0
}

pub fn latest_display_link_signal_id() -> u64 {
    0
}

pub fn latest_display_link_callback_wall_us() -> u64 {
    0
}

pub fn latest_display_link_callback_ca_time() -> f64 {
    0.0
}

pub fn latest_display_link_output_ca_time() -> f64 {
    0.0
}

pub fn set_current_display_link_signal_id(_value: u64) {}

pub fn set_current_display_link_coalesced_count(_value: u64) {}

pub fn set_current_display_link_callback_wall_us(_value: u64) {}

pub fn set_current_display_link_callback_ca_time(_value: f64) {}

pub fn set_current_display_link_output_ca_time(_value: f64) {}

pub fn clear_current_display_link_observation() {}

pub fn current_display_link_signal_id() -> u64 {
    0
}

pub fn current_display_link_coalesced_count() -> u64 {
    0
}

pub fn current_display_link_callback_wall_us() -> u64 {
    0
}

pub fn current_display_link_callback_ca_time() -> f64 {
    0.0
}

pub fn current_display_link_output_ca_time() -> f64 {
    0.0
}

pub fn trace(_event: &'static str, _detail: std::fmt::Arguments<'_>) {}
