use gpui::{App, VisibleRootGroupCx};

fn bad(group_cx: &mut VisibleRootGroupCx<'_>, _cx: &mut App) {
    group_cx.frame_prepaint_plan(|_, _| ());
}

fn main() {}
