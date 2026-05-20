#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    fn CACurrentMediaTime() -> f64;
}

pub(crate) fn ca_current_media_time() -> f64 {
    unsafe { CACurrentMediaTime() }
}
