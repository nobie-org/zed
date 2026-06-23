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

#[derive(Clone, Copy, Debug)]
enum GeneratedStyledContainerKind {
    Block,
    FlexRow,
    FlexColumn,
    FlexWrap,
    Grid,
    RelativeOverlay,
}

#[derive(Clone, Copy, Debug)]
enum GeneratedStyledNodeFacts {
    Fixed {
        width: u8,
        height: u8,
    },
    Full {
        min_width: u8,
        min_height: u8,
    },
    ClampedWidth {
        width: u8,
        min_width: u8,
        max_width: u8,
        height: u8,
    },
    MarginBorder {
        width: u8,
        height: u8,
        margin: u8,
        padding: u8,
    },
    Absolute {
        left: u8,
        top: u8,
        width: u8,
        height: u8,
    },
}

#[derive(Clone, Debug)]
enum GeneratedStyledNode {
    Container {
        facts: GeneratedStyledNodeFacts,
        kind: GeneratedStyledContainerKind,
        padding: u8,
        gap: u8,
        children: Vec<GeneratedStyledNode>,
    },
    Text {
        facts: GeneratedStyledNodeFacts,
        padding: u8,
        text_size: u8,
        text: String,
    },
}

impl GeneratedStyledNodeFacts {
    fn draw(tc: &hegel::TestCase, allow_absolute: bool) -> Self {
        match draw_u8(tc, 0, if allow_absolute { 4 } else { 3 }) {
            0 => Self::Fixed {
                width: draw_u8(tc, 20, 220),
                height: draw_u8(tc, 16, 180),
            },
            1 => Self::Full {
                min_width: draw_u8(tc, 0, 48),
                min_height: draw_u8(tc, 0, 48),
            },
            2 => {
                let min_width = draw_u8(tc, 0, 90);
                let max_width = draw_u8(tc, min_width.max(20), 240);
                Self::ClampedWidth {
                    width: draw_u8(tc, 20, 240),
                    min_width,
                    max_width,
                    height: draw_u8(tc, 16, 180),
                }
            }
            3 => Self::MarginBorder {
                width: draw_u8(tc, 20, 220),
                height: draw_u8(tc, 16, 180),
                margin: draw_u8(tc, 0, 8),
                padding: draw_u8(tc, 0, 8),
            },
            _ => Self::Absolute {
                left: draw_u8(tc, 0, 24),
                top: draw_u8(tc, 0, 24),
                width: draw_u8(tc, 16, 120),
                height: draw_u8(tc, 16, 120),
            },
        }
    }
}

impl GeneratedStyledContainerKind {
    fn draw(tc: &hegel::TestCase) -> Self {
        match draw_u8(tc, 0, 5) {
            0 => Self::Block,
            1 => Self::FlexRow,
            2 => Self::FlexColumn,
            3 => Self::FlexWrap,
            4 => Self::Grid,
            _ => Self::RelativeOverlay,
        }
    }
}

impl GeneratedStyledNode {
    fn draw(tc: &hegel::TestCase, depth: u8, allow_absolute: bool) -> Self {
        let draw_text = depth == 0 || tc.draw(generators::booleans());
        if draw_text {
            return Self::Text {
                facts: GeneratedStyledNodeFacts::draw(tc, allow_absolute),
                padding: draw_u8(tc, 0, 8),
                text_size: draw_u8(tc, 10, 22),
                text: draw_ascii_words(tc),
            };
        }

        let kind = GeneratedStyledContainerKind::draw(tc);
        let child_count = draw_u8(tc, 1, 4);
        let children_allow_absolute =
            allow_absolute || matches!(kind, GeneratedStyledContainerKind::RelativeOverlay);
        let children = (0..child_count)
            .map(|_| Self::draw(tc, depth.saturating_sub(1), children_allow_absolute))
            .collect();

        Self::Container {
            facts: GeneratedStyledNodeFacts::draw(tc, allow_absolute),
            kind,
            padding: draw_u8(tc, 0, 8),
            gap: draw_u8(tc, 0, 10),
            children,
        }
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

    fn build_at(&self, selector: &str) -> AnyElement {
        let selector_for_debug = selector.to_string();
        match self {
            Self::Container {
                facts,
                kind,
                padding,
                gap,
                children,
            } => {
                let children = children
                    .iter()
                    .enumerate()
                    .map(|(index, child)| child.build_at(&format!("{selector}/{index}")))
                    .collect::<Vec<_>>();
                let mut element = div()
                    .debug_selector(move || selector_for_debug)
                    .p(px(*padding as f32))
                    .gap(px(*gap as f32));

                element = apply_generated_styled_facts(element, *facts);
                element = match kind {
                    GeneratedStyledContainerKind::Block => element,
                    GeneratedStyledContainerKind::FlexRow => element.flex().flex_row(),
                    GeneratedStyledContainerKind::FlexColumn => element.flex().flex_col(),
                    GeneratedStyledContainerKind::FlexWrap => element.flex().flex_row().flex_wrap(),
                    GeneratedStyledContainerKind::Grid => element.grid().grid_cols(2).grid_rows(2),
                    GeneratedStyledContainerKind::RelativeOverlay => element.relative(),
                };

                element.children(children).into_any_element()
            }
            Self::Text {
                facts,
                padding,
                text_size,
                text,
            } => {
                let element = div()
                    .debug_selector(move || selector_for_debug)
                    .p(px(*padding as f32))
                    .text_size(px(*text_size as f32))
                    .child(text.clone());
                apply_generated_styled_facts(element, *facts).into_any_element()
            }
        }
    }
}

fn apply_generated_styled_facts(
    element: crate::elements::Div,
    facts: GeneratedStyledNodeFacts,
) -> crate::elements::Div {
    match facts {
        GeneratedStyledNodeFacts::Fixed { width, height } => {
            element.w(px(width as f32)).h(px(height as f32))
        }
        GeneratedStyledNodeFacts::Full {
            min_width,
            min_height,
        } => element
            .w_full()
            .h_full()
            .min_w(px(min_width as f32))
            .min_h(px(min_height as f32)),
        GeneratedStyledNodeFacts::ClampedWidth {
            width,
            min_width,
            max_width,
            height,
        } => element
            .w(px(width as f32))
            .min_w(px(min_width as f32))
            .max_w(px(max_width as f32))
            .h(px(height as f32)),
        GeneratedStyledNodeFacts::MarginBorder {
            width,
            height,
            margin,
            padding,
        } => element
            .w(px(width as f32))
            .h(px(height as f32))
            .m(px(margin as f32))
            .p(px(padding as f32))
            .border_1(),
        GeneratedStyledNodeFacts::Absolute {
            left,
            top,
            width,
            height,
        } => element
            .absolute()
            .left(px(left as f32))
            .top(px(top as f32))
            .w(px(width as f32))
            .h(px(height as f32)),
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

fn draw_generated_styled_sibling_tree(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    stable_tree: &GeneratedStyledNode,
) -> (
    Vec<(String, Bounds<Pixels>)>,
    LayoutWorkSample,
    Vec<crate::RetainedSubtreeWorkSample>,
) {
    let mut selectors = vec![
        "styled-root".to_string(),
        "styled-dynamic".to_string(),
        "styled-stable-panel".to_string(),
    ];
    selectors.extend(stable_tree.selectors_at("styled-stable/root"));

    let bounds = selectors
        .into_iter()
        .map(|selector| {
            let bounds = cx
                .update_window(window, |_, window, _| {
                    window.rendered_frame.debug_bounds.get(&selector).copied()
                })
                .unwrap()
                .unwrap_or_else(|| {
                    panic!("missing debug bounds for generated styled selector {selector}")
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
        sample.expect("generated styled draw should publish layout work"),
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
    let (bounds, sample, _) = draw_dynamic_sibling_text_tree(cx, *fresh.deref(), stable_tree);
    assert_fresh_oracle_sample(sample);
    bounds
}

fn draw_fresh_generated_styled_sibling_tree(
    cx: &mut TestAppContext,
    stable_tree: &GeneratedStyledNode,
    tick: u8,
    dynamic_width: u8,
) -> Vec<(String, Bounds<Pixels>)> {
    let fresh = cx.open_window(size(px(640.0), px(420.0)), |window, _| {
        window.force_fresh_layout_for_tests();
        GeneratedStyledSiblingView {
            stable_tree: stable_tree.clone(),
            tick,
            dynamic_width,
        }
    });
    cx.run_until_parked();
    let (bounds, sample, _) = draw_generated_styled_sibling_tree(cx, *fresh.deref(), stable_tree);
    assert_fresh_oracle_sample(sample);
    bounds
}

fn draw_fresh_generated_resizing_parent_tree(
    cx: &mut TestAppContext,
    stable_tree: &GeneratedStyledNode,
    tick: u8,
    dynamic_width: u8,
    window_size: crate::Size<Pixels>,
) -> Vec<(String, Bounds<Pixels>)> {
    let fresh = cx.open_window(window_size, |window, _| {
        window.force_fresh_layout_for_tests();
        GeneratedStyledSiblingView {
            stable_tree: stable_tree.clone(),
            tick,
            dynamic_width,
        }
    });
    cx.run_until_parked();
    let (bounds, sample, _) = draw_generated_styled_sibling_tree(cx, *fresh.deref(), stable_tree);
    assert_fresh_oracle_sample(sample);
    bounds
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

struct GeneratedStyledSiblingView {
    stable_tree: GeneratedStyledNode,
    tick: u8,
    dynamic_width: u8,
}

impl Render for GeneratedStyledSiblingView {
    fn render(
        &mut self,
        _window: &mut BuildCx<'_>,
        _cx: &mut crate::Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("generated-styled-sibling-root")
            .debug_selector(|| "styled-root".into())
            .flex()
            .flex_row()
            .gap(px(10.0))
            .w_full()
            .h_full()
            .child(
                div()
                    .id("generated-styled-dynamic")
                    .debug_selector(|| "styled-dynamic".into())
                    .flex_none()
                    .w(px(self.dynamic_width as f32))
                    .h(px(88.0))
                    .p(px(6.0))
                    .text_size(px(14.0))
                    .child(format!("dynamic styled {}", self.tick)),
            )
            .child(
                div()
                    .id("generated-styled-stable-panel")
                    .debug_selector(|| "styled-stable-panel".into())
                    .relative()
                    .flex_none()
                    .w(px(360.0))
                    .h(px(340.0))
                    .child(self.stable_tree.build_at("styled-stable/root")),
            )
    }
}

#[derive(Clone, Copy, Debug)]
struct GeneratedStyledSiblingFrameState {
    tick: u8,
    dynamic_width: u8,
}

impl GeneratedStyledSiblingFrameState {
    fn draw(tc: &hegel::TestCase) -> Self {
        Self {
            tick: draw_u8(tc, 0, 120),
            dynamic_width: draw_u8(tc, 48, 180),
        }
    }

    fn apply(self, change: GeneratedStyledSiblingFrameChange) -> Self {
        match change {
            GeneratedStyledSiblingFrameChange::Tick { delta } => Self {
                tick: self.tick.wrapping_add(delta),
                ..self
            },
            GeneratedStyledSiblingFrameChange::DynamicWidth { width } => Self {
                dynamic_width: width,
                ..self
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum GeneratedStyledSiblingFrameChange {
    Tick { delta: u8 },
    DynamicWidth { width: u8 },
}

impl GeneratedStyledSiblingFrameChange {
    fn draw(tc: &hegel::TestCase) -> Self {
        if tc.draw(generators::booleans()) {
            Self::Tick {
                delta: draw_u8(tc, 1, 16),
            }
        } else {
            Self::DynamicWidth {
                width: draw_u8(tc, 48, 180),
            }
        }
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
    let (bounds, sample, _) = draw_sibling_churn_text_tree(cx, *fresh.deref(), stable_tree, frame);
    assert_fresh_oracle_sample(sample);
    bounds
}

#[derive(Clone, Debug)]
struct GeneratedInternalEditFrame {
    tick: u8,
    mutable_width: u8,
    mutable_height: u8,
    mutable_text_size: u8,
}

impl GeneratedInternalEditFrame {
    fn draw(tc: &hegel::TestCase, tick: u8) -> Self {
        Self {
            tick,
            mutable_width: draw_u8(tc, 80, 220),
            mutable_height: draw_u8(tc, 20, 80),
            mutable_text_size: draw_u8(tc, 10, 22),
        }
    }

    fn window_size(&self) -> crate::Size<Pixels> {
        size(px(440.0), px(1080.0))
    }

    fn selectors(
        &self,
        before_tree: &GeneratedTextNode,
        after_tree: &GeneratedTextNode,
    ) -> Vec<String> {
        let mut selectors = vec![
            "root".to_string(),
            "stable-before-panel".to_string(),
            "mutable".to_string(),
            "stable-after-panel".to_string(),
        ];
        selectors.extend(before_tree.selectors_at("before/root"));
        selectors.extend(after_tree.selectors_at("after/root"));
        selectors
    }
}

struct GeneratedInternalEditTextView {
    before_tree: GeneratedTextNode,
    after_tree: GeneratedTextNode,
    frame: GeneratedInternalEditFrame,
}

impl Render for GeneratedInternalEditTextView {
    fn render(
        &mut self,
        _window: &mut BuildCx<'_>,
        _cx: &mut crate::Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("generated-internal-edit-root")
            .debug_selector(|| "root".into())
            .flex()
            .flex_col()
            .gap(px(6.0))
            .w(px(400.0))
            .h(px(1040.0))
            .child(
                div()
                    .id("generated-internal-edit-stable-before")
                    .debug_selector(|| "stable-before-panel".into())
                    .flex_none()
                    .w(px(320.0))
                    .h(px(360.0))
                    .child(self.before_tree.build_at("before/root", false)),
            )
            .child(
                div()
                    .id("generated-internal-edit-mutable")
                    .debug_selector(|| "mutable".into())
                    .flex_none()
                    .w(px(self.frame.mutable_width as f32))
                    .h(px(self.frame.mutable_height as f32))
                    .text_size(px(self.frame.mutable_text_size as f32))
                    .child(format!("mutable frame {}", self.frame.tick)),
            )
            .child(
                div()
                    .id("generated-internal-edit-stable-after")
                    .debug_selector(|| "stable-after-panel".into())
                    .flex_none()
                    .w(px(320.0))
                    .h(px(360.0))
                    .child(self.after_tree.build_at("after/root", false)),
            )
    }
}

fn draw_internal_edit_text_tree(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    before_tree: &GeneratedTextNode,
    after_tree: &GeneratedTextNode,
    frame: &GeneratedInternalEditFrame,
) -> (
    Vec<(String, Bounds<Pixels>)>,
    LayoutWorkSample,
    Vec<crate::RetainedSubtreeWorkSample>,
) {
    let bounds = frame
        .selectors(before_tree, after_tree)
        .into_iter()
        .map(|selector| {
            let bounds = cx
                .update_window(window, |_, window, _| {
                    window.rendered_frame.debug_bounds.get(&selector).copied()
                })
                .unwrap()
                .unwrap_or_else(|| {
                    panic!("missing debug bounds for internal-edit selector {selector}")
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
        sample.expect("internal-edit draw should publish layout work"),
        subtree_samples,
    )
}

fn draw_fresh_internal_edit_text_tree(
    cx: &mut TestAppContext,
    before_tree: &GeneratedTextNode,
    after_tree: &GeneratedTextNode,
    frame: &GeneratedInternalEditFrame,
) -> Vec<(String, Bounds<Pixels>)> {
    let fresh = cx.open_window(frame.window_size(), |window, _| {
        window.force_fresh_layout_for_tests();
        GeneratedInternalEditTextView {
            before_tree: before_tree.clone(),
            after_tree: after_tree.clone(),
            frame: frame.clone(),
        }
    });
    cx.run_until_parked();
    let (bounds, sample, _) =
        draw_internal_edit_text_tree(cx, *fresh.deref(), before_tree, after_tree, frame);
    assert_fresh_oracle_sample(sample);
    bounds
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

fn assert_fresh_oracle_sample(sample: LayoutWorkSample) {
    assert_eq!(
        sample.force_fresh_frame_resets, 1,
        "fresh oracle window should discard retained layout state: {sample:?}"
    );
}

fn assert_retained_frame_sample(sample: LayoutWorkSample) {
    assert_eq!(
        sample.force_fresh_frame_resets, 0,
        "retained test window should not run in forced-fresh mode: {sample:?}"
    );
}

fn assert_no_retained_writes_or_measurement(sample: LayoutWorkSample) {
    assert_retained_frame_sample(sample);
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

fn assert_subtree_solver_preservation_observed(
    sample: &crate::RetainedSubtreeWorkSample,
    target: &str,
) {
    assert!(
        sample.solver_cache_hits + sample.solver_cache_measure_observations > 0,
        "stable subtree {target} should observe preserved solver state: {sample:?}"
    );
}

fn assert_stable_subtree_did_no_work(samples: &[crate::RetainedSubtreeWorkSample], target: &str) {
    let sample = samples
        .iter()
        .find(|sample| sample.global_id.ends_with(target))
        .unwrap_or_else(|| panic!("missing retained subtree work sample for {target}"));
    assert!(
        sample.node_count > 0,
        "stable subtree {target} should contain retained nodes: {sample:?}"
    );
    assert_eq!(
        sample.retained_reuses, sample.node_count as u64,
        "stable subtree {target} should reuse every node: {sample:?}"
    );
    assert_eq!(
        sample.no_work_total(),
        0,
        "stable subtree {target} should not have retained work: {sample:?}"
    );
    assert_eq!(
        sample.measured_callbacks, 0,
        "stable subtree {target} should not run measured callbacks: {sample:?}"
    );
    assert_eq!(
        sample.conservative_text_measured_callbacks, 0,
        "stable subtree {target} should not hide measured callbacks behind exemptions: {sample:?}"
    );
    assert!(
        sample.retained_reuses > 0,
        "stable subtree {target} should reuse retained nodes: {sample:?}"
    );
    assert_subtree_solver_preservation_observed(sample, target);
}

fn assert_stable_subtree_preserved_without_solver_churn(
    samples: &[crate::RetainedSubtreeWorkSample],
    target: &str,
) {
    let sample = samples
        .iter()
        .find(|sample| sample.global_id.ends_with(target))
        .unwrap_or_else(|| panic!("missing retained subtree work sample for {target}"));
    assert!(
        sample.node_count > 0,
        "stable subtree {target} should contain retained nodes: {sample:?}"
    );
    assert_eq!(
        sample.retained_reuses, sample.node_count as u64,
        "stable subtree {target} should reuse every node: {sample:?}"
    );
    assert_eq!(
        sample.no_work_total(),
        0,
        "stable subtree {target} should not have retained or solver churn: {sample:?}"
    );
    assert_eq!(
        sample.measured_callbacks, 0,
        "stable subtree {target} should not run measured callbacks: {sample:?}"
    );
    assert_eq!(
        sample.conservative_text_measured_callbacks, 0,
        "stable subtree {target} should not hide measured callbacks behind exemptions: {sample:?}"
    );
    assert_subtree_solver_preservation_observed(sample, target);
}

fn assert_stable_subtree_retained_without_gpui_work(
    samples: &[crate::RetainedSubtreeWorkSample],
    target: &str,
) {
    let sample = samples
        .iter()
        .find(|sample| sample.global_id.ends_with(target))
        .unwrap_or_else(|| panic!("missing retained subtree work sample for {target}"));
    assert!(
        sample.node_count > 0,
        "stable subtree {target} should contain retained nodes: {sample:?}"
    );
    assert_eq!(
        sample.retained_reuses, sample.node_count as u64,
        "stable subtree {target} should reuse every node: {sample:?}"
    );
    assert_eq!(
        [
            sample.retained_misses,
            sample.mirror_node_creates,
            sample.mirror_node_removes,
            sample.mirror_set_style,
            sample.mirror_set_children,
            sample.mirror_dirty_marks,
            sample.mirror_measured_context_clears,
            sample.measured_callbacks,
            sample.conservative_text_measured_callbacks,
        ],
        [0; 9],
        "stable subtree {target} should have no GPUI retained, mirror, or measured work: {sample:?}"
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

        let (first_retained_bounds, second_retained_bounds, stable_sample, stable_subtree_samples) = {
            let retained = cx.open_window(
                available_space.map(|space| match space {
                    AvailableSpace::Definite(value) => value,
                    AvailableSpace::MinContent | AvailableSpace::MaxContent => px(240.0),
                }),
                |window, _| {
                    window.set_retained_subtree_probe_targets_for_tests(vec![
                        "generated-framework-retained-root".to_string(),
                    ]);
                    GeneratedFrameworkView { tree: tree.clone() }
                },
            );
            cx.run_until_parked();
            let window = *retained.deref();
            let (first_bounds, _) = draw_generated_div_tree(cx, window, &tree);
            cx.update_window(window, |_, window, _| window.refresh())
                .unwrap();
            cx.run_until_parked();
            let (second_bounds, sample) = draw_generated_div_tree(cx, window, &tree);
            let subtree_samples = cx
                .update_window(window, |_, window, _| {
                    window
                        .last_retained_subtree_work_samples_for_tests()
                        .to_vec()
                })
                .unwrap();
            (first_bounds, second_bounds, sample, subtree_samples)
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
            let (bounds, sample) = draw_generated_div_tree(cx, *fresh.deref(), &tree);
            assert_fresh_oracle_sample(sample);
            bounds
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
        assert_stable_subtree_preserved_without_solver_churn(
            &stable_subtree_samples,
            "generated-framework-retained-root",
        );
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
            let (bounds, sample) = draw_generated_text_tree(cx, *fresh.deref(), &tree);
            assert_fresh_oracle_sample(sample);
            bounds
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
fn generated_framework_styled_sibling_churn_matches_fresh_and_preserves_stable_subtree(
    cx: &mut TestAppContext,
) {
    hegel::Hegel::new(|tc| {
        let stable_tree = GeneratedStyledNode::draw(&tc, 3, false);
        let first_tick = draw_u8(&tc, 0, 120);
        let second_tick = first_tick.wrapping_add(draw_u8(&tc, 1, 16));
        let first_dynamic_width = draw_u8(&tc, 48, 180);
        let second_dynamic_width = draw_u8(&tc, 48, 180);

        let (
            first_retained_bounds,
            second_retained_bounds,
            second_retained_sample,
            stable_subtree_samples,
        ) = {
            let retained = cx.open_window(size(px(640.0), px(420.0)), |window, _| {
                window.set_retained_subtree_probe_targets_for_tests(vec![
                    "generated-styled-stable-panel".to_string(),
                ]);
                GeneratedStyledSiblingView {
                    stable_tree: stable_tree.clone(),
                    tick: first_tick,
                    dynamic_width: first_dynamic_width,
                }
            });
            cx.run_until_parked();
            let window = *retained.deref();
            let (first_bounds, _, _) = draw_generated_styled_sibling_tree(cx, window, &stable_tree);
            retained
                .update(cx, |view, _, cx| {
                    view.tick = second_tick;
                    view.dynamic_width = second_dynamic_width;
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
            let (second_bounds, sample, subtree_samples) =
                draw_generated_styled_sibling_tree(cx, window, &stable_tree);
            (first_bounds, second_bounds, sample, subtree_samples)
        };

        let first_fresh_bounds = draw_fresh_generated_styled_sibling_tree(
            cx,
            &stable_tree,
            first_tick,
            first_dynamic_width,
        );
        let second_fresh_bounds = draw_fresh_generated_styled_sibling_tree(
            cx,
            &stable_tree,
            second_tick,
            second_dynamic_width,
        );

        assert_eq!(
            first_retained_bounds, first_fresh_bounds,
            "initial retained generated styled frame should match fresh layout"
        );
        assert_eq!(
            second_retained_bounds, second_fresh_bounds,
            "retained generated styled sibling-churn frame should match fresh layout"
        );
        assert_eq!(
            second_retained_sample.retained_layout_fresh_compare_mismatches, 0,
            "runtime retained-vs-fresh comparison should not report mismatches"
        );
        assert_retained_frame_sample(second_retained_sample);
        assert_stable_subtree_preserved_without_solver_churn(
            &stable_subtree_samples,
            "generated-styled-stable-panel",
        );
    })
    .settings(hegel_settings(60))
    .run();
}

#[gpui::test]
fn generated_styled_sibling_frame_sequence_matches_fresh_and_preserves_stable_subtree(
    cx: &mut TestAppContext,
) {
    hegel::Hegel::new(|tc| {
        let stable_tree = GeneratedStyledNode::draw(&tc, 3, false);
        let mut state = GeneratedStyledSiblingFrameState::draw(&tc);
        let frame_count = draw_u8(&tc, 2, 6);
        let changes = (0..frame_count)
            .map(|_| GeneratedStyledSiblingFrameChange::draw(&tc))
            .collect::<Vec<_>>();

        let retained = cx.open_window(size(px(640.0), px(420.0)), |window, _| {
            window.set_retained_subtree_probe_targets_for_tests(vec![
                "generated-styled-stable-panel".to_string(),
            ]);
            GeneratedStyledSiblingView {
                stable_tree: stable_tree.clone(),
                tick: state.tick,
                dynamic_width: state.dynamic_width,
            }
        });
        cx.run_until_parked();
        let retained_window = *retained.deref();
        let (initial_retained_bounds, _, _) =
            draw_generated_styled_sibling_tree(cx, retained_window, &stable_tree);
        let initial_fresh_bounds = draw_fresh_generated_styled_sibling_tree(
            cx,
            &stable_tree,
            state.tick,
            state.dynamic_width,
        );
        assert_eq!(
            initial_retained_bounds, initial_fresh_bounds,
            "initial retained styled generated frame should match fresh layout"
        );

        for change in changes {
            state = state.apply(change);
            retained
                .update(cx, |view, _, cx| {
                    view.tick = state.tick;
                    view.dynamic_width = state.dynamic_width;
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();

            let (retained_bounds, sample, subtree_samples) =
                draw_generated_styled_sibling_tree(cx, retained_window, &stable_tree);
            let fresh_bounds = draw_fresh_generated_styled_sibling_tree(
                cx,
                &stable_tree,
                state.tick,
                state.dynamic_width,
            );

            assert_eq!(
                retained_bounds, fresh_bounds,
                "retained styled generated frame should match fresh layout after {change:?}"
            );
            assert_eq!(
                sample.retained_layout_fresh_compare_mismatches, 0,
                "runtime retained-vs-fresh comparison should not report mismatches after {change:?}"
            );
            assert_retained_frame_sample(sample);
            assert_stable_subtree_preserved_without_solver_churn(
                &subtree_samples,
                "generated-styled-stable-panel",
            );
        }
    })
    .settings(hegel_settings(40))
    .run();
}

#[gpui::test]
fn generated_parent_resize_preserves_fixed_stable_subtree_retention(cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let stable_tree = GeneratedStyledNode::draw(&tc, 3, false);
        let first_tick = draw_u8(&tc, 0, 120);
        let second_tick = first_tick.wrapping_add(draw_u8(&tc, 1, 16));
        let dynamic_width = draw_u8(&tc, 48, 180);
        let first_width = draw_u16(&tc, 540, 720);
        let second_width = draw_u16(&tc, 721, 960);
        let height = draw_u16(&tc, 380, 520);
        let first_window_size = size(px(first_width as f32), px(height as f32));
        let second_window_size = size(px(second_width as f32), px(height as f32));

        let (
            _first_retained_bounds,
            second_retained_bounds,
            second_retained_sample,
            stable_subtree_samples,
        ) = {
            let retained = cx.open_window(first_window_size, |window, _| {
                window.set_retained_subtree_probe_targets_for_tests(vec![
                    "generated-styled-stable-panel".to_string(),
                ]);
                GeneratedStyledSiblingView {
                    stable_tree: stable_tree.clone(),
                    tick: first_tick,
                    dynamic_width,
                }
            });
            cx.run_until_parked();
            let window = *retained.deref();
            let (first_bounds, _, _) = draw_generated_styled_sibling_tree(cx, window, &stable_tree);
            retained
                .update(cx, |view, _, cx| {
                    view.tick = second_tick;
                    cx.notify();
                })
                .unwrap();
            cx.simulate_window_resize(window, second_window_size);
            cx.run_until_parked();
            let (second_bounds, sample, subtree_samples) =
                draw_generated_styled_sibling_tree(cx, window, &stable_tree);
            (first_bounds, second_bounds, sample, subtree_samples)
        };

        let second_fresh_bounds = draw_fresh_generated_resizing_parent_tree(
            cx,
            &stable_tree,
            second_tick,
            dynamic_width,
            second_window_size,
        );

        assert_eq!(
            second_retained_bounds, second_fresh_bounds,
            "retained fixed stable subtree under parent resize should match fresh layout"
        );
        assert_eq!(
            second_retained_sample.retained_layout_fresh_compare_mismatches, 0,
            "runtime retained-vs-fresh comparison should not report parent-resize mismatches"
        );
        assert_retained_frame_sample(second_retained_sample);
        assert_stable_subtree_retained_without_gpui_work(
            &stable_subtree_samples,
            "generated-styled-stable-panel",
        );
    })
    .settings(hegel_settings(60))
    .run();
}

#[gpui::test]
fn minimized_parent_resize_preserves_fixed_stable_text_subtree_retention(cx: &mut TestAppContext) {
    let stable_tree = GeneratedStyledNode::Text {
        facts: GeneratedStyledNodeFacts::Fixed {
            width: 20,
            height: 16,
        },
        padding: 0,
        text_size: 10,
        text: "a".to_string(),
    };
    let first_window_size = size(px(540.0), px(380.0));
    let second_window_size = size(px(721.0), px(380.0));

    let (second_retained_bounds, second_retained_sample, stable_subtree_samples) = {
        let retained = cx.open_window(first_window_size, |window, _| {
            window.set_retained_subtree_probe_targets_for_tests(vec![
                "generated-styled-stable-panel".to_string(),
            ]);
            GeneratedStyledSiblingView {
                stable_tree: stable_tree.clone(),
                tick: 0,
                dynamic_width: 48,
            }
        });
        cx.run_until_parked();
        let window = *retained.deref();
        let _ = draw_generated_styled_sibling_tree(cx, window, &stable_tree);
        retained
            .update(cx, |view, _, cx| {
                view.tick = 1;
                cx.notify();
            })
            .unwrap();
        cx.simulate_window_resize(window, second_window_size);
        cx.run_until_parked();
        let (second_bounds, sample, subtree_samples) =
            draw_generated_styled_sibling_tree(cx, window, &stable_tree);
        (second_bounds, sample, subtree_samples)
    };

    let second_fresh_bounds =
        draw_fresh_generated_resizing_parent_tree(cx, &stable_tree, 1, 48, second_window_size);

    assert_eq!(
        second_retained_bounds, second_fresh_bounds,
        "retained minimized parent-resize frame should match fresh layout"
    );
    assert_eq!(
        second_retained_sample.retained_layout_fresh_compare_mismatches, 0,
        "runtime retained-vs-fresh comparison should not report minimized parent-resize mismatches"
    );
    assert_retained_frame_sample(second_retained_sample);
    assert_stable_subtree_retained_without_gpui_work(
        &stable_subtree_samples,
        "generated-styled-stable-panel",
    );
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
            let (bounds, sample, _) =
                draw_dynamic_sibling_text_tree(cx, *fresh.deref(), &stable_tree);
            assert_fresh_oracle_sample(sample);
            bounds
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
        assert_retained_frame_sample(second_retained_sample);
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
            assert_retained_frame_sample(sample);
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
            assert_retained_frame_sample(sample);
            assert_stable_subtree_preserved_without_solver_churn(
                &subtree_samples,
                "generated-sibling-churn-stable-panel",
            );
        }
    })
    .settings(hegel_settings(40))
    .run();
}

#[gpui::test]
fn generated_internal_edit_matches_fresh_and_preserves_unchanged_sibling_subtrees(
    cx: &mut TestAppContext,
) {
    hegel::Hegel::new(|tc| {
        let before_tree = GeneratedTextNode::draw(&tc, 3);
        let after_tree = GeneratedTextNode::draw(&tc, 3);
        let frame_count = draw_u8(&tc, 2, 6);
        let frames = (0..frame_count)
            .map(|tick| GeneratedInternalEditFrame::draw(&tc, tick))
            .collect::<Vec<_>>();
        let first_frame = frames
            .first()
            .expect("generated internal edit should contain at least one frame")
            .clone();

        let retained = cx.open_window(first_frame.window_size(), |window, _| {
            window.set_retained_subtree_probe_targets_for_tests(vec![
                "generated-internal-edit-stable-before".to_string(),
                "generated-internal-edit-stable-after".to_string(),
            ]);
            GeneratedInternalEditTextView {
                before_tree: before_tree.clone(),
                after_tree: after_tree.clone(),
                frame: first_frame.clone(),
            }
        });
        cx.run_until_parked();
        let retained_window = *retained.deref();

        let (initial_retained_bounds, _, _) = draw_internal_edit_text_tree(
            cx,
            retained_window,
            &before_tree,
            &after_tree,
            &first_frame,
        );
        let initial_fresh_bounds =
            draw_fresh_internal_edit_text_tree(cx, &before_tree, &after_tree, &first_frame);
        assert_eq!(
            initial_retained_bounds, initial_fresh_bounds,
            "initial retained internal-edit frame should match fresh layout"
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
                draw_internal_edit_text_tree(cx, retained_window, &before_tree, &after_tree, frame);
            let fresh_bounds =
                draw_fresh_internal_edit_text_tree(cx, &before_tree, &after_tree, frame);

            assert_eq!(
                retained_bounds, fresh_bounds,
                "retained internal-edit frame should match fresh layout for {frame:?}"
            );
            assert_eq!(
                sample.retained_layout_fresh_compare_mismatches, 0,
                "runtime retained-vs-fresh comparison should not report mismatches for {frame:?}"
            );
            assert_retained_frame_sample(sample);
            assert_stable_subtree_preserved_without_solver_churn(
                &subtree_samples,
                "generated-internal-edit-stable-before",
            );
            assert_stable_subtree_preserved_without_solver_churn(
                &subtree_samples,
                "generated-internal-edit-stable-after",
            );
        }
    })
    .settings(hegel_settings(40))
    .run();
}
