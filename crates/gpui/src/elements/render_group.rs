use crate::{
    AnyElement, App, Bounds, CompositeEffect, Element, ElementId, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, Window,
};
use std::{mem, panic};

/// Creates a render group builder.
///
/// A render group paints its child into an isolated renderer-owned buffer and
/// applies group effects once to the composited result.
#[track_caller]
pub fn render_group() -> RenderGroupBuilder {
    RenderGroupBuilder {
        effects: Vec::new(),
        source: Some(panic::Location::caller()),
    }
}

/// Builder for a render group before its child has been attached.
pub struct RenderGroupBuilder {
    effects: Vec<CompositeEffect>,
    source: Option<&'static panic::Location<'static>>,
}

impl RenderGroupBuilder {
    /// Applies an opacity effect to the composited child.
    pub fn opacity(mut self, alpha: f32) -> Self {
        self.effects.push(CompositeEffect::opacity(alpha));
        self
    }

    /// Attaches the child element that will be rendered as the group contents.
    pub fn child(self, child: impl IntoElement) -> RenderGroup {
        RenderGroup {
            child: child.into_any_element(),
            effects: self.effects,
            source: self.source,
        }
    }
}

/// An element that composites one child as an isolated render group.
pub struct RenderGroup {
    child: AnyElement,
    effects: Vec<CompositeEffect>,
    source: Option<&'static panic::Location<'static>>,
}

impl RenderGroup {
    /// Applies an opacity effect to the composited child.
    pub fn opacity(mut self, alpha: f32) -> Self {
        self.effects.push(CompositeEffect::opacity(alpha));
        self
    }
}

/// Extension trait for wrapping any element in a render group.
pub trait RenderGroupExt: IntoElement + Sized {
    /// Wraps this element in a render group.
    fn render_group(self) -> RenderGroup {
        render_group().child(self)
    }
}

impl<T: IntoElement> RenderGroupExt for T {}

impl Element for RenderGroup {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        self.source
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        // Cached paint ranges refer to the scene they were recorded into. A
        // render group records into a fresh sub-scene, so descendants must
        // re-enter their normal paint path instead of replaying parent-scene
        // ranges from a previous frame.
        let refreshing = mem::replace(&mut window.refreshing, true);
        self.child.prepaint(window, cx);
        window.refreshing = refreshing;
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let effects = self.effects.clone();
        window.paint_group(bounds, effects, |window| {
            self.child.paint(window, cx);
        });
    }
}

impl IntoElement for RenderGroup {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}
