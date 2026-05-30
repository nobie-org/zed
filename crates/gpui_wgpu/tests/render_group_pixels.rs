#![cfg(all(not(target_family = "wasm"), feature = "test-support"))]

use std::sync::Arc;

use gpui::{
    Background, BorderStyle, Bounds, CompositeBlendMode, CompositeEffect, ContentMask, Corners,
    DevicePixels, Edges, Hsla, PaintGroup, PlatformHeadlessRenderer, Quad, ScaledPixels, Scene,
    point, px, rgba, size, transparent_black,
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
        boundary_opacity: 1.,
        effects,
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
