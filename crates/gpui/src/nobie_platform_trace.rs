#![allow(missing_docs)]

#[cfg(not(feature = "nobie-platform-trace"))]
mod disabled;
#[cfg(feature = "nobie-platform-trace")]
mod enabled;

#[cfg(not(feature = "nobie-platform-trace"))]
pub use disabled::*;
#[cfg(feature = "nobie-platform-trace")]
pub use enabled::*;

#[cfg(all(test, not(feature = "nobie-platform-trace")))]
mod tests {
    use super::*;

    #[test]
    fn default_trace_hooks_are_inert() {
        assert!(!enabled());
        assert_eq!(next_draw_id(), 0);
        assert_eq!(next_present_id(), 0);
        assert_eq!(next_request_frame_id(), 0);
        assert_eq!(next_display_link_signal_id(), 0);
        assert_eq!(next_main_queue_probe_id(), 0);
        assert_eq!(next_platform_draw_id(), 0);
        assert_eq!(next_metal_draw_id(), 0);
        assert_eq!(current_input_boundary_id(), 0);
        assert_eq!(last_input_boundary_id(), 0);

        let input_boundary = begin_input_boundary();
        assert_eq!(input_boundary.id(), 0);
        set_current_draw_id(7);
        set_current_present_id(8);
        set_current_request_frame_id(9);
        set_current_draw_reason("test");
        set_current_display_link_signal_id(10);
        set_current_display_link_coalesced_count(11);
        set_current_display_link_callback_wall_us(12);
        set_current_display_link_callback_ca_time(13.0);
        set_current_display_link_output_ca_time(14.0);
        set_current_input_boundary_id(15);
        set_last_input_boundary_id(16);

        assert_eq!(current_draw_id(), 0);
        assert_eq!(current_present_id(), 0);
        assert_eq!(current_request_frame_id(), 0);
        assert_eq!(current_draw_reason(), "unspecified");
        assert_eq!(current_input_boundary_id(), 0);
        assert_eq!(last_input_boundary_id(), 0);
        assert_eq!(record_display_link_callback(1, 2.0, 3.0), 0);
        assert_eq!(latest_display_link_signal_id(), 0);
        assert_eq!(latest_display_link_callback_wall_us(), 0);
        assert_eq!(latest_display_link_callback_ca_time(), 0.0);
        assert_eq!(latest_display_link_output_ca_time(), 0.0);
        assert_eq!(current_display_link_signal_id(), 0);
        assert_eq!(current_display_link_coalesced_count(), 0);
        assert_eq!(current_display_link_callback_wall_us(), 0);
        assert_eq!(current_display_link_callback_ca_time(), 0.0);
        assert_eq!(current_display_link_output_ca_time(), 0.0);

        clear_current_draw_id();
        clear_current_present_id();
        clear_current_request_frame_id();
        clear_current_draw_reason();
        clear_current_display_link_observation();
        clear_current_input_boundary_id();
        clear_last_input_boundary_id();
        trace("test", format_args!("detail"));
    }
}

#[cfg(all(test, feature = "nobie-platform-trace"))]
mod tests {
    use super::*;

    #[test]
    fn enabled_trace_hooks_preserve_thread_local_observations() {
        set_current_draw_id(7);
        set_current_present_id(8);
        set_current_request_frame_id(9);
        set_current_draw_reason("test");
        set_current_display_link_signal_id(10);
        set_current_display_link_coalesced_count(11);
        set_current_display_link_callback_wall_us(12);
        set_current_display_link_callback_ca_time(13.0);
        set_current_display_link_output_ca_time(14.0);
        let input_boundary = begin_input_boundary();
        let input_boundary_id = input_boundary.id();

        assert!(input_boundary_id > 0);
        assert_eq!(current_input_boundary_id(), input_boundary_id);
        assert_eq!(last_input_boundary_id(), input_boundary_id);

        assert_eq!(current_draw_id(), 7);
        assert_eq!(current_present_id(), 8);
        assert_eq!(current_request_frame_id(), 9);
        assert_eq!(current_draw_reason(), "test");
        assert_eq!(current_display_link_signal_id(), 10);
        assert_eq!(current_display_link_coalesced_count(), 11);
        assert_eq!(current_display_link_callback_wall_us(), 12);
        assert_eq!(current_display_link_callback_ca_time(), 13.0);
        assert_eq!(current_display_link_output_ca_time(), 14.0);
        drop(input_boundary);

        clear_current_draw_id();
        clear_current_present_id();
        clear_current_request_frame_id();
        clear_current_draw_reason();
        clear_current_display_link_observation();

        assert_eq!(current_draw_id(), 0);
        assert_eq!(current_present_id(), 0);
        assert_eq!(current_request_frame_id(), 0);
        assert_eq!(current_draw_reason(), "unspecified");
        assert_eq!(current_input_boundary_id(), 0);
        assert_eq!(last_input_boundary_id(), input_boundary_id);
        assert_eq!(current_display_link_signal_id(), 0);
        assert_eq!(current_display_link_coalesced_count(), 0);
        assert_eq!(current_display_link_callback_wall_us(), 0);
        assert_eq!(current_display_link_callback_ca_time(), 0.0);
        assert_eq!(current_display_link_output_ca_time(), 0.0);
    }
}
