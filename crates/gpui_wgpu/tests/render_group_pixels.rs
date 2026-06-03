#![cfg(all(not(target_family = "wasm"), feature = "test-support"))]

use std::borrow::Cow;
use std::sync::Arc;

use gpui::{
    AtlasKey, AtlasTile, Background, BorderStyle, Bounds, CompositeBlendMode, CompositeEffect,
    ContentMask, Corners, DerivedStage, DevicePixels, Edges, Glow, GroupShape, Hsla, ImageId,
    LogicalVisualPlan, LumaThreshold, MonochromeSprite, PaintGroup, PlatformAtlas,
    PlatformHeadlessRenderer, PolychromeSprite, Quad, RenderGroupBackendCounters,
    RenderImageParams, RenderSvgParams, ScaledPixels, Scene, TransformationMatrix, point, px, rgba,
    size, transparent_black,
};
use gpui_wgpu::WgpuHeadlessRenderer;
use image::RgbaImage;

const IMAGE_SIZE: i32 = 32;

#[derive(Debug, PartialEq, Eq)]
struct Samples {
    background: [u8; 4],
    single_coverage: [u8; 4],
    double_coverage: [u8; 4],
}

fn sp(value: f32) -> ScaledPixels {
    ScaledPixels(value)
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds::new(point(sp(x), sp(y)), size(sp(width), sp(height)))
}

fn viewport() -> Bounds<ScaledPixels> {
    rect(0., 0., IMAGE_SIZE as f32, IMAGE_SIZE as f32)
}

fn mask() -> ContentMask<ScaledPixels> {
    ContentMask { bounds: viewport() }
}

fn quad(order: u32, bounds: Bounds<ScaledPixels>, background: impl Into<Background>) -> Quad {
    Quad {
        order,
        border_style: BorderStyle::Solid,
        bounds,
        content_mask: mask(),
        background: background.into(),
        border_color: transparent_black(),
        corner_radii: Corners::all(sp(0.)),
        border_widths: Edges::all(sp(0.)),
    }
}

fn paint_group(
    order: u32,
    capture_bounds: Bounds<ScaledPixels>,
    opacity: f32,
    scene: Scene,
) -> PaintGroup {
    paint_group_with_effects(
        order,
        capture_bounds,
        vec![CompositeEffect::opacity(opacity)],
        scene,
    )
}

fn paint_group_with_effects(
    order: u32,
    capture_bounds: Bounds<ScaledPixels>,
    effects: Vec<CompositeEffect>,
    scene: Scene,
) -> PaintGroup {
    PaintGroup {
        order,
        bounds: capture_bounds,
        capture_bounds,
        content_mask: mask(),
        scale_factor: 1.,
        plan: LogicalVisualPlan::from_effects(1., 1., effects),
        scene: Arc::new(scene),
    }
}

fn paint_group_with_bounds(
    order: u32,
    bounds: Bounds<ScaledPixels>,
    capture_bounds: Bounds<ScaledPixels>,
    effects: Vec<CompositeEffect>,
    scene: Scene,
) -> PaintGroup {
    PaintGroup {
        order,
        bounds,
        capture_bounds,
        content_mask: mask(),
        scale_factor: 1.,
        plan: LogicalVisualPlan::from_effects(1., 1., effects),
        scene: Arc::new(scene),
    }
}

fn black() -> Hsla {
    rgba(0x000000ff).into()
}

fn white() -> Hsla {
    rgba(0xffffffff).into()
}

fn half_white() -> Hsla {
    rgba(0xffffff80).into()
}

fn green() -> Hsla {
    rgba(0x00ff00ff).into()
}

fn red_half() -> Hsla {
    rgba(0xff000080).into()
}

fn red() -> Hsla {
    rgba(0xff0000ff).into()
}

fn blue_half() -> Hsla {
    rgba(0x0000ff80).into()
}

fn gray() -> Hsla {
    rgba(0x808080ff).into()
}

fn gray_byte(value: u8) -> Hsla {
    let value = u32::from(value);
    rgba((value << 24) | (value << 16) | (value << 8) | 0xff).into()
}

fn finished_scene(primitives: impl IntoIterator<Item = Quad>) -> Scene {
    let mut scene = Scene::default();
    for primitive in primitives {
        scene.insert_primitive(primitive);
    }
    scene.finish();
    scene
}

fn render(scene: &Scene) -> RgbaImage {
    let mut renderer = WgpuHeadlessRenderer::new().expect("create headless renderer");
    renderer
        .render_scene_to_image(
            scene,
            size(DevicePixels(IMAGE_SIZE), DevicePixels(IMAGE_SIZE)),
        )
        .expect("render scene")
}

/// Render `scene` headlessly and return the backend's measured render-group counters.
fn backend_counters(scene: &Scene) -> RenderGroupBackendCounters {
    let mut renderer = WgpuHeadlessRenderer::new().expect("create headless renderer");
    renderer
        .render_scene_to_image(
            scene,
            size(DevicePixels(IMAGE_SIZE), DevicePixels(IMAGE_SIZE)),
        )
        .expect("render scene");
    renderer
        .render_group_backend_counters()
        .expect("counters recorded after a successful render")
}

/// The planner's predicted intermediate-texture / backdrop-copy counts for one group.
fn predicted_counters(group: &PaintGroup) -> RenderGroupBackendCounters {
    let counters = group.plan.support_counters(group.capture_bounds);
    RenderGroupBackendCounters {
        intermediate_textures: counters.intermediate_textures,
        backdrop_copies: counters.backdrop_copies,
    }
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    image.get_pixel(x, y).0
}

fn samples(image: &RgbaImage) -> Samples {
    Samples {
        background: pixel(image, 1, 1),
        single_coverage: pixel(image, 6, 6),
        double_coverage: pixel(image, 12, 12),
    }
}

#[test]
fn identity_render_group_matches_inline_rendering() {
    let inline = finished_scene([
        quad(0, viewport(), black()),
        quad(1, rect(4., 4., 16., 16.), red_half()),
        quad(2, rect(10., 10., 16., 16.), blue_half()),
    ]);

    let group_scene = finished_scene([
        quad(0, rect(4., 4., 16., 16.), red_half()),
        quad(1, rect(10., 10., 16., 16.), blue_half()),
    ]);
    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group(1, rect(4., 4., 22., 22.), 1., group_scene));
    grouped.finish();

    let inline_image = render(&inline);
    let grouped_image = render(&grouped);

    assert_eq!(grouped_image.as_raw(), inline_image.as_raw());
}

// --- M2b: measured backend render-group counters match the planner ---
//
// The backend allocates one group intermediate per rendered group, plus a backdrop
// copy target and one blit when the group reads the backdrop. These tests prove the
// measured counts equal the planner's predicted RenderGroupSupportCounters for
// groups both sides agree are rendered. The law is asserted only on `renders_pixels`
// groups (or fully elided opacity<=0 groups): the backend draws degenerate groups
// the planner elides (identity opacity, opacity in (0, EPSILON], empty bounds), so
// those are intentionally out of scope here.

#[test]
fn backend_counters_match_planner_for_opacity_group() {
    let group = paint_group(
        1,
        rect(4., 4., 16., 16.),
        0.5,
        finished_scene([quad(0, rect(4., 4., 16., 16.), red_half())]),
    );
    let predicted = predicted_counters(&group);
    assert_eq!(
        predicted,
        RenderGroupBackendCounters {
            intermediate_textures: 1,
            backdrop_copies: 0,
        }
    );

    let mut scene = Scene::default();
    scene.insert_primitive(quad(0, viewport(), black()));
    scene.insert_primitive(group);
    scene.finish();

    assert_eq!(backend_counters(&scene), predicted);
}

#[test]
fn backend_counters_match_planner_for_backdrop_blur_group() {
    let group = paint_group_with_effects(
        1,
        rect(4., 4., 16., 16.),
        vec![CompositeEffect::backdrop_blur(px(4.))],
        finished_scene([quad(0, rect(6., 6., 8., 8.), red_half())]),
    );
    let predicted = predicted_counters(&group);
    assert_eq!(
        predicted,
        RenderGroupBackendCounters {
            intermediate_textures: 2,
            backdrop_copies: 1,
        }
    );

    let mut scene = Scene::default();
    scene.insert_primitive(quad(0, viewport(), black()));
    scene.insert_primitive(group);
    scene.finish();

    assert_eq!(backend_counters(&scene), predicted);
}

#[test]
fn backend_counters_sum_across_sibling_groups() {
    let opacity_group = paint_group(
        1,
        rect(2., 2., 10., 10.),
        0.5,
        finished_scene([quad(0, rect(2., 2., 10., 10.), red_half())]),
    );
    let backdrop_group = paint_group_with_effects(
        2,
        rect(14., 14., 12., 12.),
        vec![CompositeEffect::backdrop_blur(px(4.))],
        finished_scene([quad(0, rect(16., 16., 6., 6.), blue_half())]),
    );
    let predicted = RenderGroupBackendCounters {
        intermediate_textures: predicted_counters(&opacity_group).intermediate_textures
            + predicted_counters(&backdrop_group).intermediate_textures,
        backdrop_copies: predicted_counters(&opacity_group).backdrop_copies
            + predicted_counters(&backdrop_group).backdrop_copies,
    };
    assert_eq!(
        predicted,
        RenderGroupBackendCounters {
            intermediate_textures: 3,
            backdrop_copies: 1,
        }
    );

    let mut scene = Scene::default();
    scene.insert_primitive(quad(0, viewport(), black()));
    scene.insert_primitive(opacity_group);
    scene.insert_primitive(backdrop_group);
    scene.finish();

    assert_eq!(backend_counters(&scene), predicted);
}

#[test]
fn backend_counters_zero_for_elided_opacity_zero_group() {
    let group = paint_group(
        1,
        rect(4., 4., 16., 16.),
        0.0,
        finished_scene([quad(0, rect(4., 4., 16., 16.), red_half())]),
    );
    let predicted = predicted_counters(&group);
    assert_eq!(predicted, RenderGroupBackendCounters::default());

    let mut scene = Scene::default();
    scene.insert_primitive(quad(0, viewport(), black()));
    scene.insert_primitive(group);
    scene.finish();

    assert_eq!(
        backend_counters(&scene),
        RenderGroupBackendCounters::default()
    );
}

#[test]
fn backend_counters_match_planner_for_nested_groups() {
    let inner = paint_group_with_effects(
        0,
        rect(6., 6., 12., 12.),
        vec![CompositeEffect::backdrop_blur(px(4.))],
        finished_scene([quad(0, rect(8., 8., 6., 6.), blue_half())]),
    );
    let inner_predicted = predicted_counters(&inner);

    let mut inner_scene = Scene::default();
    inner_scene.insert_primitive(quad(0, rect(4., 4., 20., 20.), white()));
    inner_scene.insert_primitive(inner);
    inner_scene.finish();

    let outer = paint_group(1, rect(4., 4., 20., 20.), 0.5, inner_scene);
    let outer_predicted = predicted_counters(&outer);

    let predicted = RenderGroupBackendCounters {
        intermediate_textures: outer_predicted.intermediate_textures
            + inner_predicted.intermediate_textures,
        backdrop_copies: outer_predicted.backdrop_copies + inner_predicted.backdrop_copies,
    };
    assert_eq!(
        predicted,
        RenderGroupBackendCounters {
            intermediate_textures: 3,
            backdrop_copies: 1,
        }
    );

    let mut scene = Scene::default();
    scene.insert_primitive(quad(0, viewport(), black()));
    scene.insert_primitive(outer);
    scene.finish();

    assert_eq!(backend_counters(&scene), predicted);
}

#[test]
fn render_group_opacity_applies_after_overlapping_children_are_composited() {
    let mut group_scene = Scene::default();
    group_scene.insert_primitive(quad(0, rect(4., 4., 20., 20.), white()));
    group_scene.insert_primitive(quad(1, rect(10., 10., 8., 8.), white()));
    group_scene.finish();

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group(1, rect(4., 4., 20., 20.), 0.5, group_scene));
    grouped.finish();

    assert_eq!(
        samples(&render(&grouped)),
        Samples {
            background: [0, 0, 0, 255],
            single_coverage: [128, 128, 128, 255],
            double_coverage: [128, 128, 128, 255],
        }
    );
}

#[test]
fn per_primitive_opacity_accumulates_in_overlapping_regions() {
    let scene = finished_scene([
        quad(0, viewport(), black()),
        quad(1, rect(4., 4., 20., 20.), half_white()),
        quad(2, rect(10., 10., 8., 8.), half_white()),
    ]);

    assert_eq!(
        samples(&render(&scene)),
        Samples {
            background: [0, 0, 0, 255],
            single_coverage: [128, 128, 128, 255],
            double_coverage: [192, 192, 192, 255],
        }
    );
}

#[test]
fn nested_render_group_opacities_multiply_at_group_boundaries() {
    let inner_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), white())]);

    let mut middle_scene = Scene::default();
    middle_scene.insert_primitive(paint_group(0, rect(8., 8., 8., 8.), 0.5, inner_scene));
    middle_scene.finish();

    let mut outer = Scene::default();
    outer.insert_primitive(quad(0, viewport(), black()));
    outer.insert_primitive(paint_group(1, rect(8., 8., 8., 8.), 0.5, middle_scene));
    outer.finish();

    assert_eq!(pixel(&render(&outer), 12, 12), [64, 64, 64, 255]);
}

#[test]
fn render_group_source_color_filter_applies_to_composited_source() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), red())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![CompositeEffect::invert(1.)],
        group_scene,
    ));
    grouped.finish();

    assert_eq!(pixel(&render(&grouped), 12, 12), [0, 255, 255, 255]);
}

#[test]
fn render_group_source_color_matrix_applies_to_composited_source() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), red())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![CompositeEffect::color_matrix(
            [[0., 0., 0.], [1., 0., 0.], [0., 0., 0.]],
            [0., 0., 0.],
        )],
        group_scene,
    ));
    grouped.finish();

    assert_eq!(pixel(&render(&grouped), 12, 12), [0, 255, 0, 255]);
}

#[test]
fn render_group_source_color_filter_preserves_source_alpha() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), red_half())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![CompositeEffect::invert(1.)],
        group_scene,
    ));
    grouped.finish();

    assert_eq!(pixel(&render(&grouped), 12, 12), [0, 128, 128, 255]);
}

#[test]
fn render_group_hue_rotate_zero_is_identity() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), red())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![CompositeEffect::hue_rotate(0.)],
        group_scene,
    ));
    grouped.finish();

    assert_eq!(pixel(&render(&grouped), 12, 12), [255, 0, 0, 255]);
}

#[test]
fn render_group_hue_rotate_keeps_gray_fixed() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), gray())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![CompositeEffect::hue_rotate(120.)],
        group_scene,
    ));
    grouped.finish();

    // Neutral gray lies on the hue-rotation axis, so it is a fixed point.
    assert_eq!(pixel(&render(&grouped), 12, 12), [128, 128, 128, 255]);
}

#[test]
fn render_group_sepia_tones_source_red() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), red())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![CompositeEffect::sepia()],
        group_scene,
    ));
    grouped.finish();

    assert_eq!(pixel(&render(&grouped), 12, 12), [100, 89, 69, 255]);
}

#[test]
fn render_group_sepia_warm_biases_source_gray() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), gray())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![CompositeEffect::sepia()],
        group_scene,
    ));
    grouped.finish();

    let [r, g, b, a] = pixel(&render(&grouped), 12, 12);
    // Sepia warms neutral gray into a R > G > B ramp, preserving opacity.
    assert!(r > g && g > b, "expected warm sepia ramp, got {:?}", [r, g, b, a]);
    assert_eq!([r, g, b, a], [173, 154, 120, 255]);
}

#[test]
fn render_group_source_color_filters_are_ordered() {
    let first_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), red())]);
    let second_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), red())]);

    let mut brightness_then_invert = Scene::default();
    brightness_then_invert.insert_primitive(quad(0, viewport(), black()));
    brightness_then_invert.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![
            CompositeEffect::brightness(0.5),
            CompositeEffect::invert(1.),
        ],
        first_scene,
    ));
    brightness_then_invert.finish();

    let mut invert_then_brightness = Scene::default();
    invert_then_brightness.insert_primitive(quad(0, viewport(), black()));
    invert_then_brightness.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![
            CompositeEffect::invert(1.),
            CompositeEffect::brightness(0.5),
        ],
        second_scene,
    ));
    invert_then_brightness.finish();

    assert_eq!(
        pixel(&render(&brightness_then_invert), 12, 12),
        [128, 255, 255, 255]
    );
    assert_eq!(
        pixel(&render(&invert_then_brightness), 12, 12),
        [0, 128, 128, 255]
    );
}

#[test]
fn render_group_source_blur_samples_composited_source() {
    let group_scene = finished_scene([quad(0, rect(12., 12., 8., 8.), white())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(6., 6., 20., 20.),
        vec![CompositeEffect::source_blur(px(2.))],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let center = pixel(&image, 16, 16);
    let outside = pixel(&image, 10, 16);

    assert!(
        center[0] > outside[0],
        "center {center:?} outside {outside:?}"
    );
    assert!(
        outside[0] > 0,
        "blur should extend source into expanded capture bounds"
    );
}

#[test]
fn render_group_drop_shadow_uses_composited_source_alpha() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), red())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 14., 8.),
        vec![CompositeEffect::drop_shadow(
            point(px(6.), px(0.)),
            px(0.),
            half_white(),
        )],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    assert_eq!(pixel(&image, 12, 12), [255, 0, 0, 255]);
    assert_eq!(pixel(&image, 19, 12), [128, 128, 128, 255]);
}

#[test]
fn render_group_surface_shadow_uses_material_shape_without_source_alpha() {
    let mut source_alpha_shadow = Scene::default();
    source_alpha_shadow.insert_primitive(quad(0, viewport(), black()));
    source_alpha_shadow.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 14., 8.),
        vec![CompositeEffect::drop_shadow(
            point(px(6.), px(0.)),
            px(0.),
            half_white(),
        )],
        Scene::default(),
    ));
    source_alpha_shadow.finish();

    let mut surface_shadow = Scene::default();
    surface_shadow.insert_primitive(quad(0, viewport(), black()));
    surface_shadow.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 14., 8.),
        vec![CompositeEffect::surface_shadow(
            GroupShape::rectangle(),
            point(px(6.), px(0.)),
            px(0.),
            half_white(),
        )],
        Scene::default(),
    ));
    surface_shadow.finish();

    let source_alpha_image = render(&source_alpha_shadow);
    let surface_image = render(&surface_shadow);

    assert_eq!(
        pixel(&source_alpha_image, 19, 12),
        [0, 0, 0, 255],
        "content-alpha shadow must stay absent when the captured source is empty"
    );
    assert_eq!(
        pixel(&surface_image, 12, 12),
        [0, 0, 0, 255],
        "offset surface shadow should not fill the original material body"
    );
    assert_eq!(
        pixel(&surface_image, 19, 12),
        [128, 128, 128, 255],
        "surface shadow should be generated from the material shape even with no source alpha"
    );
}

#[test]
fn render_group_rounded_mask_clips_composited_source() {
    let group_scene = finished_scene([quad(0, rect(4., 4., 20., 20.), green())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(4., 4., 20., 20.),
        vec![CompositeEffect::rounded_mask(Corners::all(px(8.)))],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    assert_eq!(pixel(&image, 14, 14), [0, 255, 0, 255]);
    assert_eq!(pixel(&image, 4, 4), [0, 0, 0, 255]);
}

#[test]
fn render_group_rounded_mask_clips_blurred_source_body() {
    let group_scene = finished_scene([quad(0, rect(4., 4., 20., 20.), green())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(4., 4., 20., 20.),
        vec![
            CompositeEffect::source_blur(px(4.)),
            CompositeEffect::rounded_mask(Corners::all(px(8.))),
        ],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    assert!(pixel(&image, 14, 14)[1] > 200);
    assert_eq!(pixel(&image, 4, 4), [0, 0, 0, 255]);
}

#[test]
fn render_group_staged_clip_then_blur_spreads_past_mask_edge() {
    let shape = GroupShape::rounded_rect(Corners::all(px(8.)));

    let mut blur_then_clip = Scene::default();
    blur_then_clip.insert_primitive(quad(0, viewport(), black()));
    blur_then_clip.insert_primitive(paint_group_with_bounds(
        1,
        rect(8., 8., 16., 16.),
        rect(4., 4., 24., 24.),
        vec![
            CompositeEffect::source_blur(px(4.)),
            CompositeEffect::source_mask(shape),
        ],
        finished_scene([quad(0, rect(8., 8., 16., 16.), green())]),
    ));
    blur_then_clip.finish();

    let mut clip_then_blur = Scene::default();
    clip_then_blur.insert_primitive(quad(0, viewport(), black()));
    clip_then_blur.insert_primitive(paint_group_with_bounds(
        1,
        rect(8., 8., 16., 16.),
        rect(4., 4., 24., 24.),
        vec![
            CompositeEffect::source_mask_before_blur(shape),
            CompositeEffect::source_blur(px(4.)),
        ],
        finished_scene([quad(0, rect(8., 8., 16., 16.), green())]),
    ));
    clip_then_blur.finish();

    let after_mask = pixel(&render(&blur_then_clip), 7, 12);
    let before_mask = pixel(&render(&clip_then_blur), 7, 12);

    assert_eq!(after_mask, [0, 0, 0, 255]);
    assert!(
        before_mask[1] > after_mask[1],
        "clip-then-blur should spread green past the mask edge: before={before_mask:?} after={after_mask:?}"
    );
}

#[test]
fn nested_source_blur_inside_outer_mask_clips_finished_inner_group() {
    let inner_group_scene = finished_scene([quad(0, rect(12., 12., 8., 8.), green())]);

    let mut outer_group_scene = Scene::default();
    outer_group_scene.insert_primitive(paint_group_with_bounds(
        0,
        rect(12., 12., 8., 8.),
        rect(8., 8., 16., 16.),
        vec![CompositeEffect::source_blur(px(4.))],
        inner_group_scene,
    ));
    outer_group_scene.finish();

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_bounds(
        1,
        rect(10., 10., 12., 12.),
        rect(8., 8., 16., 16.),
        vec![CompositeEffect::source_mask(GroupShape::rectangle())],
        outer_group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let outside_outer_mask = pixel(&image, 9, 16);
    let inside_blur = pixel(&image, 10, 16);
    let center = pixel(&image, 16, 16);

    assert_eq!(
        outside_outer_mask,
        [0, 0, 0, 255],
        "outer mask must clip the already-blurred inner group at its boundary"
    );
    assert!(
        inside_blur[1] > outside_outer_mask[1],
        "inner blur should survive up to the inside of the outer mask: inside={inside_blur:?} outside={outside_outer_mask:?}"
    );
    assert!(
        center[1] > inside_blur[1],
        "center source should remain stronger than the blur fringe: center={center:?} fringe={inside_blur:?}"
    );
}

#[test]
fn render_group_processed_content_glow_follows_bright_pixels() {
    let group_scene = finished_scene([quad(0, rect(13., 13., 4., 4.), white())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_bounds(
        1,
        rect(12., 12., 8., 8.),
        rect(6., 6., 20., 20.),
        vec![CompositeEffect::processed_content_glow(
            [
                DerivedStage::threshold_luma(LumaThreshold::above(0.8)),
                DerivedStage::exact_blur(px(3.)),
            ],
            Glow::tinted(red()),
        )],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let glow_sample = pixel(&image, 11, 15);
    let far_sample = pixel(&image, 4, 4);

    assert!(
        glow_sample[0] > far_sample[0],
        "processed-content glow should add red near bright content: glow={glow_sample:?} far={far_sample:?}"
    );
}

#[test]
fn render_group_backdrop_tint_draws_material_without_source_fill() {
    let group_scene = Scene::default();

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![CompositeEffect::backdrop_tint(rgba(0xffffff80).into())],
        group_scene,
    ));
    grouped.finish();

    assert_eq!(pixel(&render(&grouped), 12, 12), [128, 128, 128, 255]);
    assert_eq!(pixel(&render(&grouped), 4, 4), [0, 0, 0, 255]);
}

#[test]
fn render_group_backdrop_tint_is_visible_behind_translucent_source() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), rgba(0x0000ff80))]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![CompositeEffect::backdrop_tint(rgba(0xffffff80).into())],
        group_scene,
    ));
    grouped.finish();

    assert_eq!(pixel(&render(&grouped), 12, 12), [64, 64, 192, 255]);
}

#[test]
fn render_group_backdrop_blur_samples_parent_target_at_group_order() {
    let group_scene = Scene::default();

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(quad(1, rect(8., 8., 8., 8.), white()));
    grouped.insert_primitive(paint_group_with_effects(
        2,
        rect(4., 8., 20., 8.),
        vec![CompositeEffect::backdrop_blur(px(2.))],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let center = pixel(&image, 12, 12);
    let left_edge = pixel(&image, 7, 12);
    let outside = pixel(&image, 2, 12);

    assert_eq!(outside, [0, 0, 0, 255]);
    assert!(
        center[0] > left_edge[0],
        "center {center:?} left_edge {left_edge:?}"
    );
    assert!(left_edge[0] > 0, "blur should pull white backdrop outward");
}

#[test]
fn render_group_backdrop_blur_is_visible_behind_translucent_source() {
    let group_scene = finished_scene([quad(0, rect(4., 8., 20., 8.), rgba(0x0000ff80))]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(quad(1, rect(8., 8., 8., 8.), white()));
    grouped.insert_primitive(paint_group_with_effects(
        2,
        rect(4., 8., 20., 8.),
        vec![CompositeEffect::backdrop_blur(px(2.))],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let center = pixel(&image, 12, 12);
    let left_edge = pixel(&image, 7, 12);

    assert!(
        center[0] > left_edge[0],
        "blurred backdrop should still contribute through translucent source: center {center:?} left_edge {left_edge:?}"
    );
    assert!(
        center[2] > center[0] && left_edge[2] > left_edge[0],
        "translucent source should still contribute over the blurred backdrop: center {center:?} left_edge {left_edge:?}"
    );
}

#[test]
fn render_group_backdrop_lens_refracts_parent_target_near_material_edge() {
    let group_scene = Scene::default();

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), green()));
    grouped.insert_primitive(quad(1, rect(0., 0., 8., 32.), red()));
    grouped.insert_primitive(paint_group_with_effects(
        2,
        rect(8., 8., 16., 16.),
        vec![CompositeEffect::backdrop_lens(
            px(6.),
            px(6.),
            px(0.),
            0.,
            0.,
            point(-1., 0.),
        )],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let refracted_edge = pixel(&image, 10, 16);
    let stable_center = pixel(&image, 16, 16);

    assert!(
        refracted_edge[0] > refracted_edge[1],
        "left rim should bend red backdrop pixels into the green group interior: {refracted_edge:?}"
    );
    assert!(
        stable_center[1] > stable_center[0],
        "center should remain the undisplaced green backdrop: {stable_center:?}"
    );
}

#[test]
fn render_group_backdrop_lens_lights_continuous_bevel_profile() {
    let group_scene = Scene::default();

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), rgba(0x404040ff)));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 16., 16.),
        vec![CompositeEffect::backdrop_lens(
            px(0.),
            px(6.),
            px(0.),
            0.8,
            0.,
            point(0., -1.),
        )],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let outer_edge = pixel(&image, 8, 16);
    let outer_shoulder = pixel(&image, 9, 16);
    let mid_bevel = pixel(&image, 10, 16);
    let focus_ridge = pixel(&image, 11, 16);
    let inner_falloff = pixel(&image, 12, 16);
    let stable_center = pixel(&image, 16, 16);
    let bevel_samples = [
        outer_edge[0],
        outer_shoulder[0],
        mid_bevel[0],
        focus_ridge[0],
        inner_falloff[0],
        stable_center[0],
    ];

    assert!(
        outer_edge[0] > stable_center[0],
        "outer edge should catch glancing light: outer_edge {outer_edge:?} center {stable_center:?}"
    );
    assert!(
        outer_shoulder[0] > stable_center[0],
        "rounded side should stay lit after the outer edge instead of collapsing into a dark band: outer_shoulder {outer_shoulder:?} center {stable_center:?}"
    );
    assert!(
        mid_bevel[0] > stable_center[0],
        "mid-bevel should stay optically active between the outer wall and inner focus ridge: mid_bevel {mid_bevel:?} center {stable_center:?}"
    );
    assert!(
        focus_ridge[0] > mid_bevel[0],
        "inner focus ridge should concentrate more light than the rounded bevel body for tangent-guided light: focus_ridge {focus_ridge:?} mid_bevel {mid_bevel:?}"
    );
    assert!(
        inner_falloff[0] > stable_center[0],
        "rounded side should decay back to the stable pane after the focus ridge: inner_falloff {inner_falloff:?} center {stable_center:?}"
    );
    assert!(
        bevel_samples
            .windows(2)
            .all(|samples| samples[0].abs_diff(samples[1]) <= 48),
        "rounded side should ramp continuously instead of forming separate visual rails: {bevel_samples:?}"
    );
    assert_eq!(
        stable_center,
        [64, 64, 64, 255],
        "center should stay the unchanged backdrop when the edge band is outside the sample point"
    );
}

#[test]
fn render_group_backdrop_lens_reflects_backdrop_color_along_material_edge() {
    let group_scene = Scene::default();

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(quad(1, rect(8., 11., 16., 3.), rgba(0x0000ffff)));
    grouped.insert_primitive(paint_group_with_effects(
        2,
        rect(8., 8., 16., 16.),
        vec![CompositeEffect::backdrop_lens(
            px(0.),
            px(6.),
            px(0.),
            0.7,
            0.,
            point(0., -1.),
        )],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let reflected_edge = pixel(&image, 9, 16);
    let stable_center = pixel(&image, 16, 16);

    assert!(
        reflected_edge[2] > reflected_edge[0] + 16 && reflected_edge[2] > reflected_edge[1] + 16,
        "edge reflection should carry backdrop color along the tangent instead of only whitening the bevel: {reflected_edge:?}"
    );
    assert_eq!(
        stable_center,
        [0, 0, 0, 255],
        "center pane should not pick up the edge reflection sample"
    );
}

#[test]
fn render_group_backdrop_lens_reconstructs_subpixel_backdrop_samples() {
    let group_scene = Scene::default();

    let mut grouped = Scene::default();
    for x in 0..IMAGE_SIZE {
        let color = if x % 2 == 0 {
            gray_byte(0)
        } else {
            gray_byte(255)
        };
        grouped.insert_primitive(quad(x as u32, rect(x as f32, 0., 1., 32.), color));
    }
    grouped.insert_primitive(paint_group_with_effects(
        IMAGE_SIZE as u32,
        rect(8., 8., 16., 16.),
        vec![CompositeEffect::backdrop_lens(
            px(2.5),
            px(6.),
            px(0.),
            0.,
            0.,
            point(-1., 0.),
        )],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let reconstructed = (10..14)
        .map(|x| pixel(&image, x, 16)[0])
        .collect::<Vec<_>>();

    assert!(
        reconstructed
            .iter()
            .any(|channel| (32..=223).contains(channel)),
        "subpixel lens sampling should reconstruct the backdrop between texels instead of stepping between nearest-neighbor stripes: {reconstructed:?}"
    );
}

#[test]
fn render_group_backdrop_lens_splits_chromatic_channels_at_material_edge() {
    let group_scene = Scene::default();

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(quad(1, rect(6., 0., 2., 32.), red()));
    grouped.insert_primitive(quad(2, rect(13., 0., 2., 32.), rgba(0x0000ffff)));
    grouped.insert_primitive(paint_group_with_effects(
        3,
        rect(8., 8., 16., 16.),
        vec![CompositeEffect::backdrop_lens(
            px(0.),
            px(6.),
            px(4.),
            0.,
            0.,
            point(-1., 0.),
        )],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let split_edge = pixel(&image, 10, 16);
    let stable_center = pixel(&image, 16, 16);

    assert!(
        split_edge[0] > 128 && split_edge[2] > 128,
        "chromatic split should sample red and blue from opposite sides of the material normal: split_edge {split_edge:?}"
    );
    assert!(
        split_edge[1] < 16,
        "green should come from the undisplaced center sample on the black backdrop: split_edge {split_edge:?}"
    );
    assert_eq!(
        stable_center,
        [0, 0, 0, 255],
        "center should not chromatically split once outside the material edge band"
    );
}

#[test]
fn render_group_backdrop_lens_uses_material_shape_not_source_mask() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 16., 16.), rgba(0x0000ffff))]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), green()));
    grouped.insert_primitive(quad(1, rect(0., 0., 8., 32.), red()));
    grouped.insert_primitive(quad(2, rect(0., 0., 32., 8.), red()));
    grouped.insert_primitive(paint_group_with_effects(
        3,
        rect(8., 8., 16., 16.),
        vec![
            CompositeEffect::source_mask(GroupShape::rounded_rect(Corners::all(px(8.)))),
            CompositeEffect::material_shape(GroupShape::rectangle()),
            CompositeEffect::backdrop_lens(px(6.), px(6.), px(0.), 0., 0., point(-1., -1.)),
        ],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let masked_corner_material = pixel(&image, 9, 9);
    let unmasked_source = pixel(&image, 16, 16);

    assert!(
        masked_corner_material[0] > masked_corner_material[1],
        "material lens should still refract red backdrop in a corner where the separate source mask clips source content: {masked_corner_material:?}"
    );
    assert!(
        masked_corner_material[2] < 16,
        "blue source should be clipped by source mask and must not define material optics: {masked_corner_material:?}"
    );
    assert_eq!(
        unmasked_source,
        [0, 0, 255, 255],
        "source mask should still allow opaque source in the unmasked body"
    );
}

#[test]
fn render_group_backdrop_lens_does_not_distort_opaque_source_content() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 8., 16.), rgba(0x0000ffff))]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), green()));
    grouped.insert_primitive(quad(1, rect(0., 0., 8., 32.), red()));
    grouped.insert_primitive(paint_group_with_effects(
        2,
        rect(8., 8., 16., 16.),
        vec![CompositeEffect::backdrop_lens(
            px(6.),
            px(6.),
            px(4.),
            0.8,
            0.4,
            point(-1., 0.),
        )],
        group_scene,
    ));
    grouped.finish();

    let image = render(&grouped);
    let opaque_source_edge = pixel(&image, 10, 16);
    let lensed_material_edge = pixel(&image, 18, 16);

    assert_eq!(
        opaque_source_edge,
        [0, 0, 255, 255],
        "opaque source content should composite over glass without being refracted, lit, or chromatically split"
    );
    assert!(
        lensed_material_edge != opaque_source_edge,
        "uncovered material should still show the backdrop lens so this test exercises both layers: material {lensed_material_edge:?}"
    );
}

#[test]
fn render_group_blend_mode_applies_at_group_boundary() {
    let group_scene = finished_scene([quad(0, rect(8., 8., 8., 8.), gray())]);

    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), rgba(0x8080ffff)));
    grouped.insert_primitive(paint_group_with_effects(
        1,
        rect(8., 8., 8., 8.),
        vec![CompositeEffect::blend_mode(CompositeBlendMode::Multiply)],
        group_scene,
    ));
    grouped.finish();

    assert_eq!(pixel(&render(&grouped), 12, 12), [64, 64, 128, 255]);
}

// --- Descendant capture: text (monochrome sprites) ---------------------------
//
// Render groups must capture and composite glyph sprites, not only quads. A glyph
// paints as a `MonochromeSprite` that samples a coverage tile in the monochrome
// atlas and multiplies it by `color`. These fixtures inject a synthetic
// fully-covered tile (a stand-in glyph) so the captured pixels are deterministic,
// then prove the group captures the sprite losslessly and composites group
// effects over it. Without this, broad text-in-group UI is unproven.

/// Render a scene that needs atlas-backed sprites: the build closure receives the
/// renderer's sprite atlas so it can insert tiles before the scene is rendered.
/// `render_scene_to_image` calls `atlas.before_frame()`, which flushes the staged
/// tile uploads before the draw, so the injected pixels are on the GPU.
fn render_with_atlas(build: impl FnOnce(&Arc<dyn PlatformAtlas>) -> Scene) -> RgbaImage {
    let mut renderer = WgpuHeadlessRenderer::new().expect("create headless renderer");
    let atlas = renderer.sprite_atlas().clone();
    let scene = build(&atlas);
    renderer
        .render_scene_to_image(
            &scene,
            size(DevicePixels(IMAGE_SIZE), DevicePixels(IMAGE_SIZE)),
        )
        .expect("render scene")
}

/// Allocate and upload a `side`x`side` monochrome tile whose every texel has the
/// given coverage, returning the tile a `MonochromeSprite` can sample. Uses an
/// `AtlasKey::Svg` key (monochrome texture kind) as a synthetic stand-in glyph.
fn monochrome_coverage_tile(
    atlas: &Arc<dyn PlatformAtlas>,
    key: &str,
    side: i32,
    coverage: u8,
) -> AtlasTile {
    let tile_size = size(DevicePixels(side), DevicePixels(side));
    let params = RenderSvgParams {
        path: key.to_string().into(),
        size: tile_size,
    };
    atlas
        .get_or_insert_with(&AtlasKey::Svg(params), &mut || {
            Ok(Some((
                tile_size,
                Cow::Owned(vec![coverage; (side * side) as usize]),
            )))
        })
        .expect("atlas insert succeeds")
        .expect("atlas returns a tile")
}

fn monochrome_sprite(
    order: u32,
    bounds: Bounds<ScaledPixels>,
    color: Hsla,
    tile: AtlasTile,
) -> MonochromeSprite {
    MonochromeSprite {
        order,
        pad: 0,
        bounds,
        content_mask: mask(),
        color,
        tile,
        transformation: TransformationMatrix::default(),
    }
}

#[test]
fn render_group_captures_monochrome_text_sprite_losslessly() {
    // The same glyph sprite rendered inline vs inside an identity render group
    // must produce identical pixels — proving the group captures monochrome
    // (text) sprites, not only quads.
    let inline = render_with_atlas(|atlas| {
        let tile = monochrome_coverage_tile(atlas, "glyph", 8, 0xff);
        let mut scene = Scene::default();
        scene.insert_primitive(quad(0, viewport(), black()));
        scene.insert_primitive(monochrome_sprite(1, rect(8., 8., 8., 8.), white(), tile));
        scene.finish();
        scene
    });

    let grouped = render_with_atlas(|atlas| {
        let tile = monochrome_coverage_tile(atlas, "glyph", 8, 0xff);
        let mut group_scene = Scene::default();
        group_scene.insert_primitive(monochrome_sprite(0, rect(8., 8., 8., 8.), white(), tile));
        group_scene.finish();

        let mut scene = Scene::default();
        scene.insert_primitive(quad(0, viewport(), black()));
        scene.insert_primitive(paint_group(1, rect(8., 8., 8., 8.), 1., group_scene));
        scene.finish();
        scene
    });

    assert_eq!(grouped.as_raw(), inline.as_raw());
}

#[test]
fn render_group_opacity_composites_captured_monochrome_text_sprite() {
    // A fully-covered white glyph at full opacity is white; captured inside an
    // opacity(0.5) group it must composite to half-white over black — proving the
    // group effect applies to the captured sprite coverage, not just to quads.
    let image = render_with_atlas(|atlas| {
        let tile = monochrome_coverage_tile(atlas, "glyph", 8, 0xff);
        let mut group_scene = Scene::default();
        group_scene.insert_primitive(monochrome_sprite(0, rect(8., 8., 8., 8.), white(), tile));
        group_scene.finish();

        let mut scene = Scene::default();
        scene.insert_primitive(quad(0, viewport(), black()));
        scene.insert_primitive(paint_group(1, rect(8., 8., 8., 8.), 0.5, group_scene));
        scene.finish();
        scene
    });

    assert_eq!(pixel(&image, 1, 1), [0, 0, 0, 255]);
    assert_eq!(pixel(&image, 12, 12), [128, 128, 128, 255]);
}

// --- Descendant capture: images (polychrome sprites) -------------------------
//
// Images paint as `PolychromeSprite`s sampling an RGBA tile in the polychrome
// atlas. A white, fully-opaque tile keeps the expected pixels independent of
// channel order and premultiplication, isolating capture/compositing of image
// descendants from color-format details.

/// Allocate and upload a `side`x`side` fully-opaque white polychrome tile (4
/// bytes/texel) via an `AtlasKey::Image` key (polychrome texture kind).
fn polychrome_white_tile(atlas: &Arc<dyn PlatformAtlas>, key: usize, side: i32) -> AtlasTile {
    let tile_size = size(DevicePixels(side), DevicePixels(side));
    let params = RenderImageParams {
        image_id: ImageId(key),
        frame_index: 0,
    };
    atlas
        .get_or_insert_with(&AtlasKey::Image(params), &mut || {
            Ok(Some((
                tile_size,
                Cow::Owned(vec![0xff; (side * side * 4) as usize]),
            )))
        })
        .expect("atlas insert succeeds")
        .expect("atlas returns a tile")
}

fn polychrome_sprite(
    order: u32,
    bounds: Bounds<ScaledPixels>,
    tile: AtlasTile,
) -> PolychromeSprite {
    PolychromeSprite {
        order,
        pad: 0,
        grayscale: false,
        opacity: 1.,
        bounds,
        content_mask: mask(),
        corner_radii: Corners::all(sp(0.)),
        tile,
    }
}

#[test]
fn render_group_captures_polychrome_image_sprite_losslessly() {
    // The same image sprite rendered inline vs inside an identity render group
    // must produce identical pixels — proving the group captures polychrome
    // (image) sprites, not only quads.
    let inline = render_with_atlas(|atlas| {
        let tile = polychrome_white_tile(atlas, 1, 8);
        let mut scene = Scene::default();
        scene.insert_primitive(quad(0, viewport(), black()));
        scene.insert_primitive(polychrome_sprite(1, rect(8., 8., 8., 8.), tile));
        scene.finish();
        scene
    });

    let grouped = render_with_atlas(|atlas| {
        let tile = polychrome_white_tile(atlas, 1, 8);
        let mut group_scene = Scene::default();
        group_scene.insert_primitive(polychrome_sprite(0, rect(8., 8., 8., 8.), tile));
        group_scene.finish();

        let mut scene = Scene::default();
        scene.insert_primitive(quad(0, viewport(), black()));
        scene.insert_primitive(paint_group(1, rect(8., 8., 8., 8.), 1., group_scene));
        scene.finish();
        scene
    });

    assert_eq!(grouped.as_raw(), inline.as_raw());
}

#[test]
fn render_group_opacity_composites_captured_polychrome_image_sprite() {
    // A fully-opaque white image at full opacity is white; captured inside an
    // opacity(0.5) group it must composite to half-white over black — proving the
    // group effect applies to the captured image sprite, not just to quads.
    let image = render_with_atlas(|atlas| {
        let tile = polychrome_white_tile(atlas, 1, 8);
        let mut group_scene = Scene::default();
        group_scene.insert_primitive(polychrome_sprite(0, rect(8., 8., 8., 8.), tile));
        group_scene.finish();

        let mut scene = Scene::default();
        scene.insert_primitive(quad(0, viewport(), black()));
        scene.insert_primitive(paint_group(1, rect(8., 8., 8., 8.), 0.5, group_scene));
        scene.finish();
        scene
    });

    assert_eq!(pixel(&image, 1, 1), [0, 0, 0, 255]);
    assert_eq!(pixel(&image, 12, 12), [128, 128, 128, 255]);
}

// --- Descendant capture: cached descendants (replayed primitives) ------------
//
// A cached element (`AnyView::cached`) is reused by replaying its recorded
// primitives into the CURRENT scene: `Scene::replay` walks the prior frame's
// `paint_operations` and re-`insert_primitive`s them into `self`. During
// `Window::paint_group`, the current scene *is* the group's scene (paint_group
// `mem::take`s `next_frame.scene` for the group's children), so a cached child
// replayed inside a group lands in the group scene and is captured. These
// fixtures exercise that exact `replay` mechanism rather than re-deriving it.

/// A previous-frame scene holding one recorded child primitive, plus the
/// `0..len` range a cached reuse would replay (one `paint_operation` per
/// non-empty `insert_primitive`).
fn recorded_child(child: Quad) -> (Scene, std::ops::Range<usize>) {
    let mut previous_frame = Scene::default();
    previous_frame.insert_primitive(child);
    (previous_frame, 0..1)
}

#[test]
fn render_group_captures_cached_descendant_replay_losslessly() {
    // A child replayed (as a cached reuse) inside an identity render group must
    // match the same child painted inline — the group captures replayed cached
    // primitives, not only freshly-inserted ones.
    let (previous_frame, range) = recorded_child(quad(0, rect(8., 8., 8., 8.), white()));

    let mut inline = Scene::default();
    inline.insert_primitive(quad(0, viewport(), black()));
    inline.insert_primitive(quad(1, rect(8., 8., 8., 8.), white()));
    inline.finish();

    let mut group_scene = Scene::default();
    group_scene.replay(range, &previous_frame);
    group_scene.finish();
    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group(1, rect(8., 8., 8., 8.), 1., group_scene));
    grouped.finish();

    assert_eq!(render(&grouped).as_raw(), render(&inline).as_raw());
}

#[test]
fn render_group_opacity_composites_cached_descendant_replay() {
    // A full white child replayed (cached reuse) inside an opacity(0.5) group must
    // composite to half-white over black — proving the group effect applies to the
    // captured replayed primitive.
    let (previous_frame, range) = recorded_child(quad(0, rect(8., 8., 8., 8.), white()));

    let mut group_scene = Scene::default();
    group_scene.replay(range, &previous_frame);
    group_scene.finish();
    let mut grouped = Scene::default();
    grouped.insert_primitive(quad(0, viewport(), black()));
    grouped.insert_primitive(paint_group(1, rect(8., 8., 8., 8.), 0.5, group_scene));
    grouped.finish();

    let image = render(&grouped);
    assert_eq!(pixel(&image, 1, 1), [0, 0, 0, 255]);
    assert_eq!(pixel(&image, 12, 12), [128, 128, 128, 255]);
}
