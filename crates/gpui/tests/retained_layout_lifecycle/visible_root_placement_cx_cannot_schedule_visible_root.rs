use gpui::{App, AvailableSpace, IntoElement, VisibleRootPlacementCx, div, point, px};

fn bad(placement_cx: &mut VisibleRootPlacementCx<'_>, cx: &mut App) {
    placement_cx.owner_painted_visible_root(
        div().into_any_element(),
        AvailableSpace::min_size(),
        |_| point(px(0.), px(0.)),
        cx,
    );
}

fn main() {}
