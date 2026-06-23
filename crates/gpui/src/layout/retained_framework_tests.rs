use crate::{
    AnyElement, AnyWindowHandle, AppContext as _, AvailableSpace, Bounds, BuildCx,
    InteractiveElement as _, IntoElement, LayoutWorkSample, ParentElement as _, Pixels, Render,
    Styled as _, TestAppContext, div, px, size,
};
use hegel::generators;
use std::ops::Deref;

fn hegel_settings(test_cases: u64) -> hegel::Settings {
    hegel::Settings::new().test_cases(test_cases)
}

fn draw_u8(tc: &hegel::TestCase, min: u8, max: u8) -> u8 {
    tc.draw(generators::integers::<u8>().min_value(min).max_value(max))
}

#[derive(Clone, Copy, Debug)]
enum GeneratedFlexAxis {
    Row,
    Column,
}

#[derive(Clone, Debug)]
struct GeneratedDivNode {
    width: u8,
    height: u8,
    padding: u8,
    gap: u8,
    axis: GeneratedFlexAxis,
    children: Vec<GeneratedDivNode>,
}

impl GeneratedDivNode {
    fn draw(tc: &hegel::TestCase, depth: u8) -> Self {
        let child_count = if depth == 0 || !tc.draw(generators::booleans()) {
            0
        } else {
            draw_u8(tc, 1, 3)
        };
        let children = (0..child_count)
            .map(|_| Self::draw(tc, depth.saturating_sub(1)))
            .collect();

        Self {
            width: draw_u8(tc, 12, 220),
            height: draw_u8(tc, 12, 180),
            padding: draw_u8(tc, 0, 8),
            gap: draw_u8(tc, 0, 10),
            axis: if tc.draw(generators::booleans()) {
                GeneratedFlexAxis::Row
            } else {
                GeneratedFlexAxis::Column
            },
            children,
        }
    }

    fn selectors(&self) -> Vec<String> {
        let mut selectors = Vec::new();
        self.push_selectors("root".to_string(), &mut selectors);
        selectors
    }

    fn push_selectors(&self, selector: String, selectors: &mut Vec<String>) {
        selectors.push(selector.clone());
        for (index, child) in self.children.iter().enumerate() {
            child.push_selectors(format!("{selector}/{index}"), selectors);
        }
    }

    fn build(&self) -> AnyElement {
        self.build_at("root".to_string(), true)
    }

    fn build_at(&self, selector: String, is_root: bool) -> AnyElement {
        let children = self
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| child.build_at(format!("{selector}/{index}"), false))
            .collect::<Vec<_>>();

        let selector_for_debug = selector.clone();
        let mut element = div()
            .debug_selector(move || selector_for_debug)
            .w(px(self.width as f32))
            .h(px(self.height as f32))
            .p(px(self.padding as f32))
            .gap(px(self.gap as f32));

        if !self.children.is_empty() {
            element = match self.axis {
                GeneratedFlexAxis::Row => element.flex().flex_row(),
                GeneratedFlexAxis::Column => element.flex().flex_col(),
            };
        }

        if is_root {
            element
                .id("generated-framework-retained-root")
                .children(children)
                .into_any_element()
        } else {
            element.children(children).into_any_element()
        }
    }
}

fn draw_generated_div_tree(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    tree: &GeneratedDivNode,
) -> (Vec<(String, Bounds<Pixels>)>, LayoutWorkSample) {
    let selectors = tree.selectors();

    let bounds = selectors
        .into_iter()
        .map(|selector| {
            let bounds = cx
                .update_window(window, |_, window, _| {
                    window.rendered_frame.debug_bounds.get(&selector).copied()
                })
                .unwrap()
                .unwrap_or_else(|| {
                    panic!("missing debug bounds for generated selector {selector}")
                });
            (selector, bounds)
        })
        .collect::<Vec<_>>();

    let sample = cx
        .update_window(window, |_, window, _| window.last_layout_work_sample())
        .unwrap()
        .expect("generated framework draw should publish layout work");
    (bounds, sample)
}

struct GeneratedFrameworkView {
    tree: GeneratedDivNode,
}

impl Render for GeneratedFrameworkView {
    fn render(
        &mut self,
        _window: &mut BuildCx<'_>,
        _cx: &mut crate::Context<Self>,
    ) -> impl IntoElement {
        self.tree.build()
    }
}

fn assert_no_retained_writes_or_measurement(sample: LayoutWorkSample) {
    assert_eq!(
        sample.measured_layout_calls, 0,
        "stable generated div frame should not run measured callbacks"
    );
    assert_eq!(
        [
            sample.retained_layout_creates,
            sample.retained_layout_style_updates,
            sample.retained_layout_child_list_updates,
            sample.retained_layout_dirty_marks,
            sample.retained_layout_measured_context_clears,
            sample.retained_layout_removes,
            sample.retained_layout_miss_no_previous,
            sample.retained_layout_miss_style,
            sample.retained_layout_miss_kind,
            sample.retained_layout_miss_measured_kind,
            sample.retained_layout_miss_child_count,
            sample.retained_layout_miss_child_subtree,
            sample.retained_layout_miss_no_exact_child,
        ],
        [0; 13],
        "stable generated div frame should not mutate retained layout"
    );
}

#[gpui::test]
fn generated_framework_div_tree_matches_fresh_and_stable_repeat_preserves_work(
    cx: &mut TestAppContext,
) {
    hegel::Hegel::new(|tc| {
        let tree = GeneratedDivNode::draw(&tc, 3);
        let available_space = size(
            AvailableSpace::Definite(px(draw_u8(&tc, 80, 240) as f32)),
            AvailableSpace::Definite(px(draw_u8(&tc, 80, 240) as f32)),
        );

        let (first_retained_bounds, second_retained_bounds, stable_sample) = {
            let retained = cx.open_window(
                available_space.map(|space| match space {
                    AvailableSpace::Definite(value) => value,
                    AvailableSpace::MinContent | AvailableSpace::MaxContent => px(240.0),
                }),
                |_, _| GeneratedFrameworkView { tree: tree.clone() },
            );
            cx.run_until_parked();
            let window = *retained.deref();
            let (first_bounds, _) = draw_generated_div_tree(cx, window, &tree);
            cx.update_window(window, |_, window, _| window.refresh())
                .unwrap();
            cx.run_until_parked();
            let (second_bounds, sample) = draw_generated_div_tree(cx, window, &tree);
            (first_bounds, second_bounds, sample)
        };

        let fresh_bounds = {
            let fresh = cx.open_window(
                available_space.map(|space| match space {
                    AvailableSpace::Definite(value) => value,
                    AvailableSpace::MinContent | AvailableSpace::MaxContent => px(240.0),
                }),
                |window, _| {
                    window.force_fresh_layout_for_tests();
                    GeneratedFrameworkView { tree: tree.clone() }
                },
            );
            cx.run_until_parked();
            draw_generated_div_tree(cx, *fresh.deref(), &tree).0
        };

        assert_eq!(
            first_retained_bounds, fresh_bounds,
            "retained framework draw should match a fresh-layout framework draw"
        );
        assert_eq!(
            first_retained_bounds, second_retained_bounds,
            "stable retained framework redraw should publish identical bounds"
        );
        assert_no_retained_writes_or_measurement(stable_sample);
    })
    .settings(hegel_settings(80))
    .run();
}
