use gpui::{App, AvailableSpace, IntoElement, VisibleRootPlacementCx, div};

fn bad(placement_cx: &mut VisibleRootPlacementCx<'_>, cx: &mut App) {
    placement_cx.measure_scratch_root(div().into_any_element(), AvailableSpace::min_size(), cx);
}

fn main() {}
