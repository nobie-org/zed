use crate::{
    AnyElement, AnyEntity, AnyWeakEntity, App, Bounds, ContentMask, Context, Element, ElementId,
    Entity, EntityId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Length,
    PaintIndex, Pixels, PrepaintStateIndex, Render, Size, Style, StyleRefinement, TextStyle,
    WeakEntity,
};
use crate::{Empty, Window};
use anyhow::Result;
use collections::FxHashSet;
use refineable::Refineable;
use std::mem;
use std::panic::{self, AssertUnwindSafe};
use std::rc::Rc;
use std::{any::TypeId, fmt, ops::Range};

use crate::window::CachedViewMissReason;

struct AnyViewState {
    prepaint_range: Range<PrepaintStateIndex>,
    paint_range: Range<PaintIndex>,
    cache_key: ViewCacheKey,
    accessed_entities: FxHashSet<EntityId>,
    automatic_root_style: Option<Style>,
    cache_kind: AnyViewCacheKind,
}

#[derive(Default, PartialEq)]
struct ViewCacheKey {
    bounds: Bounds<Pixels>,
    content_mask: ContentMask<Pixels>,
    text_style: TextStyle,
    rem_size: Pixels,
    scale_factor: f32,
    viewport_size: Size<Pixels>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnyViewCacheKind {
    Automatic,
    Manual,
}

#[derive(Clone, Debug)]
enum AnyViewCacheMode {
    Automatic,
    Manual(Rc<StyleRefinement>),
}

/// Opaque request-layout state used by [`AnyView`].
pub struct AnyViewRequestLayoutState(AnyViewRequestLayoutStateInner);

enum AnyViewRequestLayoutStateInner {
    Rendered {
        element: AnyElement,
        accessed_entities: FxHashSet<EntityId>,
        automatic_root_style: Option<Style>,
    },
    CachedCandidate,
}

/// A dynamically-typed handle to a view, which can be downcast to a [Entity] for a specific type.
#[derive(Clone, Debug)]
pub struct AnyView {
    entity: AnyEntity,
    render: fn(&AnyView, &mut Window, &mut App) -> AnyElement,
    cache_mode: AnyViewCacheMode,
}

impl<V: Render> From<Entity<V>> for AnyView {
    fn from(value: Entity<V>) -> Self {
        AnyView {
            entity: value.into_any(),
            render: any_view::render::<V>,
            cache_mode: AnyViewCacheMode::Automatic,
        }
    }
}

impl AnyView {
    /// Indicate that this view should be cached when using it as an element.
    /// When using this method, the view's previous layout and paint will be recycled from the previous frame if [Context::notify] has not been called since it was rendered.
    /// The one exception is when [Window::refresh] is called, in which case caching is ignored.
    pub fn cached(mut self, style: StyleRefinement) -> Self {
        self.cache_mode = AnyViewCacheMode::Manual(style.into());
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
                cache_mode: self.cache_mode,
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

    fn cache_kind(&self) -> AnyViewCacheKind {
        match self.cache_mode {
            AnyViewCacheMode::Automatic => AnyViewCacheKind::Automatic,
            AnyViewCacheMode::Manual(_) => AnyViewCacheKind::Manual,
        }
    }

    fn current_cache_key(&self, bounds: Bounds<Pixels>, window: &Window) -> ViewCacheKey {
        ViewCacheKey {
            bounds,
            content_mask: window.content_mask(),
            text_style: window.text_style(),
            rem_size: window.rem_size(),
            scale_factor: window.scale_factor(),
            viewport_size: window.viewport_size(),
        }
    }

    fn automatic_cached_root_layout(
        &self,
        global_id: &GlobalElementId,
        window: &mut Window,
    ) -> Result<LayoutId, CachedViewMissReason> {
        let Some(element_state) = window.element_state::<AnyViewState>(global_id) else {
            return Err(CachedViewMissReason::NoPreviousState);
        };

        if element_state.cache_kind != AnyViewCacheKind::Automatic {
            return Err(CachedViewMissReason::CacheKeyChanged);
        }

        let cache_key = self.current_cache_key(element_state.cache_key.bounds, window);
        if element_state.cache_key.content_mask != cache_key.content_mask
            || element_state.cache_key.text_style != cache_key.text_style
            || element_state.cache_key.rem_size != cache_key.rem_size
            || element_state.cache_key.scale_factor != cache_key.scale_factor
            || element_state.cache_key.viewport_size != cache_key.viewport_size
        {
            return Err(CachedViewMissReason::CacheKeyChanged);
        }

        if window.cached_view_site_is_duplicate(self.entity_id(), global_id) {
            return Err(CachedViewMissReason::DuplicateSite);
        }

        if window.cached_view_site_is_dirty(self.entity_id(), global_id) {
            return Err(CachedViewMissReason::DirtyDependency);
        }

        if window.dirty_views.contains(&self.entity_id()) {
            return Err(CachedViewMissReason::DirtyView);
        }

        if !element_state.accessed_entities.is_empty()
            && !window.cached_view_site_dependencies_are_registered(self.entity_id(), global_id)
        {
            return Err(CachedViewMissReason::DependencyRegistrationMissing);
        }

        if window.refreshing {
            return Err(CachedViewMissReason::Refreshing);
        }

        let Some(root_style) = element_state.automatic_root_style.clone() else {
            return Err(CachedViewMissReason::OuterLayoutUnknown);
        };

        if !automatic_root_style_is_admissible(&root_style) {
            return Err(CachedViewMissReason::OuterLayoutUnknown);
        }

        window.claim_cached_view_retained_layout_root(self.entity_id(), global_id)
    }

    fn render_for_uncached_request_layout(
        &self,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyViewRequestLayoutState) {
        let mut element = (self.render)(self, window, cx);
        let layout_id = element.request_layout(window, cx);
        (
            layout_id,
            AnyViewRequestLayoutState(AnyViewRequestLayoutStateInner::Rendered {
                element,
                accessed_entities: FxHashSet::default(),
                automatic_root_style: None,
            }),
        )
    }

    fn render_for_automatic_request_layout(
        &self,
        global_id: Option<&GlobalElementId>,
        miss_reason: CachedViewMissReason,
        retain_layout: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyViewRequestLayoutState) {
        let mut effective_miss_reason = miss_reason;
        let retain_layout = if retain_layout {
            global_id.is_none_or(|global_id| {
                let claimed =
                    window.claim_cached_view_retained_layout_site(self.entity_id(), global_id);
                if !claimed {
                    effective_miss_reason = CachedViewMissReason::DuplicateSite;
                }
                claimed
            })
        } else {
            false
        };
        let retained_global_id = global_id.filter(|_| retain_layout);
        if let Some(global_id) = retained_global_id {
            window.begin_cached_view_retained_layout_scope(self.entity_id(), global_id);
        }

        let render_result = panic::catch_unwind(AssertUnwindSafe(|| {
            cx.detect_accessed_entities(|cx| {
                let mut element = (self.render)(self, window, cx);
                let layout_id = element.request_layout(window, cx);
                (element, layout_id)
            })
        }));
        let ((element, layout_id), accessed_entities) = match render_result {
            Ok(result) => result,
            Err(payload) => {
                if let Some(global_id) = retained_global_id {
                    window.discard_cached_view_retained_layout_scope(self.entity_id(), global_id);
                }
                panic::resume_unwind(payload);
            }
        };
        let automatic_root_style = retained_global_id
            .and_then(|_| window.take_requested_layout_style(layout_id))
            .filter(automatic_root_style_is_admissible);

        if let Some(global_id) = retained_global_id {
            if automatic_root_style.is_some() {
                window.finish_cached_view_retained_layout_scope(
                    self.entity_id(),
                    global_id,
                    layout_id,
                );
            } else {
                window.discard_cached_view_retained_layout_scope(self.entity_id(), global_id);
            }
        }

        #[cfg(any(test, feature = "test-support"))]
        {
            let effective_miss_reason = if retain_layout && automatic_root_style.is_none() {
                CachedViewMissReason::OuterLayoutUnknown
            } else {
                effective_miss_reason
            };
            window.record_cached_view_miss(effective_miss_reason);
            window.record_automatic_cached_view_miss();
        }

        (
            layout_id,
            AnyViewRequestLayoutState(AnyViewRequestLayoutStateInner::Rendered {
                element,
                accessed_entities,
                automatic_root_style,
            }),
        )
    }
}

fn automatic_root_style_is_admissible(style: &Style) -> bool {
    matches!(style.size.width, Length::Definite(_))
        && matches!(style.size.height, Length::Definite(_))
}

fn automatic_global_id_is_admissible(global_id: &GlobalElementId) -> bool {
    global_id.len() > 1
}

impl PartialEq for AnyView {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity
    }
}

impl Eq for AnyView {}

impl Element for AnyView {
    type RequestLayoutState = AnyViewRequestLayoutState;
    type PrepaintState = Option<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        Some(ElementId::View(self.entity_id()))
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        window.with_rendered_view(self.entity_id(), |window| {
            // Disable caching when inspecting so that mouse_hit_test has all hitboxes.
            let caching_disabled = window.is_inspector_picking(cx);
            match &self.cache_mode {
                AnyViewCacheMode::Manual(style) if !caching_disabled => {
                    if let Some(global_id) = global_id {
                        window.remove_cached_view_retained_layout(self.entity_id(), global_id);
                    }
                    let mut root_style = Style::default();
                    root_style.refine(style);
                    let layout_id = window.request_layout(root_style, None, cx);
                    (
                        layout_id,
                        AnyViewRequestLayoutState(AnyViewRequestLayoutStateInner::CachedCandidate),
                    )
                }
                AnyViewCacheMode::Automatic if !caching_disabled => {
                    let Some(global_id) =
                        global_id.filter(|global_id| automatic_global_id_is_admissible(global_id))
                    else {
                        return self.render_for_uncached_request_layout(window, cx);
                    };

                    let miss_reason = match self.automatic_cached_root_layout(global_id, window) {
                        Ok(layout_id) => {
                            return (
                                layout_id,
                                AnyViewRequestLayoutState(
                                    AnyViewRequestLayoutStateInner::CachedCandidate,
                                ),
                            );
                        }
                        Err(reason) => reason,
                    };

                    self.render_for_automatic_request_layout(
                        Some(global_id),
                        miss_reason,
                        miss_reason != CachedViewMissReason::DuplicateSite,
                        window,
                        cx,
                    )
                }
                AnyViewCacheMode::Automatic => {
                    #[cfg(any(test, feature = "test-support"))]
                    window.record_automatic_cached_view_miss();
                    self.render_for_uncached_request_layout(window, cx)
                }
                AnyViewCacheMode::Manual(_) => self.render_for_uncached_request_layout(window, cx),
            }
        })
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout_state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        window.set_view_id(self.entity_id());
        window.with_rendered_view(self.entity_id(), |window| {
            if let AnyViewRequestLayoutStateInner::Rendered {
                element,
                accessed_entities,
                automatic_root_style,
            } = mem::replace(
                &mut request_layout_state.0,
                AnyViewRequestLayoutStateInner::CachedCandidate,
            ) {
                let prepaint_start = window.prepaint_index();
                let mut element = element;
                element.prepaint(window, cx);
                let prepaint_end = window.prepaint_index();

                if let Some(global_id) = global_id {
                    let cache_key = self.current_cache_key(bounds, window);
                    if window.cached_view_site_is_duplicate(self.entity_id(), global_id) {
                        window.mark_cached_view_site_seen::<AnyViewState>(
                            self.entity_id(),
                            global_id,
                        );
                    } else if let Some(automatic_root_style) = automatic_root_style {
                        window.with_element_state::<AnyViewState, _>(global_id, |_, window| {
                            window.mark_cached_view_site_seen::<AnyViewState>(
                                self.entity_id(),
                                global_id,
                            );
                            window.replace_cached_view_site_dependencies::<AnyViewState>(
                                self.entity_id(),
                                global_id,
                                &accessed_entities,
                            );
                            (
                                (),
                                AnyViewState {
                                    accessed_entities,
                                    automatic_root_style: Some(automatic_root_style),
                                    cache_kind: AnyViewCacheKind::Automatic,
                                    prepaint_range: prepaint_start..prepaint_end,
                                    paint_range: PaintIndex::default()..PaintIndex::default(),
                                    cache_key,
                                },
                            )
                        });
                    }
                }

                return Some(element);
            }

            let global_id = global_id.unwrap();
            window.with_element_state::<AnyViewState, _>(global_id, |element_state, window| {
                window.mark_cached_view_site_seen::<AnyViewState>(self.entity_id(), global_id);
                let cache_key = self.current_cache_key(bounds, window);
                let cache_kind = self.cache_kind();

                let miss_reason = match element_state.as_ref() {
                    None => Some(CachedViewMissReason::NoPreviousState),
                    Some(element_state) if element_state.cache_kind != cache_kind => {
                        Some(CachedViewMissReason::CacheKeyChanged)
                    }
                    Some(element_state) if element_state.cache_key != cache_key => {
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
                    {
                        window.record_cached_view_hit();
                        if element_state.automatic_root_style.is_some() {
                            window.record_automatic_cached_view_hit();
                        }
                    }

                    let prepaint_start = window.prepaint_index();
                    window.reuse_prepaint(element_state.prepaint_range.clone());
                    cx.entities
                        .extend_accessed(&element_state.accessed_entities);
                    let prepaint_end = window.prepaint_index();
                    element_state.prepaint_range = prepaint_start..prepaint_end;

                    return (None, element_state);
                }

                #[cfg(any(test, feature = "test-support"))]
                {
                    window.record_cached_view_miss(miss_reason.unwrap());
                    if cache_kind == AnyViewCacheKind::Automatic {
                        window.record_automatic_cached_view_miss();
                    }
                }

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
                        automatic_root_style: None,
                        cache_kind,
                        prepaint_range: prepaint_start..prepaint_end,
                        paint_range: PaintIndex::default()..PaintIndex::default(),
                        cache_key,
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
            let has_cached_state = !caching_disabled
                && global_id.is_some_and(|global_id| {
                    if element.is_some() {
                        window
                            .next_frame_element_state::<AnyViewState>(global_id)
                            .is_some()
                    } else {
                        window.element_state::<AnyViewState>(global_id).is_some()
                    }
                });
            if has_cached_state {
                window.with_element_state::<AnyViewState, _>(
                    global_id.expect("cached view state requires an element id"),
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
            cache_mode: AnyViewCacheMode::Automatic,
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

            div()
                .id("cached-dependency-child")
                .size_full()
                .child(value.to_string())
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

    struct AutomaticDependencyRoot {
        child: Entity<CachedDependencyChild>,
        show_child: bool,
        label: usize,
        render_count: Rc<Cell<usize>>,
    }

    impl Render for AutomaticDependencyRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count.set(self.render_count.get() + 1);
            div()
                .size_full()
                .child(self.label.to_string())
                .when(self.show_child, |parent| parent.child(self.child.clone()))
        }
    }

    struct DuplicateAutomaticDependencyRoot {
        child: Entity<CachedDependencyChild>,
        duplicate: bool,
        label: usize,
    }

    impl Render for DuplicateAutomaticDependencyRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(self.label.to_string())
                .child(self.child.clone())
                .when(self.duplicate, |parent| parent.child(self.child.clone()))
        }
    }

    struct ModeSwitchingDependencyRoot {
        child: Entity<CachedDependencyChild>,
        manual: bool,
        label: usize,
    }

    impl Render for ModeSwitchingDependencyRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let child = AnyView::from(self.child.clone());
            let child = if self.manual {
                child.cached(StyleRefinement::default().flex().flex_col().size_full())
            } else {
                child
            };

            div().size_full().child(self.label.to_string()).child(child)
        }
    }

    struct IntrinsicDependencyChild {
        render_count: Rc<Cell<usize>>,
    }

    impl Render for IntrinsicDependencyChild {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count.set(self.render_count.get() + 1);
            div().id("intrinsic-dependency-child").child("intrinsic")
        }
    }

    struct ShapeShiftingDependencyChild {
        models: Vec<Entity<CachedDependencyModel>>,
        definite_root: bool,
        render_count: Rc<Cell<usize>>,
        observed_values: Rc<RefCell<Vec<usize>>>,
    }

    impl Render for ShapeShiftingDependencyChild {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.render_count.set(self.render_count.get() + 1);
            let value = self.models[0].read_with(cx, |model, _| model.value);
            self.observed_values.borrow_mut().push(value);

            let child = div()
                .id("shape-shifting-dependency-child")
                .child(value.to_string());
            if self.definite_root {
                child.size_full()
            } else {
                child
            }
        }
    }

    struct ShapeShiftingDependencyRoot {
        child: Entity<ShapeShiftingDependencyChild>,
        label: usize,
    }

    impl Render for ShapeShiftingDependencyRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(self.label.to_string())
                .child(self.child.clone())
        }
    }

    struct AutomaticIntrinsicRoot {
        child: Entity<IntrinsicDependencyChild>,
        label: usize,
    }

    impl Render for AutomaticIntrinsicRoot {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(self.label.to_string())
                .child(self.child.clone())
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

    gpui::proptest::proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            ..Default::default()
        })]

        #[test]
        fn cached_view_automatic_child_cache_follows_site_state_machine(
            operations in gpui::proptest::collection::vec(0u8..8, 1..64)
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

                AutomaticDependencyRoot {
                    child,
                    show_child: true,
                    label: 0,
                    render_count: Rc::new(Cell::new(0)),
                }
            }
        });
        let any_window = window.into();
        let models = models.borrow().clone();
        let child = child_slot.borrow().clone().unwrap();
        let mut oracle = CachedViewOracle::new();
        if render_count.get() > 0 {
            oracle.draw();
        }

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
                7 => {
                    window
                        .update(&mut cx, |root, _window, cx| {
                            root.label += 1;
                            cx.notify();
                        })
                        .unwrap();
                    cx.run_until_parked();
                    draw_root(&mut cx, any_window);
                    oracle.draw();
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
    fn cached_view_automatic_child_replays_when_parent_rerenders() {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let root_render_count = Rc::new(Cell::new(0));
        let child_render_count = Rc::new(Cell::new(0));
        let observed_values = Rc::new(RefCell::new(Vec::new()));

        let window = cx.add_window({
            let root_render_count = root_render_count.clone();
            let child_render_count = child_render_count.clone();
            let observed_values = observed_values.clone();

            move |_, cx| {
                let model = cx.new(|_| CachedDependencyModel { value: 0 });
                let child = cx.new(|_| CachedDependencyChild {
                    models: vec![model],
                    active_model: 0,
                    render_count: child_render_count,
                    observed_values,
                });

                AutomaticDependencyRoot {
                    child,
                    show_child: true,
                    label: 0,
                    render_count: root_render_count,
                }
            }
        });
        let any_window = window.into();

        draw_root(&mut cx, any_window);
        let root_render_count_after_first_draw = root_render_count.get();
        let child_render_count_after_first_draw = child_render_count.get();
        let observed_values_after_first_draw = observed_values.borrow().clone();
        let counters_after_first_draw = cx
            .update_window(any_window, |_, window, _| {
                window.debug_cached_view_counters()
            })
            .unwrap();
        let retained_layout_node_creates_after_first_draw = cx
            .update_window(any_window, |_, window, _| {
                window.debug_retained_layout_node_creates()
            })
            .unwrap();
        let retained_layout_node_count_after_first_draw = cx
            .update_window(any_window, |_, window, _| {
                window.debug_retained_layout_node_count()
            })
            .unwrap();
        assert!(retained_layout_node_count_after_first_draw > 0);

        window
            .update(&mut cx, |root, _window, cx| {
                root.label = 1;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        draw_root(&mut cx, any_window);

        assert_eq!(
            root_render_count.get(),
            root_render_count_after_first_draw + 1
        );
        assert_eq!(
            child_render_count.get(),
            child_render_count_after_first_draw
        );
        assert_eq!(
            &*observed_values.borrow(),
            &observed_values_after_first_draw
        );
        cx.update_window(any_window, |_, window, _| {
            let counters = window.debug_cached_view_counters();
            assert_eq!(
                counters.automatic_hits,
                counters_after_first_draw.automatic_hits + 1
            );
            assert_eq!(
                counters.automatic_misses,
                counters_after_first_draw.automatic_misses
            );
            assert_eq!(counters.outer_layout_unknown_misses, 0);
            assert_eq!(
                window.debug_retained_layout_node_creates(),
                retained_layout_node_creates_after_first_draw
            );
            assert_eq!(
                window.debug_retained_layout_node_count(),
                retained_layout_node_count_after_first_draw
            );
            assert_eq!(window.debug_cached_view_dependency_site_count(), 1);
        })
        .unwrap();
    }

    #[test]
    fn cached_view_automatic_child_rerenders_when_retained_dependency_notifies() {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let model = Rc::new(RefCell::new(None));
        let child_render_count = Rc::new(Cell::new(0));
        let observed_values = Rc::new(RefCell::new(Vec::new()));

        let window = cx.add_window({
            let model_slot = model.clone();
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
                *model_slot.borrow_mut() = Some(model);

                AutomaticDependencyRoot {
                    child,
                    show_child: true,
                    label: 0,
                    render_count: Rc::new(Cell::new(0)),
                }
            }
        });
        let any_window = window.into();

        draw_root(&mut cx, any_window);
        draw_root(&mut cx, any_window);
        let child_render_count_after_clean_draws = child_render_count.get();
        let observed_values_after_clean_draws = observed_values.borrow().clone();

        let model = model.borrow().clone().unwrap();
        model.update(&mut cx, |model, cx| {
            model.value = 1;
            cx.notify();
        });
        cx.run_until_parked();
        draw_root(&mut cx, any_window);

        assert_eq!(
            child_render_count.get(),
            child_render_count_after_clean_draws + 1
        );
        let mut expected_observed_values = observed_values_after_clean_draws;
        expected_observed_values.push(1);
        assert_eq!(&*observed_values.borrow(), &expected_observed_values);
        cx.update_window(any_window, |_, window, _| {
            let counters = window.debug_cached_view_counters();
            assert!(counters.automatic_hits >= 1);
            assert_eq!(counters.automatic_misses, 2);
            assert_eq!(counters.dirty_dependency_misses, 1);
            assert_eq!(window.debug_cached_view_dependency_site_count(), 1);
        })
        .unwrap();
    }

    #[test]
    fn cached_view_automatic_duplicate_after_retained_hit_is_not_reused_twice() {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let child_render_count = Rc::new(Cell::new(0));
        let observed_values = Rc::new(RefCell::new(Vec::new()));

        let window = cx.add_window({
            let child_render_count = child_render_count.clone();
            let observed_values = observed_values.clone();

            move |_, cx| {
                let model = cx.new(|_| CachedDependencyModel { value: 0 });
                let child = cx.new(|_| CachedDependencyChild {
                    models: vec![model],
                    active_model: 0,
                    render_count: child_render_count,
                    observed_values,
                });

                DuplicateAutomaticDependencyRoot {
                    child,
                    duplicate: false,
                    label: 0,
                }
            }
        });
        let any_window = window.into();

        draw_root(&mut cx, any_window);
        let child_render_count_after_first_draw = child_render_count.get();
        draw_root(&mut cx, any_window);
        assert_eq!(
            child_render_count.get(),
            child_render_count_after_first_draw
        );
        cx.update_window(any_window, |_, window, _| {
            assert!(window.debug_retained_layout_node_count() > 0);
            assert_eq!(window.debug_cached_view_dependency_site_count(), 1);
        })
        .unwrap();

        window
            .update(&mut cx, |root, _window, cx| {
                root.duplicate = true;
                root.label = 1;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        draw_root(&mut cx, any_window);
        assert_eq!(
            child_render_count.get(),
            child_render_count_after_first_draw + 2
        );
        cx.update_window(any_window, |_, window, _| {
            let counters = window.debug_cached_view_counters();
            assert!(counters.duplicate_site_misses >= 1);
            assert_eq!(window.debug_cached_view_dependency_site_count(), 0);
            assert_eq!(window.debug_cached_view_duplicate_site_count(), 1);
            assert_eq!(window.debug_retained_layout_node_count(), 0);
        })
        .unwrap();

        draw_root(&mut cx, any_window);
        assert_eq!(
            child_render_count.get(),
            child_render_count_after_first_draw + 4
        );
        cx.update_window(any_window, |_, window, _| {
            assert_eq!(window.debug_cached_view_dependency_site_count(), 0);
            assert_eq!(window.debug_cached_view_duplicate_site_count(), 1);
            assert_eq!(window.debug_retained_layout_node_count(), 0);
        })
        .unwrap();
        assert!(observed_values.borrow().iter().all(|value| *value == 0));
    }

    #[test]
    fn cached_view_switching_from_automatic_to_manual_drops_retained_layout() {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let child_render_count = Rc::new(Cell::new(0));
        let observed_values = Rc::new(RefCell::new(Vec::new()));

        let window = cx.add_window({
            let child_render_count = child_render_count.clone();
            let observed_values = observed_values.clone();

            move |_, cx| {
                let model = cx.new(|_| CachedDependencyModel { value: 0 });
                let child = cx.new(|_| CachedDependencyChild {
                    models: vec![model],
                    active_model: 0,
                    render_count: child_render_count,
                    observed_values,
                });

                ModeSwitchingDependencyRoot {
                    child,
                    manual: false,
                    label: 0,
                }
            }
        });
        let any_window = window.into();

        draw_root(&mut cx, any_window);
        let child_render_count_after_first_draw = child_render_count.get();
        draw_root(&mut cx, any_window);
        assert_eq!(
            child_render_count.get(),
            child_render_count_after_first_draw
        );
        let counters_after_automatic_hit = cx
            .update_window(any_window, |_, window, _| {
                assert!(window.debug_retained_layout_node_count() > 0);
                window.debug_cached_view_counters()
            })
            .unwrap();

        window
            .update(&mut cx, |root, _window, cx| {
                root.manual = true;
                root.label = 1;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        draw_root(&mut cx, any_window);
        assert_eq!(
            child_render_count.get(),
            child_render_count_after_first_draw + 1
        );
        cx.update_window(any_window, |_, window, _| {
            let counters = window.debug_cached_view_counters();
            assert_eq!(
                counters.automatic_hits,
                counters_after_automatic_hit.automatic_hits
            );
            assert_eq!(
                counters.cache_key_changed_misses,
                counters_after_automatic_hit.cache_key_changed_misses + 1
            );
            assert_eq!(window.debug_retained_layout_node_count(), 0);
            assert_eq!(window.debug_cached_view_dependency_site_count(), 1);
        })
        .unwrap();

        draw_root(&mut cx, any_window);
        assert_eq!(
            child_render_count.get(),
            child_render_count_after_first_draw + 1
        );
    }

    #[test]
    fn cached_view_automatic_child_without_outer_layout_contract_is_not_retained() {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let child_render_count = Rc::new(Cell::new(0));

        let window = cx.add_window({
            let child_render_count = child_render_count.clone();

            move |_, cx| {
                let child = cx.new(|_| IntrinsicDependencyChild {
                    render_count: child_render_count,
                });

                AutomaticIntrinsicRoot { child, label: 0 }
            }
        });
        let any_window = window.into();

        draw_root(&mut cx, any_window);
        let child_render_count_after_first_draw = child_render_count.get();
        let counters_after_first_draw = cx
            .update_window(any_window, |_, window, _| {
                window.debug_cached_view_counters()
            })
            .unwrap();
        window
            .update(&mut cx, |root, _window, cx| {
                root.label = 1;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        draw_root(&mut cx, any_window);

        assert_eq!(
            child_render_count.get(),
            child_render_count_after_first_draw + 1
        );
        cx.update_window(any_window, |_, window, _| {
            let counters = window.debug_cached_view_counters();
            assert_eq!(
                counters.automatic_hits,
                counters_after_first_draw.automatic_hits
            );
            assert_eq!(
                counters.automatic_misses,
                counters_after_first_draw.automatic_misses + 1
            );
            assert_eq!(
                counters.outer_layout_unknown_misses,
                counters_after_first_draw.outer_layout_unknown_misses + 1
            );
            assert_eq!(window.debug_cached_view_dependency_site_count(), 0);
        })
        .unwrap();
    }

    #[test]
    fn cached_view_automatic_child_drops_previous_state_when_outer_layout_becomes_unknown() {
        let mut cx = TestAppContext::single();
        cx.set_auto_draw_test_windows(false);

        let child_slot = Rc::new(RefCell::new(None));
        let child_render_count = Rc::new(Cell::new(0));
        let observed_values = Rc::new(RefCell::new(Vec::new()));

        let window = cx.add_window({
            let child_slot = child_slot.clone();
            let child_render_count = child_render_count.clone();
            let observed_values = observed_values.clone();

            move |_, cx| {
                let model = cx.new(|_| CachedDependencyModel { value: 0 });
                let child = cx.new(|_| ShapeShiftingDependencyChild {
                    models: vec![model],
                    definite_root: true,
                    render_count: child_render_count,
                    observed_values,
                });
                *child_slot.borrow_mut() = Some(child.clone());

                ShapeShiftingDependencyRoot { child, label: 0 }
            }
        });
        let any_window = window.into();

        draw_root(&mut cx, any_window);
        let child_render_count_after_first_draw = child_render_count.get();
        let observed_values_after_first_draw = observed_values.borrow().clone();
        window
            .update(&mut cx, |root, _window, cx| {
                root.label = 1;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        draw_root(&mut cx, any_window);
        assert_eq!(
            child_render_count.get(),
            child_render_count_after_first_draw
        );
        cx.update_window(any_window, |_, window, _| {
            assert_eq!(window.debug_cached_view_dependency_site_count(), 1);
        })
        .unwrap();

        let child = child_slot.borrow().clone().unwrap();
        child.update(&mut cx, |child, cx| {
            child.definite_root = false;
            cx.notify();
        });
        cx.run_until_parked();
        draw_root(&mut cx, any_window);
        assert_eq!(
            child_render_count.get(),
            child_render_count_after_first_draw + 1
        );
        cx.update_window(any_window, |_, window, _| {
            assert_eq!(window.debug_cached_view_dependency_site_count(), 0);
        })
        .unwrap();

        window
            .update(&mut cx, |root, _window, cx| {
                root.label = 2;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        draw_root(&mut cx, any_window);
        assert_eq!(
            child_render_count.get(),
            child_render_count_after_first_draw + 2
        );
        let mut expected_observed_values = observed_values_after_first_draw;
        expected_observed_values.extend([0, 0]);
        assert_eq!(&*observed_values.borrow(), &expected_observed_values);
        cx.update_window(any_window, |_, window, _| {
            let counters = window.debug_cached_view_counters();
            assert!(counters.outer_layout_unknown_misses >= 1);
            assert_eq!(window.debug_cached_view_dependency_site_count(), 0);
        })
        .unwrap();
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
