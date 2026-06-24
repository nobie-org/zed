use gpui::{LayoutId, Window};

fn bad(window: &Window, layout_id: LayoutId) {
    let _ = window.layout_bounds(layout_id);
}

fn main() {}
