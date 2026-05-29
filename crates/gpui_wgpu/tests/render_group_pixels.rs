#![cfg(all(not(target_family = "wasm"), feature = "test-support"))]

use std::sync::Arc;

use gpui::{
    Background, BorderStyle, Bounds, CompositeEffect, ContentMask, Corners, DevicePixels, Edges,
    Hsla, PaintGroup, PlatformHeadlessRenderer, Quad, ScaledPixels, Scene, point, rgba, size,
    transparent_black,
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
    PaintGroup {
        order,
        bounds: capture_bounds,
        capture_bounds,
        content_mask: mask(),
        boundary_opacity: 1.,
        effects: vec![CompositeEffect::opacity(opacity)],
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

fn red_half() -> Hsla {
    rgba(0xff000080).into()
}

fn blue_half() -> Hsla {
    rgba(0x0000ff80).into()
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
