use crate::{
    AnyElement, App, Bounds, CompositeEffect, Corners, Element, ElementId, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, Window,
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
    /// Applies one composite effect to the group.
    pub fn effect(mut self, effect: CompositeEffect) -> Self {
        self.effects.push(effect);
        self
    }

    /// Applies a sequence of composite effects to the group.
    pub fn effects(mut self, effects: impl IntoIterator<Item = CompositeEffect>) -> Self {
        self.effects.extend(effects);
        self
    }

    /// Applies an opacity effect to the composited child.
    pub fn opacity(mut self, alpha: f32) -> Self {
        self.effects.push(CompositeEffect::opacity(alpha));
        self
    }

    /// Multiplies the composited source color channels by `factor`.
    pub fn brightness(mut self, factor: f32) -> Self {
        self.effects.push(CompositeEffect::brightness(factor));
        self
    }

    /// Scales composited source color distance from mid-gray by `factor`.
    pub fn contrast(mut self, factor: f32) -> Self {
        self.effects.push(CompositeEffect::contrast(factor));
        self
    }

    /// Adjusts composited source color saturation by `factor`.
    pub fn saturate(mut self, factor: f32) -> Self {
        self.effects.push(CompositeEffect::saturate(factor));
        self
    }

    /// Mixes the composited source color toward grayscale by `amount`.
    pub fn grayscale(mut self, amount: f32) -> Self {
        self.effects.push(CompositeEffect::grayscale(amount));
        self
    }

    /// Mixes the composited source color toward its inverse by `amount`.
    pub fn invert(mut self, amount: f32) -> Self {
        self.effects.push(CompositeEffect::invert(amount));
        self
    }

    /// Applies an affine source color matrix to unpremultiplied RGB.
    pub fn color_matrix(mut self, matrix: [[f32; 3]; 3], offset: [f32; 3]) -> Self {
        self.effects
            .push(CompositeEffect::color_matrix(matrix, offset));
        self
    }

    /// Applies a Gaussian blur to the composited source image.
    pub fn source_blur(mut self, radius: Pixels) -> Self {
        self.effects.push(CompositeEffect::source_blur(radius));
        self
    }

    /// Draws a drop shadow from the composited source image's alpha channel.
    pub fn drop_shadow(mut self, offset: Point<Pixels>, blur_radius: Pixels, color: Hsla) -> Self {
        self.effects
            .push(CompositeEffect::drop_shadow(offset, blur_radius, color));
        self
    }

    /// Masks the composited source image with a rounded rectangle matching the
    /// group layout bounds.
    pub fn rounded_mask(mut self, corner_radii: Corners<Pixels>) -> Self {
        self.effects
            .push(CompositeEffect::rounded_mask(corner_radii));
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
    /// Applies one composite effect to the group.
    pub fn effect(mut self, effect: CompositeEffect) -> Self {
        self.effects.push(effect);
        self
    }

    /// Applies a sequence of composite effects to the group.
    pub fn effects(mut self, effects: impl IntoIterator<Item = CompositeEffect>) -> Self {
        self.effects.extend(effects);
        self
    }

    /// Applies an opacity effect to the composited child.
    pub fn opacity(mut self, alpha: f32) -> Self {
        self.effects.push(CompositeEffect::opacity(alpha));
        self
    }

    /// Multiplies the composited source color channels by `factor`.
    pub fn brightness(mut self, factor: f32) -> Self {
        self.effects.push(CompositeEffect::brightness(factor));
        self
    }

    /// Scales composited source color distance from mid-gray by `factor`.
    pub fn contrast(mut self, factor: f32) -> Self {
        self.effects.push(CompositeEffect::contrast(factor));
        self
    }

    /// Adjusts composited source color saturation by `factor`.
    pub fn saturate(mut self, factor: f32) -> Self {
        self.effects.push(CompositeEffect::saturate(factor));
        self
    }

    /// Mixes the composited source color toward grayscale by `amount`.
    pub fn grayscale(mut self, amount: f32) -> Self {
        self.effects.push(CompositeEffect::grayscale(amount));
        self
    }

    /// Mixes the composited source color toward its inverse by `amount`.
    pub fn invert(mut self, amount: f32) -> Self {
        self.effects.push(CompositeEffect::invert(amount));
        self
    }

    /// Applies an affine source color matrix to unpremultiplied RGB.
    pub fn color_matrix(mut self, matrix: [[f32; 3]; 3], offset: [f32; 3]) -> Self {
        self.effects
            .push(CompositeEffect::color_matrix(matrix, offset));
        self
    }

    /// Applies a Gaussian blur to the composited source image.
    pub fn source_blur(mut self, radius: Pixels) -> Self {
        self.effects.push(CompositeEffect::source_blur(radius));
        self
    }

    /// Draws a drop shadow from the composited source image's alpha channel.
    pub fn drop_shadow(mut self, offset: Point<Pixels>, blur_radius: Pixels, color: Hsla) -> Self {
        self.effects
            .push(CompositeEffect::drop_shadow(offset, blur_radius, color));
        self
    }

    /// Masks the composited source image with a rounded rectangle matching the
    /// group layout bounds.
    pub fn rounded_mask(mut self, corner_radii: Corners<Pixels>) -> Self {
        self.effects
            .push(CompositeEffect::rounded_mask(corner_radii));
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
