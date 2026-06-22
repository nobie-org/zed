use crate::{
    AnyElement, App, Bounds, Composite, CompositeBlendMode, CompositeEffect, ContentLayer, Corners,
    DerivedLayer, Element, ElementId, GlassSurface, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, LayoutId, Pixels, Point, Window, scene::RenderGroupInput,
};
use std::{mem, panic};

/// Creates a render group builder.
///
/// A render group paints its child into an isolated renderer-owned buffer and
/// applies group effects once to the composited result.
#[track_caller]
pub fn render_group() -> RenderGroupBuilder {
    RenderGroupBuilder {
        input: RenderGroupInput::default(),
        source: Some(panic::Location::caller()),
    }
}

/// Builder for a render group before its child has been attached.
pub struct RenderGroupBuilder {
    input: RenderGroupInput,
    source: Option<&'static panic::Location<'static>>,
}

impl RenderGroupBuilder {
    /// Applies one composite effect to the group.
    pub fn effect(mut self, effect: CompositeEffect) -> Self {
        self.input.raw_mut().push(effect);
        self
    }

    /// Applies a sequence of composite effects to the group.
    pub fn effects(mut self, effects: impl IntoIterator<Item = CompositeEffect>) -> Self {
        self.input.raw_mut().extend(effects);
        self
    }

    /// Applies a raw sequence of composite effects to the group.
    pub fn raw_composite_effects(
        mut self,
        effects: impl IntoIterator<Item = CompositeEffect>,
    ) -> Self {
        self.input.raw_mut().extend(effects);
        self
    }

    /// Applies a semantic surface layer to the group.
    pub fn surface(mut self, surface: GlassSurface) -> Self {
        self.input.semantic_mut().surface(surface);
        self
    }

    /// Applies semantic content/source effects to the group.
    pub fn content(mut self, content: ContentLayer) -> Self {
        self.input.semantic_mut().content(content);
        self
    }

    /// Applies a semantic derived layer to the group.
    pub fn derived(mut self, derived: DerivedLayer) -> Self {
        self.input.semantic_mut().derived(derived);
        self
    }

    /// Applies final compositing options to the group.
    pub fn composite(mut self, composite: Composite) -> Self {
        self.input.semantic_mut().composite(composite);
        self
    }

    /// Applies an opacity effect to the composited child.
    pub fn opacity(self, alpha: f32) -> Self {
        self.effect(CompositeEffect::opacity(alpha))
    }

    /// Multiplies the composited source color channels by `factor`.
    pub fn brightness(self, factor: f32) -> Self {
        self.effect(CompositeEffect::brightness(factor))
    }

    /// Scales composited source color distance from mid-gray by `factor`.
    pub fn contrast(self, factor: f32) -> Self {
        self.effect(CompositeEffect::contrast(factor))
    }

    /// Adjusts composited source color saturation by `factor`.
    pub fn saturate(self, factor: f32) -> Self {
        self.effect(CompositeEffect::saturate(factor))
    }

    /// Mixes the composited source color toward grayscale by `amount`.
    pub fn grayscale(self, amount: f32) -> Self {
        self.effect(CompositeEffect::grayscale(amount))
    }

    /// Mixes the composited source color toward its inverse by `amount`.
    pub fn invert(self, amount: f32) -> Self {
        self.effect(CompositeEffect::invert(amount))
    }

    /// Applies an affine source color matrix to unpremultiplied RGB.
    pub fn color_matrix(self, matrix: [[f32; 3]; 3], offset: [f32; 3]) -> Self {
        self.effect(CompositeEffect::color_matrix(matrix, offset))
    }

    /// Multiplies backdrop color channels by `factor`.
    pub fn backdrop_brightness(self, factor: f32) -> Self {
        self.effect(CompositeEffect::backdrop_brightness(factor))
    }

    /// Scales backdrop color distance from mid-gray by `factor`.
    pub fn backdrop_contrast(self, factor: f32) -> Self {
        self.effect(CompositeEffect::backdrop_contrast(factor))
    }

    /// Adjusts backdrop color saturation by `factor`.
    pub fn backdrop_saturate(self, factor: f32) -> Self {
        self.effect(CompositeEffect::backdrop_saturate(factor))
    }

    /// Mixes backdrop color toward grayscale by `amount`.
    pub fn backdrop_grayscale(self, amount: f32) -> Self {
        self.effect(CompositeEffect::backdrop_grayscale(amount))
    }

    /// Mixes backdrop color toward its inverse by `amount`.
    pub fn backdrop_invert(self, amount: f32) -> Self {
        self.effect(CompositeEffect::backdrop_invert(amount))
    }

    /// Applies an affine backdrop color matrix to unpremultiplied RGB.
    pub fn backdrop_color_matrix(self, matrix: [[f32; 3]; 3], offset: [f32; 3]) -> Self {
        self.effect(CompositeEffect::backdrop_color_matrix(matrix, offset))
    }

    /// Applies a Gaussian blur to the composited source image.
    pub fn source_blur(self, radius: Pixels) -> Self {
        self.effect(CompositeEffect::source_blur(radius))
    }

    /// Applies a Gaussian blur to the already-rendered backdrop under the group.
    pub fn backdrop_blur(self, radius: Pixels) -> Self {
        self.effect(CompositeEffect::backdrop_blur(radius))
    }

    /// Refracts already-rendered backdrop pixels through the group's material
    /// shape, with lensing strongest near the group edge.
    pub fn backdrop_lens(
        self,
        refraction_radius: Pixels,
        rim_width: Pixels,
        chromatic_aberration: Pixels,
        highlight_strength: f32,
        shadow_strength: f32,
        light_direction: Point<f32>,
    ) -> Self {
        self.effect(CompositeEffect::backdrop_lens(
            refraction_radius,
            rim_width,
            chromatic_aberration,
            highlight_strength,
            shadow_strength,
            light_direction,
        ))
    }

    /// Draws a translucent tint over the backdrop material under the group.
    pub fn backdrop_tint(self, color: Hsla) -> Self {
        self.effect(CompositeEffect::backdrop_tint(color))
    }

    /// Draws a drop shadow from the composited source image's alpha channel.
    pub fn drop_shadow(self, offset: Point<Pixels>, blur_radius: Pixels, color: Hsla) -> Self {
        self.effect(CompositeEffect::drop_shadow(offset, blur_radius, color))
    }

    /// Masks the composited source image with a rounded rectangle matching the
    /// group layout bounds.
    pub fn rounded_mask(self, corner_radii: Corners<Pixels>) -> Self {
        self.effect(CompositeEffect::rounded_mask(corner_radii))
    }

    /// Applies a final blend mode when compositing the group against its backdrop.
    pub fn blend_mode(self, mode: CompositeBlendMode) -> Self {
        self.effect(CompositeEffect::blend_mode(mode))
    }

    /// Attaches the child element that will be rendered as the group contents.
    pub fn child(self, child: impl IntoElement) -> RenderGroup {
        RenderGroup {
            child: child.into_any_element(),
            input: self.input,
            source: self.source,
        }
    }
}

/// An element that composites one child as an isolated render group.
pub struct RenderGroup {
    child: AnyElement,
    input: RenderGroupInput,
    source: Option<&'static panic::Location<'static>>,
}

impl RenderGroup {
    /// Applies one composite effect to the group.
    pub fn effect(mut self, effect: CompositeEffect) -> Self {
        self.input.raw_mut().push(effect);
        self
    }

    /// Applies a sequence of composite effects to the group.
    pub fn effects(mut self, effects: impl IntoIterator<Item = CompositeEffect>) -> Self {
        self.input.raw_mut().extend(effects);
        self
    }

    /// Applies a raw sequence of composite effects to the group.
    pub fn raw_composite_effects(
        mut self,
        effects: impl IntoIterator<Item = CompositeEffect>,
    ) -> Self {
        self.input.raw_mut().extend(effects);
        self
    }

    /// Applies a semantic surface layer to the group.
    pub fn surface(mut self, surface: GlassSurface) -> Self {
        self.input.semantic_mut().surface(surface);
        self
    }

    /// Applies semantic content/source effects to the group.
    pub fn content(mut self, content: ContentLayer) -> Self {
        self.input.semantic_mut().content(content);
        self
    }

    /// Applies a semantic derived layer to the group.
    pub fn derived(mut self, derived: DerivedLayer) -> Self {
        self.input.semantic_mut().derived(derived);
        self
    }

    /// Applies final compositing options to the group.
    pub fn composite(mut self, composite: Composite) -> Self {
        self.input.semantic_mut().composite(composite);
        self
    }

    /// Applies an opacity effect to the composited child.
    pub fn opacity(self, alpha: f32) -> Self {
        self.effect(CompositeEffect::opacity(alpha))
    }

    /// Multiplies the composited source color channels by `factor`.
    pub fn brightness(self, factor: f32) -> Self {
        self.effect(CompositeEffect::brightness(factor))
    }

    /// Scales composited source color distance from mid-gray by `factor`.
    pub fn contrast(self, factor: f32) -> Self {
        self.effect(CompositeEffect::contrast(factor))
    }

    /// Adjusts composited source color saturation by `factor`.
    pub fn saturate(self, factor: f32) -> Self {
        self.effect(CompositeEffect::saturate(factor))
    }

    /// Mixes the composited source color toward grayscale by `amount`.
    pub fn grayscale(self, amount: f32) -> Self {
        self.effect(CompositeEffect::grayscale(amount))
    }

    /// Mixes the composited source color toward its inverse by `amount`.
    pub fn invert(self, amount: f32) -> Self {
        self.effect(CompositeEffect::invert(amount))
    }

    /// Applies an affine source color matrix to unpremultiplied RGB.
    pub fn color_matrix(self, matrix: [[f32; 3]; 3], offset: [f32; 3]) -> Self {
        self.effect(CompositeEffect::color_matrix(matrix, offset))
    }

    /// Multiplies backdrop color channels by `factor`.
    pub fn backdrop_brightness(self, factor: f32) -> Self {
        self.effect(CompositeEffect::backdrop_brightness(factor))
    }

    /// Scales backdrop color distance from mid-gray by `factor`.
    pub fn backdrop_contrast(self, factor: f32) -> Self {
        self.effect(CompositeEffect::backdrop_contrast(factor))
    }

    /// Adjusts backdrop color saturation by `factor`.
    pub fn backdrop_saturate(self, factor: f32) -> Self {
        self.effect(CompositeEffect::backdrop_saturate(factor))
    }

    /// Mixes backdrop color toward grayscale by `amount`.
    pub fn backdrop_grayscale(self, amount: f32) -> Self {
        self.effect(CompositeEffect::backdrop_grayscale(amount))
    }

    /// Mixes backdrop color toward its inverse by `amount`.
    pub fn backdrop_invert(self, amount: f32) -> Self {
        self.effect(CompositeEffect::backdrop_invert(amount))
    }

    /// Applies an affine backdrop color matrix to unpremultiplied RGB.
    pub fn backdrop_color_matrix(self, matrix: [[f32; 3]; 3], offset: [f32; 3]) -> Self {
        self.effect(CompositeEffect::backdrop_color_matrix(matrix, offset))
    }

    /// Applies a Gaussian blur to the composited source image.
    pub fn source_blur(self, radius: Pixels) -> Self {
        self.effect(CompositeEffect::source_blur(radius))
    }

    /// Applies a Gaussian blur to the already-rendered backdrop under the group.
    pub fn backdrop_blur(self, radius: Pixels) -> Self {
        self.effect(CompositeEffect::backdrop_blur(radius))
    }

    /// Refracts already-rendered backdrop pixels through the group's material
    /// shape, with lensing strongest near the group edge.
    pub fn backdrop_lens(
        self,
        refraction_radius: Pixels,
        rim_width: Pixels,
        chromatic_aberration: Pixels,
        highlight_strength: f32,
        shadow_strength: f32,
        light_direction: Point<f32>,
    ) -> Self {
        self.effect(CompositeEffect::backdrop_lens(
            refraction_radius,
            rim_width,
            chromatic_aberration,
            highlight_strength,
            shadow_strength,
            light_direction,
        ))
    }

    /// Draws a translucent tint over the backdrop material under the group.
    pub fn backdrop_tint(self, color: Hsla) -> Self {
        self.effect(CompositeEffect::backdrop_tint(color))
    }

    /// Draws a drop shadow from the composited source image's alpha channel.
    pub fn drop_shadow(self, offset: Point<Pixels>, blur_radius: Pixels, color: Hsla) -> Self {
        self.effect(CompositeEffect::drop_shadow(offset, blur_radius, color))
    }

    /// Masks the composited source image with a rounded rectangle matching the
    /// group layout bounds.
    pub fn rounded_mask(self, corner_radii: Corners<Pixels>) -> Self {
        self.effect(CompositeEffect::rounded_mask(corner_radii))
    }

    /// Applies a final blend mode when compositing the group against its backdrop.
    pub fn blend_mode(self, mode: CompositeBlendMode) -> Self {
        self.effect(CompositeEffect::blend_mode(mode))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GroupShape;

    #[test]
    fn semantic_layers_reject_raw_effects() {
        let result = panic::catch_unwind(|| {
            let _ = render_group()
                .surface(GlassSurface::for_shape(GroupShape::rectangle()))
                .effect(CompositeEffect::opacity(0.5));
        });

        assert!(result.is_err());
    }

    #[test]
    fn raw_effects_reject_semantic_layers() {
        let result = panic::catch_unwind(|| {
            let _ = render_group()
                .effect(CompositeEffect::opacity(0.5))
                .surface(GlassSurface::for_shape(GroupShape::rectangle()));
        });

        assert!(result.is_err());
    }
}

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
        let input = self.input.clone();
        window.paint_group(bounds, input, |window| {
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
