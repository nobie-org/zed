use gpui::{App, AvailableSpace, IntoElement, VisibleRootGroupCx, div, point, px};

fn bad(group_cx: &mut VisibleRootGroupCx<'_>, cx: &mut App) {
    group_cx.owner_painted_visible_root(
        div().into_any_element(),
        AvailableSpace::min_size(),
        |_| point(px(0.), px(0.)),
        cx,
    );
}

fn main() {}
