use gpui::{AnyTooltip, AnyView, Point};
use std::rc::Rc;

fn make_tooltip(view: AnyView) -> AnyTooltip {
    AnyTooltip {
        view,
        mouse_position: Point::default(),
        check_visible_and_update: Rc::new(|_, _, _| true),
    }
}

fn main() {}
