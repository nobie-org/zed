use gpui::{App, AvailableSpace, IntoElement, VisibleRootGroupCx, div};

fn bad(group_cx: &mut VisibleRootGroupCx<'_>, cx: &mut App) {
    group_cx.measure_scratch_root(div().into_any_element(), AvailableSpace::min_size(), cx);
}

fn main() {}
