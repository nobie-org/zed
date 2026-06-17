use crate::{
    AnyElement, AnyEntity, AnyWeakEntity, App, Bounds, ContentMask, Context, Element, ElementId,
    Entity, EntityId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, PaintIndex,
    Pixels, PrepaintStateIndex, Render, Style, StyleRefinement, TextStyle, WeakEntity,
};
use crate::{Empty, Window};
use anyhow::Result;
use collections::FxHashSet;
use refineable::Refineable;
use std::mem;
use std::rc::Rc;
use std::{any::TypeId, fmt, ops::Range};

use crate::window::CachedViewMissReason;

struct AnyViewState {
    prepaint_range: Range<PrepaintStateIndex>,
    paint_range: Range<PaintIndex>,
    cache_key: ViewCacheKey,
    accessed_entities: FxHashSet<EntityId>,
}

#[derive(Default)]
struct ViewCacheKey {
    bounds: Bounds<Pixels>,
    content_mask: ContentMask<Pixels>,
    text_style: TextStyle,
}

/// A dynamically-typed handle to a view, which can be downcast to a [Entity] for a specific type.
#[derive(Clone, Debug)]
pub struct AnyView {
    entity: AnyEntity,
    render: fn(&AnyView, &mut Window, &mut App) -> AnyElement,
    cached_style: Option<Rc<StyleRefinement>>,
}

impl<V: Render> From<Entity<V>> for AnyView {
    fn from(value: Entity<V>) -> Self {
        AnyView {
            entity: value.into_any(),
            render: any_view::render::<V>,
            cached_style: None,
        }
    }
}

impl AnyView {
    /// Indicate that this view should be cached when using it as an element.
    /// When using this method, the view's previous layout and paint will be recycled from the previous frame if [Context::notify] has not been called since it was rendered.
    /// The one exception is when [Window::refresh] is called, in which case caching is ignored.
    pub fn cached(mut self, style: StyleRefinement) -> Self {
        self.cached_style = Some(style.into());
        self
    }

    /// Convert this to a weak handle.
    pub fn downgrade(&self) -> AnyWeakView {
        AnyWeakView {
            entity: self.entity.downgrade(),
            render: self.render,
        }
    }

    /// Convert this to a [Entity] of a specific type.
    /// If this handle does not contain a view of the specified type, returns itself in an `Err` variant.
    pub fn downcast<T: 'static>(self) -> Result<Entity<T>, Self> {
        match self.entity.downcast() {
            Ok(entity) => Ok(entity),
            Err(entity) => Err(Self {
                entity,
                render: self.render,
                cached_style: self.cached_style,
            }),
        }
    }

    /// Gets the [TypeId] of the underlying view.
    pub fn entity_type(&self) -> TypeId {
        self.entity.entity_type
    }

    /// Gets the entity id of this handle.
    pub fn entity_id(&self) -> EntityId {
        self.entity.entity_id()
    }
}

impl PartialEq for AnyView {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity
    }
}

impl Eq for AnyView {}

impl Element for AnyView {
    type RequestLayoutState = Option<AnyElement>;
    type PrepaintState = Option<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::View(self.entity_id()))
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        window.with_rendered_view(self.entity_id(), |window| {
            // Disable caching when inspecting so that mouse_hit_test has all hitboxes.
            let caching_disabled = window.is_inspector_picking(cx);
            match self.cached_style.as_ref() {
                Some(style) if !caching_disabled => {
                    let mut root_style = Style::default();
                    root_style.refine(style);
                    let layout_id = window.request_layout(root_style, None, cx);
                    (layout_id, None)
                }
                _ => {
                    let mut element = (self.render)(self, window, cx);
                    let layout_id = element.request_layout(window, cx);
                    (layout_id, Some(element))
                }
            }
        })
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        window.set_view_id(self.entity_id());
        window.with_rendered_view(self.entity_id(), |window| {
            if let Some(mut element) = element.take() {
                element.prepaint(window, cx);
                return Some(element);
            }

            let global_id = global_id.unwrap();
            window.with_element_state::<AnyViewState, _>(global_id, |element_state, window| {
                window.mark_cached_view_site_seen::<AnyViewState>(self.entity_id(), global_id);
                let content_mask = window.content_mask();
                let text_style = window.text_style();

                let miss_reason = match element_state.as_ref() {
                    None => Some(CachedViewMissReason::NoPreviousState),
                    Some(element_state)
                        if element_state.cache_key.bounds != bounds
                            || element_state.cache_key.content_mask != content_mask
                            || element_state.cache_key.text_style != text_style =>
                    {
                        Some(CachedViewMissReason::CacheKeyChanged)
                    }
                    Some(_)
                        if window.cached_view_site_is_duplicate(self.entity_id(), global_id) =>
                    {
                        Some(CachedViewMissReason::DuplicateSite)
                    }
                    Some(_) if window.cached_view_site_is_dirty(self.entity_id(), global_id) => {
                        Some(CachedViewMissReason::DirtyDependency)
                    }
                    Some(_) if window.dirty_views.contains(&self.entity_id()) => {
                        Some(CachedViewMissReason::DirtyView)
                    }
                    Some(element_state)
                        if !element_state.accessed_entities.is_empty()
                            && !window.cached_view_site_dependencies_are_registered(
                                self.entity_id(),
                                global_id,
                            ) =>
                    {
                        Some(CachedViewMissReason::DependencyRegistrationMissing)
                    }
                    Some(_) if window.refreshing => Some(CachedViewMissReason::Refreshing),
                    Some(_) => None,
                };

                if let Some(mut element_state) = element_state
                    && miss_reason.is_none()
                {
                    #[cfg(any(test, feature = "test-support"))]
                    window.record_cached_view_hit();

                    let prepaint_start = window.prepaint_index();
                    window.reuse_prepaint(element_state.prepaint_range.clone());
                    cx.entities
                        .extend_accessed(&element_state.accessed_entities);
                    let prepaint_end = window.prepaint_index();
                    element_state.prepaint_range = prepaint_start..prepaint_end;

                    return (None, element_state);
                }

                #[cfg(any(test, feature = "test-support"))]
                window.record_cached_view_miss(miss_reason.unwrap());

                let refreshing = mem::replace(&mut window.refreshing, true);
                let prepaint_start = window.prepaint_index();
                let (mut element, accessed_entities) = cx.detect_accessed_entities(|cx| {
                    let mut element = (self.render)(self, window, cx);
                    element.layout_as_root(bounds.size.into(), window, cx);
                    element.prepaint_at(bounds.origin, window, cx);
                    element
                });

                let prepaint_end = window.prepaint_index();
                window.refreshing = refreshing;
                window.replace_cached_view_site_dependencies::<AnyViewState>(
                    self.entity_id(),
                    global_id,
                    &accessed_entities,
                );

                (
                    Some(element),
                    AnyViewState {
                        accessed_entities,
                        prepaint_range: prepaint_start..prepaint_end,
                        paint_range: PaintIndex::default()..PaintIndex::default(),
                        cache_key: ViewCacheKey {
                            bounds,
                            content_mask,
                            text_style,
                        },
                    },
                )
            })
        })
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        element: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_rendered_view(self.entity_id(), |window| {
            let caching_disabled = window.is_inspector_picking(cx);
            if self.cached_style.is_some() && !caching_disabled {
                window.with_element_state::<AnyViewState, _>(
                    global_id.unwrap(),
                    |element_state, window| {
                        let mut element_state = element_state.unwrap();

                        let paint_start = window.paint_index();

                        if let Some(element) = element {
                            let refreshing = mem::replace(&mut window.refreshing, true);
                            element.paint(window, cx);
                            window.refreshing = refreshing;
                        } else {
                            window.reuse_paint(element_state.paint_range.clone());
                        }

                        let paint_end = window.paint_index();
                        element_state.paint_range = paint_start..paint_end;

                        ((), element_state)
                    },
                )
            } else {
                element.as_mut().unwrap().paint(window, cx);
            }
        });
    }
}

impl<V: 'static + Render> IntoElement for Entity<V> {
    type Element = AnyView;

    fn into_element(self) -> Self::Element {
        self.into()
    }
}

impl IntoElement for AnyView {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// A weak, dynamically-typed view handle that does not prevent the view from being released.
pub struct AnyWeakView {
    entity: AnyWeakEntity,
    render: fn(&AnyView, &mut Window, &mut App) -> AnyElement,
}

impl AnyWeakView {
    /// Convert to a strongly-typed handle if the referenced view has not yet been released.
    pub fn upgrade(&self) -> Option<AnyView> {
        let entity = self.entity.upgrade()?;
        Some(AnyView {
            entity,
            render: self.render,
            cached_style: None,
        })
    }
}

impl<V: 'static + Render> From<WeakEntity<V>> for AnyWeakView {
    fn from(view: WeakEntity<V>) -> Self {
        AnyWeakView {
            entity: view.into(),
            render: any_view::render::<V>,
        }
    }
}

impl PartialEq for AnyWeakView {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity
    }
}

impl std::fmt::Debug for AnyWeakView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnyWeakView")
            .field("entity_id", &self.entity.entity_id)
            .finish_non_exhaustive()
    }
}

mod any_view {
    use crate::{AnyElement, AnyView, App, IntoElement, Render, Window};

    pub(crate) fn render<V: 'static + Render>(
        view: &AnyView,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let view = view.clone().downcast::<V>().unwrap();
        view.update(cx, |view, cx| view.render(window, cx).into_any_element())
    }
}

/// A view that renders nothing
pub struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AnyWindowHandle, AppContext as _, TestAppContext, div, prelude::*};
    use gpui::proptest::prelude::{ProptestConfig, prop_assert_eq};
    use std::cell::{Cell, RefCell};

    struct CachedDependencyModel {
        value: usize,
    }

    struct CachedDependencyChild {
        models: Vec<Entity<CachedDependencyModel>>,
        active_model: usize,
        render_count: Rc<Cell<usize>>,
        observed_values: Rc<RefCell<Vec<usize>>>,
    }

    impl Render for CachedDependencyChild {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count.set(self.render_count.get() + 1);
            let value = self.models[self.active_model].read_with(cx, |model, _| model.value);
            self.observed_values.borrow_mut().push(value);

            div().id("cached-dependency-child").child(value.to_string())
        }
    }

    struct CachedDependencyRoot {
        child: Entity<CachedDependencyChild>,
        show_child: bool,
    }

    impl Render for CachedDependencyRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().when(self.show_child, |parent| {
                parent.child(
                    AnyView::from(self.child.clone())
                        .cached(StyleRefinement::default().flex().flex_col().size_full()),
                )
            })
        }
    }

    struct CachedDependencyParent {
        child: Entity<CachedDependencyChild>,
        render_count: Rc<Cell<usize>>,
    }

    impl Render for CachedDependencyParent {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count.set(self.render_count.get() + 1);
            div().size_full().child(
                AnyView::from(self.child.clone())
                    .cached(StyleRefinement::default().flex().flex_col().size_full()),
            )
        }
    }

    struct NestedCachedDependencyRoot {
        parent: Entity<CachedDependencyParent>,
    }

    impl Render for NestedCachedDependencyRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(
                AnyView::from(self.parent.clone())
                    .cached(StyleRefinement::default().flex().flex_col().size_full()),
            )
        }
    }

    struct DuplicateCachedDependencyRoot {
        child: Entity<CachedDependencyChild>,
    }

    impl Render for DuplicateCachedDependencyRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(
                    AnyView::from(self.child.clone())
                        .cached(StyleRefinement::default().flex().flex_col().size_full()),
                )
                .child(
                    AnyView::from(self.child.clone())
                        .cached(StyleRefinement::default().flex().flex_col().size_full()),
                )
        }
    }

    fn draw_root(cx: &mut TestAppContext, window: AnyWindowHandle) {
        cx.update_window(window, |_, window, cx| window.draw(cx).clear())
            .unwrap();
    }

    struct CachedViewOracle {
        visible: bool,
        has_cached_site: bool,
        dirty_site: bool,
        child_dirty: bool,
        active_model: usize,
        cached_dependency: Option<usize>,
        model_values: [usize; 2],
        render_count: usize,
        observed_values: Vec<usize>,
    }

    impl CachedViewOracle {
        fn new() -> Self {
            Self {
                visible: true,
                has_cached_site: false,
                dirty_site: false,
                child_dirty: false,
                active_model: 0,
                cached_dependency: None,
                model_values: [0, 100],
                render_count: 0,
                observed_values: Vec::new(),
            }
        }

        fn draw(&mut self) {
            if !self.visible {
                self.has_cached_site = false;
                self.dirty_site = false;
                self.child_dirty = false;
                self.cached_dependency = None;
                return;
            }

            if self.has_cached_site && !self.dirty_site && !self.child_dirty {
                return;
            }

            self.render_count += 1;
            self.observed_values
                .push(self.model_values[self.active_model]);
            self.has_cached_site = true;
            self.dirty_site = false;
            self.child_dirty = false;
            self.cached_dependency = Some(self.active_model);
        }

        fn notify_model(&mut self, model_ix: usize) {
            self.model_values[model_ix] += 1;
            if self.has_cached_site && self.cached_dependency == Some(model_ix) {
                self.dirty_site = true;
            }
        }

        fn switch_model(&mut self, model_ix: usize) {
            self.active_model = model_ix;
            if self.has_cached_site {
                self.child_dirty = true;
            }
        }
    }

    gpui::proptest::proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            ..Default::default()
        })]

        #[test]
        fn cached_view_dependency_cache_follows_site_state_machine(
            operations in gpui::proptest::collection::vec(0u8..7, 1..64)
        ) {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let models = Rc::new(RefCell::new(Vec::new()));
        let child_slot = Rc::new(RefCell::new(None));
        let render_count = Rc::new(Cell::new(0));
        let observed_values = Rc::new(RefCell::new(Vec::new()));

        let window = cx.add_window({
            let models_slot = models.clone();
            let child_slot = child_slot.clone();
            let render_count = render_count.clone();
            let observed_values = observed_values.clone();

            move |_, cx| {
                let model_a = cx.new(|_| CachedDependencyModel { value: 0 });
                let model_b = cx.new(|_| CachedDependencyModel { value: 100 });
                let child = cx.new(|_| CachedDependencyChild {
                    models: vec![model_a.clone(), model_b.clone()],
                    active_model: 0,
                    render_count,
                    observed_values,
                });
                *models_slot.borrow_mut() = vec![model_a, model_b];
                *child_slot.borrow_mut() = Some(child.clone());

                CachedDependencyRoot {
                    child,
                    show_child: true,
                }
            }
        });
        let any_window = window.into();
        let models = models.borrow().clone();
        let child = child_slot.borrow().clone().unwrap();
        let mut oracle = CachedViewOracle::new();

        draw_root(&mut cx, any_window);
        oracle.draw();
        prop_assert_eq!(render_count.get(), oracle.render_count);
        prop_assert_eq!(&*observed_values.borrow(), &oracle.observed_values);

        for operation in operations {
            match operation {
                0 => {
                    draw_root(&mut cx, any_window);
                    oracle.draw();
                }
                1 | 2 => {
                    let model_ix = (operation - 1) as usize;
                    models[model_ix].update(&mut cx, |model, cx| {
                        model.value += 1;
                        cx.notify();
                    });
                    cx.run_until_parked();
                    oracle.notify_model(model_ix);
                }
                3 | 4 => {
                    let model_ix = (operation - 3) as usize;
                    child.update(&mut cx, |child, cx| {
                        child.active_model = model_ix;
                        cx.notify();
                    });
                    cx.run_until_parked();
                    oracle.switch_model(model_ix);
                }
                5 => {
                    window
                        .update(&mut cx, |root, _window, cx| {
                            root.show_child = false;
                            cx.notify();
                        })
                        .unwrap();
                    cx.run_until_parked();
                    oracle.visible = false;
                }
                6 => {
                    window
                        .update(&mut cx, |root, _window, cx| {
                            root.show_child = true;
                            cx.notify();
                        })
                        .unwrap();
                    cx.run_until_parked();
                    oracle.visible = true;
                }
                _ => unreachable!(),
            }

            prop_assert_eq!(render_count.get(), oracle.render_count);
            prop_assert_eq!(&*observed_values.borrow(), &oracle.observed_values);
            let site_count = cx
                .update_window(any_window, |_, window, _| {
                    window.debug_cached_view_dependency_site_count()
                })
                .unwrap();
            prop_assert_eq!(site_count, usize::from(oracle.has_cached_site));
        }

        draw_root(&mut cx, any_window);
        oracle.draw();
        prop_assert_eq!(render_count.get(), oracle.render_count);
        prop_assert_eq!(&*observed_values.borrow(), &oracle.observed_values);
        let site_count = cx
            .update_window(any_window, |_, window, _| {
                window.debug_cached_view_dependency_site_count()
            })
            .unwrap();
        prop_assert_eq!(site_count, usize::from(oracle.has_cached_site));
        }
    }

    #[test]
    fn cached_view_rerenders_when_retained_dependency_notifies() {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let model = Rc::new(RefCell::new(None));
        let render_count = Rc::new(Cell::new(0));
        let observed_values = Rc::new(RefCell::new(Vec::new()));

        let window = cx.add_window({
            let model_slot = model.clone();
            let render_count = render_count.clone();
            let observed_values = observed_values.clone();

            move |_, cx| {
                let model = cx.new(|_| CachedDependencyModel { value: 0 });
                let child = cx.new(|_| CachedDependencyChild {
                    models: vec![model.clone()],
                    active_model: 0,
                    render_count,
                    observed_values,
                });
                *model_slot.borrow_mut() = Some(model);

                CachedDependencyRoot {
                    child,
                    show_child: true,
                }
            }
        });
        let any_window = window.into();

        draw_root(&mut cx, any_window);
        assert_eq!(render_count.get(), 1);

        draw_root(&mut cx, any_window);
        assert_eq!(render_count.get(), 1);

        let model = model.borrow().clone().unwrap();
        model.update(&mut cx, |model, cx| {
            model.value = 1;
            cx.notify();
        });
        cx.run_until_parked();

        draw_root(&mut cx, any_window);

        assert_eq!(render_count.get(), 2);
        assert_eq!(&*observed_values.borrow(), &[0, 1]);

        let counters = cx
            .update_window(any_window, |_, window, _| {
                window.debug_cached_view_counters()
            })
            .unwrap();
        assert!(counters.hits >= 1);
        assert_eq!(counters.no_previous_state_misses, 1);
        assert_eq!(counters.dirty_dependency_misses, 1);
    }

    #[test]
    fn cached_view_dependency_replacement_forgets_old_dependency() {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let models = Rc::new(RefCell::new(Vec::new()));
        let child_slot = Rc::new(RefCell::new(None));
        let render_count = Rc::new(Cell::new(0));
        let observed_values = Rc::new(RefCell::new(Vec::new()));

        let window = cx.add_window({
            let models_slot = models.clone();
            let child_slot = child_slot.clone();
            let render_count = render_count.clone();
            let observed_values = observed_values.clone();

            move |_, cx| {
                let model_a = cx.new(|_| CachedDependencyModel { value: 0 });
                let model_b = cx.new(|_| CachedDependencyModel { value: 100 });
                let child = cx.new(|_| CachedDependencyChild {
                    models: vec![model_a.clone(), model_b.clone()],
                    active_model: 0,
                    render_count,
                    observed_values,
                });
                *models_slot.borrow_mut() = vec![model_a, model_b];
                *child_slot.borrow_mut() = Some(child.clone());

                CachedDependencyRoot {
                    child,
                    show_child: true,
                }
            }
        });
        let any_window = window.into();

        draw_root(&mut cx, any_window);
        assert_eq!(render_count.get(), 1);

        let child = child_slot.borrow().clone().unwrap();
        child.update(&mut cx, |child, cx| {
            child.active_model = 1;
            cx.notify();
        });
        cx.run_until_parked();
        draw_root(&mut cx, any_window);
        assert_eq!(render_count.get(), 2);

        let model_a = models.borrow()[0].clone();
        model_a.update(&mut cx, |model, cx| {
            model.value = 1;
            cx.notify();
        });
        cx.run_until_parked();
        draw_root(&mut cx, any_window);
        assert_eq!(render_count.get(), 2);

        let model_b = models.borrow()[1].clone();
        model_b.update(&mut cx, |model, cx| {
            model.value = 101;
            cx.notify();
        });
        cx.run_until_parked();
        draw_root(&mut cx, any_window);

        assert_eq!(render_count.get(), 3);
        assert_eq!(&*observed_values.borrow(), &[0, 100, 101]);
    }

    #[test]
    fn nested_cached_view_dependencies_survive_parent_replay() {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let model = Rc::new(RefCell::new(None));
        let parent_render_count = Rc::new(Cell::new(0));
        let child_render_count = Rc::new(Cell::new(0));
        let observed_values = Rc::new(RefCell::new(Vec::new()));

        let window = cx.add_window({
            let model_slot = model.clone();
            let parent_render_count = parent_render_count.clone();
            let child_render_count = child_render_count.clone();
            let observed_values = observed_values.clone();

            move |_, cx| {
                let model = cx.new(|_| CachedDependencyModel { value: 0 });
                let child = cx.new(|_| CachedDependencyChild {
                    models: vec![model.clone()],
                    active_model: 0,
                    render_count: child_render_count,
                    observed_values,
                });
                let parent = cx.new(|_| CachedDependencyParent {
                    child,
                    render_count: parent_render_count,
                });
                *model_slot.borrow_mut() = Some(model);

                NestedCachedDependencyRoot { parent }
            }
        });
        let any_window = window.into();

        draw_root(&mut cx, any_window);
        assert_eq!(parent_render_count.get(), 1);
        assert_eq!(child_render_count.get(), 1);

        draw_root(&mut cx, any_window);
        assert_eq!(parent_render_count.get(), 1);
        assert_eq!(child_render_count.get(), 1);
        cx.update_window(any_window, |_, window, _| {
            assert_eq!(window.debug_cached_view_dependency_site_count(), 2);
        })
        .unwrap();

        let model = model.borrow().clone().unwrap();
        model.update(&mut cx, |model, cx| {
            model.value = 1;
            cx.notify();
        });
        cx.run_until_parked();
        draw_root(&mut cx, any_window);

        assert_eq!(parent_render_count.get(), 2);
        assert_eq!(child_render_count.get(), 2);
        assert_eq!(&*observed_values.borrow(), &[0, 1]);
    }

    #[test]
    fn duplicate_cached_view_site_is_retention_ineligible() {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let render_count = Rc::new(Cell::new(0));
        let observed_values = Rc::new(RefCell::new(Vec::new()));

        let window = cx.add_window({
            let render_count = render_count.clone();
            let observed_values = observed_values.clone();

            move |_, cx| {
                let model = cx.new(|_| CachedDependencyModel { value: 0 });
                let child = cx.new(|_| CachedDependencyChild {
                    models: vec![model],
                    active_model: 0,
                    render_count,
                    observed_values,
                });

                DuplicateCachedDependencyRoot { child }
            }
        });
        let any_window = window.into();
        let initial_render_count = render_count.get();

        draw_root(&mut cx, any_window);
        assert_eq!(render_count.get(), initial_render_count + 2);
        cx.update_window(any_window, |_, window, _| {
            assert_eq!(window.debug_cached_view_dependency_site_count(), 0);
            assert_eq!(window.debug_cached_view_duplicate_site_count(), 1);
        })
        .unwrap();

        draw_root(&mut cx, any_window);
        assert_eq!(render_count.get(), initial_render_count + 4);
        assert!(observed_values.borrow().iter().all(|value| *value == 0));
    }

    #[test]
    fn cached_view_dependencies_are_removed_when_site_unmounts() {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let model = Rc::new(RefCell::new(None));
        let render_count = Rc::new(Cell::new(0));
        let observed_values = Rc::new(RefCell::new(Vec::new()));

        let window = cx.add_window({
            let model_slot = model.clone();
            let render_count = render_count.clone();
            let observed_values = observed_values.clone();

            move |_, cx| {
                let model = cx.new(|_| CachedDependencyModel { value: 0 });
                let child = cx.new(|_| CachedDependencyChild {
                    models: vec![model.clone()],
                    active_model: 0,
                    render_count,
                    observed_values,
                });
                *model_slot.borrow_mut() = Some(model);

                CachedDependencyRoot {
                    child,
                    show_child: true,
                }
            }
        });
        let any_window = window.into();

        draw_root(&mut cx, any_window);
        cx.update_window(any_window, |_, window, _| {
            assert_eq!(window.debug_cached_view_dependency_site_count(), 1);
        })
        .unwrap();

        window
            .update(&mut cx, |root, _window, cx| {
                root.show_child = false;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        draw_root(&mut cx, any_window);

        cx.update_window(any_window, |_, window, _| {
            assert_eq!(window.debug_cached_view_dependency_site_count(), 0);
        })
        .unwrap();

        let model = model.borrow().clone().unwrap();
        model.update(&mut cx, |model, cx| {
            model.value = 1;
            cx.notify();
        });
        cx.run_until_parked();
        draw_root(&mut cx, any_window);

        assert_eq!(render_count.get(), 1);
        assert_eq!(&*observed_values.borrow(), &[0]);
    }
}
