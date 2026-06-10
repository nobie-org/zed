use anyhow::{Result, anyhow};
use core_foundation::{base::TCFType, string::CFString};
use core_foundation_sys::{
    base::{CFRelease, CFTypeRef},
    runloop::{
        CFRunLoopAddSource, CFRunLoopGetMain, CFRunLoopRef, CFRunLoopSourceContext,
        CFRunLoopSourceCreate, CFRunLoopSourceInvalidate, CFRunLoopSourceRef,
        CFRunLoopSourceSignal, CFRunLoopWakeUp, kCFRunLoopCommonModes,
    },
};
use core_graphics::display::CGDirectDisplayID;
use gpui::nobie_platform_trace;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{ffi::c_void, ptr};
use util::ResultExt;

use crate::quartzcore_time::ca_current_media_time;

pub struct DisplayLink {
    display_link: Option<sys::DisplayLink>,
    _frame_requests: Box<FrameRequestSource>,
}

struct FrameRequestSource {
    data: *mut c_void,
    callback: extern "C" fn(*mut c_void),
    main_run_loop: CFRunLoopRef,
    source: CFRunLoopSourceRef,
}

impl FrameRequestSource {
    fn new(data: *mut c_void, callback: extern "C" fn(*mut c_void)) -> Result<Box<Self>> {
        let mut frame_requests = Box::new(Self {
            data,
            callback,
            main_run_loop: unsafe { CFRunLoopGetMain() },
            source: ptr::null_mut(),
        });
        let mut context = CFRunLoopSourceContext {
            version: 0,
            info: &mut *frame_requests as *mut FrameRequestSource as *mut c_void,
            retain: None,
            release: None,
            copyDescription: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: frame_request_source_perform,
        };
        let source = unsafe { CFRunLoopSourceCreate(ptr::null(), 0, &mut context) };
        if source.is_null() {
            return Err(anyhow!("failed to create display link run-loop source"));
        }
        unsafe {
            CFRunLoopAddSource(frame_requests.main_run_loop, source, kCFRunLoopCommonModes);
            let event_tracking_mode = CFString::from_static_string("NSEventTrackingRunLoopMode");
            CFRunLoopAddSource(
                frame_requests.main_run_loop,
                source,
                event_tracking_mode.as_concrete_TypeRef(),
            );
        }
        frame_requests.source = source;
        Ok(frame_requests)
    }

    fn signal(&self) {
        unsafe {
            CFRunLoopSourceSignal(self.source);
            CFRunLoopWakeUp(self.main_run_loop);
        }
    }
}

impl Drop for FrameRequestSource {
    fn drop(&mut self) {
        unsafe {
            if !self.source.is_null() {
                CFRunLoopSourceInvalidate(self.source);
                CFRelease(self.source as CFTypeRef);
            }
        }
    }
}

extern "C" fn frame_request_source_perform(info: *const c_void) {
    unsafe {
        let frame_requests = &*(info as *const FrameRequestSource);
        (frame_requests.callback)(frame_requests.data);
    }
}

impl DisplayLink {
    pub fn new(
        display_id: CGDirectDisplayID,
        data: *mut c_void,
        callback: extern "C" fn(*mut c_void),
    ) -> Result<DisplayLink> {
        unsafe extern "C" fn display_link_callback(
            _display_link_out: *mut sys::CVDisplayLink,
            _current_time: *const sys::CVTimeStamp,
            _output_time: *const sys::CVTimeStamp,
            _flags_in: i64,
            _flags_out: *mut i64,
            frame_requests: *mut c_void,
        ) -> i32 {
            // Record this CV-thread fire BEFORE merging to the main queue.
            // The signal id minted here is the monotonic count of CV
            // callbacks; the main-thread `step` reads it via
            // `latest_display_link_signal_id` to compute how many fires
            // coalesced into one step. Without this record_* call the rate
            // of CV fires is invisible to the probe and the probe cannot
            // distinguish OS-level display-link throttling from main-queue
            // coalescing.
            //
            // Gate the timing capture on `nobie_platform_trace::enabled()`
            // so the SystemTime::now() syscall and CACurrentMediaTime FFI
            // call don't fire on every CV tick (~120/s) in default builds.
            // This matches the existing pattern in `metal_renderer.rs` and
            // makes the PR's "behavior unchanged when
            // NOBIE_GPUI_TRACE_PLATFORM_PRESENT is unset" claim true.
            // `record_display_link_callback` also early-returns when
            // disabled; the outer gate just spares the timing work.
            if nobie_platform_trace::enabled() {
                let callback_wall_us = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_micros().try_into().unwrap_or(u64::MAX))
                    .unwrap_or(0);
                let callback_ca_time = ca_current_media_time();
                // output_time predicts when the next vsync output will be
                // visible. host_time is mach absolute units; converting to
                // CA media time would require multiplying by the mach
                // timebase ratio. The signal id and callback timing are
                // enough for the OS-vs-coalescing question, so leave the
                // output CA time at zero until we add a mach_timebase
                // helper.
                let _ = _output_time;
                nobie_platform_trace::record_display_link_callback(
                    callback_wall_us,
                    callback_ca_time,
                    0.0,
                );
            }
            unsafe {
                let frame_requests = &*(frame_requests as *const FrameRequestSource);
                frame_requests.signal();
                0
            }
        }

        unsafe {
            let frame_requests = FrameRequestSource::new(data, callback)?;

            let display_link = sys::DisplayLink::new(
                display_id,
                display_link_callback,
                &*frame_requests as *const FrameRequestSource as *mut c_void,
            )?;

            Ok(Self {
                display_link: Some(display_link),
                _frame_requests: frame_requests,
            })
        }
    }

    pub fn start(&mut self) -> Result<()> {
        unsafe {
            self.display_link.as_mut().unwrap().start()?;
        }
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        unsafe {
            self.display_link.as_mut().unwrap().stop()?;
        }
        Ok(())
    }
}

impl Drop for DisplayLink {
    fn drop(&mut self) {
        self.stop().log_err();
        // We see occasional segfaults on the CVDisplayLink thread.
        //
        // It seems possible that this happens because CVDisplayLinkRelease releases the CVDisplayLink
        // on the main thread immediately, but the background thread that CVDisplayLink uses for timers
        // is still accessing it.
        //
        // We might also want to upgrade to CADisplayLink, but that requires dropping old macOS support.
        std::mem::forget(self.display_link.take());
    }
}

mod sys {
    //! Derived from display-link crate under the following license:
    //! <https://github.com/BrainiumLLC/display-link/blob/master/LICENSE-MIT>
    //! Apple docs: [CVDisplayLink](https://developer.apple.com/documentation/corevideo/cvdisplaylinkoutputcallback?language=objc)
    #![allow(dead_code, non_upper_case_globals)]

    use anyhow::Result;
    use core_graphics::display::CGDirectDisplayID;
    use foreign_types::{ForeignType, foreign_type};
    use std::{
        ffi::c_void,
        fmt::{self, Debug, Formatter},
    };

    #[derive(Debug)]
    pub enum CVDisplayLink {}

    foreign_type! {
        pub unsafe type DisplayLink {
            type CType = CVDisplayLink;
            fn drop = CVDisplayLinkRelease;
            fn clone = CVDisplayLinkRetain;
        }
    }

    impl Debug for DisplayLink {
        fn fmt(&self, formatter: &mut Formatter) -> fmt::Result {
            formatter
                .debug_tuple("DisplayLink")
                .field(&self.as_ptr())
                .finish()
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub(crate) struct CVTimeStamp {
        pub version: u32,
        pub video_time_scale: i32,
        pub video_time: i64,
        pub host_time: u64,
        pub rate_scalar: f64,
        pub video_refresh_period: i64,
        pub smpte_time: CVSMPTETime,
        pub flags: u64,
        pub reserved: u64,
    }

    pub type CVTimeStampFlags = u64;

    pub const kCVTimeStampVideoTimeValid: CVTimeStampFlags = 1 << 0;
    pub const kCVTimeStampHostTimeValid: CVTimeStampFlags = 1 << 1;
    pub const kCVTimeStampSMPTETimeValid: CVTimeStampFlags = 1 << 2;
    pub const kCVTimeStampVideoRefreshPeriodValid: CVTimeStampFlags = 1 << 3;
    pub const kCVTimeStampRateScalarValid: CVTimeStampFlags = 1 << 4;
    pub const kCVTimeStampTopField: CVTimeStampFlags = 1 << 16;
    pub const kCVTimeStampBottomField: CVTimeStampFlags = 1 << 17;
    pub const kCVTimeStampVideoHostTimeValid: CVTimeStampFlags =
        kCVTimeStampVideoTimeValid | kCVTimeStampHostTimeValid;
    pub const kCVTimeStampIsInterlaced: CVTimeStampFlags =
        kCVTimeStampTopField | kCVTimeStampBottomField;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub(crate) struct CVSMPTETime {
        pub subframes: i16,
        pub subframe_divisor: i16,
        pub counter: u32,
        pub time_type: u32,
        pub flags: u32,
        pub hours: i16,
        pub minutes: i16,
        pub seconds: i16,
        pub frames: i16,
    }

    pub type CVSMPTETimeType = u32;

    pub const kCVSMPTETimeType24: CVSMPTETimeType = 0;
    pub const kCVSMPTETimeType25: CVSMPTETimeType = 1;
    pub const kCVSMPTETimeType30Drop: CVSMPTETimeType = 2;
    pub const kCVSMPTETimeType30: CVSMPTETimeType = 3;
    pub const kCVSMPTETimeType2997: CVSMPTETimeType = 4;
    pub const kCVSMPTETimeType2997Drop: CVSMPTETimeType = 5;
    pub const kCVSMPTETimeType60: CVSMPTETimeType = 6;
    pub const kCVSMPTETimeType5994: CVSMPTETimeType = 7;

    pub type CVSMPTETimeFlags = u32;

    pub const kCVSMPTETimeValid: CVSMPTETimeFlags = 1 << 0;
    pub const kCVSMPTETimeRunning: CVSMPTETimeFlags = 1 << 1;

    pub type CVDisplayLinkOutputCallback = unsafe extern "C" fn(
        display_link_out: *mut CVDisplayLink,
        // A pointer to the current timestamp. This represents the timestamp when the callback is called.
        current_time: *const CVTimeStamp,
        // A pointer to the output timestamp. This represents the timestamp for when the frame will be displayed.
        output_time: *const CVTimeStamp,
        // Unused
        flags_in: i64,
        // Unused
        flags_out: *mut i64,
        // A pointer to app-defined data.
        display_link_context: *mut c_void,
    ) -> i32;

    #[link(name = "CoreFoundation", kind = "framework")]
    #[link(name = "CoreVideo", kind = "framework")]
    #[allow(improper_ctypes, unknown_lints, clippy::duplicated_attributes)]
    unsafe extern "C" {
        pub fn CVDisplayLinkCreateWithActiveCGDisplays(
            display_link_out: *mut *mut CVDisplayLink,
        ) -> i32;
        pub fn CVDisplayLinkSetCurrentCGDisplay(
            display_link: &mut DisplayLinkRef,
            display_id: u32,
        ) -> i32;
        pub fn CVDisplayLinkSetOutputCallback(
            display_link: &mut DisplayLinkRef,
            callback: CVDisplayLinkOutputCallback,
            user_info: *mut c_void,
        ) -> i32;
        pub fn CVDisplayLinkStart(display_link: &mut DisplayLinkRef) -> i32;
        pub fn CVDisplayLinkStop(display_link: &mut DisplayLinkRef) -> i32;
        pub fn CVDisplayLinkRelease(display_link: *mut CVDisplayLink);
        pub fn CVDisplayLinkRetain(display_link: *mut CVDisplayLink) -> *mut CVDisplayLink;
    }

    impl DisplayLink {
        /// Apple docs: [CVDisplayLinkCreateWithCGDisplay](https://developer.apple.com/documentation/corevideo/1456981-cvdisplaylinkcreatewithcgdisplay?language=objc)
        pub unsafe fn new(
            display_id: CGDirectDisplayID,
            callback: CVDisplayLinkOutputCallback,
            user_info: *mut c_void,
        ) -> Result<Self> {
            unsafe {
                let mut display_link: *mut CVDisplayLink = 0 as _;

                let code = CVDisplayLinkCreateWithActiveCGDisplays(&mut display_link);
                anyhow::ensure!(code == 0, "could not create display link, code: {}", code);

                let mut display_link = DisplayLink::from_ptr(display_link);

                let code = CVDisplayLinkSetOutputCallback(&mut display_link, callback, user_info);
                anyhow::ensure!(code == 0, "could not set output callback, code: {}", code);

                let code = CVDisplayLinkSetCurrentCGDisplay(&mut display_link, display_id);
                anyhow::ensure!(
                    code == 0,
                    "could not assign display to display link, code: {}",
                    code
                );

                Ok(display_link)
            }
        }
    }

    impl DisplayLinkRef {
        /// Apple docs: [CVDisplayLinkStart](https://developer.apple.com/documentation/corevideo/1457193-cvdisplaylinkstart?language=objc)
        pub unsafe fn start(&mut self) -> Result<()> {
            unsafe {
                let code = CVDisplayLinkStart(self);
                anyhow::ensure!(code == 0, "could not start display link, code: {}", code);
                Ok(())
            }
        }

        /// Apple docs: [CVDisplayLinkStop](https://developer.apple.com/documentation/corevideo/1457281-cvdisplaylinkstop?language=objc)
        pub unsafe fn stop(&mut self) -> Result<()> {
            unsafe {
                let code = CVDisplayLinkStop(self);
                anyhow::ensure!(code == 0, "could not stop display link, code: {}", code);
                Ok(())
            }
        }
    }
}
