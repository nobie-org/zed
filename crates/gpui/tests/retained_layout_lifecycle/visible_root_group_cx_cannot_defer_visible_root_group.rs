use gpui::{App, VisibleRootGroupCx};

fn bad(group_cx: &mut VisibleRootGroupCx<'_>, cx: &mut App) {
    group_cx.defer_visible_root_group(Vec::new(), |_, _, _| todo!(), cx);
}

fn main() {}
