use gpui::{App, Style, Window};

fn bad(window: &mut Window, cx: &mut App) {
    let _ = window.request_layout(Style::default(), [], cx);
}

fn main() {}
