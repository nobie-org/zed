use gpui::{AnyTooltip, AnyView, Point, Style};

fn make_tooltip(view: AnyView) -> AnyTooltip {
    AnyTooltip::new(view, Point::default(), |_, window, cx| {
        window.request_layout(Style::default(), [], cx);
        true
    })
}

fn main() {}
