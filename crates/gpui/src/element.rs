//! Elements are the workhorses of GPUI. They are responsible for laying out and painting all of
//! the contents of a window. Elements form a tree and are laid out according to the web layout
//! standards through GPUI's retained layout engine. Most of the time,
//! you won't need to interact with this module or these APIs directly. Elements provide their
//! own APIs and GPUI, or other element implementation, uses the APIs in this module to convert
//! that element tree into the pixels you see on the screen.
//!
//! # Element Basics
//!
//! Elements are constructed by calling [`Render::render()`] on the root view of the window,
//! which recursively constructs the element tree from the current state of the application,.
//! These elements are then laid out by GPUI, and painted to the screen according to their own
//! implementation of [`Element::paint()`]. Before the start of the next frame, the entire element
//! tree and any callbacks they have registered with GPUI are dropped and the process repeats.
//!
//! But some state is too simple and voluminous to store in every view that needs it, e.g.
//! whether a hover has been started or not. For this, GPUI provides the [`Element::PrepaintState`], associated type.
//!
//! # Implementing your own elements
//!
//! Elements are intended to be the low level, imperative API to GPUI. They are responsible for upholding,
//! or breaking, GPUI's features as they deem necessary. As an example, most GPUI elements are expected
//! to stay in the bounds that their parent element gives them. But with [`Window::with_content_mask`],
//! you can ignore this restriction and paint anywhere inside of the window's bounds. This is useful for overlays
//! and popups and anything else that shows up 'on top' of other elements.
//! With great power, comes great responsibility.
//!
//! However, most of the time, you won't need to implement your own elements. GPUI provides a number of
//! elements that should cover most common use cases out of the box and it's recommended that you use those
//! to construct `components`, using the [`RenderOnce`] trait and the `#[derive(IntoElement)]` macro. Only implement
//! elements when you need to take manual control of the layout and painting process, such as when using
//! your own custom layout algorithm or rendering a code editor.

use crate::{
    App, ArenaBox, AvailableSpace, Bounds, Context, DispatchNodeId, ElementId, FocusHandle,
    InspectorElementId, LayoutId, Pixels, Point, SharedString, Size, Style, Window,
    util::FluentBuilder,
    window::{
        BuildCx, DetachedRootLayoutPass, LayoutRequestCx, PaintCx, PrepaintCx, with_element_arena,
    },
};
use derive_more::{Deref, DerefMut};
use std::{
    any::{Any, type_name},
    fmt::{self, Debug, Display},
    mem, panic,
    sync::Arc,
};

/// Implemented by types that participate in laying out and painting the contents of a window.
/// Elements form a tree and are laid out according to web-based layout rules.
/// You can create custom elements by implementing this trait, see the module-level documentation
/// for more details.
pub trait Element: 'static + IntoElement {
    /// The type of state returned from [`Element::request_layout`]. A mutable reference to this state is subsequently
    /// provided to [`Element::prepaint`] and [`Element::paint`].
    type RequestLayoutState: 'static;

    /// The type of state returned from [`Element::prepaint`]. A mutable reference to this state is subsequently
    /// provided to [`Element::paint`].
    type PrepaintState: 'static;

    /// If this element has a unique identifier, return it here. This is used to track elements across frames, and
    /// will cause a GlobalElementId to be passed to the request_layout, prepaint, and paint methods.
    ///
    /// The global id can in turn be used to access state that's connected to an element with the same id across
    /// frames. This id must be unique among children of the first containing element with an id.
    fn id(&self) -> Option<ElementId>;

    /// Source location where this element was constructed, used to disambiguate elements in the
    /// inspector and navigate to their source code.
    fn source_location(&self) -> Option<&'static panic::Location<'static>>;

    /// Before an element can be painted, we need to know where it's going to be and how big it is.
    /// Use this method to request a layout from GPUI and initialize the element's state.
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut LayoutRequestCx<'_>,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState);

    /// After laying out an element, we need to commit its bounds to the current frame for hitbox
    /// purposes. The state argument is the same state that was returned from [`Element::request_layout()`].
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut PrepaintCx<'_>,
        cx: &mut App,
    ) -> Self::PrepaintState;

    /// Once layout has been completed, this method will be called to paint the element to the screen.
    /// The state argument is the same state that was returned from [`Element::request_layout()`].
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut PaintCx<'_>,
        cx: &mut App,
    );

    /// Convert this element into a dynamically-typed [`AnyElement`].
    fn into_any(self) -> AnyElement {
        AnyElement::new(self)
    }
}

/// Implemented by any type that can be converted into an element.
pub trait IntoElement: Sized {
    /// The specific type of element into which the implementing type is converted.
    /// Useful for converting other types into elements automatically, like Strings
    type Element: Element;

    /// Convert self into a type that implements [`Element`].
    fn into_element(self) -> Self::Element;

    /// Convert self into a dynamically-typed [`AnyElement`].
    fn into_any_element(self) -> AnyElement {
        self.into_element().into_any()
    }
}

impl<T: IntoElement> FluentBuilder for T {}

/// An object that can be drawn to the screen. This is the trait that distinguishes "views" from
/// other entities. Views are `Entity`'s which `impl Render` and drawn to the screen.
pub trait Render: 'static + Sized {
    /// Render this view into an element tree.
    fn render(&mut self, window: &mut BuildCx<'_>, cx: &mut Context<Self>) -> impl IntoElement;
}

impl Render for Empty {
    fn render(&mut self, _window: &mut BuildCx<'_>, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// You can derive [`IntoElement`] on any type that implements this trait.
/// It is used to construct reusable `components` out of plain data. Think of
/// components as a recipe for a certain pattern of elements. RenderOnce allows
/// you to invoke this pattern, without breaking the fluent builder pattern of
/// the element APIs.
pub trait RenderOnce: 'static {
    /// Render this component into an element tree. Note that this method
    /// takes ownership of self, as compared to [`Render::render()`] method
    /// which takes a mutable reference.
    fn render(self, window: &mut BuildCx<'_>, cx: &mut App) -> impl IntoElement;
}

/// This is a helper trait to provide a uniform interface for constructing elements that
/// can accept any number of any kind of child elements
pub trait ParentElement {
    /// Extend this element's children with the given child elements.
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>);

    /// Add a single child element to this element.
    fn child(mut self, child: impl IntoElement) -> Self
    where
        Self: Sized,
    {
        self.extend(std::iter::once(child.into_element().into_any()));
        self
    }

    /// Add multiple child elements to this element.
    fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self
    where
        Self: Sized,
    {
        self.extend(children.into_iter().map(|child| child.into_any_element()));
        self
    }
}

/// An element for rendering components. An implementation detail of the [`IntoElement`] derive macro
/// for [`RenderOnce`]
#[doc(hidden)]
pub struct Component<C: RenderOnce> {
    component: Option<C>,
    #[cfg(debug_assertions)]
    source: &'static core::panic::Location<'static>,
}

impl<C: RenderOnce> Component<C> {
    /// Create a new component from the given RenderOnce type.
    #[track_caller]
    pub fn new(component: C) -> Self {
        Component {
            component: Some(component),
            #[cfg(debug_assertions)]
            source: core::panic::Location::caller(),
        }
    }
}

fn prepaint_component(
    (element, name): &mut (AnyElement, &'static str),
    window: &mut PrepaintCx<'_>,
    cx: &mut App,
) {
    window.with_id(ElementId::Name(SharedString::new_static(name)), |window| {
        element.prepaint(window, cx);
    })
}

fn paint_component(
    (element, name): &mut (AnyElement, &'static str),
    window: &mut PaintCx<'_>,
    cx: &mut App,
) {
    window.with_id(ElementId::Name(SharedString::new_static(name)), |window| {
        element.paint(window, cx);
    })
}
impl<C: RenderOnce> Element for Component<C> {
    type RequestLayoutState = (AnyElement, &'static str);
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        #[cfg(debug_assertions)]
        return Some(self.source);

        #[cfg(not(debug_assertions))]
        return None;
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut LayoutRequestCx<'_>,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        window.with_id(ElementId::Name(type_name::<C>().into()), |window| {
            let mut element = window.build(|window| {
                self.component
                    .take()
                    .unwrap()
                    .render(window, cx)
                    .into_any_element()
            });

            let layout_id = element.request_layout(window, cx);
            (layout_id, (element, type_name::<C>()))
        })
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut PrepaintCx<'_>,
        cx: &mut App,
    ) {
        prepaint_component(state, window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut PaintCx<'_>,
        cx: &mut App,
    ) {
        paint_component(state, window, cx);
    }
}

impl<C: RenderOnce> IntoElement for Component<C> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// A globally unique identifier for an element, used to track state across frames.
#[derive(Deref, DerefMut, Clone, Default, Debug, Eq, PartialEq, Hash)]
pub struct GlobalElementId(pub(crate) Arc<[ElementId]>);

impl Display for GlobalElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, element_id) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{}", element_id)?;
        }
        Ok(())
    }
}

trait ElementObject {
    fn inner_element(&mut self) -> &mut dyn Any;

    fn request_layout(&mut self, window: &mut LayoutRequestCx<'_>, cx: &mut App) -> LayoutId;

    fn prepaint(&mut self, window: &mut PrepaintCx<'_>, cx: &mut App);

    fn paint(&mut self, window: &mut PaintCx<'_>, cx: &mut App);

    fn request_detached_root_layout(
        &mut self,
        available_space: Size<AvailableSpace>,
        pass: &mut DetachedRootLayoutPass,
        window: &mut Window,
        cx: &mut App,
    ) -> DetachedRootLayoutRequest;

    fn mark_detached_root_layout_computed(
        &mut self,
        layout_id: LayoutId,
        available_space: Size<AvailableSpace>,
        pass: &mut DetachedRootLayoutPass,
    );
}

/// A wrapper around an implementer of [`Element`] that allows it to be drawn in a window.
pub struct Drawable<E: Element> {
    /// The drawn element.
    pub element: E,
    phase: ElementDrawPhase<E::RequestLayoutState, E::PrepaintState>,
}

#[derive(Default)]
enum ElementDrawPhase<RequestLayoutState, PrepaintState> {
    #[default]
    Start,
    RequestLayout {
        layout_id: LayoutId,
        owner: LayoutRequestOwner,
        global_id: Option<GlobalElementId>,
        inspector_id: Option<InspectorElementId>,
        request_layout: RequestLayoutState,
    },
    LayoutComputed {
        layout_id: LayoutId,
        global_id: Option<GlobalElementId>,
        inspector_id: Option<InspectorElementId>,
        available_space: Size<AvailableSpace>,
        request_layout: RequestLayoutState,
    },
    Prepaint {
        node_id: DispatchNodeId,
        global_id: Option<GlobalElementId>,
        inspector_id: Option<InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: RequestLayoutState,
        prepaint: PrepaintState,
    },
    Painted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LayoutRequestOwner {
    /// The layout intent was requested as a child of another current-frame intent.
    ///
    /// It may be committed under its parent root, but it is not legal authority
    /// for an independent retained-root solve.
    AttachedChild,
    /// The layout intent was requested specifically for an independent root solve.
    ///
    /// This is used by element probes, tooltips, lists, and other detached
    /// prepaint roots whose geometry is solved outside a parent element.
    DetachedRoot,
}

/// Layout fact requested for a detached root by the frame owner.
///
/// This is not solve authority. It only reports the layout id and retained
/// identity produced by the element's normal `request_layout` path. The
/// private frame owner decides whether and when that root is solved.
pub(crate) struct DetachedRootLayoutRequest {
    layout_id: LayoutId,
    global_id: Option<GlobalElementId>,
    needs_solve: bool,
}

impl DetachedRootLayoutRequest {
    pub(crate) fn layout_id(&self) -> LayoutId {
        self.layout_id
    }

    pub(crate) fn global_id(&self) -> Option<&GlobalElementId> {
        self.global_id.as_ref()
    }

    pub(crate) fn needs_solve(&self) -> bool {
        self.needs_solve
    }
}

/// A wrapper around an implementer of [`Element`] that allows it to be drawn in a window.
impl<E: Element> Drawable<E> {
    pub(crate) fn new(element: E) -> Self {
        Drawable {
            element,
            phase: ElementDrawPhase::Start,
        }
    }

    fn request_layout(&mut self, window: &mut LayoutRequestCx<'_>, cx: &mut App) -> LayoutId {
        self.request_layout_owned_by(LayoutRequestOwner::AttachedChild, window, cx)
    }

    fn request_layout_owned_by(
        &mut self,
        owner: LayoutRequestOwner,
        window: &mut LayoutRequestCx<'_>,
        cx: &mut App,
    ) -> LayoutId {
        match mem::take(&mut self.phase) {
            ElementDrawPhase::Start => {
                let global_id = self
                    .element
                    .id()
                    .map(|element_id| window.push_element_id(element_id));

                let inspector_id;
                #[cfg(any(feature = "inspector", debug_assertions))]
                {
                    inspector_id = self
                        .element
                        .source_location()
                        .map(|source| window.build_inspector_element_id(source));
                }
                #[cfg(not(any(feature = "inspector", debug_assertions)))]
                {
                    inspector_id = None;
                }

                let (layout_id, request_layout) = self.element.request_layout(
                    global_id.as_ref(),
                    inspector_id.as_ref(),
                    window,
                    cx,
                );

                if global_id.is_some() {
                    window.pop_element_id();
                }

                self.phase = ElementDrawPhase::RequestLayout {
                    layout_id,
                    owner,
                    global_id,
                    inspector_id,
                    request_layout,
                };
                layout_id
            }
            _ => panic!("must call request_layout only once"),
        }
    }

    pub(crate) fn prepaint(&mut self, window: &mut PrepaintCx<'_>, cx: &mut App) {
        match mem::take(&mut self.phase) {
            ElementDrawPhase::RequestLayout {
                layout_id,
                owner: _,
                global_id,
                inspector_id,
                mut request_layout,
            }
            | ElementDrawPhase::LayoutComputed {
                layout_id,
                global_id,
                inspector_id,
                mut request_layout,
                ..
            } => {
                if let Some(element_id) = self.element.id() {
                    let current_global_id = window.push_element_id(element_id);
                    debug_assert_eq!(&*global_id.as_ref().unwrap().0, &*current_global_id.0);
                }

                let bounds = window.layout_bounds(layout_id);
                let node_id = window.push_dispatch_node();
                let prepaint = self.element.prepaint(
                    global_id.as_ref(),
                    inspector_id.as_ref(),
                    bounds,
                    &mut request_layout,
                    window,
                    cx,
                );
                window.pop_dispatch_node();

                if global_id.is_some() {
                    window.pop_element_id();
                }

                self.phase = ElementDrawPhase::Prepaint {
                    node_id,
                    global_id,
                    inspector_id,
                    bounds,
                    request_layout,
                    prepaint,
                };
            }
            _ => panic!("must call request_layout before prepaint"),
        }
    }

    pub(crate) fn paint(
        &mut self,
        window: &mut PaintCx<'_>,
        cx: &mut App,
    ) -> (E::RequestLayoutState, E::PrepaintState) {
        match mem::take(&mut self.phase) {
            ElementDrawPhase::Prepaint {
                node_id,
                global_id,
                inspector_id,
                bounds,
                mut request_layout,
                mut prepaint,
                ..
            } => {
                if let Some(element_id) = self.element.id() {
                    let current_global_id = window.push_element_id(element_id);
                    debug_assert_eq!(&*global_id.as_ref().unwrap().0, &*current_global_id.0);
                }

                window.set_active_dispatch_node(node_id);
                self.element.paint(
                    global_id.as_ref(),
                    inspector_id.as_ref(),
                    bounds,
                    &mut request_layout,
                    &mut prepaint,
                    window,
                    cx,
                );

                if global_id.is_some() {
                    window.pop_element_id();
                }

                self.phase = ElementDrawPhase::Painted;
                (request_layout, prepaint)
            }
            _ => panic!("must call prepaint before paint"),
        }
    }

    pub(crate) fn request_detached_root_layout(
        &mut self,
        available_space: Size<AvailableSpace>,
        _pass: &mut DetachedRootLayoutPass,
        window: &mut Window,
        cx: &mut App,
    ) -> DetachedRootLayoutRequest {
        if matches!(&self.phase, ElementDrawPhase::Start) {
            let mut layout_cx = LayoutRequestCx::new(window);
            self.request_layout_owned_by(LayoutRequestOwner::DetachedRoot, &mut layout_cx, cx);
        }

        match &self.phase {
            ElementDrawPhase::RequestLayout {
                layout_id,
                owner,
                global_id,
                ..
            } => {
                assert_eq!(
                    *owner,
                    LayoutRequestOwner::DetachedRoot,
                    "attached layout requests cannot be computed as detached roots"
                );
                DetachedRootLayoutRequest {
                    layout_id: *layout_id,
                    global_id: global_id.clone(),
                    needs_solve: true,
                }
            }
            ElementDrawPhase::LayoutComputed {
                layout_id,
                available_space: prev_available_space,
                ..
            } => {
                assert_eq!(
                    available_space, *prev_available_space,
                    "cannot compute one layout root with two available-space values in one frame"
                );
                DetachedRootLayoutRequest {
                    layout_id: *layout_id,
                    global_id: None,
                    needs_solve: false,
                }
            }
            _ => panic!("cannot layout detached root after prepaint"),
        }
    }

    pub(crate) fn mark_detached_root_layout_computed(
        &mut self,
        computed_layout_id: LayoutId,
        available_space: Size<AvailableSpace>,
        _pass: &mut DetachedRootLayoutPass,
    ) {
        match mem::take(&mut self.phase) {
            ElementDrawPhase::RequestLayout {
                layout_id,
                owner,
                global_id,
                inspector_id,
                request_layout,
            } => {
                assert_eq!(
                    owner,
                    LayoutRequestOwner::DetachedRoot,
                    "attached layout requests cannot be marked as detached-root layouts"
                );
                assert_eq!(
                    layout_id, computed_layout_id,
                    "detached root layout id must match the solved retained root"
                );
                self.phase = ElementDrawPhase::LayoutComputed {
                    layout_id,
                    global_id,
                    inspector_id,
                    available_space,
                    request_layout,
                };
            }
            _ => panic!("must request detached-root layout before marking it computed"),
        }
    }
}

impl<E> ElementObject for Drawable<E>
where
    E: Element,
    E::RequestLayoutState: 'static,
{
    fn inner_element(&mut self) -> &mut dyn Any {
        &mut self.element
    }

    #[inline]
    fn request_layout(&mut self, window: &mut LayoutRequestCx<'_>, cx: &mut App) -> LayoutId {
        Drawable::request_layout(self, window, cx)
    }

    #[inline]
    fn prepaint(&mut self, window: &mut PrepaintCx<'_>, cx: &mut App) {
        Drawable::prepaint(self, window, cx);
    }

    #[inline]
    fn paint(&mut self, window: &mut PaintCx<'_>, cx: &mut App) {
        Drawable::paint(self, window, cx);
    }

    #[inline]
    fn request_detached_root_layout(
        &mut self,
        available_space: Size<AvailableSpace>,
        pass: &mut DetachedRootLayoutPass,
        window: &mut Window,
        cx: &mut App,
    ) -> DetachedRootLayoutRequest {
        Drawable::request_detached_root_layout(self, available_space, pass, window, cx)
    }

    #[inline]
    fn mark_detached_root_layout_computed(
        &mut self,
        layout_id: LayoutId,
        available_space: Size<AvailableSpace>,
        pass: &mut DetachedRootLayoutPass,
    ) {
        Drawable::mark_detached_root_layout_computed(self, layout_id, available_space, pass)
    }
}

/// A dynamically typed element that can be used to store any element type.
pub struct AnyElement(ArenaBox<dyn ElementObject>);

impl AnyElement {
    pub(crate) fn new<E>(element: E) -> Self
    where
        E: 'static + Element,
        E::RequestLayoutState: Any,
    {
        let element = with_element_arena(|arena| arena.alloc(|| Drawable::new(element)))
            .map(|element| element as &mut dyn ElementObject);
        AnyElement(element)
    }

    /// Attempt to downcast a reference to the boxed element to a specific type.
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.0.inner_element().downcast_mut::<T>()
    }

    /// Request the layout ID of the element stored in this `AnyElement`.
    /// Used for laying out child elements in a parent element.
    pub fn request_layout(&mut self, window: &mut LayoutRequestCx<'_>, cx: &mut App) -> LayoutId {
        self.0.request_layout(window, cx)
    }

    /// Prepares the element to be painted by storing its bounds, giving it a chance to draw hitboxes and
    /// request autoscroll before the final paint pass is confirmed.
    pub fn prepaint(&mut self, window: &mut PrepaintCx<'_>, cx: &mut App) -> Option<FocusHandle> {
        let focus_assigned = window.focus_is_assigned();

        self.0.prepaint(window, cx);

        window.focus_assigned_since(focus_assigned, cx)
    }

    /// Paints the element stored in this `AnyElement`.
    pub fn paint(&mut self, window: &mut PaintCx<'_>, cx: &mut App) {
        self.0.paint(window, cx);
    }

    pub(crate) fn request_detached_root_layout(
        &mut self,
        available_space: Size<AvailableSpace>,
        pass: &mut DetachedRootLayoutPass,
        window: &mut Window,
        cx: &mut App,
    ) -> DetachedRootLayoutRequest {
        self.0
            .request_detached_root_layout(available_space, pass, window, cx)
    }

    pub(crate) fn mark_detached_root_layout_computed(
        &mut self,
        layout_id: LayoutId,
        available_space: Size<AvailableSpace>,
        pass: &mut DetachedRootLayoutPass,
    ) {
        self.0
            .mark_detached_root_layout_computed(layout_id, available_space, pass)
    }

    /// Prepaints this element at the given absolute origin.
    /// If any element in the subtree beneath this element is focused, its FocusHandle is returned.
    pub(crate) fn prepaint_at(
        &mut self,
        origin: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<FocusHandle> {
        window.with_absolute_element_offset(origin, |window| {
            let mut prepaint_cx = PrepaintCx::new(window);
            self.prepaint(&mut prepaint_cx, cx)
        })
    }
}

impl Element for AnyElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut LayoutRequestCx<'_>,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.request_layout(window, cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut PrepaintCx<'_>,
        cx: &mut App,
    ) {
        self.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut PaintCx<'_>,
        cx: &mut App,
    ) {
        self.paint(window, cx);
    }
}

impl IntoElement for AnyElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }

    fn into_any_element(self) -> AnyElement {
        self
    }
}

/// The empty element, which renders nothing.
pub struct Empty;

impl IntoElement for Empty {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Empty {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut LayoutRequestCx<'_>,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (
            window.request_layout(
                Style {
                    display: crate::Display::None,
                    ..Default::default()
                },
                None,
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut PrepaintCx<'_>,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut PaintCx<'_>,
        _cx: &mut App,
    ) {
    }
}
