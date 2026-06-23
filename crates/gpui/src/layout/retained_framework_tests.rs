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
        self.selectors_at("root")
    }

    fn selectors_at(&self, root: &str) -> Vec<String> {
        let mut selectors = Vec::new();
        self.push_selectors(root.to_string(), &mut selectors);
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

#[derive(Clone, Debug)]
enum GeneratedTextNode {
    Container {
        width: u8,
        padding: u8,
        gap: u8,
        axis: GeneratedFlexAxis,
        children: Vec<GeneratedTextNode>,
    },
    TextBox {
        width: u8,
        padding: u8,
        text_size: u8,
        text: String,
    },
}

impl GeneratedTextNode {
    fn draw(tc: &hegel::TestCase, depth: u8) -> Self {
        if depth == 0 || tc.draw(generators::booleans()) {
            return Self::TextBox {
                width: draw_u8(tc, 24, 180),
                padding: draw_u8(tc, 0, 6),
                text_size: draw_u8(tc, 10, 20),
                text: draw_ascii_words(tc),
            };
        }

        let child_count = draw_u8(tc, 1, 4);
        let children = (0..child_count)
            .map(|_| Self::draw(tc, depth.saturating_sub(1)))
            .collect();

        Self::Container {
            width: draw_u8(tc, 40, 220),
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
        self.selectors_at("root")
    }

    fn selectors_at(&self, root: &str) -> Vec<String> {
        let mut selectors = Vec::new();
        self.push_selectors(root.to_string(), &mut selectors);
        selectors
    }

    fn push_selectors(&self, selector: String, selectors: &mut Vec<String>) {
        selectors.push(selector.clone());
        if let Self::Container { children, .. } = self {
            for (index, child) in children.iter().enumerate() {
                child.push_selectors(format!("{selector}/{index}"), selectors);
            }
        }
    }

    fn build(&self) -> AnyElement {
        self.build_at("root", true)
    }

    fn build_at(&self, selector: &str, is_root: bool) -> AnyElement {
        let selector_for_debug = selector.to_string();
        match self {
            Self::Container {
                width,
                padding,
                gap,
                axis,
                children,
            } => {
                let children = children
                    .iter()
                    .enumerate()
                    .map(|(index, child)| child.build_at(&format!("{selector}/{index}"), false))
                    .collect::<Vec<_>>();
                let mut element = div()
                    .debug_selector(move || selector_for_debug)
                    .w(px(*width as f32))
                    .p(px(*padding as f32))
                    .gap(px(*gap as f32));

                if !children.is_empty() {
                    element = match axis {
                        GeneratedFlexAxis::Row => element.flex().flex_row(),
                        GeneratedFlexAxis::Column => element.flex().flex_col(),
                    };
                }

                if is_root {
                    element
                        .id("generated-framework-text-retained-root")
                        .children(children)
                        .into_any_element()
                } else {
                    element.children(children).into_any_element()
                }
            }
            Self::TextBox {
                width,
                padding,
                text_size,
                text,
            } => {
                let element = div()
                    .debug_selector(move || selector_for_debug)
                    .w(px(*width as f32))
                    .p(px(*padding as f32))
                    .text_size(px(*text_size as f32))
                    .child(text.clone());

                if is_root {
                    element
                        .id("generated-framework-text-retained-root")
                        .into_any_element()
                } else {
                    element.into_any_element()
                }
            }
        }
    }
}

fn draw_ascii_words(tc: &hegel::TestCase) -> String {
    let word_count = draw_u8(tc, 1, 10);
    (0..word_count)
        .map(|_| {
            let len = draw_u8(tc, 1, 8);
            (0..len)
                .map(|_| (b'a' + draw_u8(tc, 0, 25)) as char)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ")
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

fn draw_generated_text_tree(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    tree: &GeneratedTextNode,
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
                    panic!("missing debug bounds for generated text selector {selector}")
                });
            (selector, bounds)
        })
        .collect::<Vec<_>>();

    let sample = cx
        .update_window(window, |_, window, _| window.last_layout_work_sample())
        .unwrap()
        .expect("generated framework text draw should publish layout work");
    (bounds, sample)
}

fn draw_dynamic_sibling_text_tree(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    stable_tree: &GeneratedTextNode,
) -> (
    Vec<(String, Bounds<Pixels>)>,
    LayoutWorkSample,
    Vec<crate::RetainedSubtreeWorkSample>,
) {
    let mut selectors = vec![
        "root".to_string(),
        "dynamic".to_string(),
        "stable-panel".to_string(),
    ];
    selectors.extend(stable_tree.selectors_at("stable/root"));

    let bounds = selectors
        .into_iter()
        .map(|selector| {
            let bounds = cx
                .update_window(window, |_, window, _| {
                    window.rendered_frame.debug_bounds.get(&selector).copied()
                })
                .unwrap()
                .unwrap_or_else(|| {
                    panic!("missing debug bounds for dynamic sibling selector {selector}")
                });
            (selector, bounds)
        })
        .collect::<Vec<_>>();

    let (sample, subtree_samples) = cx
        .update_window(window, |_, window, _| {
            (
                window.last_layout_work_sample(),
                window
                    .last_retained_subtree_work_samples_for_tests()
                    .to_vec(),
            )
        })
        .unwrap();
    (
        bounds,
        sample.expect("dynamic sibling draw should publish layout work"),
        subtree_samples,
    )
}

fn draw_fresh_dynamic_sibling_text_tree(
    cx: &mut TestAppContext,
    stable_tree: &GeneratedTextNode,
    state: GeneratedDynamicSiblingFrameState,
) -> Vec<(String, Bounds<Pixels>)> {
    let fresh = cx.open_window(state.window_size(), |window, _| {
        window.force_fresh_layout_for_tests();
        GeneratedDynamicSiblingTextView {
            stable_tree: stable_tree.clone(),
            tick: state.tick,
            dynamic_width: state.dynamic_width,
        }
    });
    cx.run_until_parked();
    draw_dynamic_sibling_text_tree(cx, *fresh.deref(), stable_tree).0
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

struct GeneratedFrameworkTextView {
    tree: GeneratedTextNode,
}

impl Render for GeneratedFrameworkTextView {
    fn render(
        &mut self,
        _window: &mut BuildCx<'_>,
        _cx: &mut crate::Context<Self>,
    ) -> impl IntoElement {
        self.tree.build()
    }
}

struct GeneratedDynamicSiblingTextView {
    stable_tree: GeneratedTextNode,
    tick: u8,
    dynamic_width: u8,
}

impl Render for GeneratedDynamicSiblingTextView {
    fn render(
        &mut self,
        _window: &mut BuildCx<'_>,
        _cx: &mut crate::Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("generated-dynamic-sibling-root")
            .debug_selector(|| "root".into())
            .flex()
            .flex_row()
            .gap(px(8.0))
            .w_full()
            .child(
                div()
                    .id("dynamic-label")
                    .debug_selector(|| "dynamic".into())
                    .w(px(self.dynamic_width as f32))
                    .text_size(px(14.0))
                    .child(format!("frame {}", self.tick)),
            )
            .child(
                div()
                    .id("generated-stable-panel")
                    .debug_selector(|| "stable-panel".into())
                    .child(self.stable_tree.build_at("stable/root", false)),
            )
    }
}

#[derive(Clone, Debug)]
struct GeneratedChurnSibling {
    key: u8,
    width: u8,
    height: u8,
    text_size: u8,
}

impl GeneratedChurnSibling {
    fn draw(tc: &hegel::TestCase, key: u8) -> Self {
        Self {
            key,
            width: draw_u8(tc, 80, 180),
            height: draw_u8(tc, 16, 36),
            text_size: draw_u8(tc, 10, 16),
        }
    }

    fn selector(&self, side: &'static str) -> String {
        format!("{side}/{}", self.key)
    }

    fn build(&self, side: &'static str, tick: u8) -> AnyElement {
        let selector = self.selector(side);
        let key = self.key as usize;
        div()
            .id((side, key))
            .debug_selector(move || selector)
            .flex_none()
            .w(px(self.width as f32))
            .h(px(self.height as f32))
            .text_size(px(self.text_size as f32))
            .child(format!("{side} {} tick {}", self.key, tick))
            .into_any_element()
    }
}

#[derive(Clone, Debug)]
struct GeneratedSiblingChurnFrame {
    tick: u8,
    before: Vec<GeneratedChurnSibling>,
    after: Vec<GeneratedChurnSibling>,
}

impl GeneratedSiblingChurnFrame {
    fn draw(tc: &hegel::TestCase, tick: u8) -> Self {
        Self {
            tick,
            before: Self::draw_siblings(tc),
            after: Self::draw_siblings(tc),
        }
    }

    fn draw_siblings(tc: &hegel::TestCase) -> Vec<GeneratedChurnSibling> {
        let count = draw_u8(tc, 0, 4);
        let mut keys = (0..count).collect::<Vec<_>>();
        for index in 0..keys.len() {
            let swap_with = draw_u8(tc, 0, count.saturating_sub(1)) as usize;
            keys.swap(index, swap_with);
        }
        keys.into_iter()
            .map(|key| GeneratedChurnSibling::draw(tc, key))
            .collect()
    }

    fn window_size(&self) -> crate::Size<Pixels> {
        size(px(420.0), px(960.0))
    }

    fn selectors(&self, stable_tree: &GeneratedTextNode) -> Vec<String> {
        let mut selectors = vec!["root".to_string(), "stable-panel".to_string()];
        selectors.extend(self.before.iter().map(|sibling| sibling.selector("before")));
        selectors.extend(self.after.iter().map(|sibling| sibling.selector("after")));
        selectors.extend(stable_tree.selectors_at("stable/root"));
        selectors
    }
}

struct GeneratedSiblingChurnTextView {
    stable_tree: GeneratedTextNode,
    frame: GeneratedSiblingChurnFrame,
}

impl Render for GeneratedSiblingChurnTextView {
    fn render(
        &mut self,
        _window: &mut BuildCx<'_>,
        _cx: &mut crate::Context<Self>,
    ) -> impl IntoElement {
        let before = self
            .frame
            .before
            .iter()
            .map(|sibling| sibling.build("before", self.frame.tick))
            .collect::<Vec<_>>();
        let after = self
            .frame
            .after
            .iter()
            .map(|sibling| sibling.build("after", self.frame.tick))
            .collect::<Vec<_>>();

        div()
            .id("generated-sibling-churn-root")
            .debug_selector(|| "root".into())
            .flex()
            .flex_col()
            .gap(px(4.0))
            .w(px(360.0))
            .h(px(920.0))
            .children(before)
            .child(
                div()
                    .id("generated-sibling-churn-stable-panel")
                    .debug_selector(|| "stable-panel".into())
                    .flex_none()
                    .w(px(300.0))
                    .h(px(360.0))
                    .child(self.stable_tree.build_at("stable/root", false)),
            )
            .children(after)
    }
}

fn draw_sibling_churn_text_tree(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    stable_tree: &GeneratedTextNode,
    frame: &GeneratedSiblingChurnFrame,
) -> (
    Vec<(String, Bounds<Pixels>)>,
    LayoutWorkSample,
    Vec<crate::RetainedSubtreeWorkSample>,
) {
    let bounds = frame
        .selectors(stable_tree)
        .into_iter()
        .map(|selector| {
            let bounds = cx
                .update_window(window, |_, window, _| {
                    window.rendered_frame.debug_bounds.get(&selector).copied()
                })
                .unwrap()
                .unwrap_or_else(|| {
                    panic!("missing debug bounds for sibling churn selector {selector}")
                });
            (selector, bounds)
        })
        .collect::<Vec<_>>();

    let (sample, subtree_samples) = cx
        .update_window(window, |_, window, _| {
            (
                window.last_layout_work_sample(),
                window
                    .last_retained_subtree_work_samples_for_tests()
                    .to_vec(),
            )
        })
        .unwrap();

    (
        bounds,
        sample.expect("sibling churn draw should publish layout work"),
        subtree_samples,
    )
}

fn draw_fresh_sibling_churn_text_tree(
    cx: &mut TestAppContext,
    stable_tree: &GeneratedTextNode,
    frame: &GeneratedSiblingChurnFrame,
) -> Vec<(String, Bounds<Pixels>)> {
    let fresh = cx.open_window(frame.window_size(), |window, _| {
        window.force_fresh_layout_for_tests();
        GeneratedSiblingChurnTextView {
            stable_tree: stable_tree.clone(),
            frame: frame.clone(),
        }
    });
    cx.run_until_parked();
    draw_sibling_churn_text_tree(cx, *fresh.deref(), stable_tree, frame).0
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GeneratedDynamicSiblingFrameState {
    window_width: u16,
    window_height: u16,
    tick: u8,
    dynamic_width: u8,
}

impl GeneratedDynamicSiblingFrameState {
    fn draw(tc: &hegel::TestCase) -> Self {
        Self {
            window_width: draw_u16(tc, 360, 520),
            window_height: draw_u16(tc, 220, 360),
            tick: draw_u8(tc, 0, 120),
            dynamic_width: draw_u8(tc, 64, 128),
        }
    }

    fn window_size(self) -> crate::Size<Pixels> {
        size(px(self.window_width as f32), px(self.window_height as f32))
    }

    fn apply(self, change: GeneratedDynamicSiblingFrameChange) -> Self {
        match change {
            GeneratedDynamicSiblingFrameChange::Tick { delta } => Self {
                tick: self.tick.wrapping_add(delta),
                ..self
            },
            GeneratedDynamicSiblingFrameChange::DynamicWidth { width } => Self {
                dynamic_width: width,
                ..self
            },
            GeneratedDynamicSiblingFrameChange::Resize { width, height } => Self {
                window_width: width,
                window_height: height,
                ..self
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeneratedDynamicSiblingFrameChange {
    Tick { delta: u8 },
    DynamicWidth { width: u8 },
    Resize { width: u16, height: u16 },
}

impl GeneratedDynamicSiblingFrameChange {
    fn draw(tc: &hegel::TestCase) -> Self {
        match draw_u8(tc, 0, 2) {
            0 => Self::Tick {
                delta: draw_u8(tc, 1, 16),
            },
            1 => Self::DynamicWidth {
                width: draw_u8(tc, 64, 128),
            },
            _ => Self::Resize {
                width: draw_u16(tc, 360, 520),
                height: draw_u16(tc, 220, 360),
            },
        }
    }

    fn keeps_stable_subtree_constraints(self) -> bool {
        matches!(self, Self::Tick { .. })
    }
}

fn draw_u16(tc: &hegel::TestCase, min: u16, max: u16) -> u16 {
    tc.draw(generators::integers::<u16>().min_value(min).max_value(max))
}

fn assert_no_retained_writes_or_measurement(sample: LayoutWorkSample) {
    assert_eq!(
        sample.measured_layout_calls, 0,
        "stable generated framework frame should not run measured callbacks"
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
        "stable generated framework frame should not mutate retained layout"
    );
}

fn assert_solver_cache_did_not_churn(
    sample: LayoutWorkSample,
    require_hits: bool,
    expect_measure_observations: bool,
) {
    if require_hits {
        assert!(
            sample.solver_cache_hits > 0,
            "stable generated framework frame should observe solver cache hits: {sample:?}"
        );
    }
    assert_eq!(
        [
            sample.solver_cache_stores,
            sample.solver_cache_misses,
            sample.solver_cache_clears,
        ],
        [0; 3],
        "stable generated framework frame should not observe solver cache churn: {sample:?}"
    );
    if expect_measure_observations {
        assert!(
            sample.solver_cache_measure_observations > 0,
            "stable generated framework text frame should observe cached measurement queries: {sample:?}"
        );
    }
}

fn assert_stable_subtree_did_no_work(samples: &[crate::RetainedSubtreeWorkSample], target: &str) {
    let sample = samples
        .iter()
        .find(|sample| sample.global_id.ends_with(target))
        .unwrap_or_else(|| panic!("missing retained subtree work sample for {target}"));
    assert_eq!(
        sample.no_work_total(),
        0,
        "stable subtree {target} should not have retained work: {sample:?}"
    );
    assert!(
        sample.retained_reuses > 0,
        "stable subtree {target} should reuse retained nodes: {sample:?}"
    );
    assert!(
        sample.solver_cache_hits > 0 || sample.solver_cache_measure_observations > 0,
        "stable subtree {target} should observe solver cache reuse or cached measurement proofs: {sample:?}"
    );
}

fn assert_stable_subtree_preserved_without_solver_churn(
    samples: &[crate::RetainedSubtreeWorkSample],
    target: &str,
) {
    let sample = samples
        .iter()
        .find(|sample| sample.global_id.ends_with(target))
        .unwrap_or_else(|| panic!("missing retained subtree work sample for {target}"));
    assert_eq!(
        sample.no_work_total(),
        0,
        "stable subtree {target} should not have retained or solver churn: {sample:?}"
    );
    assert!(
        sample.retained_reuses > 0,
        "stable subtree {target} should retain nodes through sibling churn: {sample:?}"
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
        assert_solver_cache_did_not_churn(stable_sample, false, false);
    })
    .settings(hegel_settings(80))
    .run();
}

#[gpui::test]
fn generated_framework_text_tree_matches_fresh_and_stable_repeat_preserves_work(
    cx: &mut TestAppContext,
) {
    hegel::Hegel::new(|tc| {
        let tree = GeneratedTextNode::draw(&tc, 3);
        let available_size = size(px(draw_u8(&tc, 100, 240) as f32), px(240.0));

        let (first_retained_bounds, second_retained_bounds, stable_sample) = {
            let retained = cx.open_window(available_size, |_, _| GeneratedFrameworkTextView {
                tree: tree.clone(),
            });
            cx.run_until_parked();
            let window = *retained.deref();
            let (first_bounds, _) = draw_generated_text_tree(cx, window, &tree);
            cx.update_window(window, |_, window, _| window.refresh())
                .unwrap();
            cx.run_until_parked();
            let (second_bounds, sample) = draw_generated_text_tree(cx, window, &tree);
            (first_bounds, second_bounds, sample)
        };

        let fresh_bounds = {
            let fresh = cx.open_window(available_size, |window, _| {
                window.force_fresh_layout_for_tests();
                GeneratedFrameworkTextView { tree: tree.clone() }
            });
            cx.run_until_parked();
            draw_generated_text_tree(cx, *fresh.deref(), &tree).0
        };

        assert_eq!(
            first_retained_bounds, fresh_bounds,
            "retained framework text draw should match a fresh-layout framework draw"
        );
        assert_eq!(
            first_retained_bounds, second_retained_bounds,
            "stable retained framework text redraw should publish identical bounds"
        );
        assert_no_retained_writes_or_measurement(stable_sample);
        assert_solver_cache_did_not_churn(stable_sample, true, true);
    })
    .settings(hegel_settings(50))
    .run();
}

#[gpui::test]
fn dynamic_text_sibling_does_not_poison_stable_generated_text_subtree(cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let stable_tree = GeneratedTextNode::draw(&tc, 3);
        let first_tick = draw_u8(&tc, 0, 120);
        let second_tick = first_tick.saturating_add(1);
        let available_size = size(px(420.0), px(260.0));

        let (
            first_retained_bounds,
            second_retained_bounds,
            second_retained_sample,
            stable_subtree_samples,
        ) = {
            let retained = cx.open_window(available_size, |window, _| {
                window.set_retained_subtree_probe_targets_for_tests(vec![
                    "generated-stable-panel".to_string(),
                ]);
                GeneratedDynamicSiblingTextView {
                    stable_tree: stable_tree.clone(),
                    tick: first_tick,
                    dynamic_width: 88,
                }
            });
            cx.run_until_parked();
            let window = *retained.deref();
            let (first_bounds, _, _) = draw_dynamic_sibling_text_tree(cx, window, &stable_tree);
            retained
                .update(cx, |view, _, cx| {
                    view.tick = second_tick;
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
            let (second_bounds, sample, subtree_samples) =
                draw_dynamic_sibling_text_tree(cx, window, &stable_tree);
            (first_bounds, second_bounds, sample, subtree_samples)
        };

        let fresh_second_bounds = {
            let fresh = cx.open_window(available_size, |window, _| {
                window.force_fresh_layout_for_tests();
                GeneratedDynamicSiblingTextView {
                    stable_tree: stable_tree.clone(),
                    tick: second_tick,
                    dynamic_width: 88,
                }
            });
            cx.run_until_parked();
            draw_dynamic_sibling_text_tree(cx, *fresh.deref(), &stable_tree).0
        };

        assert_eq!(
            second_retained_bounds, fresh_second_bounds,
            "retained dynamic-sibling framework draw should match fresh layout"
        );
        assert_eq!(
            first_retained_bounds, second_retained_bounds,
            "changing fixed-width dynamic text should not move the stable sibling tree"
        );
        assert_stable_subtree_did_no_work(&stable_subtree_samples, "generated-stable-panel");
        assert_eq!(
            second_retained_sample.retained_layout_fresh_compare_mismatches, 0,
            "runtime retained-vs-fresh comparison should not report mismatches"
        );
    })
    .settings(hegel_settings(60))
    .run();
}

#[gpui::test]
fn generated_dynamic_text_frame_sequence_matches_fresh_and_keeps_stable_subtree_local(
    cx: &mut TestAppContext,
) {
    hegel::Hegel::new(|tc| {
        let stable_tree = GeneratedTextNode::draw(&tc, 3);
        let mut state = GeneratedDynamicSiblingFrameState::draw(&tc);
        let frame_count = draw_u8(&tc, 2, 6);
        let changes = (0..frame_count)
            .map(|_| GeneratedDynamicSiblingFrameChange::draw(&tc))
            .collect::<Vec<_>>();

        let retained = cx.open_window(state.window_size(), |window, _| {
            window.set_retained_subtree_probe_targets_for_tests(vec![
                "generated-stable-panel".to_string(),
            ]);
            GeneratedDynamicSiblingTextView {
                stable_tree: stable_tree.clone(),
                tick: state.tick,
                dynamic_width: state.dynamic_width,
            }
        });
        cx.run_until_parked();
        let retained_window = *retained.deref();
        let (initial_retained_bounds, _, _) =
            draw_dynamic_sibling_text_tree(cx, retained_window, &stable_tree);
        let initial_fresh_bounds = draw_fresh_dynamic_sibling_text_tree(cx, &stable_tree, state);
        assert_eq!(
            initial_retained_bounds, initial_fresh_bounds,
            "initial retained generated dynamic frame should match fresh layout"
        );

        for change in changes {
            state = state.apply(change);
            match change {
                GeneratedDynamicSiblingFrameChange::Tick { .. }
                | GeneratedDynamicSiblingFrameChange::DynamicWidth { .. } => {
                    retained
                        .update(cx, |view, _, cx| {
                            view.tick = state.tick;
                            view.dynamic_width = state.dynamic_width;
                            cx.notify();
                        })
                        .unwrap();
                }
                GeneratedDynamicSiblingFrameChange::Resize { .. } => {
                    cx.simulate_window_resize(retained_window, state.window_size());
                }
            }
            cx.run_until_parked();

            let (retained_bounds, sample, subtree_samples) =
                draw_dynamic_sibling_text_tree(cx, retained_window, &stable_tree);
            let fresh_bounds = draw_fresh_dynamic_sibling_text_tree(cx, &stable_tree, state);
            assert_eq!(
                retained_bounds, fresh_bounds,
                "retained generated dynamic frame should match fresh layout after {change:?}"
            );
            assert_eq!(
                sample.retained_layout_fresh_compare_mismatches, 0,
                "runtime retained-vs-fresh comparison should not report mismatches after {change:?}"
            );
            if change.keeps_stable_subtree_constraints() {
                assert_stable_subtree_did_no_work(&subtree_samples, "generated-stable-panel");
            }
        }
    })
    .settings(hegel_settings(40))
    .run();
}

#[gpui::test]
fn generated_sibling_churn_matches_fresh_and_preserves_stable_subtree(cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let stable_tree = GeneratedTextNode::draw(&tc, 3);
        let frame_count = draw_u8(&tc, 2, 6);
        let frames = (0..frame_count)
            .map(|tick| GeneratedSiblingChurnFrame::draw(&tc, tick))
            .collect::<Vec<_>>();
        let first_frame = frames
            .first()
            .expect("generated sibling churn should contain at least one frame")
            .clone();

        let retained = cx.open_window(first_frame.window_size(), |window, _| {
            window.set_retained_subtree_probe_targets_for_tests(vec![
                "generated-sibling-churn-stable-panel".to_string(),
            ]);
            GeneratedSiblingChurnTextView {
                stable_tree: stable_tree.clone(),
                frame: first_frame.clone(),
            }
        });
        cx.run_until_parked();
        let retained_window = *retained.deref();

        let (initial_retained_bounds, _, _) =
            draw_sibling_churn_text_tree(cx, retained_window, &stable_tree, &first_frame);
        let initial_fresh_bounds =
            draw_fresh_sibling_churn_text_tree(cx, &stable_tree, &first_frame);
        assert_eq!(
            initial_retained_bounds, initial_fresh_bounds,
            "initial retained sibling-churn frame should match fresh layout"
        );

        for frame in frames.iter().skip(1) {
            retained
                .update(cx, |view, _, cx| {
                    view.frame = frame.clone();
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();

            let (retained_bounds, sample, subtree_samples) =
                draw_sibling_churn_text_tree(cx, retained_window, &stable_tree, frame);
            let fresh_bounds = draw_fresh_sibling_churn_text_tree(cx, &stable_tree, frame);

            assert_eq!(
                retained_bounds, fresh_bounds,
                "retained sibling-churn frame should match fresh layout for {frame:?}"
            );
            assert_eq!(
                sample.retained_layout_fresh_compare_mismatches, 0,
                "runtime retained-vs-fresh comparison should not report mismatches for {frame:?}"
            );
            assert_stable_subtree_preserved_without_solver_churn(
                &subtree_samples,
                "generated-sibling-churn-stable-panel",
            );
        }
    })
    .settings(hegel_settings(40))
    .run();
}
