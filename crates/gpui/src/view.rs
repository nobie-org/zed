use crate::Empty;
use crate::{
    AnyElement, AnyEntity, AnyWeakEntity, App, Bounds, BuildCx, Context, Element, ElementId,
    Entity, EntityId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, LayoutRequestCx,
    PaintCx, PaintIndex, Pixels, PrepaintCx, PrepaintStateIndex, Render, Style, StyleRefinement,
    WeakEntity,
};
use anyhow::Result;
use collections::FxHashSet;
use refineable::Refineable;
use std::rc::Rc;
use std::{any::TypeId, fmt, ops::Range};

struct AnyViewState {
    prepaint_range: Range<PrepaintStateIndex>,
    paint_range: Range<PaintIndex>,
    accessed_entities: FxHashSet<EntityId>,
}

/// A dynamically-typed handle to a view, which can be downcast to a [Entity] for a specific type.
#[derive(Clone, Debug)]
pub struct AnyView {
    entity: AnyEntity,
    render: fn(&AnyView, &mut BuildCx<'_>, &mut App) -> AnyElement,
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
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut LayoutRequestCx<'_>,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        window.with_rendered_view(self.entity_id(), |window| {
            // Disable caching when inspecting so that mouse_hit_test has all hitboxes.
            let caching_disabled = window.is_inspector_picking(cx);
            if let Some(style) = self.cached_style.as_ref()
                && !caching_disabled
            {
                let mut root_style = Style::default();
                root_style.refine(style);
                let cache_can_replay = global_id.is_some_and(|global_id| {
                    window.has_element_state::<AnyViewState>(global_id)
                        && !window.is_view_dirty(self.entity_id())
                        && !window.is_refreshing()
                });

                if cache_can_replay {
                    let layout_id =
                        window.request_layout_with_global_id(global_id, root_style, [], cx);
                    return (layout_id, None);
                }

                let mut element = window.build(|window| (self.render)(self, window, cx));
                let child_layout_id = element.request_layout(window, cx);
                let layout_id = window.request_layout_with_global_id(
                    global_id,
                    root_style,
                    Some(child_layout_id),
                    cx,
                );
                (layout_id, Some(element))
            } else {
                let mut element = window.build(|window| (self.render)(self, window, cx));
                let child_layout_id = element.request_layout(window, cx);
                (child_layout_id, Some(element))
            }
        })
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut PrepaintCx<'_>,
        cx: &mut App,
    ) -> Option<AnyElement> {
        window.set_view_id(self.entity_id());
        window.with_rendered_view(self.entity_id(), |window| {
            if let Some(mut element) = element.take() {
                let caching_disabled = window.is_inspector_picking(cx);
                if self.cached_style.is_some() && !caching_disabled {
                    return window.with_element_state::<AnyViewState, _>(
                        global_id.unwrap(),
                        |_, window| {
                            let prepaint_start = window.prepaint_index();
                            let ((), accessed_entities) = cx.detect_accessed_entities(|cx| {
                                window.with_refreshing(true, |window| {
                                    element.prepaint(window, cx);
                                });
                            });
                            let prepaint_end = window.prepaint_index();

                            (
                                Some(element),
                                AnyViewState {
                                    accessed_entities,
                                    prepaint_range: prepaint_start..prepaint_end,
                                    paint_range: PaintIndex::default()..PaintIndex::default(),
                                },
                            )
                        },
                    );
                } else {
                    element.prepaint(window, cx);
                    return Some(element);
                }
            }

            window.with_element_state::<AnyViewState, _>(
                global_id.unwrap(),
                |element_state, window| {
                    if let Some(mut element_state) = element_state
                        && !window.is_view_dirty(self.entity_id())
                        && !window.is_refreshing()
                    {
                        let prepaint_start = window.prepaint_index();
                        window.reuse_prepaint(element_state.prepaint_range.clone());
                        cx.entities
                            .extend_accessed(&element_state.accessed_entities);
                        let prepaint_end = window.prepaint_index();
                        element_state.prepaint_range = prepaint_start..prepaint_end;

                        return (None, element_state);
                    }

                    panic!("cached view prepaint cache miss without a rendered element")
                },
            )
        })
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        element: &mut Self::PrepaintState,
        window: &mut PaintCx<'_>,
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
                            window.with_refreshing(true, |window| {
                                element.paint(window, cx);
                            });
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
    render: fn(&AnyView, &mut BuildCx<'_>, &mut App) -> AnyElement,
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
    use crate::{AnyElement, AnyView, App, BuildCx, IntoElement, Render};

    pub(crate) fn render<V: 'static + Render>(
        view: &AnyView,
        window: &mut BuildCx<'_>,
        cx: &mut App,
    ) -> AnyElement {
        let view = view.clone().downcast::<V>().unwrap();
        view.update(cx, |view, cx| view.render(window, cx).into_any_element())
    }
}

/// A view that renders nothing
pub struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _window: &mut BuildCx<'_>, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AppContext as _, InteractiveElement as _, Styled as _, TestAppContext, div, px, size,
    };
    use std::ops::Deref;

    struct CachedParentView {
        child: Entity<CachedChildView>,
    }

    impl Render for CachedParentView {
        fn render(
            &mut self,
            _window: &mut BuildCx<'_>,
            _cx: &mut Context<Self>,
        ) -> impl IntoElement {
            AnyView::from(self.child.clone()).cached(StyleRefinement::default())
        }
    }

    struct CachedChildView {
        wide: bool,
    }

    impl Render for CachedChildView {
        fn render(
            &mut self,
            _window: &mut BuildCx<'_>,
            _cx: &mut Context<Self>,
        ) -> impl IntoElement {
            let width = if self.wide { px(80.) } else { px(40.) };
            div().id("cached-child-box").w(width).h(px(20.))
        }
    }

    #[gpui::test]
    fn cached_any_view_wrapper_is_retained_when_child_layout_changes(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(800.), px(600.)), |_, cx| {
            let child = cx.new(|_| CachedChildView { wide: false });
            CachedParentView { child }
        });
        cx.run_until_parked();

        let first_sample = cx
            .update_window(*window.deref(), |_, window, _| {
                window.last_layout_work_sample()
            })
            .unwrap()
            .expect("opening the window should draw once");
        assert!(
            first_sample.retained_layout_creates > 0,
            "the first retained frame should build retained occurrences"
        );

        window
            .update(cx, |parent, _window, cx| {
                parent.child.update(cx, |child, cx| {
                    child.wide = true;
                    cx.notify();
                });
            })
            .unwrap();
        cx.run_until_parked();

        let second_sample = cx
            .update_window(*window.deref(), |_, window, _| {
                window.last_layout_work_sample()
            })
            .unwrap()
            .expect("notifying the child should draw again");

        assert_eq!(
            second_sample.retained_layout_creates, 0,
            "a cached AnyView wrapper with stable view identity must recommit locally, not rebuild"
        );
        assert_eq!(
            second_sample.retained_layout_removes, 0,
            "a child layout change must not detach the cached AnyView wrapper subtree"
        );
        assert_eq!(
            second_sample.retained_layout_miss_no_previous, 0,
            "the cached AnyView wrapper should have a previous retained occurrence"
        );
    }
}
