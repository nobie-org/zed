#![cfg(all(not(target_family = "wasm"), feature = "test-support"))]

use std::sync::Arc;

use gpui::{
    Background, BorderStyle, Bounds, CompositeBlendMode, CompositeEffect, ContentMask, Corners,
    DevicePixels, Edges, GroupShape, Hsla, LogicalVisualPlan, PaintGroup, PlatformHeadlessRenderer,
    Quad, ScaledPixels, Scene, point, px, rgba, size, transparent_black,
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
    let outer_wall = pixel(&image, 8, 16);
    let mid_bevel = pixel(&image, 10, 16);
    let focus_ridge = pixel(&image, 11, 16);
    let stable_center = pixel(&image, 16, 16);

    assert!(
        outer_wall[0] > stable_center[0],
        "outer edge wall should catch light: outer_wall {outer_wall:?} center {stable_center:?}"
    );
    assert!(
        mid_bevel[0] > stable_center[0],
        "mid-bevel should stay optically active between the outer wall and inner focus ridge: mid_bevel {mid_bevel:?} center {stable_center:?}"
    );
    assert!(
        focus_ridge[0] > mid_bevel[0],
        "inner focus ridge should concentrate more light than the rounded bevel body for tangent-guided light: focus_ridge {focus_ridge:?} mid_bevel {mid_bevel:?}"
    );
    assert_eq!(
        stable_center,
        [64, 64, 64, 255],
        "center should stay the unchanged backdrop when the edge band is outside the sample point"
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
