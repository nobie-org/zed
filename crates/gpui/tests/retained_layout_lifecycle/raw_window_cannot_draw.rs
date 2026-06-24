use gpui::{App, Window};

fn bad(window: &mut Window, cx: &mut App) {
    let _ = window.draw(cx);
    let _ = window.draw_for_app(cx);
}

fn main() {}
