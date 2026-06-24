use gpui::{BuildCx, Context, IntoElement, Render, div};

struct BadView;

impl Render for BadView {
    fn render(&mut self, window: &mut BuildCx<'_>, cx: &mut Context<Self>) -> impl IntoElement {
        window.measure_scratch_root(
            div().into_any_element(),
            gpui::AvailableSpace::min_size(),
            cx,
        );
        div()
    }
}

fn main() {}
