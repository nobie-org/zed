// todo("windows"): remove
#![cfg_attr(windows, allow(dead_code))]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AtlasTextureId, AtlasTile, Background, Bounds, ContentMask, Corners, Edges, Hsla, Pixels,
    Point, Radians, Rgba, ScaledPixels, Size, bounds_tree::BoundsTree, point, px, solid_background,
    transparent_black,
};
use std::{
    fmt::{Debug, Display},
    iter::Peekable,
    ops::{Add, Range, Sub},
    slice,
    sync::Arc,
};

#[allow(non_camel_case_types, unused)]
#[expect(missing_docs)]
pub type PathVertex_ScaledPixels = PathVertex<ScaledPixels>;

#[expect(missing_docs)]
pub type DrawOrder = u32;

#[derive(Default)]
#[expect(missing_docs)]
pub struct Scene {
    pub(crate) paint_operations: Vec<PaintOperation>,
    primitive_bounds: BoundsTree<ScaledPixels>,
    layer_stack: Vec<DrawOrder>,
    pub shadows: Vec<Shadow>,
    pub quads: Vec<Quad>,
    pub paths: Vec<Path<ScaledPixels>>,
    pub underlines: Vec<Underline>,
    pub monochrome_sprites: Vec<MonochromeSprite>,
    pub subpixel_sprites: Vec<SubpixelSprite>,
    pub polychrome_sprites: Vec<PolychromeSprite>,
    pub surfaces: Vec<PaintMetalTexture>,
    pub groups: Vec<PaintGroup>,
}

#[expect(missing_docs)]
impl Scene {
    pub fn clear(&mut self) {
        self.paint_operations.clear();
        self.primitive_bounds.clear();
        self.layer_stack.clear();
        self.paths.clear();
        self.shadows.clear();
        self.quads.clear();
        self.underlines.clear();
        self.monochrome_sprites.clear();
        self.subpixel_sprites.clear();
        self.polychrome_sprites.clear();
        self.surfaces.clear();
        self.groups.clear();
    }

    pub fn len(&self) -> usize {
        self.paint_operations.len()
    }

    pub fn push_layer(&mut self, bounds: Bounds<ScaledPixels>) {
        let order = self.primitive_bounds.insert(bounds);
        self.layer_stack.push(order);
        self.paint_operations
            .push(PaintOperation::StartLayer(bounds));
    }

    pub fn pop_layer(&mut self) {
        self.layer_stack.pop();
        self.paint_operations.push(PaintOperation::EndLayer);
    }

    pub fn insert_primitive(&mut self, primitive: impl Into<Primitive>) {
        let mut primitive = primitive.into();
        let clipped_bounds = primitive
            .bounds()
            .intersect(&primitive.content_mask().bounds);

        if clipped_bounds.is_empty() {
            return;
        }

        let order = self
            .layer_stack
            .last()
            .copied()
            .unwrap_or_else(|| self.primitive_bounds.insert(clipped_bounds));
        match &mut primitive {
            Primitive::Shadow(shadow) => {
                shadow.order = order;
                self.shadows.push(*shadow);
            }
            Primitive::Quad(quad) => {
                quad.order = order;
                self.quads.push(*quad);
            }
            Primitive::Path(path) => {
                path.order = order;
                path.id = PathId(self.paths.len());
                self.paths.push(path.clone());
            }
            Primitive::Underline(underline) => {
                underline.order = order;
                self.underlines.push(*underline);
            }
            Primitive::MonochromeSprite(sprite) => {
                sprite.order = order;
                self.monochrome_sprites.push(*sprite);
            }
            Primitive::SubpixelSprite(sprite) => {
                sprite.order = order;
                self.subpixel_sprites.push(*sprite);
            }
            Primitive::PolychromeSprite(sprite) => {
                sprite.order = order;
                self.polychrome_sprites.push(*sprite);
            }
            Primitive::Surface(surface) => {
                surface.order = order;
                self.surfaces.push(surface.clone());
            }
            Primitive::Group(group) => {
                group.order = order;
                self.groups.push(group.clone());
            }
        }
        self.paint_operations
            .push(PaintOperation::Primitive(primitive));
    }

    pub fn replay(&mut self, range: Range<usize>, prev_scene: &Scene) {
        for operation in &prev_scene.paint_operations[range] {
            match operation {
                PaintOperation::Primitive(primitive) => self.insert_primitive(primitive.clone()),
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds),
                PaintOperation::EndLayer => self.pop_layer(),
            }
        }
    }

    pub fn finish(&mut self) {
        self.shadows.sort_by_key(|shadow| shadow.order);
        self.quads.sort_by_key(|quad| quad.order);
        self.paths.sort_by_key(|path| path.order);
        self.underlines.sort_by_key(|underline| underline.order);
        self.monochrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.subpixel_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.polychrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.tile_id));
        self.surfaces.sort_by_key(|surface| surface.order);
        self.groups.sort_by_key(|group| group.order);
    }

    /// Returns whether any render group in this scene needs parent-target pixels.
    pub fn requires_backdrop_effects(&self) -> bool {
        self.groups.iter().any(|group| {
            group.plan.requirements().reads_backdrop || group.scene.requires_backdrop_effects()
        })
    }

    #[cfg_attr(
        all(
            any(target_os = "linux", target_os = "freebsd"),
            not(any(feature = "x11", feature = "wayland"))
        ),
        allow(dead_code)
    )]
    pub fn batches(&self) -> impl Iterator<Item = PrimitiveBatch> + '_ {
        BatchIterator {
            shadows_start: 0,
            shadows_iter: self.shadows.iter().peekable(),
            quads_start: 0,
            quads_iter: self.quads.iter().peekable(),
            paths_start: 0,
            paths_iter: self.paths.iter().peekable(),
            underlines_start: 0,
            underlines_iter: self.underlines.iter().peekable(),
            monochrome_sprites_start: 0,
            monochrome_sprites_iter: self.monochrome_sprites.iter().peekable(),
            subpixel_sprites_start: 0,
            subpixel_sprites_iter: self.subpixel_sprites.iter().peekable(),
            polychrome_sprites_start: 0,
            polychrome_sprites_iter: self.polychrome_sprites.iter().peekable(),
            surfaces_start: 0,
            surfaces_iter: self.surfaces.iter().peekable(),
            groups_start: 0,
            groups_iter: self.groups.iter().peekable(),
        }
    }

    pub(crate) fn visual_bounds(&self) -> Option<Bounds<ScaledPixels>> {
        let mut bounds = None;

        for shadow in &self.shadows {
            Self::extend_visual_bounds(
                &mut bounds,
                shadow
                    .bounds
                    .dilate(shadow.blur_radius * 3.)
                    .intersect(&shadow.content_mask.bounds),
            );
        }

        for quad in &self.quads {
            Self::extend_visual_bounds(
                &mut bounds,
                quad.bounds.intersect(&quad.content_mask.bounds),
            );
        }

        for path in &self.paths {
            Self::extend_visual_bounds(
                &mut bounds,
                path.bounds.intersect(&path.content_mask.bounds),
            );
        }

        for underline in &self.underlines {
            Self::extend_visual_bounds(
                &mut bounds,
                underline.bounds.intersect(&underline.content_mask.bounds),
            );
        }

        for sprite in &self.monochrome_sprites {
            Self::extend_visual_bounds(
                &mut bounds,
                sprite.bounds.intersect(&sprite.content_mask.bounds),
            );
        }

        for sprite in &self.subpixel_sprites {
            Self::extend_visual_bounds(
                &mut bounds,
                sprite.bounds.intersect(&sprite.content_mask.bounds),
            );
        }

        for sprite in &self.polychrome_sprites {
            Self::extend_visual_bounds(
                &mut bounds,
                sprite.bounds.intersect(&sprite.content_mask.bounds),
            );
        }

        for surface in &self.surfaces {
            Self::extend_visual_bounds(
                &mut bounds,
                surface.bounds.intersect(&surface.content_mask.bounds),
            );
        }

        for group in &self.groups {
            Self::extend_visual_bounds(
                &mut bounds,
                group.capture_bounds.intersect(&group.content_mask.bounds),
            );
        }

        bounds
    }

    fn extend_visual_bounds(bounds: &mut Option<Bounds<ScaledPixels>>, next: Bounds<ScaledPixels>) {
        if next.is_empty() {
            return;
        }

        *bounds = Some(if let Some(bounds) = *bounds {
            bounds.union(&next)
        } else {
            next
        });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Default)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
pub(crate) enum PrimitiveKind {
    Shadow,
    #[default]
    Quad,
    Path,
    Underline,
    MonochromeSprite,
    SubpixelSprite,
    PolychromeSprite,
    Surface,
    Group,
}

pub(crate) enum PaintOperation {
    Primitive(Primitive),
    StartLayer(Bounds<ScaledPixels>),
    EndLayer,
}

#[derive(Clone)]
#[expect(missing_docs)]
#[non_exhaustive]
pub enum Primitive {
    Shadow(Shadow),
    Quad(Quad),
    Path(Path<ScaledPixels>),
    Underline(Underline),
    MonochromeSprite(MonochromeSprite),
    SubpixelSprite(SubpixelSprite),
    PolychromeSprite(PolychromeSprite),
    Surface(PaintMetalTexture),
    Group(PaintGroup),
}

#[expect(missing_docs)]
impl Primitive {
    pub fn bounds(&self) -> &Bounds<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.bounds,
            Primitive::Quad(quad) => &quad.bounds,
            Primitive::Path(path) => &path.bounds,
            Primitive::Underline(underline) => &underline.bounds,
            Primitive::MonochromeSprite(sprite) => &sprite.bounds,
            Primitive::SubpixelSprite(sprite) => &sprite.bounds,
            Primitive::PolychromeSprite(sprite) => &sprite.bounds,
            Primitive::Surface(surface) => &surface.bounds,
            Primitive::Group(group) => &group.capture_bounds,
        }
    }

    pub fn content_mask(&self) -> &ContentMask<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.content_mask,
            Primitive::Quad(quad) => &quad.content_mask,
            Primitive::Path(path) => &path.content_mask,
            Primitive::Underline(underline) => &underline.content_mask,
            Primitive::MonochromeSprite(sprite) => &sprite.content_mask,
            Primitive::SubpixelSprite(sprite) => &sprite.content_mask,
            Primitive::PolychromeSprite(sprite) => &sprite.content_mask,
            Primitive::Surface(surface) => &surface.content_mask,
            Primitive::Group(group) => &group.content_mask,
        }
    }
}

#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
struct BatchIterator<'a> {
    shadows_start: usize,
    shadows_iter: Peekable<slice::Iter<'a, Shadow>>,
    quads_start: usize,
    quads_iter: Peekable<slice::Iter<'a, Quad>>,
    paths_start: usize,
    paths_iter: Peekable<slice::Iter<'a, Path<ScaledPixels>>>,
    underlines_start: usize,
    underlines_iter: Peekable<slice::Iter<'a, Underline>>,
    monochrome_sprites_start: usize,
    monochrome_sprites_iter: Peekable<slice::Iter<'a, MonochromeSprite>>,
    subpixel_sprites_start: usize,
    subpixel_sprites_iter: Peekable<slice::Iter<'a, SubpixelSprite>>,
    polychrome_sprites_start: usize,
    polychrome_sprites_iter: Peekable<slice::Iter<'a, PolychromeSprite>>,
    surfaces_start: usize,
    surfaces_iter: Peekable<slice::Iter<'a, PaintMetalTexture>>,
    groups_start: usize,
    groups_iter: Peekable<slice::Iter<'a, PaintGroup>>,
}

impl<'a> Iterator for BatchIterator<'a> {
    type Item = PrimitiveBatch;

    fn next(&mut self) -> Option<Self::Item> {
        let mut orders_and_kinds = [
            (
                self.shadows_iter.peek().map(|s| s.order),
                PrimitiveKind::Shadow,
            ),
            (self.quads_iter.peek().map(|q| q.order), PrimitiveKind::Quad),
            (self.paths_iter.peek().map(|q| q.order), PrimitiveKind::Path),
            (
                self.underlines_iter.peek().map(|u| u.order),
                PrimitiveKind::Underline,
            ),
            (
                self.monochrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::MonochromeSprite,
            ),
            (
                self.subpixel_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::SubpixelSprite,
            ),
            (
                self.polychrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::PolychromeSprite,
            ),
            (
                self.surfaces_iter.peek().map(|s| s.order),
                PrimitiveKind::Surface,
            ),
            (
                self.groups_iter.peek().map(|s| s.order),
                PrimitiveKind::Group,
            ),
        ];
        orders_and_kinds.sort_by_key(|(order, kind)| (order.unwrap_or(u32::MAX), *kind));

        let first = orders_and_kinds[0];
        let second = orders_and_kinds[1];
        let (batch_kind, max_order_and_kind) = if first.0.is_some() {
            (first.1, (second.0.unwrap_or(u32::MAX), second.1))
        } else {
            return None;
        };

        match batch_kind {
            PrimitiveKind::Shadow => {
                let shadows_start = self.shadows_start;
                let mut shadows_end = shadows_start + 1;
                self.shadows_iter.next();
                while self
                    .shadows_iter
                    .next_if(|shadow| (shadow.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    shadows_end += 1;
                }
                self.shadows_start = shadows_end;
                Some(PrimitiveBatch::Shadows(shadows_start..shadows_end))
            }
            PrimitiveKind::Quad => {
                let quads_start = self.quads_start;
                let mut quads_end = quads_start + 1;
                self.quads_iter.next();
                while self
                    .quads_iter
                    .next_if(|quad| (quad.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    quads_end += 1;
                }
                self.quads_start = quads_end;
                Some(PrimitiveBatch::Quads(quads_start..quads_end))
            }
            PrimitiveKind::Path => {
                let paths_start = self.paths_start;
                let mut paths_end = paths_start + 1;
                self.paths_iter.next();
                while self
                    .paths_iter
                    .next_if(|path| (path.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    paths_end += 1;
                }
                self.paths_start = paths_end;
                Some(PrimitiveBatch::Paths(paths_start..paths_end))
            }
            PrimitiveKind::Underline => {
                let underlines_start = self.underlines_start;
                let mut underlines_end = underlines_start + 1;
                self.underlines_iter.next();
                while self
                    .underlines_iter
                    .next_if(|underline| (underline.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    underlines_end += 1;
                }
                self.underlines_start = underlines_end;
                Some(PrimitiveBatch::Underlines(underlines_start..underlines_end))
            }
            PrimitiveKind::MonochromeSprite => {
                let texture_id = self.monochrome_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.monochrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.monochrome_sprites_iter.next();
                while self
                    .monochrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.monochrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::MonochromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::SubpixelSprite => {
                let texture_id = self.subpixel_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.subpixel_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.subpixel_sprites_iter.next();
                while self
                    .subpixel_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.subpixel_sprites_start = sprites_end;
                Some(PrimitiveBatch::SubpixelSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::PolychromeSprite => {
                let texture_id = self.polychrome_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.polychrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.polychrome_sprites_iter.next();
                while self
                    .polychrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.polychrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::PolychromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::Surface => {
                let surfaces_start = self.surfaces_start;
                let mut surfaces_end = surfaces_start + 1;
                self.surfaces_iter.next();
                while self
                    .surfaces_iter
                    .next_if(|surface| (surface.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    surfaces_end += 1;
                }
                self.surfaces_start = surfaces_end;
                Some(PrimitiveBatch::Surfaces(surfaces_start..surfaces_end))
            }
            PrimitiveKind::Group => {
                let groups_start = self.groups_start;
                let mut groups_end = groups_start + 1;
                self.groups_iter.next();
                while self
                    .groups_iter
                    .next_if(|group| (group.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    groups_end += 1;
                }
                self.groups_start = groups_end;
                Some(PrimitiveBatch::Groups(groups_start..groups_end))
            }
        }
    }
}

#[derive(Debug)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum PrimitiveBatch {
    Shadows(Range<usize>),
    Quads(Range<usize>),
    Paths(Range<usize>),
    Underlines(Range<usize>),
    MonochromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    SubpixelSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    PolychromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    Surfaces(Range<usize>),
    Groups(Range<usize>),
}

#[derive(Default, Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Quad {
    pub order: DrawOrder,
    pub border_style: BorderStyle,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub border_color: Hsla,
    pub corner_radii: Corners<ScaledPixels>,
    pub border_widths: Edges<ScaledPixels>,
}

impl From<Quad> for Primitive {
    fn from(quad: Quad) -> Self {
        Primitive::Quad(quad)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Underline {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub thickness: ScaledPixels,
    pub wavy: u32,
}

impl From<Underline> for Primitive {
    fn from(underline: Underline) -> Self {
        Primitive::Underline(underline)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
#[expect(missing_docs)]
pub struct Shadow {
    pub order: DrawOrder,
    pub blur_radius: ScaledPixels,
    pub bounds: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
}

impl From<Shadow> for Primitive {
    fn from(shadow: Shadow) -> Self {
        Primitive::Shadow(shadow)
    }
}

/// The style of a border.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub enum BorderStyle {
    /// A solid border.
    #[default]
    Solid = 0,
    /// A dashed border.
    Dashed = 1,
}

/// A data type representing a 2 dimensional transformation that can be applied to an element.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct TransformationMatrix {
    /// 2x2 matrix containing rotation and scale,
    /// stored row-major
    pub rotation_scale: [[f32; 2]; 2],
    /// translation vector
    pub translation: [f32; 2],
}

impl Eq for TransformationMatrix {}

impl TransformationMatrix {
    /// The unit matrix, has no effect.
    pub fn unit() -> Self {
        Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [0.0, 0.0],
        }
    }

    /// Move the origin by a given point
    pub fn translate(mut self, point: Point<ScaledPixels>) -> Self {
        self.compose(Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [point.x.0, point.y.0],
        })
    }

    /// Clockwise rotation in radians around the origin
    pub fn rotate(self, angle: Radians) -> Self {
        self.compose(Self {
            rotation_scale: [
                [angle.0.cos(), -angle.0.sin()],
                [angle.0.sin(), angle.0.cos()],
            ],
            translation: [0.0, 0.0],
        })
    }

    /// Scale around the origin
    pub fn scale(self, size: Size<f32>) -> Self {
        self.compose(Self {
            rotation_scale: [[size.width, 0.0], [0.0, size.height]],
            translation: [0.0, 0.0],
        })
    }

    /// Perform matrix multiplication with another transformation
    /// to produce a new transformation that is the result of
    /// applying both transformations: first, `other`, then `self`.
    #[inline]
    pub fn compose(self, other: TransformationMatrix) -> TransformationMatrix {
        if other == Self::unit() {
            return self;
        }
        // Perform matrix multiplication
        TransformationMatrix {
            rotation_scale: [
                [
                    self.rotation_scale[0][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][0],
                    self.rotation_scale[0][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][1],
                ],
                [
                    self.rotation_scale[1][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][0],
                    self.rotation_scale[1][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][1],
                ],
            ],
            translation: [
                self.translation[0]
                    + self.rotation_scale[0][0] * other.translation[0]
                    + self.rotation_scale[0][1] * other.translation[1],
                self.translation[1]
                    + self.rotation_scale[1][0] * other.translation[0]
                    + self.rotation_scale[1][1] * other.translation[1],
            ],
        }
    }

    /// Apply transformation to a point, mainly useful for debugging
    pub fn apply(&self, point: Point<Pixels>) -> Point<Pixels> {
        let input = [point.x.0, point.y.0];
        let mut output = self.translation;
        for (i, output_cell) in output.iter_mut().enumerate() {
            for (k, input_cell) in input.iter().enumerate() {
                *output_cell += self.rotation_scale[i][k] * *input_cell;
            }
        }
        Point::new(output[0].into(), output[1].into())
    }
}

impl Default for TransformationMatrix {
    fn default() -> Self {
        Self::unit()
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct MonochromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<MonochromeSprite> for Primitive {
    fn from(sprite: MonochromeSprite) -> Self {
        Primitive::MonochromeSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct SubpixelSprite {
    pub order: DrawOrder,
    pub pad: u32, // align to 8 bytes
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<SubpixelSprite> for Primitive {
    fn from(sprite: SubpixelSprite) -> Self {
        Primitive::SubpixelSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PolychromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub grayscale: bool,
    pub opacity: f32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub tile: AtlasTile,
}

impl From<PolychromeSprite> for Primitive {
    fn from(sprite: PolychromeSprite) -> Self {
        Primitive::PolychromeSprite(sprite)
    }
}

#[derive(Clone, Debug)]
#[allow(missing_docs)]
pub struct PaintMetalTexture {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    #[cfg(target_os = "macos")]
    pub texture: metal::Texture,
}

impl From<PaintMetalTexture> for Primitive {
    fn from(surface: PaintMetalTexture) -> Self {
        Primitive::Surface(surface)
    }
}

#[derive(Clone)]
#[allow(missing_docs)]
pub struct PaintGroup {
    order: DrawOrder,
    bounds: Bounds<ScaledPixels>,
    capture_bounds: Bounds<ScaledPixels>,
    content_mask: ContentMask<ScaledPixels>,
    plan: LogicalVisualPlan,
    scene: Arc<Scene>,
}

impl PaintGroup {
    pub(crate) fn new(
        order: DrawOrder,
        bounds: Bounds<ScaledPixels>,
        capture_bounds: Bounds<ScaledPixels>,
        content_mask: ContentMask<ScaledPixels>,
        plan: LogicalVisualPlan,
        scene: Scene,
    ) -> Self {
        Self {
            order,
            bounds,
            capture_bounds,
            content_mask,
            plan,
            scene: Arc::new(scene),
        }
    }

    #[cfg(any(test, feature = "backend-test-support"))]
    #[doc(hidden)]
    /// Builds a paint group for renderer backend tests outside the `gpui` crate.
    pub fn new_for_backend_test(
        order: DrawOrder,
        bounds: Bounds<ScaledPixels>,
        capture_bounds: Bounds<ScaledPixels>,
        content_mask: ContentMask<ScaledPixels>,
        plan: LogicalVisualPlan,
        scene: Scene,
    ) -> Self {
        Self::new(order, bounds, capture_bounds, content_mask, plan, scene)
    }

    /// Returns the semantic group bounds used for masks and material shapes.
    pub fn bounds(&self) -> Bounds<ScaledPixels> {
        self.bounds
    }

    /// Returns the screen-space bounds captured into the group intermediate.
    pub fn capture_bounds(&self) -> Bounds<ScaledPixels> {
        self.capture_bounds
    }

    /// Returns the content mask constraining this group.
    pub fn content_mask(&self) -> ContentMask<ScaledPixels> {
        self.content_mask
    }

    /// Returns the logical effect plan for this group.
    pub fn plan(&self) -> &LogicalVisualPlan {
        &self.plan
    }

    /// Returns the captured child scene rendered into the group intermediate.
    pub fn scene(&self) -> &Scene {
        self.scene.as_ref()
    }
}

impl From<PaintGroup> for Primitive {
    fn from(group: PaintGroup) -> Self {
        Primitive::Group(group)
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum CompositeEffect {
    Opacity(f32),
    SourceColorFilter(SourceColorFilter),
    SourceBlur(Pixels),
    SourceDirectionalBlur(Point<Pixels>),
    SourceMask(GroupShape),
    SourceMaskBeforeBlur(GroupShape),
    /// Clips the composited source/content image by an arbitrary filled path,
    /// rasterized as a coverage mask in the same screen/device space as the
    /// group. Unlike [`SourceMask`], this is not restricted to the analytic
    /// rounded-rect/superellipse family. When both are present on a group, the
    /// path mask wins for the source mask.
    SourceMaskPath(Arc<Path<Pixels>>),
    BackdropColorFilter(SourceColorFilter),
    BackdropBlur(Pixels),
    BackdropLens(CompositeBackdropLens<Pixels>),
    BackdropTint(Hsla),
    MaterialShape(SurfaceSilhouette<Pixels>),
    DropShadow(CompositeDropShadow<Pixels>),
    SurfaceShadow(CompositeSurfaceShadow<Pixels>),
    ProcessedContentGlow(CompositeProcessedContentGlow),
    RoundedMask(Corners<Pixels>),
    BlendMode(CompositeBlendMode),
}

/// The corner SDF family a render-group shape uses.
///
/// `RoundedRect` is the circular-arc corner used everywhere today. `Superellipse`
/// generalizes the corner to an Lp-norm (squircle) with the given exponent; the
/// exponent is the shader's `n` and a value of `2.0` is bit-identical to
/// `RoundedRect`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum GroupShapeKind {
    /// Circular-arc corners (the historical rounded rectangle).
    #[default]
    RoundedRect,
    /// Lp-norm (squircle) corners with the given exponent.
    Superellipse(f32),
}

impl GroupShapeKind {
    /// Returns the renderer-facing `(kind, exponent)` shader parameters.
    ///
    /// `RoundedRect` maps to `(0, 0.)`; `Superellipse(n)` maps to `(1, n)`.
    pub fn shader_params(self) -> (u32, f32) {
        match self {
            Self::RoundedRect => (0, 0.),
            Self::Superellipse(exponent) => (1, exponent),
        }
    }
}

/// A render-group shape in group layout coordinates.
///
/// V1 supports rounded rectangles matching the group's layout bounds, plus an
/// additive superellipse (squircle) corner variant that is visual-only. The type
/// is intentionally distinct from child border styling: a recipe may choose to
/// use the same radii for child style, content clipping, and surface material,
/// but raw render groups do not infer that coupling from children.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GroupShape {
    /// A rounded rectangle with circular-arc corners.
    RoundedRect(Corners<Pixels>),
    /// A superellipse (squircle) with Lp-norm corners.
    Superellipse {
        /// Corner radii in group layout coordinates.
        corner_radii: Corners<Pixels>,
        /// The Lp-norm exponent (`n`), clamped to at least `2.0`.
        exponent: f32,
    },
}

impl GroupShape {
    /// Returns a rectangular group shape.
    pub fn rectangle() -> Self {
        Self::RoundedRect(Corners::all(Pixels(0.)))
    }

    /// Returns a rounded-rectangle group shape.
    pub fn rounded_rect(corner_radii: Corners<Pixels>) -> Self {
        Self::RoundedRect(corner_radii.map(|radius| Pixels(radius.0.max(0.))))
    }

    /// Returns a superellipse (squircle) group shape.
    ///
    /// Radii are clamped to be non-negative and the exponent is clamped to at
    /// least `2.0` (below which the shape would pinch inside the rounded rect).
    pub fn superellipse(corner_radii: Corners<Pixels>, exponent: f32) -> Self {
        Self::Superellipse {
            corner_radii: corner_radii.map(|radius| Pixels(radius.0.max(0.))),
            exponent: exponent.max(2.0),
        }
    }

    pub(crate) fn corner_radii(&self) -> Corners<Pixels> {
        match self {
            Self::RoundedRect(corner_radii) => *corner_radii,
            Self::Superellipse { corner_radii, .. } => *corner_radii,
        }
    }

    pub(crate) fn shape_kind(&self) -> GroupShapeKind {
        match self {
            Self::RoundedRect(_) => GroupShapeKind::RoundedRect,
            Self::Superellipse { exponent, .. } => GroupShapeKind::Superellipse(*exponent),
        }
    }
}

/// Maximum number of analytic primitives in a surface silhouette union.
///
/// This is a shader ABI bound, not a product limit. Larger arbitrary shapes
/// should use an explicit sampled/source-mask path rather than silently falling
/// back from geometry-owned surface semantics.
pub const MAX_SURFACE_SILHOUETTE_PRIMITIVES: usize = 4;

/// One analytic shape member in a render-group surface silhouette.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct SurfacePrimitive<P: Clone + Copy + Debug + Default + PartialEq> {
    bounds: Bounds<P>,
    corner_radii: Corners<P>,
    shape_kind: GroupShapeKind,
}

impl SurfacePrimitive<Pixels> {
    /// Creates a surface primitive from explicit bounds and a group-shape family.
    pub fn new(bounds: Bounds<Pixels>, shape: GroupShape) -> Self {
        Self {
            bounds,
            corner_radii: shape.corner_radii(),
            shape_kind: shape.shape_kind(),
        }
    }

    fn scale(&self, factor: f32) -> SurfacePrimitive<ScaledPixels> {
        SurfacePrimitive {
            bounds: self.bounds.scale(factor),
            corner_radii: self.corner_radii.scale(factor),
            shape_kind: self.shape_kind,
        }
    }
}

impl<P: Clone + Copy + Debug + Default + PartialEq> SurfacePrimitive<P> {
    /// Returns this primitive's explicit scene-space bounds.
    pub fn bounds(&self) -> Bounds<P> {
        self.bounds
    }

    /// Returns this primitive's corner radii.
    pub fn corner_radii(&self) -> Corners<P> {
        self.corner_radii
    }

    /// Returns this primitive's analytic shape family.
    pub fn shape_kind(&self) -> GroupShapeKind {
        self.shape_kind
    }
}

/// A declared material/elevation support region for render-group surface effects.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub enum SurfaceSilhouette<P: Clone + Copy + Debug + Default + PartialEq> {
    GroupShape {
        corner_radii: Corners<P>,
        shape_kind: GroupShapeKind,
    },
    Union(Vec<SurfacePrimitive<P>>),
    GroupShapeUnion {
        corner_radii: Corners<P>,
        shape_kind: GroupShapeKind,
        primitives: Vec<SurfacePrimitive<P>>,
    },
}

/// Error returned when a surface silhouette cannot be represented by the current
/// bounded analytic shader ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum SurfaceSilhouetteError {
    Empty,
    TooManyPrimitives { limit: usize, requested: usize },
}

impl Display for SurfaceSilhouetteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(
                formatter,
                "surface silhouette union must contain a primitive"
            ),
            Self::TooManyPrimitives { limit, requested } => write!(
                formatter,
                "surface silhouette union has {requested} primitives, but the current limit is {limit}"
            ),
        }
    }
}

impl std::error::Error for SurfaceSilhouetteError {}

impl SurfaceSilhouette<Pixels> {
    /// Creates a surface silhouette from a small union of analytic primitives.
    ///
    /// The union denotes the max/OR of member coverage. It rejects empty or
    /// oversized inputs instead of silently dropping primitives or falling back
    /// to content-alpha capture.
    pub fn union(
        primitives: impl IntoIterator<Item = SurfacePrimitive<Pixels>>,
    ) -> Result<Self, SurfaceSilhouetteError> {
        let primitives = primitives.into_iter().collect::<Vec<_>>();
        if primitives.is_empty() {
            return Err(SurfaceSilhouetteError::Empty);
        }
        if primitives.len() > MAX_SURFACE_SILHOUETTE_PRIMITIVES {
            return Err(SurfaceSilhouetteError::TooManyPrimitives {
                limit: MAX_SURFACE_SILHOUETTE_PRIMITIVES,
                requested: primitives.len(),
            });
        }
        Ok(Self::Union(primitives))
    }

    /// Creates a surface silhouette from the render group's own bounds shape
    /// plus a small union of explicit analytic primitives.
    ///
    /// This lets callers model a group-owned slab joined to child-owned
    /// scene-space geometry without guessing the group's layout bounds or
    /// falling back to content-alpha capture.
    pub fn union_with_group_shape(
        shape: GroupShape,
        primitives: impl IntoIterator<Item = SurfacePrimitive<Pixels>>,
    ) -> Result<Self, SurfaceSilhouetteError> {
        let primitives = primitives.into_iter().collect::<Vec<_>>();
        let requested = primitives.len() + 1;
        if requested > MAX_SURFACE_SILHOUETTE_PRIMITIVES {
            return Err(SurfaceSilhouetteError::TooManyPrimitives {
                limit: MAX_SURFACE_SILHOUETTE_PRIMITIVES,
                requested,
            });
        }
        Ok(Self::GroupShapeUnion {
            corner_radii: shape.corner_radii(),
            shape_kind: shape.shape_kind(),
            primitives,
        })
    }

    fn scale(&self, factor: f32) -> SurfaceSilhouette<ScaledPixels> {
        match self {
            Self::GroupShape {
                corner_radii,
                shape_kind,
            } => SurfaceSilhouette::GroupShape {
                corner_radii: corner_radii.scale(factor),
                shape_kind: *shape_kind,
            },
            Self::Union(primitives) => SurfaceSilhouette::Union(
                primitives
                    .iter()
                    .map(|primitive| primitive.scale(factor))
                    .collect(),
            ),
            Self::GroupShapeUnion {
                corner_radii,
                shape_kind,
                primitives,
            } => SurfaceSilhouette::GroupShapeUnion {
                corner_radii: corner_radii.scale(factor),
                shape_kind: *shape_kind,
                primitives: primitives
                    .iter()
                    .map(|primitive| primitive.scale(factor))
                    .collect(),
            },
        }
    }
}

impl From<GroupShape> for SurfaceSilhouette<Pixels> {
    fn from(shape: GroupShape) -> Self {
        Self::GroupShape {
            corner_radii: shape.corner_radii(),
            shape_kind: shape.shape_kind(),
        }
    }
}

impl<P: Clone + Copy + Debug + Default + PartialEq> SurfaceSilhouette<P> {
    /// Returns whether this silhouette is a group-bounds shape.
    pub fn is_group_shape(&self) -> bool {
        matches!(self, Self::GroupShape { .. })
    }

    /// Returns explicit union primitives when this silhouette contains them.
    pub fn primitives(&self) -> &[SurfacePrimitive<P>] {
        match self {
            Self::GroupShape { .. } => &[],
            Self::Union(primitives) => primitives,
            Self::GroupShapeUnion { primitives, .. } => primitives,
        }
    }

    /// Returns fixed-width renderer-facing data for this silhouette.
    pub fn sprite_data(&self, group_bounds: Bounds<P>) -> SurfaceSilhouetteSpriteData<P> {
        let mut data = SurfaceSilhouetteSpriteData::default();
        match self {
            Self::GroupShape {
                corner_radii,
                shape_kind,
            } => {
                data.count = 1;
                data.bounds[0] = group_bounds;
                data.corner_radii[0] = *corner_radii;
                data.shape_kinds[0] = *shape_kind;
            }
            Self::Union(primitives) => {
                data.count = primitives.len() as u32;
                for (index, primitive) in primitives.iter().enumerate() {
                    data.bounds[index] = primitive.bounds;
                    data.corner_radii[index] = primitive.corner_radii;
                    data.shape_kinds[index] = primitive.shape_kind;
                }
            }
            Self::GroupShapeUnion {
                corner_radii,
                shape_kind,
                primitives,
            } => {
                data.count = primitives.len() as u32 + 1;
                data.bounds[0] = group_bounds;
                data.corner_radii[0] = *corner_radii;
                data.shape_kinds[0] = *shape_kind;
                for (index, primitive) in primitives.iter().enumerate() {
                    let data_index = index + 1;
                    data.bounds[data_index] = primitive.bounds;
                    data.corner_radii[data_index] = primitive.corner_radii;
                    data.shape_kinds[data_index] = primitive.shape_kind;
                }
            }
        }
        data
    }
}

/// Renderer-facing fixed-width surface-silhouette data.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct SurfaceSilhouetteSpriteData<P: Clone + Copy + Debug + Default + PartialEq> {
    pub count: u32,
    pub bounds: [Bounds<P>; MAX_SURFACE_SILHOUETTE_PRIMITIVES],
    pub corner_radii: [Corners<P>; MAX_SURFACE_SILHOUETTE_PRIMITIVES],
    pub shape_kinds: [GroupShapeKind; MAX_SURFACE_SILHOUETTE_PRIMITIVES],
}

impl<P: Clone + Copy + Debug + Default + PartialEq> Default for SurfaceSilhouetteSpriteData<P> {
    fn default() -> Self {
        Self {
            count: 0,
            bounds: [Bounds::default(); MAX_SURFACE_SILHOUETTE_PRIMITIVES],
            corner_radii: [Corners::default(); MAX_SURFACE_SILHOUETTE_PRIMITIVES],
            shape_kinds: [GroupShapeKind::RoundedRect; MAX_SURFACE_SILHOUETTE_PRIMITIVES],
        }
    }
}

impl<P: Clone + Copy + Debug + Default + PartialEq> SurfaceSilhouetteSpriteData<P> {
    /// Returns packed `(kind, exponent)` shader params for each primitive slot.
    pub fn shape_params(&self) -> [[f32; 4]; MAX_SURFACE_SILHOUETTE_PRIMITIVES] {
        let mut params = [[0., 0., 0., 0.]; MAX_SURFACE_SILHOUETTE_PRIMITIVES];
        for (index, shape_kind) in self.shape_kinds.iter().enumerate() {
            let (kind, exponent) = shape_kind.shader_params();
            params[index] = [kind as f32, exponent, 0., 0.];
        }
        params
    }
}

impl SurfaceSilhouetteSpriteData<ScaledPixels> {
    /// Returns the visual bounds of all packed surface primitives.
    pub fn bounds(&self) -> Option<Bounds<ScaledPixels>> {
        let count = (self.count as usize).min(MAX_SURFACE_SILHOUETTE_PRIMITIVES);
        let mut packed_bounds = self.bounds[..count].iter();
        let mut bounds = *packed_bounds.next()?;
        for next_bounds in packed_bounds {
            bounds = bounds.union(next_bounds);
        }
        Some(bounds)
    }
}

impl CompositeEffect {
    /// Creates an opacity effect for a render group.
    pub fn opacity(alpha: f32) -> Self {
        Self::Opacity(alpha.clamp(0., 1.))
    }

    /// Multiplies the source color channels by `factor`.
    pub fn brightness(factor: f32) -> Self {
        Self::SourceColorFilter(SourceColorFilter::brightness(factor))
    }

    /// Scales source color distance from mid-gray by `factor`.
    pub fn contrast(factor: f32) -> Self {
        Self::SourceColorFilter(SourceColorFilter::contrast(factor))
    }

    /// Adjusts source color saturation by `factor`.
    pub fn saturate(factor: f32) -> Self {
        Self::SourceColorFilter(SourceColorFilter::saturate(factor))
    }

    /// Mixes the source color toward grayscale by `amount`.
    pub fn grayscale(amount: f32) -> Self {
        Self::SourceColorFilter(SourceColorFilter::grayscale(amount))
    }

    /// Mixes the source color toward its inverse by `amount`.
    pub fn invert(amount: f32) -> Self {
        Self::SourceColorFilter(SourceColorFilter::invert(amount))
    }

    /// Applies an affine source color matrix to unpremultiplied RGB.
    pub fn color_matrix(matrix: [[f32; 3]; 3], offset: [f32; 3]) -> Self {
        Self::SourceColorFilter(SourceColorFilter::color_matrix(matrix, offset))
    }

    /// Rotates source hue by `degrees` around the gray axis.
    pub fn hue_rotate(degrees: f32) -> Self {
        Self::SourceColorFilter(SourceColorFilter::hue_rotate(degrees))
    }

    /// Applies a sepia tone to source color.
    pub fn sepia() -> Self {
        Self::SourceColorFilter(SourceColorFilter::sepia())
    }

    /// Maps source luma to a gradient between `dark` (black-luma) and `light`
    /// (white-luma) — a duotone. Folds into the source-color matrix; no resource.
    pub fn duotone(dark: Rgba, light: Rgba) -> Self {
        Self::SourceColorFilter(SourceColorFilter::duotone(dark, light))
    }

    /// Posterizes source color to `levels` (>= 2) bands.
    pub fn posterize(levels: f32) -> Self {
        Self::SourceColorFilter(SourceColorFilter::posterize(levels))
    }

    /// Thresholds source color to black/white by luma at `threshold` (0..1).
    pub fn threshold(threshold: f32) -> Self {
        Self::SourceColorFilter(SourceColorFilter::threshold(threshold))
    }

    /// Solarizes source color, inverting channels at or above `threshold` (0..1).
    pub fn solarize(threshold: f32) -> Self {
        Self::SourceColorFilter(SourceColorFilter::solarize(threshold))
    }

    /// Multiplies backdrop color channels by `factor`.
    pub fn backdrop_brightness(factor: f32) -> Self {
        Self::BackdropColorFilter(SourceColorFilter::brightness(factor))
    }

    /// Scales backdrop color distance from mid-gray by `factor`.
    pub fn backdrop_contrast(factor: f32) -> Self {
        Self::BackdropColorFilter(SourceColorFilter::contrast(factor))
    }

    /// Adjusts backdrop color saturation by `factor`.
    pub fn backdrop_saturate(factor: f32) -> Self {
        Self::BackdropColorFilter(SourceColorFilter::saturate(factor))
    }

    /// Mixes backdrop color toward grayscale by `amount`.
    pub fn backdrop_grayscale(amount: f32) -> Self {
        Self::BackdropColorFilter(SourceColorFilter::grayscale(amount))
    }

    /// Mixes backdrop color toward its inverse by `amount`.
    pub fn backdrop_invert(amount: f32) -> Self {
        Self::BackdropColorFilter(SourceColorFilter::invert(amount))
    }

    /// Applies an affine backdrop color matrix to unpremultiplied RGB.
    pub fn backdrop_color_matrix(matrix: [[f32; 3]; 3], offset: [f32; 3]) -> Self {
        Self::BackdropColorFilter(SourceColorFilter::color_matrix(matrix, offset))
    }

    /// Applies a Gaussian blur to the composited source image.
    pub fn source_blur(radius: Pixels) -> Self {
        Self::SourceBlur(Pixels(radius.0.max(0.)))
    }

    /// Applies a single-pass directional (motion) blur to the composited source
    /// image along one vector.
    ///
    /// `angle` is in radians (`0` points along `+x`) and `length` is the blur
    /// half-extent in pixels. The stored vector is `direction * length`, a
    /// shader-friendly precomputed streak. When directional blur is active it
    /// replaces the isotropic [`source_blur`](Self::source_blur) for source
    /// sampling: if both are set on a group, directional blur wins.
    pub fn directional_blur(angle: f32, length: Pixels) -> Self {
        let l = length.0.max(0.);
        Self::SourceDirectionalBlur(point(px(l * angle.cos()), px(l * angle.sin())))
    }

    /// Clips the composited source/content image by a group shape.
    pub fn source_mask(shape: GroupShape) -> Self {
        Self::SourceMask(shape)
    }

    /// Clips source/content before subsequent source blur samples are taken.
    pub fn source_mask_before_blur(shape: GroupShape) -> Self {
        Self::SourceMaskBeforeBlur(shape)
    }

    /// Clips the composited source/content image by an arbitrary filled path.
    ///
    /// The path is rasterized into a coverage mask in the group's screen/device
    /// space; its alpha selects the source. This is the first multi-pass group
    /// effect and only feeds the source-mask consumer (not material shape or
    /// surface shadow).
    pub fn source_mask_path(path: Arc<Path<Pixels>>) -> Self {
        Self::SourceMaskPath(path)
    }

    /// Applies a Gaussian blur to the already-rendered backdrop under the group.
    pub fn backdrop_blur(radius: Pixels) -> Self {
        Self::BackdropBlur(Pixels(radius.0.max(0.)))
    }

    /// Refracts already-rendered backdrop pixels through the group's material
    /// shape, producing lensing that is strongest near the group edge.
    pub fn backdrop_lens(
        refraction_radius: Pixels,
        rim_width: Pixels,
        chromatic_aberration: Pixels,
        highlight_strength: f32,
        shadow_strength: f32,
        light_direction: Point<f32>,
    ) -> Self {
        Self::BackdropLens(CompositeBackdropLens::new(
            refraction_radius,
            rim_width,
            chromatic_aberration,
            highlight_strength,
            shadow_strength,
            light_direction,
        ))
    }

    /// Draws a translucent tint over the backdrop material under the group.
    pub fn backdrop_tint(color: Hsla) -> Self {
        Self::BackdropTint(color)
    }

    /// Defines the material domain used by backdrop materials and lens normals.
    pub fn material_shape(shape: impl Into<SurfaceSilhouette<Pixels>>) -> Self {
        Self::MaterialShape(shape.into())
    }

    /// Draws a drop shadow from the composited source image's alpha channel.
    pub fn drop_shadow(offset: Point<Pixels>, blur_radius: Pixels, color: Hsla) -> Self {
        Self::drop_shadow_with_mode(offset, blur_radius, color, RenderGroupShadowMode::Separable)
    }

    /// Draws a drop shadow from the composited source image's alpha channel
    /// using an explicit render-group shadow mode.
    pub fn drop_shadow_with_mode(
        offset: Point<Pixels>,
        blur_radius: Pixels,
        color: Hsla,
        mode: RenderGroupShadowMode,
    ) -> Self {
        Self::DropShadow(CompositeDropShadow {
            shadow: CompositeShadow {
                offset,
                blur_radius: Pixels(blur_radius.0.max(0.)),
                color,
                mode,
            },
        })
    }

    /// Draws a drop shadow from an explicit surface/material shape.
    pub fn surface_shadow(
        shape: impl Into<SurfaceSilhouette<Pixels>>,
        offset: Point<Pixels>,
        blur_radius: Pixels,
        color: Hsla,
    ) -> Self {
        Self::surface_shadow_with_mode(
            shape,
            offset,
            blur_radius,
            color,
            RenderGroupShadowMode::Exact,
        )
    }

    /// Draws a drop shadow from an explicit surface/material shape using an
    /// explicit render-group shadow mode.
    pub fn surface_shadow_with_mode(
        shape: impl Into<SurfaceSilhouette<Pixels>>,
        offset: Point<Pixels>,
        blur_radius: Pixels,
        color: Hsla,
        mode: RenderGroupShadowMode,
    ) -> Self {
        Self::SurfaceShadow(CompositeSurfaceShadow {
            silhouette: shape.into(),
            shadow: CompositeShadow {
                offset,
                blur_radius: Pixels(blur_radius.0.max(0.)),
                color,
                mode,
            },
        })
    }

    /// Draws a tinted glow from processed content pixels.
    pub fn processed_content_glow(
        stages: impl IntoIterator<Item = DerivedStage>,
        glow: Glow,
    ) -> Self {
        Self::ProcessedContentGlow(CompositeProcessedContentGlow {
            stages: stages.into_iter().collect(),
            color: glow.color,
        })
    }

    /// Masks the composited source image by a rounded rectangle matching the
    /// render group's layout bounds.
    pub fn rounded_mask(corner_radii: Corners<Pixels>) -> Self {
        Self::RoundedMask(corner_radii)
    }

    /// Applies a final blend mode when compositing the group against its backdrop.
    pub fn blend_mode(mode: CompositeBlendMode) -> Self {
        Self::BlendMode(mode)
    }

    /// Returns whether this effect leaves the composited image unchanged.
    pub fn is_identity(&self) -> bool {
        match self {
            Self::Opacity(alpha) => (*alpha - 1.).abs() <= f32::EPSILON,
            Self::SourceColorFilter(filter) => filter.is_identity(),
            Self::SourceBlur(radius) => radius.0 <= f32::EPSILON,
            Self::SourceDirectionalBlur(v) => {
                v.x.0.abs() <= f32::EPSILON && v.y.0.abs() <= f32::EPSILON
            }
            Self::SourceMask(_) => false,
            Self::SourceMaskBeforeBlur(_) => false,
            Self::SourceMaskPath(_) => false,
            Self::BackdropColorFilter(filter) => filter.is_identity(),
            Self::BackdropBlur(radius) => radius.0 <= f32::EPSILON,
            Self::BackdropLens(lens) => lens.is_identity(),
            Self::BackdropTint(color) => color.a <= f32::EPSILON,
            Self::MaterialShape(_) => true,
            Self::DropShadow(shadow) => shadow.shadow.color.a <= f32::EPSILON,
            Self::SurfaceShadow(shadow) => shadow.shadow.color.a <= f32::EPSILON,
            Self::ProcessedContentGlow(glow) => glow.color.a <= f32::EPSILON,
            Self::RoundedMask(_) => false,
            Self::BlendMode(mode) => *mode == CompositeBlendMode::Normal,
        }
    }

    /// Returns whether this effect needs pixels from the parent render target.
    pub fn reads_backdrop(&self) -> bool {
        match self {
            Self::BackdropColorFilter(filter) => !filter.is_identity(),
            Self::BackdropBlur(radius) => radius.0 > f32::EPSILON,
            Self::BackdropLens(lens) => !lens.is_identity(),
            Self::BackdropTint(color) => color.a > f32::EPSILON,
            Self::BlendMode(mode) => *mode != CompositeBlendMode::Normal,
            Self::Opacity(_)
            | Self::SourceColorFilter(_)
            | Self::SourceBlur(_)
            | Self::SourceDirectionalBlur(_)
            | Self::SourceMask(_)
            | Self::SourceMaskBeforeBlur(_)
            | Self::SourceMaskPath(_)
            | Self::DropShadow(_)
            | Self::SurfaceShadow(_)
            | Self::ProcessedContentGlow(_)
            | Self::MaterialShape(_)
            | Self::RoundedMask(_) => false,
        }
    }
}

/// Shared render-group shadow profile after its source support has been chosen.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct CompositeShadow<P: Clone + Debug + Default + PartialEq> {
    pub offset: Point<P>,
    pub blur_radius: P,
    pub color: Hsla,
    pub mode: RenderGroupShadowMode,
}

/// A drop shadow effect derived from a render group's composited source alpha.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct CompositeDropShadow<P: Clone + Debug + Default + PartialEq> {
    pub shadow: CompositeShadow<P>,
}

/// A drop shadow effect derived from an explicit surface/material shape.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct CompositeSurfaceShadow<P: Clone + Copy + Debug + Default + PartialEq> {
    pub silhouette: SurfaceSilhouette<P>,
    pub shadow: CompositeShadow<P>,
}

impl CompositeSurfaceShadow<ScaledPixels> {
    /// Returns the tight draw bounds for this shadow under a group content mask.
    pub fn draw_bounds(
        &self,
        group_bounds: Bounds<ScaledPixels>,
        content_mask: ContentMask<ScaledPixels>,
    ) -> Option<Bounds<ScaledPixels>> {
        let silhouette_bounds = self.silhouette.sprite_data(group_bounds).bounds()?;
        let bounds = (silhouette_bounds + self.shadow.offset)
            .dilate(gaussian_kernel_outset(self.shadow.blur_radius))
            .intersect(&content_mask.bounds);
        (!bounds.is_empty()).then_some(bounds)
    }
}

/// A glow derived from processed content pixels.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct CompositeProcessedContentGlow {
    pub stages: Vec<DerivedStage>,
    pub color: Hsla,
}

/// Physical quality mode for render-group shadows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum RenderGroupShadowMode {
    /// Full-resolution separable Gaussian blur. This is the default mode for
    /// content-alpha shadows because it preserves the Gaussian model with O(2r)
    /// samples/pixel.
    #[default]
    Separable,
    /// Full-resolution two-dimensional Gaussian blur. Intended for debug
    /// comparison against the separable path; cost scales as O(r²).
    Exact,
    /// Lower-resolution content-alpha blur. This is a named future quality tier;
    /// current planners reject it loudly instead of silently substituting another mode.
    Downsampled,
}

impl RenderGroupShadowMode {
    /// Stable label for diagnostics and inspector surfaces.
    pub fn label(self) -> &'static str {
        match self {
            Self::Separable => "separable",
            Self::Exact => "exact-debug",
            Self::Downsampled => "downsampled",
        }
    }
}

/// A validated processed-content glow in device pixels.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct CompositeProcessedContentGlowPlan {
    pub luma_threshold: f32,
    pub blur_radius: ScaledPixels,
    pub color: Hsla,
}

/// Backdrop lensing derived from a render group's material shape.
///
/// The lens displaces parent-target samples along the material shape's signed
/// distance gradient. Refraction and chromatic-aberration radii are strongest
/// within `rim_width` of the shape edge and fade toward the center.
#[derive(Copy, Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct CompositeBackdropLens<P: Clone + Copy + Debug + Default + PartialEq> {
    refraction_radius: P,
    rim_width: P,
    chromatic_aberration: P,
    highlight_strength: f32,
    shadow_strength: f32,
    light_direction: Point<f32>,
}

impl CompositeBackdropLens<Pixels> {
    /// Creates a backdrop lens effect.
    pub fn new(
        refraction_radius: Pixels,
        rim_width: Pixels,
        chromatic_aberration: Pixels,
        highlight_strength: f32,
        shadow_strength: f32,
        light_direction: Point<f32>,
    ) -> Self {
        Self {
            refraction_radius: Pixels(refraction_radius.0.max(0.)),
            rim_width: Pixels(rim_width.0.max(0.)),
            chromatic_aberration: Pixels(chromatic_aberration.0.max(0.)),
            highlight_strength: highlight_strength.max(0.),
            shadow_strength: shadow_strength.max(0.),
            light_direction,
        }
    }

    fn scale(self, scale_factor: f32) -> CompositeBackdropLens<ScaledPixels> {
        CompositeBackdropLens {
            refraction_radius: self.refraction_radius.scale(scale_factor),
            rim_width: self.rim_width.scale(scale_factor),
            chromatic_aberration: self.chromatic_aberration.scale(scale_factor),
            highlight_strength: self.highlight_strength,
            shadow_strength: self.shadow_strength,
            light_direction: self.light_direction,
        }
    }

    /// Returns whether this lens leaves backdrop pixels unchanged.
    pub fn is_identity(&self) -> bool {
        self.rim_width.0 <= f32::EPSILON
            || (self.refraction_radius.0 <= f32::EPSILON
                && self.chromatic_aberration.0 <= f32::EPSILON
                && self.highlight_strength <= f32::EPSILON
                && self.shadow_strength <= f32::EPSILON)
    }
}

impl CompositeBackdropLens<ScaledPixels> {
    /// Returns whether this lens leaves backdrop pixels unchanged.
    pub fn is_identity(&self) -> bool {
        self.rim_width.0 <= f32::EPSILON
            || (self.refraction_radius.0 <= f32::EPSILON
                && self.chromatic_aberration.0 <= f32::EPSILON
                && self.highlight_strength <= f32::EPSILON
                && self.shadow_strength <= f32::EPSILON)
    }
}

impl<P: Clone + Copy + Debug + Default + PartialEq> CompositeBackdropLens<P> {
    /// Returns the maximum backdrop-sample displacement.
    pub fn refraction_radius(&self) -> P {
        self.refraction_radius
    }

    /// Returns the edge band over which lensing fades in.
    pub fn rim_width(&self) -> P {
        self.rim_width
    }

    /// Returns the per-channel displacement around the refracted sample.
    pub fn chromatic_aberration(&self) -> P {
        self.chromatic_aberration
    }

    /// Returns the rim highlight strength.
    pub fn highlight_strength(&self) -> f32 {
        self.highlight_strength
    }

    /// Returns the opposite-edge darkening strength.
    pub fn shadow_strength(&self) -> f32 {
        self.shadow_strength
    }

    /// Returns the screen-space light direction.
    pub fn light_direction(&self) -> Point<f32> {
        self.light_direction
    }
}

/// Lens parameters for a glass surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassLens {
    refraction_radius: Pixels,
    rim_width: Pixels,
    chromatic_aberration: Pixels,
    highlight_strength: f32,
    shadow_strength: f32,
    light_direction: Point<f32>,
}

impl GlassLens {
    /// Creates an edge refraction lens.
    pub fn edge_refraction(refraction_radius: Pixels) -> Self {
        let refraction_radius = Pixels(refraction_radius.0.max(0.));
        Self {
            refraction_radius,
            rim_width: refraction_radius,
            chromatic_aberration: Pixels(0.),
            highlight_strength: 0.,
            shadow_strength: 0.,
            light_direction: Point { x: -0.55, y: -0.85 },
        }
    }

    /// Creates a soft edge refraction lens.
    pub fn soft_edge_refraction(refraction_radius: Pixels) -> Self {
        Self::edge_refraction(refraction_radius)
    }

    /// Sets the rim width over which the lens fades in.
    pub fn rim(mut self, rim_width: Pixels) -> Self {
        self.rim_width = Pixels(rim_width.0.max(0.));
        self
    }

    /// Sets per-channel chromatic split around the refracted sample.
    pub fn chromatic_split(mut self, chromatic_aberration: Pixels) -> Self {
        self.chromatic_aberration = Pixels(chromatic_aberration.0.max(0.));
        self
    }

    /// Sets rim lighting strengths.
    pub fn lighting(mut self, highlight_strength: f32, shadow_strength: f32) -> Self {
        self.highlight_strength = highlight_strength.max(0.);
        self.shadow_strength = shadow_strength.max(0.);
        self
    }

    /// Sets the screen-space light direction.
    pub fn light_direction(mut self, light_direction: Point<f32>) -> Self {
        self.light_direction = light_direction;
        self
    }

    fn into_composite_lens(self) -> CompositeBackdropLens<Pixels> {
        CompositeBackdropLens::new(
            self.refraction_radius,
            self.rim_width,
            self.chromatic_aberration,
            self.highlight_strength,
            self.shadow_strength,
            self.light_direction,
        )
    }

    fn is_identity(&self) -> bool {
        self.into_composite_lens().is_identity()
    }
}

/// A backdrop-sampling glass surface for a render group.
#[derive(Clone, Debug, PartialEq)]
pub struct GlassSurface {
    shape: GroupShape,
    frost_radius: Pixels,
    color_filter: SourceColorFilter,
    tint: Hsla,
    lens: Option<GlassLens>,
}

impl GlassSurface {
    /// Creates a glass surface for the given material shape.
    pub fn for_shape(shape: GroupShape) -> Self {
        Self {
            shape,
            frost_radius: Pixels(0.),
            color_filter: SourceColorFilter::identity(),
            tint: transparent_black(),
            lens: None,
        }
    }

    /// Applies backdrop frost/blur to the surface.
    pub fn frost(mut self, radius: Pixels) -> Self {
        self.frost_radius = Pixels(radius.0.max(0.));
        self
    }

    /// Applies a backdrop tint over the surface.
    pub fn tint(mut self, color: Hsla) -> Self {
        self.tint = color;
        self
    }

    /// Applies a lens to the surface.
    pub fn lens(mut self, lens: GlassLens) -> Self {
        self.lens = Some(lens);
        self
    }

    /// Multiplies backdrop color channels by `factor`.
    pub fn brightness(mut self, factor: f32) -> Self {
        self.color_filter = self
            .color_filter
            .then(SourceColorFilter::brightness(factor));
        self
    }

    /// Scales backdrop color distance from mid-gray by `factor`.
    pub fn contrast(mut self, factor: f32) -> Self {
        self.color_filter = self.color_filter.then(SourceColorFilter::contrast(factor));
        self
    }

    /// Adjusts backdrop color saturation by `factor`.
    pub fn saturate(mut self, factor: f32) -> Self {
        self.color_filter = self.color_filter.then(SourceColorFilter::saturate(factor));
        self
    }

    /// Mixes backdrop color toward grayscale by `amount`.
    pub fn grayscale(mut self, amount: f32) -> Self {
        self.color_filter = self.color_filter.then(SourceColorFilter::grayscale(amount));
        self
    }

    /// Mixes backdrop color toward its inverse by `amount`.
    pub fn invert(mut self, amount: f32) -> Self {
        self.color_filter = self.color_filter.then(SourceColorFilter::invert(amount));
        self
    }

    pub(crate) fn push_effects(&self, effects: &mut Vec<CompositeEffect>) {
        let has_material = self.frost_radius.0 > f32::EPSILON
            || !self.color_filter.is_identity()
            || self.lens.is_some_and(|lens| !lens.is_identity())
            || self.tint.a > f32::EPSILON;
        if !has_material {
            return;
        }

        effects.push(CompositeEffect::material_shape(self.shape));
        if self.frost_radius.0 > f32::EPSILON {
            effects.push(CompositeEffect::backdrop_blur(self.frost_radius));
        }
        if !self.color_filter.is_identity() {
            effects.push(CompositeEffect::BackdropColorFilter(self.color_filter));
        }
        if let Some(lens) = self.lens {
            effects.push(CompositeEffect::BackdropLens(lens.into_composite_lens()));
        }
        if self.tint.a > f32::EPSILON {
            effects.push(CompositeEffect::backdrop_tint(self.tint));
        }
    }
}

/// Content/source effects for a render group.
#[derive(Clone, Debug, PartialEq)]
pub struct ContentLayer {
    source_mask: Option<GroupShape>,
    source_blur: Pixels,
    color_filter: SourceColorFilter,
    stages: Option<Vec<ContentStage>>,
}

impl Default for ContentLayer {
    fn default() -> Self {
        Self {
            source_mask: None,
            source_blur: Pixels(0.),
            color_filter: SourceColorFilter::identity(),
            stages: None,
        }
    }
}

/// An ordered content/source stage.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ContentStage {
    /// Clips content to the given group shape at this point in the content chain.
    ClipTo(GroupShape),
    /// Applies exact Gaussian blur at this point in the content chain.
    ExactBlur(Pixels),
}

impl ContentStage {
    /// Clips content to the given group shape at this point in the content chain.
    pub fn clip_to(shape: GroupShape) -> Self {
        Self::ClipTo(shape)
    }

    /// Applies exact Gaussian blur at this point in the content chain.
    pub fn exact_blur(radius: Pixels) -> Self {
        Self::ExactBlur(Pixels(radius.0.max(0.)))
    }
}

impl ContentLayer {
    /// Creates an ordered content layer where stage order is part of the visual meaning.
    pub fn staged(stages: impl IntoIterator<Item = ContentStage>) -> Self {
        Self {
            stages: Some(stages.into_iter().collect()),
            ..Self::default()
        }
    }

    /// Creates a content layer clipped to the given shape.
    pub fn clipped_to(shape: GroupShape) -> Self {
        Self {
            source_mask: Some(shape),
            ..Self::default()
        }
    }

    /// Creates an unclipped content layer.
    pub fn unclipped() -> Self {
        Self::default()
    }

    /// Applies a source/content blur.
    pub fn blur(mut self, radius: Pixels) -> Self {
        let radius = Pixels(radius.0.max(0.));
        if let Some(stages) = &mut self.stages {
            stages.push(ContentStage::ExactBlur(radius));
        } else {
            self.source_blur = radius;
        }
        self
    }

    /// Clips content to the given group shape.
    pub fn clip_to(mut self, shape: GroupShape) -> Self {
        if let Some(stages) = &mut self.stages {
            stages.push(ContentStage::ClipTo(shape));
        } else {
            self.source_mask = Some(shape);
        }
        self
    }

    /// Multiplies content color channels by `factor`.
    pub fn brightness(mut self, factor: f32) -> Self {
        self.color_filter = self
            .color_filter
            .then(SourceColorFilter::brightness(factor));
        self
    }

    /// Scales content color distance from mid-gray by `factor`.
    pub fn contrast(mut self, factor: f32) -> Self {
        self.color_filter = self.color_filter.then(SourceColorFilter::contrast(factor));
        self
    }

    /// Adjusts content color saturation by `factor`.
    pub fn saturate(mut self, factor: f32) -> Self {
        self.color_filter = self.color_filter.then(SourceColorFilter::saturate(factor));
        self
    }

    /// Mixes content color toward grayscale by `amount`.
    pub fn grayscale(mut self, amount: f32) -> Self {
        self.color_filter = self.color_filter.then(SourceColorFilter::grayscale(amount));
        self
    }

    /// Mixes content color toward its inverse by `amount`.
    pub fn invert(mut self, amount: f32) -> Self {
        self.color_filter = self.color_filter.then(SourceColorFilter::invert(amount));
        self
    }

    pub(crate) fn push_effects(&self, effects: &mut Vec<CompositeEffect>) {
        if let Some(stages) = &self.stages {
            for (index, stage) in stages.iter().enumerate() {
                match *stage {
                    ContentStage::ClipTo(shape) => {
                        let blur_after = stages[index + 1..]
                            .iter()
                            .any(|stage| matches!(stage, ContentStage::ExactBlur(radius) if radius.0 > f32::EPSILON));
                        if blur_after {
                            effects.push(CompositeEffect::source_mask_before_blur(shape));
                        } else {
                            effects.push(CompositeEffect::source_mask(shape));
                        }
                    }
                    ContentStage::ExactBlur(radius) => {
                        if radius.0 > f32::EPSILON {
                            effects.push(CompositeEffect::source_blur(radius));
                        }
                    }
                }
            }
        } else if let Some(mask) = self.source_mask {
            effects.push(CompositeEffect::source_mask(mask));
        }
        if self.source_blur.0 > f32::EPSILON {
            effects.push(CompositeEffect::source_blur(self.source_blur));
        }
        if !self.color_filter.is_identity() {
            effects.push(CompositeEffect::SourceColorFilter(self.color_filter));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DerivedLayerSource {
    ContentAlpha(RenderGroupShadowMode),
    SurfaceShape {
        shape: GroupShape,
        mode: RenderGroupShadowMode,
    },
    ProcessedContent,
}

/// Extra layers derived from named render-group provenance.
#[derive(Clone, Debug, PartialEq)]
pub struct DerivedLayer {
    source: DerivedLayerSource,
    effects: Vec<CompositeEffect>,
}

impl DerivedLayer {
    /// Creates an empty content-alpha derived layer builder.
    pub fn from_content_alpha() -> Self {
        Self::from_content_alpha_with_shadow_mode(RenderGroupShadowMode::Separable)
    }

    /// Creates an empty content-alpha derived layer builder with an explicit
    /// shadow quality mode for any later [`shadow`](Self::shadow) calls.
    pub fn from_content_alpha_with_shadow_mode(mode: RenderGroupShadowMode) -> Self {
        Self {
            source: DerivedLayerSource::ContentAlpha(mode),
            effects: Vec::new(),
        }
    }

    /// Creates an empty surface-shape derived layer builder.
    pub fn from_surface_shape(shape: GroupShape) -> Self {
        Self::from_surface_shape_shadow_mode(shape, RenderGroupShadowMode::Exact)
    }

    /// Creates an empty surface-shape derived layer builder with an explicit
    /// shadow quality mode for any later [`shadow`](Self::shadow) calls.
    pub fn from_surface_shape_shadow_mode(shape: GroupShape, mode: RenderGroupShadowMode) -> Self {
        Self {
            source: DerivedLayerSource::SurfaceShape { shape, mode },
            effects: Vec::new(),
        }
    }

    /// Creates a processed-content derived layer builder.
    pub fn from_processed_content(
        stages: impl IntoIterator<Item = DerivedStage>,
    ) -> ProcessedContentDerivedLayer {
        ProcessedContentDerivedLayer {
            stages: stages.into_iter().collect(),
        }
    }

    /// Adds a shadow derived from this layer's named provenance.
    ///
    /// `blur_radius` is consumed as the Gaussian sigma (the backend kernel
    /// weights by `exp(-d^2 / 2*sigma^2)` and samples `3*sigma` taps), and is
    /// capped after display scaling at `MAX_EXACT_GAUSSIAN_SIGMA` scaled
    /// pixels — a shadow over the cap is planning-rejected (it does not
    /// paint) and warn-logged once. Larger product shadows should use the
    /// element `BoxShadow` primitive instead.
    pub fn shadow(mut self, offset: Point<Pixels>, blur_radius: Pixels, color: Hsla) -> Self {
        let effect = match self.source {
            DerivedLayerSource::ContentAlpha(mode) => {
                CompositeEffect::drop_shadow_with_mode(offset, blur_radius, color, mode)
            }
            DerivedLayerSource::SurfaceShape { shape, mode } => {
                CompositeEffect::surface_shadow_with_mode(shape, offset, blur_radius, color, mode)
            }
            DerivedLayerSource::ProcessedContent => {
                panic!("processed-content derived layers support glow(), not shadow()")
            }
        };
        self.effects.push(effect);
        self
    }

    pub(crate) fn push_effects(&self, effects: &mut Vec<CompositeEffect>) {
        effects.extend(self.effects.iter().cloned());
    }
}

/// A builder for layers derived from processed content pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessedContentDerivedLayer {
    stages: Vec<DerivedStage>,
}

impl ProcessedContentDerivedLayer {
    /// Adds a tinted glow derived from processed content pixels.
    pub fn glow(self, glow: Glow) -> DerivedLayer {
        DerivedLayer {
            source: DerivedLayerSource::ProcessedContent,
            effects: vec![CompositeEffect::processed_content_glow(self.stages, glow)],
        }
    }
}

/// An ordered processed-content derived stage.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum DerivedStage {
    /// Extracts pixels whose unpremultiplied luma is at or above the threshold.
    ThresholdLuma(LumaThreshold),
    /// Applies exact Gaussian blur to the extracted processed-content pixels.
    ExactBlur(Pixels),
}

impl DerivedStage {
    /// Extracts pixels whose unpremultiplied luma is at or above the threshold.
    pub fn threshold_luma(threshold: LumaThreshold) -> Self {
        Self::ThresholdLuma(threshold)
    }

    /// Applies exact Gaussian blur to the extracted processed-content pixels.
    pub fn exact_blur(radius: Pixels) -> Self {
        Self::ExactBlur(Pixels(radius.0.max(0.)))
    }
}

/// Luma threshold used by processed-content derived stages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LumaThreshold {
    value: f32,
}

impl LumaThreshold {
    /// Creates a threshold that keeps pixels at or above `value`.
    pub fn above(value: f32) -> Self {
        Self {
            value: value.clamp(0., 1.),
        }
    }

    /// Returns the normalized threshold value.
    pub fn value(self) -> f32 {
        self.value
    }
}

/// A tinted glow derived from processed content pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    color: Hsla,
}

impl Glow {
    /// Creates a glow with the given straight RGBA color.
    pub fn tinted(color: Hsla) -> Self {
        Self { color }
    }

    /// Returns the glow color.
    pub fn color(self) -> Hsla {
        self.color
    }
}

/// Final blend operation used when compositing a render group against its backdrop.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum CompositeBlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    PlusLighter,
    Difference,
    Exclusion,
    HardLight,
}

impl CompositeBlendMode {
    /// Returns the stable shader discriminant for this blend mode.
    pub fn shader_code(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::Multiply => 1,
            Self::Screen => 2,
            Self::Overlay => 3,
            Self::Darken => 4,
            Self::Lighten => 5,
            Self::PlusLighter => 6,
            Self::Difference => 7,
            Self::Exclusion => 8,
            Self::HardLight => 9,
        }
    }
}

/// Final render-group compositing options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Composite {
    opacity: f32,
    blend_mode: CompositeBlendMode,
}

impl Default for Composite {
    fn default() -> Self {
        Self::normal()
    }
}

impl Composite {
    /// Returns normal final compositing.
    pub fn normal() -> Self {
        Self {
            opacity: 1.,
            blend_mode: CompositeBlendMode::Normal,
        }
    }

    /// Sets whole-group opacity at the parent boundary.
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0., 1.);
        self
    }

    /// Sets the final blend mode.
    pub fn blend_mode(mut self, blend_mode: CompositeBlendMode) -> Self {
        self.blend_mode = blend_mode;
        self
    }

    pub(crate) fn push_effects(&self, effects: &mut Vec<CompositeEffect>) {
        if (self.opacity - 1.).abs() > f32::EPSILON {
            effects.push(CompositeEffect::opacity(self.opacity));
        }
        if self.blend_mode != CompositeBlendMode::Normal {
            effects.push(CompositeEffect::blend_mode(self.blend_mode));
        }
    }
}

/// Structured semantic render-group input before renderer planning.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticRenderGroupSpec {
    surface: Option<GlassSurface>,
    content: ContentLayer,
    derived_layers: Vec<DerivedLayer>,
    composite: Composite,
}

impl Default for SemanticRenderGroupSpec {
    fn default() -> Self {
        Self {
            surface: None,
            content: ContentLayer::default(),
            derived_layers: Vec::new(),
            composite: Composite::normal(),
        }
    }
}

impl SemanticRenderGroupSpec {
    /// Builds a semantic render-group input value from authored layers.
    ///
    /// This is primarily useful for inspectors and tests that need to ask the
    /// planner how semantic authoring lowers, without duplicating private
    /// lowering rules.
    pub fn from_layers(
        surface: Option<GlassSurface>,
        content: ContentLayer,
        derived_layers: impl IntoIterator<Item = DerivedLayer>,
        composite: Composite,
    ) -> Self {
        Self {
            surface,
            content,
            derived_layers: derived_layers.into_iter().collect(),
            composite,
        }
    }

    pub(crate) fn surface(&mut self, surface: GlassSurface) {
        self.surface = Some(surface);
    }

    pub(crate) fn content(&mut self, content: ContentLayer) {
        self.content = content;
    }

    pub(crate) fn derived(&mut self, derived: DerivedLayer) {
        self.derived_layers.push(derived);
    }

    pub(crate) fn composite(&mut self, composite: Composite) {
        self.composite = composite;
    }

    fn effects(&self) -> Vec<CompositeEffect> {
        let mut effects = Vec::new();
        if let Some(surface) = &self.surface {
            surface.push_effects(&mut effects);
        }
        self.content.push_effects(&mut effects);
        for derived in &self.derived_layers {
            derived.push_effects(&mut effects);
        }
        self.composite.push_effects(&mut effects);
        effects
    }

    fn has_active_effect(&self) -> bool {
        self.effects().iter().any(|effect| !effect.is_identity())
    }
}

/// Render-group input after API mode validation and before logical planning.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RenderGroupInput {
    /// No render-group effects have been authored yet.
    None,
    /// Semantic layer input: surface, content, derived layers, and boundary composite.
    Semantic(SemanticRenderGroupSpec),
    /// Raw GPUI primitive syntax with documented normalization rules.
    Raw(Vec<CompositeEffect>),
}

impl Default for RenderGroupInput {
    fn default() -> Self {
        Self::None
    }
}

impl RenderGroupInput {
    pub(crate) fn semantic_mut(&mut self) -> &mut SemanticRenderGroupSpec {
        match self {
            Self::None => {
                *self = Self::Semantic(SemanticRenderGroupSpec::default());
                match self {
                    Self::Semantic(spec) => spec,
                    Self::None | Self::Raw(_) => unreachable!(),
                }
            }
            Self::Semantic(spec) => spec,
            Self::Raw(_) => panic!(
                "raw render-group effects cannot be mixed with semantic surface/content/derived/composite layers"
            ),
        }
    }

    pub(crate) fn raw_mut(&mut self) -> &mut Vec<CompositeEffect> {
        match self {
            Self::None => {
                *self = Self::Raw(Vec::new());
                match self {
                    Self::Raw(effects) => effects,
                    Self::None | Self::Semantic(_) => unreachable!(),
                }
            }
            Self::Raw(effects) => effects,
            Self::Semantic(_) => panic!(
                "semantic render-group layers cannot be mixed with raw CompositeEffect lists"
            ),
        }
    }

    pub(crate) fn effects(&self) -> Vec<CompositeEffect> {
        match self {
            Self::None => Vec::new(),
            Self::Semantic(spec) => spec.effects(),
            Self::Raw(effects) => effects.clone(),
        }
    }

    pub(crate) fn has_active_effect(&self) -> bool {
        match self {
            Self::None => false,
            Self::Semantic(spec) => spec.has_active_effect(),
            Self::Raw(effects) => effects.iter().any(|effect| !effect.is_identity()),
        }
    }
}

/// Backend-independent visual meaning for a render group.
#[derive(Clone, Debug, PartialEq)]
pub struct LogicalVisualPlan {
    effects: Vec<CompositeEffect>,
    accepted_effects: Vec<CompositeEffect>,
    planning_rejections: Vec<RenderGroupPlanningRejection>,
    normalized: CompositeEffectPlan,
    dependencies: RenderGroupDependencies,
    requirements: RenderGroupRequirements,
    physical: PhysicalRenderGroupPlan,
}

impl LogicalVisualPlan {
    /// Lowers validated render-group input into a backend-independent plan.
    pub fn from_input(scale_factor: f32, boundary_opacity: f32, input: &RenderGroupInput) -> Self {
        Self::from_effects(scale_factor, boundary_opacity, input.effects())
    }

    /// Lowers raw normalized effect syntax into a backend-independent plan.
    pub fn from_effects(
        scale_factor: f32,
        boundary_opacity: f32,
        effects: Vec<CompositeEffect>,
    ) -> Self {
        let (accepted_effects, planning_rejections) =
            reject_unsupported_exact_effects(scale_factor, &effects);
        let normalized =
            CompositeEffectPlan::from_effects(scale_factor, boundary_opacity, &accepted_effects);
        let dependencies =
            RenderGroupDependencies::from_effect_plan(&accepted_effects, &normalized);
        let requirements =
            RenderGroupRequirements::from_effect_plan(scale_factor, &accepted_effects, &normalized);
        let physical = PhysicalRenderGroupPlan::from_effect_plan(&requirements, &normalized);
        Self {
            effects,
            accepted_effects,
            planning_rejections,
            normalized,
            dependencies,
            requirements,
            physical,
        }
    }

    /// Returns the raw syntax that produced this plan.
    pub fn effects(&self) -> &[CompositeEffect] {
        &self.effects
    }

    /// Returns the raw effects accepted into the current backend plan.
    pub fn accepted_effects(&self) -> &[CompositeEffect] {
        &self.accepted_effects
    }

    /// Returns typed planning rejections produced while lowering this plan.
    pub fn planning_rejections(&self) -> &[RenderGroupPlanningRejection] {
        &self.planning_rejections
    }

    /// Returns the normalized current physical slice consumed by backends.
    pub fn normalized_effects(&self) -> &CompositeEffectPlan {
        &self.normalized
    }

    /// Returns backend-independent requirements.
    pub fn requirements(&self) -> RenderGroupRequirements {
        self.requirements
    }

    /// Returns backend-independent source/backdrop/shape dependencies.
    pub fn dependencies(&self) -> RenderGroupDependencies {
        self.dependencies
    }

    /// Returns the current physical execution summary.
    pub fn physical_plan(&self) -> PhysicalRenderGroupPlan {
        self.physical
    }

    /// Returns stable support counters for this plan over an already computed group region.
    pub fn support_counters(
        &self,
        group_bounds: Bounds<ScaledPixels>,
        capture_bounds: Bounds<ScaledPixels>,
        content_mask: ContentMask<ScaledPixels>,
    ) -> RenderGroupSupportCounters {
        let has_rejections = !self.planning_rejections.is_empty();
        let has_accepted_visual_work =
            plan_has_visual_work(&self.accepted_effects, &self.normalized);
        let renders_pixels = has_accepted_visual_work
            && self.normalized.opacity() > f32::EPSILON
            && !capture_bounds.is_empty();
        let source_capture_pixels = if renders_pixels && self.requirements.requires_source_capture {
            bounds_pixel_area(capture_bounds)
        } else {
            0
        };
        let backdrop_read_pixels = if renders_pixels && self.requirements.reads_backdrop {
            bounds_pixel_area(capture_bounds.dilate(self.requirements.backdrop_outset))
        } else {
            0
        };
        let content_alpha_shadow_sample_count_estimate = if renders_pixels {
            self.physical
                .content_alpha_shadow_sample_count_estimate(capture_bounds, &self.normalized)
        } else {
            0
        };
        let surface_shadow_draw_pixels = if renders_pixels {
            self.normalized
                .surface_shadow_draw_bounds(group_bounds, content_mask)
                .into_iter()
                .map(bounds_pixel_area)
                .sum()
        } else {
            0
        };
        let (shadow_sources, shadow_modes) = if renders_pixels {
            (
                self.normalized.shadow_source_counters(),
                self.normalized.shadow_mode_counters(),
            )
        } else {
            (
                RenderGroupShadowSourceCounters::default(),
                RenderGroupShadowModeCounters::default(),
            )
        };
        let content_alpha_shadow_max_kernel_radius = if renders_pixels {
            self.normalized.content_alpha_shadow_max_kernel_radius()
        } else {
            0
        };

        RenderGroupSupportCounters {
            rendered_groups: u32::from(renders_pixels),
            elided_groups: u32::from(!renders_pixels && !has_rejections),
            rejected_groups: u32::from(has_rejections),
            source_capture_groups: u32::from(
                renders_pixels && self.physical.kind == RenderGroupPhysicalPlanKind::SourceCapture,
            ),
            direct_surface_groups: u32::from(
                renders_pixels
                    && self.physical.kind == RenderGroupPhysicalPlanKind::DirectSurfaceEffects,
            ),
            source_capture_pixels,
            backdrop_read_pixels,
            surface_shadow_draw_pixels,
            logical_passes: u32::from(self.requirements.pass_count) * u32::from(renders_pixels),
            physical_passes: u32::from(self.physical.pass_count) * u32::from(renders_pixels),
            intermediate_textures: u32::from(self.physical.intermediate_textures)
                * u32::from(renders_pixels),
            backdrop_copies: u32::from(self.physical.backdrop_copies) * u32::from(renders_pixels),
            shadow_sources,
            shadow_modes,
            content_alpha_shadow_max_kernel_radius,
            content_alpha_shadow_sample_count_estimate,
        }
    }
}

/// Backend-independent source, destination, and shape dependencies implied by a plan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderGroupDependencies {
    /// The plan samples the captured source image.
    pub source_pixels: bool,
    /// The plan derives pixels from captured source alpha.
    pub source_alpha: bool,
    /// The plan applies an authored source/content mask.
    pub source_mask: bool,
    /// The plan samples already-painted parent target pixels.
    pub backdrop_pixels: bool,
    /// The plan needs destination pixels for boundary composition.
    pub destination_pixels: bool,
    /// The plan reads the material/surface shape.
    pub material_shape: bool,
    /// The plan derives a material normal from the material/surface shape.
    pub material_normal: bool,
}

impl RenderGroupDependencies {
    fn from_effect_plan(effects: &[CompositeEffect], normalized: &CompositeEffectPlan) -> Self {
        let source_pixels = plan_has_visual_work(effects, normalized)
            && !normalized.can_render_direct_surface_effects();
        let source_alpha = !normalized.drop_shadows().is_empty();
        let source_mask =
            normalized.source_mask().is_some() || normalized.source_mask_path().is_some();
        let backdrop_pixels = normalized.has_backdrop_material();
        let destination_pixels = normalized.blend_mode() != CompositeBlendMode::Normal;
        let material_shape = (normalized.material_shape().is_some()
            && normalized.has_backdrop_material())
            || !normalized.surface_shadows().is_empty();
        let material_normal = normalized
            .backdrop_lens()
            .is_some_and(|lens| !lens.is_identity());

        Self {
            source_pixels,
            source_alpha,
            source_mask,
            backdrop_pixels,
            destination_pixels,
            material_shape,
            material_normal,
        }
    }
}

/// Stable render-group support counters derived from a logical plan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderGroupSupportCounters {
    /// Groups whose accepted plan has visible work for the supplied capture bounds.
    pub rendered_groups: u32,
    /// Groups with no accepted visible work and no planning rejection.
    pub elided_groups: u32,
    /// Groups with at least one typed planning rejection.
    pub rejected_groups: u32,
    /// Rendered groups that require source-pixel capture.
    pub source_capture_groups: u32,
    /// Rendered groups whose effects can be drawn directly from surface geometry.
    pub direct_surface_groups: u32,
    /// Device pixels in the source capture region.
    pub source_capture_pixels: u64,
    /// Device-pixel footprint that backdrop-sampling effects may read.
    pub backdrop_read_pixels: u64,
    /// Device-pixel footprint shaded by surface-geometry shadows.
    pub surface_shadow_draw_pixels: u64,
    /// Logical passes required by accepted visible work.
    pub logical_passes: u32,
    /// Physical passes in the current backend summary for accepted visible work.
    pub physical_passes: u32,
    /// Intermediate textures in the current backend summary for accepted visible work.
    pub intermediate_textures: u32,
    /// Backdrop texture copies in the current backend summary for accepted visible work.
    pub backdrop_copies: u32,
    /// Shadow source kinds present in the accepted visible work.
    pub shadow_sources: RenderGroupShadowSourceCounters,
    /// Shadow blur/quality modes present in the accepted visible work.
    pub shadow_modes: RenderGroupShadowModeCounters,
    /// Maximum Gaussian kernel radius, in source pixels, among accepted
    /// content-alpha shadows.
    pub content_alpha_shadow_max_kernel_radius: u32,
    /// Estimated source-alpha samples performed for content-alpha shadows over
    /// the supplied capture bounds. This is a model counter, not a GPU timer.
    pub content_alpha_shadow_sample_count_estimate: u64,
}

/// Additive render-group shadow-source diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderGroupShadowSourceCounters {
    /// Shadows derived from captured content/source alpha.
    pub content_alpha: u32,
    /// Shadows derived from explicit surface/geometry shape.
    pub surface_geometry: u32,
}

/// Additive render-group shadow-mode diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderGroupShadowModeCounters {
    /// Shadows using the full-resolution separable path.
    pub separable: u32,
    /// Shadows using the full 2D exact/debug path.
    pub exact: u32,
    /// Shadows requesting a lower-resolution tier.
    pub downsampled: u32,
}

/// Measured render-group resource counts a backend actually produced while
/// rendering a scene, for comparison against the planner's predicted
/// [`RenderGroupSupportCounters`].
///
/// Counts only **group** intermediates (each group's render target plus any
/// backdrop-copy target) and backdrop blits, summed over every rendered group
/// including nested ones. Root-overflow and path-rasterization intermediates are
/// excluded because the planner does not count them in its per-group totals.
///
/// The exact-match law (`measured == predicted`) holds for groups the backend and
/// planner agree are rendered; it does not for degenerate groups the backend draws
/// but the planner elides (identity opacity, opacity in `(0, EPSILON]`, or empty
/// capture bounds). See `gpui_wgpu/tests/render_group_pixels.rs`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderGroupBackendCounters {
    /// Rendered groups that captured source pixels.
    pub source_capture_groups: u32,
    /// Rendered groups that drew direct surface effects and replayed children inline.
    pub direct_surface_groups: u32,
    /// Source capture pixels represented by rendered groups.
    pub source_capture_pixels: u64,
    /// Surface-shadow pixels shaded by rendered groups.
    pub surface_shadow_draw_pixels: u64,
    /// Group intermediate textures allocated (group target + backdrop-copy target).
    pub intermediate_textures: u32,
    /// Backdrop texture copies performed.
    pub backdrop_copies: u32,
    /// Shadow source kinds the backend actually drew.
    pub shadow_sources: RenderGroupShadowSourceCounters,
    /// Shadow blur/quality modes the backend actually drew.
    pub shadow_modes: RenderGroupShadowModeCounters,
    /// Maximum Gaussian kernel radius represented by the backend's accepted
    /// content-alpha shadow work.
    pub content_alpha_shadow_max_kernel_radius: u32,
    /// Estimated source-alpha samples represented by the backend's content-alpha
    /// shadow work.
    pub content_alpha_shadow_sample_count_estimate: u64,
}

impl RenderGroupBackendCounters {
    /// Adds backend-visible support counters for one rendered group.
    pub fn add_support_counters(&mut self, support: RenderGroupSupportCounters) {
        self.source_capture_groups += support.source_capture_groups;
        self.direct_surface_groups += support.direct_surface_groups;
        self.source_capture_pixels += support.source_capture_pixels;
        self.surface_shadow_draw_pixels += support.surface_shadow_draw_pixels;
        self.shadow_sources.content_alpha += support.shadow_sources.content_alpha;
        self.shadow_sources.surface_geometry += support.shadow_sources.surface_geometry;
        self.shadow_modes.separable += support.shadow_modes.separable;
        self.shadow_modes.exact += support.shadow_modes.exact;
        self.shadow_modes.downsampled += support.shadow_modes.downsampled;
        self.content_alpha_shadow_max_kernel_radius = self
            .content_alpha_shadow_max_kernel_radius
            .max(support.content_alpha_shadow_max_kernel_radius);
        self.content_alpha_shadow_sample_count_estimate +=
            support.content_alpha_shadow_sample_count_estimate;
    }
}

/// A typed reason why an authored render-group effect could not enter the exact plan.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct RenderGroupPlanningRejection {
    pub effect: RenderGroupRejectedEffect,
    pub reason: RenderGroupPlanningRejectionReason,
    pub suggestion: &'static str,
}

/// Render-group effect provenance for typed planning rejection metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum RenderGroupRejectedEffect {
    SourceBlur,
    BackdropBlur,
    DropShadow,
    SurfaceShadow,
    ProcessedContentGlow,
}

impl RenderGroupPlanningRejection {
    /// Stable one-line author-facing description for logs and diagnostics.
    pub fn describe(&self) -> String {
        let provenance = self.effect.provenance();
        let reason = match &self.reason {
            RenderGroupPlanningRejectionReason::LimitExceeded {
                limit,
                requested,
                unit,
            } => {
                let unit = match unit {
                    RenderGroupLimitUnit::GaussianSigma => "gaussian sigma (scaled px)",
                };
                format!(
                    "requested {} exceeds the exact-effect limit {} [{unit}]",
                    requested.0, limit.0
                )
            }
            RenderGroupPlanningRejectionReason::UnsupportedStageSequence { expected } => {
                format!("unsupported stage sequence (expected {expected})")
            }
            RenderGroupPlanningRejectionReason::UnsupportedRenderGroupShadowMode { mode } => {
                format!("unsupported render-group shadow mode {}", mode.label())
            }
        };
        format!("{provenance}: {reason}; {}", self.suggestion)
    }
}

/// Warn-logs each distinct planning rejection once per process.
///
/// A planning-rejected effect does not paint at all, so silence here means an
/// authored visual (a derived shadow, a frost, a glow) vanishes with no
/// signal — the NOBS-5648 failure mode. Deduplicated by the rejection's
/// describe() line so per-frame replanning cannot spam the log.
pub(crate) fn log_planning_rejections_once(rejections: &[RenderGroupPlanningRejection]) {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    for rejection in rejections {
        let line = rejection.describe();
        let mut seen = SEEN
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if seen.insert(line.clone()) {
            log::warn!("render group dropped an authored effect: {line}");
        }
    }
}

impl RenderGroupRejectedEffect {
    /// Returns a stable author-facing provenance string for diagnostics.
    pub fn provenance(self) -> &'static str {
        match self {
            Self::SourceBlur => "content.blur",
            Self::BackdropBlur => "surface.frost",
            Self::DropShadow => "derived.content_alpha.shadow",
            Self::SurfaceShadow => "derived.surface_shape.shadow",
            Self::ProcessedContentGlow => "derived.processed_content.glow",
        }
    }
}

/// Machine-readable planning rejection details.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum RenderGroupPlanningRejectionReason {
    LimitExceeded {
        limit: ScaledPixels,
        requested: ScaledPixels,
        unit: RenderGroupLimitUnit,
    },
    UnsupportedStageSequence {
        expected: &'static str,
    },
    UnsupportedRenderGroupShadowMode {
        mode: RenderGroupShadowMode,
    },
}

/// Unit for a render-group planning limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum RenderGroupLimitUnit {
    GaussianSigma,
}

/// Render-group capability a tool can ask the GPUI planner to classify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum RenderGroupCapabilityProbe {
    SourceColor,
    ExactSourceBlur,
    DirectionalBlur,
    SourceDistortion,
    SourceMask,
    PathSourceMask,
    TextCaptureFidelity,
    BackdropMaterial,
    BackdropLens,
    MaterialResource,
    MaterialLighting,
    AdaptiveBackdropPolicy,
    ContentAlphaShadow,
    ProcessedContentGlow,
    DerivedStroke,
    DerivedReflection,
    InnerEffect,
    CompositeOpacity,
    CompositeBlend,
    DestinationMask,
    TransformedGroup,
    GeometryWarp,
    RoundedGroupShape,
    SuperellipseGroupShape,
    SpatialSampling,
    TemporalMaterial,
    TemporalInteraction,
    TemporalTransition,
    NamedMaterialPolicy,
    ResourceBackedEffect,
    MultiInputEffect,
    DebugVisualizer,
    BackendDiagnostics,
}

impl RenderGroupCapabilityProbe {
    /// Returns the current planner-owned support report for this capability.
    pub fn report(self) -> RenderGroupCapabilityReport {
        use RenderGroupCapabilityRejectionReason as Reason;

        match self {
            Self::SourceColor
            | Self::SourceMask
            | Self::PathSourceMask
            | Self::BackdropMaterial
            | Self::BackdropLens
            | Self::MaterialLighting
            | Self::ContentAlphaShadow
            | Self::CompositeOpacity
            | Self::CompositeBlend
            | Self::RoundedGroupShape
            | Self::DirectionalBlur
            | Self::SuperellipseGroupShape => RenderGroupCapabilityReport::rendered(
                self,
                "current normalized render-group plan has a backend path",
            ),
            Self::ExactSourceBlur => RenderGroupCapabilityReport::partial(
                self,
                Reason::ExactKernelLimit,
                "exact Gaussian blur is accepted only within the current kernel limit",
                "use an explicitly approximate tier or lower the exact blur radius",
            ),
            Self::ProcessedContentGlow => RenderGroupCapabilityReport::rendered(
                self,
                "processed-content glow has distinct source-pixel provenance and a backend path",
            ),
            Self::DebugVisualizer => RenderGroupCapabilityReport::inspector_only(
                self,
                Reason::InspectorOnly,
                "debug visualization is tooling metadata, not a group pixel effect",
                "add a rendered overlay only with explicit debug-layer semantics",
            ),
            Self::BackendDiagnostics => RenderGroupCapabilityReport::inspector_only(
                self,
                Reason::BackendDiagnosticsMissing,
                "intermediate-texture and backdrop-copy counts are measured by the wgpu and metal backends and validated against the planner; copied pixels, texture-pool behavior, shader variants, and timing are not wired",
                "connect the remaining backend diagnostic counters (copied pixels, texture-pool behavior, shader variants, timing)",
            ),
            Self::SourceDistortion => RenderGroupCapabilityReport::unsupported(
                self,
                Reason::LogicalNodeMissing,
                "no source-distortion logical node exists",
                "add source-distortion nodes with coordinate-space and resampling laws",
            ),
            Self::TextCaptureFidelity => RenderGroupCapabilityReport::unsupported(
                self,
                Reason::FixtureMissing,
                "active capture text behavior is not fixture-backed across backends",
                "add text/cached-descendant fixtures before claiming support",
            ),
            Self::MaterialResource | Self::ResourceBackedEffect => {
                RenderGroupCapabilityReport::unsupported(
                    self,
                    Reason::ResourceBindingMissing,
                    "no effect resource identity, sampler policy, or cache key is in the plan",
                    "add resource-backed logical nodes and backend bindings",
                )
            }
            Self::AdaptiveBackdropPolicy | Self::NamedMaterialPolicy => {
                RenderGroupCapabilityReport::unsupported(
                    self,
                    Reason::PolicyMissing,
                    "policy-selected materials are not named planner inputs",
                    "add named accessibility/theme material policies instead of silent fallback",
                )
            }
            Self::DerivedStroke | Self::InnerEffect => RenderGroupCapabilityReport::unsupported(
                self,
                Reason::ProvenancePathMissing,
                "no distinct derived-layer provenance path exists for this effect",
                "add a provenance-specific derived node before rendering pixels",
            ),
            Self::DerivedReflection => RenderGroupCapabilityReport::unsupported(
                self,
                Reason::MultiInputPassMissing,
                "derived reflection needs generated pixels and bounds beyond the current normalized slots",
                "add an explicit derived reflection node with source/destination provenance",
            ),
            Self::DestinationMask => RenderGroupCapabilityReport::unsupported(
                self,
                Reason::DestinationReadMissing,
                "destination masking is not represented in the current physical plan",
                "add destination-read capability rows and backend rejection metadata",
            ),
            Self::TransformedGroup => RenderGroupCapabilityReport::unsupported(
                self,
                Reason::CoordinatePlanMissing,
                "transformed group capture/effect/hit bounds are not a planner node",
                "add transform-space requirements and backend fixtures",
            ),
            Self::GeometryWarp | Self::SpatialSampling | Self::MultiInputEffect => {
                RenderGroupCapabilityReport::unsupported(
                    self,
                    Reason::PassGraphMissing,
                    "the effect cannot be represented honestly by normalized singleton slots",
                    "introduce an explicit pass graph when a concrete public effect requires it",
                )
            }
            Self::TemporalMaterial | Self::TemporalInteraction | Self::TemporalTransition => {
                RenderGroupCapabilityReport::unsupported(
                    self,
                    Reason::TemporalDependencyMissing,
                    "time/input/lifecycle invalidation keys are not planner dependencies",
                    "add deterministic dependency keys and invalidation policy",
                )
            }
        }
    }
}

/// Current support tier for a render-group capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum RenderGroupCapabilityStatus {
    Rendered,
    Partial,
    Approximation,
    InspectorOnly,
    Unsupported,
}

impl RenderGroupCapabilityStatus {
    /// Short label for inspector display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Rendered => "rendered",
            Self::Partial => "partial",
            Self::Approximation => "approximation",
            Self::InspectorOnly => "inspector only",
            Self::Unsupported => "unsupported",
        }
    }
}

/// Typed reason a capability cannot currently claim rendered support.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
#[non_exhaustive]
pub enum RenderGroupCapabilityRejectionReason {
    ExactKernelLimit,
    ApproximationOnly,
    InspectorOnly,
    LogicalNodeMissing,
    PassGraphMissing,
    ResourceBindingMissing,
    TemporalDependencyMissing,
    PolicyMissing,
    BackendDiagnosticsMissing,
    FixtureMissing,
    ProvenancePathMissing,
    MultiInputPassMissing,
    DestinationReadMissing,
    CoordinatePlanMissing,
}

impl RenderGroupCapabilityRejectionReason {
    /// Short label for inspector display.
    pub fn label(self) -> &'static str {
        match self {
            Self::ExactKernelLimit => "exact kernel limit",
            Self::ApproximationOnly => "approximation only",
            Self::InspectorOnly => "inspector only",
            Self::LogicalNodeMissing => "missing logical node",
            Self::PassGraphMissing => "needs pass graph",
            Self::ResourceBindingMissing => "missing resource binding",
            Self::TemporalDependencyMissing => "missing temporal dependency",
            Self::PolicyMissing => "missing material policy",
            Self::BackendDiagnosticsMissing => "missing backend diagnostics",
            Self::FixtureMissing => "missing fixture",
            Self::ProvenancePathMissing => "missing provenance path",
            Self::MultiInputPassMissing => "needs multi-input pass",
            Self::DestinationReadMissing => "needs destination read",
            Self::CoordinatePlanMissing => "missing coordinate plan",
        }
    }
}

/// Planner-owned capability report for inspector and tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct RenderGroupCapabilityReport {
    pub probe: RenderGroupCapabilityProbe,
    pub status: RenderGroupCapabilityStatus,
    pub rejection: Option<RenderGroupCapabilityRejectionReason>,
    pub evidence: &'static str,
    pub suggestion: &'static str,
}

impl RenderGroupCapabilityReport {
    fn rendered(probe: RenderGroupCapabilityProbe, evidence: &'static str) -> Self {
        Self {
            probe,
            status: RenderGroupCapabilityStatus::Rendered,
            rejection: None,
            evidence,
            suggestion: "assert backend fixtures and limits for public support claims",
        }
    }

    fn partial(
        probe: RenderGroupCapabilityProbe,
        rejection: RenderGroupCapabilityRejectionReason,
        evidence: &'static str,
        suggestion: &'static str,
    ) -> Self {
        Self {
            probe,
            status: RenderGroupCapabilityStatus::Partial,
            rejection: Some(rejection),
            evidence,
            suggestion,
        }
    }

    #[allow(dead_code)]
    fn approximation(
        probe: RenderGroupCapabilityProbe,
        rejection: RenderGroupCapabilityRejectionReason,
        evidence: &'static str,
        suggestion: &'static str,
    ) -> Self {
        Self {
            probe,
            status: RenderGroupCapabilityStatus::Approximation,
            rejection: Some(rejection),
            evidence,
            suggestion,
        }
    }

    fn inspector_only(
        probe: RenderGroupCapabilityProbe,
        rejection: RenderGroupCapabilityRejectionReason,
        evidence: &'static str,
        suggestion: &'static str,
    ) -> Self {
        Self {
            probe,
            status: RenderGroupCapabilityStatus::InspectorOnly,
            rejection: Some(rejection),
            evidence,
            suggestion,
        }
    }

    fn unsupported(
        probe: RenderGroupCapabilityProbe,
        rejection: RenderGroupCapabilityRejectionReason,
        evidence: &'static str,
        suggestion: &'static str,
    ) -> Self {
        Self {
            probe,
            status: RenderGroupCapabilityStatus::Unsupported,
            rejection: Some(rejection),
            evidence,
            suggestion,
        }
    }
}

/// Backend-independent requirements implied by a logical visual plan.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[allow(missing_docs)]
pub struct RenderGroupRequirements {
    pub reads_backdrop: bool,
    pub source_outset: ScaledPixels,
    pub backdrop_outset: ScaledPixels,
    pub output_outset: ScaledPixels,
    pub requires_source_capture: bool,
    pub pass_count: u8,
    pub intermediate_textures: u8,
    pub backdrop_copies: u8,
}

impl RenderGroupRequirements {
    fn from_effect_plan(
        scale_factor: f32,
        effects: &[CompositeEffect],
        normalized: &CompositeEffectPlan,
    ) -> Self {
        let output_outset = CompositeEffectPlan::visual_outset(scale_factor, effects);
        let source_outset = effects
            .iter()
            .fold(ScaledPixels(0.), |outset, effect| match effect {
                CompositeEffect::SourceBlur(radius) => ScaledPixels(
                    outset
                        .0
                        .max(gaussian_kernel_outset(radius.scale(scale_factor)).0),
                ),
                CompositeEffect::SourceDirectionalBlur(v) => ScaledPixels(
                    outset
                        .0
                        .max(directional_kernel_outset(v.scale(scale_factor)).0),
                ),
                CompositeEffect::ProcessedContentGlow(glow) => {
                    if let Some(glow) = lower_processed_content_glow(scale_factor, glow) {
                        ScaledPixels(outset.0.max(gaussian_kernel_outset(glow.blur_radius).0))
                    } else {
                        outset
                    }
                }
                _ => outset,
            });
        let backdrop_outset =
            effects
                .iter()
                .fold(ScaledPixels(0.), |outset, effect| match effect {
                    CompositeEffect::BackdropBlur(radius) => ScaledPixels(
                        outset
                            .0
                            .max(gaussian_kernel_outset(radius.scale(scale_factor)).0),
                    ),
                    CompositeEffect::BackdropLens(lens) => ScaledPixels(
                        outset
                            .0
                            .max(lens.refraction_radius().scale(scale_factor).0)
                            .max(lens.chromatic_aberration().scale(scale_factor).0),
                    ),
                    _ => outset,
                });
        let reads_backdrop = normalized.reads_backdrop();
        let direct_surface_effects = normalized.can_render_direct_surface_effects();
        let has_derived = !normalized.drop_shadows().is_empty()
            || !normalized.surface_shadows().is_empty()
            || !normalized.processed_content_glows().is_empty();
        let pass_count = if direct_surface_effects {
            1
        } else {
            1 + u8::from(reads_backdrop)
                + u8::from(has_derived)
                + u8::from(normalized.opacity() > 0.)
        };
        let intermediate_textures = if direct_surface_effects {
            0
        } else {
            1 + u8::from(reads_backdrop)
        };

        Self {
            reads_backdrop,
            source_outset,
            backdrop_outset,
            output_outset,
            requires_source_capture: !direct_surface_effects,
            pass_count,
            intermediate_textures,
            backdrop_copies: u8::from(reads_backdrop && !direct_surface_effects),
        }
    }
}

/// Backend execution family selected for a render group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum RenderGroupPhysicalPlanKind {
    #[default]
    SourceCapture,
    DirectSurfaceEffects,
}

/// Backend-specific execution summary for the current render-group renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct PhysicalRenderGroupPlan {
    pub kind: RenderGroupPhysicalPlanKind,
    pub pass_count: u8,
    pub intermediate_textures: u8,
    pub backdrop_copies: u8,
    pub content_alpha_shadow_blur_passes: u8,
}

impl PhysicalRenderGroupPlan {
    fn from_effect_plan(
        requirements: &RenderGroupRequirements,
        normalized: &CompositeEffectPlan,
    ) -> Self {
        let content_alpha_shadow_blur_passes =
            normalized.content_alpha_separable_blur_profile_count();
        let kind = if requirements.requires_source_capture {
            RenderGroupPhysicalPlanKind::SourceCapture
        } else {
            RenderGroupPhysicalPlanKind::DirectSurfaceEffects
        };
        Self {
            kind,
            pass_count: requirements
                .pass_count
                .saturating_add(content_alpha_shadow_blur_passes),
            intermediate_textures: requirements
                .intermediate_textures
                .saturating_add(content_alpha_shadow_blur_passes),
            backdrop_copies: requirements.backdrop_copies,
            content_alpha_shadow_blur_passes,
        }
    }

    fn content_alpha_shadow_sample_count_estimate(
        &self,
        capture_bounds: Bounds<ScaledPixels>,
        normalized: &CompositeEffectPlan,
    ) -> u64 {
        let area = bounds_pixel_area(capture_bounds);
        if area == 0 {
            return 0;
        }

        let mut total_samples_per_pixel = 0_u64;
        for shadow in normalized.drop_shadows() {
            let kernel_width = gaussian_kernel_width(shadow.shadow.blur_radius);
            total_samples_per_pixel =
                total_samples_per_pixel.saturating_add(match shadow.shadow.mode {
                    RenderGroupShadowMode::Separable => kernel_width,
                    RenderGroupShadowMode::Exact => kernel_width.saturating_mul(kernel_width),
                    RenderGroupShadowMode::Downsampled => 0,
                });
        }
        let horizontal_samples_per_pixel: u64 = normalized
            .content_alpha_separable_blur_profiles()
            .map(gaussian_kernel_width)
            .sum();
        area.saturating_mul(total_samples_per_pixel.saturating_add(horizontal_samples_per_pixel))
    }
}

fn bounds_pixel_area(bounds: Bounds<ScaledPixels>) -> u64 {
    if bounds.is_empty() {
        return 0;
    }

    let width = bounds.size.width.0.max(0.).ceil() as u64;
    let height = bounds.size.height.0.max(0.).ceil() as u64;
    width.saturating_mul(height)
}

fn gaussian_kernel_radius(sigma: ScaledPixels) -> u64 {
    if sigma.0 <= f32::EPSILON {
        return 0;
    }
    ((sigma.0 * 3.).ceil().max(0.) as u64).min(24)
}

fn gaussian_kernel_width(sigma: ScaledPixels) -> u64 {
    gaussian_kernel_radius(sigma)
        .saturating_mul(2)
        .saturating_add(1)
}

fn plan_has_visual_work(effects: &[CompositeEffect], normalized: &CompositeEffectPlan) -> bool {
    effects.iter().any(|effect| !effect.is_identity())
        || (normalized.opacity() - 1.).abs() > f32::EPSILON
}

const MAX_EXACT_GAUSSIAN_SIGMA: f32 = 8.;

fn reject_unsupported_exact_effects(
    scale_factor: f32,
    effects: &[CompositeEffect],
) -> (Vec<CompositeEffect>, Vec<RenderGroupPlanningRejection>) {
    let mut accepted_effects = Vec::with_capacity(effects.len());
    let mut planning_rejections = Vec::new();
    let mut accepted_source_sigma = ScaledPixels(0.);
    let mut accepted_backdrop_sigma = ScaledPixels(0.);

    for effect in effects {
        match effect {
            CompositeEffect::SourceBlur(radius) => {
                let requested =
                    combined_gaussian_sigma(accepted_source_sigma, radius.scale(scale_factor));
                if requested.0 > MAX_EXACT_GAUSSIAN_SIGMA {
                    planning_rejections.push(exact_blur_limit_rejection(
                        RenderGroupRejectedEffect::SourceBlur,
                        requested,
                    ));
                    continue;
                }
                accepted_source_sigma = requested;
            }
            CompositeEffect::BackdropBlur(radius) => {
                let requested =
                    combined_gaussian_sigma(accepted_backdrop_sigma, radius.scale(scale_factor));
                if requested.0 > MAX_EXACT_GAUSSIAN_SIGMA {
                    planning_rejections.push(exact_blur_limit_rejection(
                        RenderGroupRejectedEffect::BackdropBlur,
                        requested,
                    ));
                    continue;
                }
                accepted_backdrop_sigma = requested;
            }
            CompositeEffect::DropShadow(shadow) => {
                if shadow.shadow.mode == RenderGroupShadowMode::Downsampled {
                    planning_rejections.push(RenderGroupPlanningRejection {
                        effect: RenderGroupRejectedEffect::DropShadow,
                        reason: RenderGroupPlanningRejectionReason::UnsupportedRenderGroupShadowMode {
                            mode: shadow.shadow.mode,
                        },
                        suggestion: "use RenderGroupShadowMode::Separable for content-alpha shadows until the downsampled backend lands",
                    });
                    continue;
                }
                let requested = shadow.shadow.blur_radius.scale(scale_factor);
                if requested.0 > MAX_EXACT_GAUSSIAN_SIGMA {
                    planning_rejections.push(exact_blur_limit_rejection(
                        RenderGroupRejectedEffect::DropShadow,
                        requested,
                    ));
                    continue;
                }
            }
            CompositeEffect::SurfaceShadow(shadow) => {
                if shadow.shadow.mode != RenderGroupShadowMode::Exact {
                    planning_rejections.push(RenderGroupPlanningRejection {
                        effect: RenderGroupRejectedEffect::SurfaceShadow,
                        reason: RenderGroupPlanningRejectionReason::UnsupportedRenderGroupShadowMode {
                            mode: shadow.shadow.mode,
                        },
                        suggestion: "use RenderGroupShadowMode::Exact for surface shadows until a surface-specific optimized mode lands",
                    });
                    continue;
                }
                let requested = shadow.shadow.blur_radius.scale(scale_factor);
                if requested.0 > MAX_EXACT_GAUSSIAN_SIGMA {
                    planning_rejections.push(exact_blur_limit_rejection(
                        RenderGroupRejectedEffect::SurfaceShadow,
                        requested,
                    ));
                    continue;
                }
            }
            CompositeEffect::ProcessedContentGlow(glow) => {
                let Some(glow) = lower_processed_content_glow(scale_factor, glow) else {
                    planning_rejections.push(RenderGroupPlanningRejection {
                        effect: RenderGroupRejectedEffect::ProcessedContentGlow,
                        reason: RenderGroupPlanningRejectionReason::UnsupportedStageSequence {
                            expected: "threshold_luma(...) followed by exact_blur(...)",
                        },
                        suggestion: "use DerivedStage::threshold_luma(...) followed by DerivedStage::exact_blur(...)",
                    });
                    continue;
                };
                if glow.blur_radius.0 > MAX_EXACT_GAUSSIAN_SIGMA {
                    planning_rejections.push(exact_blur_limit_rejection(
                        RenderGroupRejectedEffect::ProcessedContentGlow,
                        glow.blur_radius,
                    ));
                    continue;
                }
            }
            _ => {}
        }
        accepted_effects.push(effect.clone());
    }

    (accepted_effects, planning_rejections)
}

fn combined_gaussian_sigma(current: ScaledPixels, next: ScaledPixels) -> ScaledPixels {
    ScaledPixels((current.0.powi(2) + next.0.powi(2)).sqrt())
}

fn exact_blur_limit_rejection(
    effect: RenderGroupRejectedEffect,
    requested: ScaledPixels,
) -> RenderGroupPlanningRejection {
    RenderGroupPlanningRejection {
        effect,
        reason: RenderGroupPlanningRejectionReason::LimitExceeded {
            limit: ScaledPixels(MAX_EXACT_GAUSSIAN_SIGMA),
            requested,
            unit: RenderGroupLimitUnit::GaussianSigma,
        },
        suggestion: "use an explicitly approximate blur tier or reduce the exact blur radius",
    }
}

fn lower_processed_content_glow(
    scale_factor: f32,
    glow: &CompositeProcessedContentGlow,
) -> Option<CompositeProcessedContentGlowPlan> {
    let [
        DerivedStage::ThresholdLuma(threshold),
        DerivedStage::ExactBlur(radius),
    ] = glow.stages.as_slice()
    else {
        return None;
    };

    Some(CompositeProcessedContentGlowPlan {
        luma_threshold: threshold.value(),
        blur_radius: radius.scale(scale_factor),
        color: glow.color,
    })
}

/// An affine source-color transform applied to an already-composited render
/// group image.
///
/// The transform operates on unpremultiplied linear RGB and preserves alpha.
/// This keeps coverage separate from source color effects and lets opacity
/// remain a distinct group effect.
/// A non-linear tone operation applied to source color after the affine matrix.
///
/// Tone ops cannot be folded into the color matrix, so they ride alongside it on
/// [`SourceColorFilter`] and are applied in the shader after the matrix, on
/// straight (unpremultiplied) RGB, before the final clamp.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub enum SourceToneOp {
    /// No tone operation.
    #[default]
    None,
    /// Quantize each channel to `levels` (clamped to >= 2) bands.
    Posterize(f32),
    /// Map to black/white by whether luma reaches the threshold (0..1).
    Threshold(f32),
    /// Invert channels at or above the threshold (0..1).
    Solarize(f32),
}

impl SourceToneOp {
    /// Returns the stable shader discriminant and parameter for this tone op.
    pub fn shader_code_and_param(self) -> (u32, f32) {
        match self {
            Self::None => (0, 0.),
            Self::Posterize(levels) => (1, levels.max(2.)),
            Self::Threshold(threshold) => (2, threshold),
            Self::Solarize(threshold) => (3, threshold),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct SourceColorFilter {
    matrix: [[f32; 3]; 3],
    offset: [f32; 3],
    tone: SourceToneOp,
}

impl SourceColorFilter {
    /// Returns the identity source color filter.
    pub fn identity() -> Self {
        Self {
            matrix: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            offset: [0., 0., 0.],
            tone: SourceToneOp::None,
        }
    }

    /// Returns a filter that multiplies source color channels by `factor`.
    pub fn brightness(factor: f32) -> Self {
        let factor = factor.max(0.);
        Self {
            matrix: [[factor, 0., 0.], [0., factor, 0.], [0., 0., factor]],
            offset: [0., 0., 0.],
            tone: SourceToneOp::None,
        }
    }

    /// Returns a filter that scales source color distance from mid-gray by `factor`.
    pub fn contrast(factor: f32) -> Self {
        let factor = factor.max(0.);
        let offset = 0.5 * (1. - factor);
        Self {
            matrix: [[factor, 0., 0.], [0., factor, 0.], [0., 0., factor]],
            offset: [offset, offset, offset],
            tone: SourceToneOp::None,
        }
    }

    /// Returns a filter that adjusts source color saturation by `factor`.
    pub fn saturate(factor: f32) -> Self {
        let factor = factor.max(0.);
        let luma = [0.2126, 0.7152, 0.0722];
        let mut matrix = [[0.; 3]; 3];
        for row in 0..3 {
            for column in 0..3 {
                matrix[row][column] = luma[column] * (1. - factor);
            }
            matrix[row][row] += factor;
        }
        Self {
            matrix,
            offset: [0., 0., 0.],
            tone: SourceToneOp::None,
        }
    }

    /// Returns a filter that mixes source color toward grayscale by `amount`.
    pub fn grayscale(amount: f32) -> Self {
        Self::saturate(1. - amount.clamp(0., 1.))
    }

    /// Returns a filter that mixes source color toward its inverse by `amount`.
    pub fn invert(amount: f32) -> Self {
        let amount = amount.clamp(0., 1.);
        let scale = 1. - 2. * amount;
        Self {
            matrix: [[scale, 0., 0.], [0., scale, 0.], [0., 0., scale]],
            offset: [amount, amount, amount],
            tone: SourceToneOp::None,
        }
    }

    /// Returns an affine source-color matrix over unpremultiplied RGB.
    pub fn color_matrix(matrix: [[f32; 3]; 3], offset: [f32; 3]) -> Self {
        Self {
            matrix,
            offset,
            tone: SourceToneOp::None,
        }
    }

    /// Returns a tone-only filter that posterizes source color to `levels` bands.
    pub fn posterize(levels: f32) -> Self {
        Self::identity().with_tone(SourceToneOp::Posterize(levels))
    }

    /// Returns a tone-only filter that thresholds source luma at `threshold` (0..1).
    pub fn threshold(threshold: f32) -> Self {
        Self::identity().with_tone(SourceToneOp::Threshold(threshold))
    }

    /// Returns a tone-only filter that solarizes source color at `threshold` (0..1).
    pub fn solarize(threshold: f32) -> Self {
        Self::identity().with_tone(SourceToneOp::Solarize(threshold))
    }

    fn with_tone(mut self, tone: SourceToneOp) -> Self {
        self.tone = tone;
        self
    }

    /// Returns a filter that rotates source hue by `degrees` around the gray
    /// axis. Uses the same luma basis as `saturate`/`grayscale`, so neutral gray
    /// is a fixed point and 0 / 360 degrees are the identity.
    pub fn hue_rotate(degrees: f32) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let luma = [0.2126, 0.7152, 0.0722];
        // SVG `feColorMatrix type="hueRotate"` sin-generator keyed to `luma`;
        // every row sums to 0, which keeps neutral gray unchanged.
        let sin_gen = [
            [-luma[0], -luma[1], 1. - luma[2]],
            [0.143, 0.140, -0.283],
            [-(1. - luma[0]), luma[1], luma[2]],
        ];
        let mut matrix = [[0.; 3]; 3];
        for row in 0..3 {
            for column in 0..3 {
                let base = luma[column];
                let identity = if row == column { 1. } else { 0. };
                matrix[row][column] = base + cos * (identity - base) + sin * sin_gen[row][column];
            }
        }
        Self {
            matrix,
            offset: [0., 0., 0.],
            tone: SourceToneOp::None,
        }
    }

    /// Returns a filter that applies a sepia tone to source color.
    pub fn sepia() -> Self {
        Self {
            matrix: [
                [0.393, 0.769, 0.189],
                [0.349, 0.686, 0.168],
                [0.272, 0.534, 0.131],
            ],
            offset: [0., 0., 0.],
            tone: SourceToneOp::None,
        }
    }

    /// Returns a duotone filter that maps source luma to a gradient between two
    /// colors: black-luma → `dark`, white-luma → `light`. Because the mapping is
    /// affine in luma (`out = dark + luma * (light - dark)` with the same luma
    /// basis as `saturate`/`hue_rotate`), it folds exactly into the source-color
    /// matrix + offset — no resource texture or shader change required.
    pub fn duotone(dark: Rgba, light: Rgba) -> Self {
        let luma = [0.2126, 0.7152, 0.0722];
        let dark = [dark.r, dark.g, dark.b];
        let light = [light.r, light.g, light.b];
        let mut matrix = [[0.; 3]; 3];
        for row in 0..3 {
            let span = light[row] - dark[row];
            for column in 0..3 {
                matrix[row][column] = span * luma[column];
            }
        }
        Self {
            matrix,
            offset: dark,
            tone: SourceToneOp::None,
        }
    }

    /// Returns the filter produced by applying `self` and then `next`.
    pub fn then(self, next: Self) -> Self {
        let mut matrix = [[0.; 3]; 3];
        for row in 0..3 {
            for column in 0..3 {
                matrix[row][column] = (0..3)
                    .map(|index| next.matrix[row][index] * self.matrix[index][column])
                    .sum();
            }
        }

        let mut offset = [0.; 3];
        for row in 0..3 {
            offset[row] = next.offset[row]
                + (0..3)
                    .map(|index| next.matrix[row][index] * self.offset[index])
                    .sum::<f32>();
        }

        // Tone ops are non-linear and don't fold into the matrix; the most
        // recently set one wins.
        let tone = if next.tone != SourceToneOp::None {
            next.tone
        } else {
            self.tone
        };

        Self {
            matrix,
            offset,
            tone,
        }
    }

    /// Returns whether this filter leaves source colors unchanged.
    pub fn is_identity(&self) -> bool {
        *self == Self::identity()
    }

    /// Returns the affine matrix and offset for renderer consumption.
    pub fn components(&self) -> ([[f32; 3]; 3], [f32; 3]) {
        (self.matrix, self.offset)
    }

    /// Returns the non-linear tone operation applied after the matrix.
    pub fn tone(&self) -> SourceToneOp {
        self.tone
    }
}

/// Renderer-facing normalization of a render group's ordered effect list.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct CompositeEffectPlan {
    opacity: f32,
    source_color_filter: SourceColorFilter,
    source_blur_radius: ScaledPixels,
    source_directional_blur: Point<ScaledPixels>,
    source_mask: Option<Corners<ScaledPixels>>,
    source_mask_shape: GroupShapeKind,
    source_mask_blur_order: SourceMaskBlurOrder,
    source_mask_path: Option<Arc<Path<ScaledPixels>>>,
    backdrop_color_filter: SourceColorFilter,
    backdrop_blur_radius: ScaledPixels,
    backdrop_lens: Option<CompositeBackdropLens<ScaledPixels>>,
    backdrop_tint: Hsla,
    material_shape: Option<SurfaceSilhouette<ScaledPixels>>,
    drop_shadows: Vec<CompositeDropShadow<ScaledPixels>>,
    surface_shadows: Vec<CompositeSurfaceShadow<ScaledPixels>>,
    processed_content_glows: Vec<CompositeProcessedContentGlowPlan>,
    rounded_mask: Option<Corners<ScaledPixels>>,
    blend_mode: CompositeBlendMode,
}

/// How source/content masks compose with source blur.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum SourceMaskBlurOrder {
    #[default]
    AfterBlur,
    BeforeBlur,
}

impl SourceMaskBlurOrder {
    /// Returns the stable shader discriminant.
    pub fn shader_code(self) -> u32 {
        match self {
            Self::AfterBlur => 0,
            Self::BeforeBlur => 1,
        }
    }
}

fn composite_tint(below: Hsla, above: Hsla) -> Hsla {
    if above.a <= f32::EPSILON {
        return below;
    }
    if above.a >= 1. {
        return above;
    }

    let below = below.to_rgb();
    let above = above.to_rgb();
    let alpha = above.a + below.a * (1. - above.a);
    if alpha <= f32::EPSILON {
        return transparent_black();
    }

    Hsla::from(Rgba {
        r: (above.r * above.a + below.r * below.a * (1. - above.a)) / alpha,
        g: (above.g * above.a + below.g * below.a * (1. - above.a)) / alpha,
        b: (above.b * above.a + below.b * below.a * (1. - above.a)) / alpha,
        a: alpha,
    })
}

fn gaussian_kernel_outset(sigma: ScaledPixels) -> ScaledPixels {
    ScaledPixels((sigma.0 * 3.).min(24.))
}

/// Symmetric capture outset for a directional (motion) blur, in device pixels.
///
/// The directional shader caps its streak at `min(ceil(length), 24)` taps, so
/// the capture region must dilate by at most 24 device pixels — matching
/// [`gaussian_kernel_outset`]'s cap so the dilated intermediate is never larger
/// than the kernel can reach.
fn directional_kernel_outset(scaled: Point<ScaledPixels>) -> ScaledPixels {
    ScaledPixels(scaled.x.0.abs().max(scaled.y.0.abs()).min(24.))
}

impl CompositeEffectPlan {
    /// Normalizes an ordered render-group effect list for renderer consumption.
    pub fn from_effects(
        scale_factor: f32,
        boundary_opacity: f32,
        effects: &[CompositeEffect],
    ) -> Self {
        let mut plan = Self {
            opacity: boundary_opacity,
            source_color_filter: SourceColorFilter::identity(),
            source_blur_radius: ScaledPixels(0.),
            source_directional_blur: point(ScaledPixels(0.), ScaledPixels(0.)),
            source_mask: None,
            source_mask_shape: GroupShapeKind::RoundedRect,
            source_mask_blur_order: SourceMaskBlurOrder::AfterBlur,
            source_mask_path: None,
            backdrop_color_filter: SourceColorFilter::identity(),
            backdrop_blur_radius: ScaledPixels(0.),
            backdrop_lens: None,
            backdrop_tint: transparent_black(),
            material_shape: None,
            drop_shadows: Vec::new(),
            surface_shadows: Vec::new(),
            processed_content_glows: Vec::new(),
            rounded_mask: None,
            blend_mode: CompositeBlendMode::Normal,
        };

        for effect in effects {
            match effect {
                CompositeEffect::Opacity(alpha) => {
                    plan.opacity *= *alpha;
                }
                CompositeEffect::SourceColorFilter(filter) => {
                    plan.source_color_filter = plan.source_color_filter.then(*filter);
                }
                CompositeEffect::SourceBlur(radius) => {
                    let radius = radius.scale(scale_factor);
                    plan.source_blur_radius =
                        ScaledPixels((plan.source_blur_radius.0.powi(2) + radius.0.powi(2)).sqrt());
                }
                CompositeEffect::SourceDirectionalBlur(v) => {
                    // Directional blur does not compose with the isotropic source
                    // blur; the last-declared directional vector wins.
                    plan.source_directional_blur = v.scale(scale_factor);
                }
                CompositeEffect::SourceMask(shape) => {
                    plan.source_mask = Some(shape.corner_radii().scale(scale_factor));
                    plan.source_mask_shape = shape.shape_kind();
                    plan.source_mask_blur_order = SourceMaskBlurOrder::AfterBlur;
                }
                CompositeEffect::SourceMaskBeforeBlur(shape) => {
                    plan.source_mask = Some(shape.corner_radii().scale(scale_factor));
                    plan.source_mask_shape = shape.shape_kind();
                    plan.source_mask_blur_order = SourceMaskBlurOrder::BeforeBlur;
                }
                CompositeEffect::SourceMaskPath(path) => {
                    // Scale the mesh into device space (the same space all group
                    // geometry lives in) and force the fill to opaque white so the
                    // rasterized coverage lands cleanly in the mask's alpha
                    // channel (`mask.a == coverage`). The path mask wins over any
                    // analytic source mask: the renderer selects sampled mode when
                    // `source_mask_path` is set.
                    let mut scaled = path.scale(scale_factor);
                    scaled.color = solid_background(Hsla::white());
                    plan.source_mask_path = Some(Arc::new(scaled));
                    plan.source_mask_blur_order = SourceMaskBlurOrder::AfterBlur;
                }
                CompositeEffect::BackdropColorFilter(filter) => {
                    plan.backdrop_color_filter = plan.backdrop_color_filter.then(*filter);
                }
                CompositeEffect::BackdropBlur(radius) => {
                    let radius = radius.scale(scale_factor);
                    plan.backdrop_blur_radius = ScaledPixels(
                        (plan.backdrop_blur_radius.0.powi(2) + radius.0.powi(2)).sqrt(),
                    );
                }
                CompositeEffect::BackdropLens(lens) => {
                    plan.backdrop_lens = Some(lens.scale(scale_factor));
                }
                CompositeEffect::BackdropTint(color) => {
                    plan.backdrop_tint = composite_tint(plan.backdrop_tint, *color);
                }
                CompositeEffect::MaterialShape(shape) => {
                    plan.material_shape = Some(shape.scale(scale_factor));
                }
                CompositeEffect::DropShadow(shadow) => {
                    plan.drop_shadows.push(CompositeDropShadow {
                        shadow: CompositeShadow {
                            offset: shadow.shadow.offset.scale(scale_factor),
                            blur_radius: shadow.shadow.blur_radius.scale(scale_factor),
                            color: shadow.shadow.color,
                            mode: shadow.shadow.mode,
                        },
                    });
                }
                CompositeEffect::SurfaceShadow(shadow) => {
                    plan.surface_shadows.push(CompositeSurfaceShadow {
                        silhouette: shadow.silhouette.scale(scale_factor),
                        shadow: CompositeShadow {
                            offset: shadow.shadow.offset.scale(scale_factor),
                            blur_radius: shadow.shadow.blur_radius.scale(scale_factor),
                            color: shadow.shadow.color,
                            mode: shadow.shadow.mode,
                        },
                    });
                }
                CompositeEffect::ProcessedContentGlow(glow) => {
                    if let Some(glow) = lower_processed_content_glow(scale_factor, glow) {
                        plan.processed_content_glows.push(glow);
                    }
                }
                CompositeEffect::RoundedMask(corner_radii) => {
                    let corner_radii = corner_radii.scale(scale_factor);
                    plan.source_mask = Some(corner_radii);
                    plan.source_mask_shape = GroupShapeKind::RoundedRect;
                    plan.source_mask_blur_order = SourceMaskBlurOrder::AfterBlur;
                    plan.material_shape = Some(SurfaceSilhouette::GroupShape {
                        corner_radii,
                        shape_kind: GroupShapeKind::RoundedRect,
                    });
                    plan.rounded_mask = Some(corner_radii);
                }
                CompositeEffect::BlendMode(mode) => {
                    plan.blend_mode = *mode;
                }
            }
        }

        plan.opacity = plan.opacity.clamp(0., 1.);
        plan
    }

    /// Returns the normalized group opacity.
    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    /// Returns the normalized source color filter.
    pub fn source_color_filter(&self) -> SourceColorFilter {
        self.source_color_filter
    }

    /// Returns the normalized Gaussian source blur radius in device pixels.
    pub fn source_blur_radius(&self) -> ScaledPixels {
        self.source_blur_radius
    }

    /// Returns the normalized directional (motion) blur half-extent vector in
    /// device pixels. A zero vector means directional blur is inactive; when it
    /// is active it replaces the isotropic source blur for source sampling.
    pub fn source_directional_blur(&self) -> Point<ScaledPixels> {
        self.source_directional_blur
    }

    /// Returns the optional source/content mask in device pixels.
    pub fn source_mask(&self) -> Option<Corners<ScaledPixels>> {
        self.source_mask
    }

    /// Returns the corner-SDF family of the source/content mask.
    pub fn source_mask_shape(&self) -> GroupShapeKind {
        self.source_mask_shape
    }

    /// Returns whether source masking happens before or after source blur.
    pub fn source_mask_blur_order(&self) -> SourceMaskBlurOrder {
        self.source_mask_blur_order
    }

    /// Returns the optional arbitrary-path source mask in device pixels.
    ///
    /// When present, the renderer rasterizes this path into a coverage mask and
    /// selects sampled source-mask mode; the path mask supersedes any analytic
    /// [`source_mask`](Self::source_mask) for source clipping.
    pub fn source_mask_path(&self) -> Option<&Arc<Path<ScaledPixels>>> {
        self.source_mask_path.as_ref()
    }

    /// Returns the normalized backdrop color filter.
    pub fn backdrop_color_filter(&self) -> SourceColorFilter {
        self.backdrop_color_filter
    }

    /// Returns the normalized Gaussian backdrop blur radius in device pixels.
    pub fn backdrop_blur_radius(&self) -> ScaledPixels {
        self.backdrop_blur_radius
    }

    /// Returns the optional normalized backdrop lens in device pixels.
    pub fn backdrop_lens(&self) -> Option<CompositeBackdropLens<ScaledPixels>> {
        self.backdrop_lens
    }

    /// Returns the normalized backdrop tint.
    pub fn backdrop_tint(&self) -> Hsla {
        self.backdrop_tint
    }

    /// Returns the optional material shape in device pixels.
    pub fn material_shape(&self) -> Option<&SurfaceSilhouette<ScaledPixels>> {
        self.material_shape.as_ref()
    }

    /// Returns whether the plan needs a backdrop material layer.
    pub fn has_backdrop_material(&self) -> bool {
        self.backdrop_blur_radius.0 > f32::EPSILON
            || !self.backdrop_color_filter.is_identity()
            || self.backdrop_lens.is_some_and(|lens| !lens.is_identity())
            || self.backdrop_tint.a > f32::EPSILON
    }

    /// Returns the final blend mode.
    pub fn blend_mode(&self) -> CompositeBlendMode {
        self.blend_mode
    }

    /// Returns whether this plan needs pixels from the parent render target.
    pub fn reads_backdrop(&self) -> bool {
        self.has_backdrop_material() || self.blend_mode != CompositeBlendMode::Normal
    }

    /// Returns whether accepted visual effects can render without capturing
    /// source pixels into an offscreen group texture.
    pub fn can_render_direct_surface_effects(&self) -> bool {
        !self.surface_shadows.is_empty()
            && self.drop_shadows.is_empty()
            && self.processed_content_glows.is_empty()
            && (self.opacity - 1.).abs() <= f32::EPSILON
            && self.source_color_filter.is_identity()
            && self.source_blur_radius.0 <= f32::EPSILON
            && self.source_directional_blur.x.0.abs() <= f32::EPSILON
            && self.source_directional_blur.y.0.abs() <= f32::EPSILON
            && self.source_mask.is_none()
            && self.source_mask_path.is_none()
            && self.rounded_mask.is_none()
            && !self.has_backdrop_material()
            && self.blend_mode == CompositeBlendMode::Normal
    }

    /// Returns source-alpha drop shadows in declared order.
    pub fn drop_shadows(&self) -> &[CompositeDropShadow<ScaledPixels>] {
        &self.drop_shadows
    }

    /// Returns the additive shadow-source counters implied by this normalized plan.
    pub fn shadow_source_counters(&self) -> RenderGroupShadowSourceCounters {
        RenderGroupShadowSourceCounters {
            content_alpha: self.drop_shadows.len() as u32,
            surface_geometry: self.surface_shadows.len() as u32,
        }
    }

    /// Returns the additive shadow-mode counters implied by this normalized plan.
    pub fn shadow_mode_counters(&self) -> RenderGroupShadowModeCounters {
        let mut counters = RenderGroupShadowModeCounters::default();
        for shadow in &self.drop_shadows {
            match shadow.shadow.mode {
                RenderGroupShadowMode::Separable => counters.separable += 1,
                RenderGroupShadowMode::Exact => counters.exact += 1,
                RenderGroupShadowMode::Downsampled => counters.downsampled += 1,
            }
        }
        for shadow in &self.surface_shadows {
            match shadow.shadow.mode {
                RenderGroupShadowMode::Separable => counters.separable += 1,
                RenderGroupShadowMode::Exact => counters.exact += 1,
                RenderGroupShadowMode::Downsampled => counters.downsampled += 1,
            }
        }
        counters
    }

    /// Returns the maximum sampled Gaussian kernel radius for content-alpha
    /// shadows in this plan.
    pub fn content_alpha_shadow_max_kernel_radius(&self) -> u32 {
        self.drop_shadows
            .iter()
            .map(|shadow| {
                gaussian_kernel_radius(shadow.shadow.blur_radius).min(u32::MAX as u64) as u32
            })
            .max()
            .unwrap_or(0)
    }

    fn content_alpha_separable_blur_profiles(&self) -> impl Iterator<Item = ScaledPixels> + '_ {
        let mut profiles = Vec::new();
        for shadow in &self.drop_shadows {
            if shadow.shadow.mode == RenderGroupShadowMode::Separable
                && shadow.shadow.blur_radius.0 > f32::EPSILON
                && !profiles.iter().any(|profile: &ScaledPixels| {
                    profile.0.to_bits() == shadow.shadow.blur_radius.0.to_bits()
                })
            {
                profiles.push(shadow.shadow.blur_radius);
            }
        }
        profiles.into_iter()
    }

    fn content_alpha_separable_blur_profile_count(&self) -> u8 {
        self.content_alpha_separable_blur_profiles()
            .count()
            .min(u8::MAX as usize) as u8
    }

    /// Returns surface-shape drop shadows in declared order.
    pub fn surface_shadows(&self) -> &[CompositeSurfaceShadow<ScaledPixels>] {
        &self.surface_shadows
    }

    /// Returns the tight draw bounds for surface-shadow work in declared order.
    pub fn surface_shadow_draw_bounds(
        &self,
        group_bounds: Bounds<ScaledPixels>,
        content_mask: ContentMask<ScaledPixels>,
    ) -> Vec<Bounds<ScaledPixels>> {
        self.surface_shadows
            .iter()
            .filter_map(|shadow| shadow.draw_bounds(group_bounds, content_mask))
            .collect()
    }

    /// Returns processed-content glows in declared order.
    pub fn processed_content_glows(&self) -> &[CompositeProcessedContentGlowPlan] {
        &self.processed_content_glows
    }

    /// Returns the optional rounded group mask in device pixels.
    pub fn rounded_mask(&self) -> Option<Corners<ScaledPixels>> {
        self.rounded_mask
    }

    /// Returns the symmetric visual outset required by this effect plan.
    pub fn visual_outset(scale_factor: f32, effects: &[CompositeEffect]) -> ScaledPixels {
        let mut outset = ScaledPixels(0.);
        for effect in effects {
            match effect {
                CompositeEffect::SourceBlur(radius) => {
                    outset = ScaledPixels(
                        outset
                            .0
                            .max(gaussian_kernel_outset(radius.scale(scale_factor)).0),
                    );
                }
                CompositeEffect::SourceDirectionalBlur(v) => {
                    outset = ScaledPixels(
                        outset
                            .0
                            .max(directional_kernel_outset(v.scale(scale_factor)).0),
                    );
                }
                CompositeEffect::DropShadow(shadow) => {
                    let offset = shadow.shadow.offset.scale(scale_factor);
                    let blur_outset =
                        gaussian_kernel_outset(shadow.shadow.blur_radius.scale(scale_factor)).0;
                    let shadow_outset = offset.x.0.abs().max(offset.y.0.abs()) + blur_outset;
                    outset = ScaledPixels(outset.0.max(shadow_outset));
                }
                CompositeEffect::SurfaceShadow(shadow) => {
                    let offset = shadow.shadow.offset.scale(scale_factor);
                    let blur_outset =
                        gaussian_kernel_outset(shadow.shadow.blur_radius.scale(scale_factor)).0;
                    let shadow_outset = offset.x.0.abs().max(offset.y.0.abs()) + blur_outset;
                    outset = ScaledPixels(outset.0.max(shadow_outset));
                }
                CompositeEffect::ProcessedContentGlow(glow) => {
                    if let Some(glow) = lower_processed_content_glow(scale_factor, glow) {
                        outset =
                            ScaledPixels(outset.0.max(gaussian_kernel_outset(glow.blur_radius).0));
                    }
                }
                CompositeEffect::BackdropBlur(radius) => {
                    outset = ScaledPixels(
                        outset
                            .0
                            .max(gaussian_kernel_outset(radius.scale(scale_factor)).0),
                    );
                }
                CompositeEffect::BackdropLens(_) => {}
                CompositeEffect::Opacity(_)
                | CompositeEffect::SourceColorFilter(_)
                | CompositeEffect::SourceMask(_)
                | CompositeEffect::SourceMaskBeforeBlur(_)
                | CompositeEffect::SourceMaskPath(_)
                | CompositeEffect::BackdropColorFilter(_)
                | CompositeEffect::BackdropTint(_)
                | CompositeEffect::BlendMode(_)
                | CompositeEffect::MaterialShape(_)
                | CompositeEffect::RoundedMask(_) => {}
            }
        }
        outset
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[expect(missing_docs)]
pub struct PathId(pub usize);

/// A line made up of a series of vertices and control points.
#[derive(Clone, Debug, PartialEq)]
#[expect(missing_docs)]
pub struct Path<P: Clone + Debug + Default + PartialEq> {
    pub id: PathId,
    pub order: DrawOrder,
    pub bounds: Bounds<P>,
    pub content_mask: ContentMask<P>,
    pub vertices: Vec<PathVertex<P>>,
    pub color: Background,
    start: Point<P>,
    current: Point<P>,
    contour_count: usize,
}

impl Path<Pixels> {
    /// Create a new path with the given starting point.
    pub fn new(start: Point<Pixels>) -> Self {
        Self {
            id: PathId(0),
            order: DrawOrder::default(),
            vertices: Vec::new(),
            start,
            current: start,
            bounds: Bounds {
                origin: start,
                size: Default::default(),
            },
            content_mask: Default::default(),
            color: Default::default(),
            contour_count: 0,
        }
    }

    /// Scale this path by the given factor.
    pub fn scale(&self, factor: f32) -> Path<ScaledPixels> {
        Path {
            id: self.id,
            order: self.order,
            bounds: self.bounds.scale(factor),
            content_mask: self.content_mask.scale(factor),
            vertices: self
                .vertices
                .iter()
                .map(|vertex| vertex.scale(factor))
                .collect(),
            start: self.start.map(|start| start.scale(factor)),
            current: self.current.scale(factor),
            contour_count: self.contour_count,
            color: self.color,
        }
    }

    /// Move the start, current point to the given point.
    pub fn move_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        self.start = to;
        self.current = to;
    }

    /// Draw a straight line from the current point to the given point.
    pub fn line_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }
        self.current = to;
    }

    /// Draw a curve from the current point to the given point, using the given control point.
    pub fn curve_to(&mut self, to: Point<Pixels>, ctrl: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }

        self.push_triangle(
            (self.current, ctrl, to),
            (point(0., 0.), point(0.5, 0.), point(1., 1.)),
        );
        self.current = to;
    }

    /// Push a triangle to the Path.
    pub fn push_triangle(
        &mut self,
        xy: (Point<Pixels>, Point<Pixels>, Point<Pixels>),
        st: (Point<f32>, Point<f32>, Point<f32>),
    ) {
        self.bounds = self
            .bounds
            .union(&Bounds {
                origin: xy.0,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.1,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.2,
                size: Default::default(),
            });

        self.vertices.push(PathVertex {
            xy_position: xy.0,
            st_position: st.0,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.1,
            st_position: st.1,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.2,
            st_position: st.2,
            content_mask: Default::default(),
        });
    }
}

impl<T> Path<T>
where
    T: Clone + Debug + Default + PartialEq + PartialOrd + Add<T, Output = T> + Sub<Output = T>,
{
    #[allow(unused)]
    #[expect(missing_docs)]
    pub fn clipped_bounds(&self) -> Bounds<T> {
        self.bounds.intersect(&self.content_mask.bounds)
    }
}

impl From<Path<ScaledPixels>> for Primitive {
    fn from(path: Path<ScaledPixels>) -> Self {
        Primitive::Path(path)
    }
}

#[derive(Clone, Debug, PartialEq)]
#[repr(C)]
#[expect(missing_docs)]
pub struct PathVertex<P: Clone + Debug + Default + PartialEq> {
    pub xy_position: Point<P>,
    pub st_position: Point<f32>,
    pub content_mask: ContentMask<P>,
}

#[expect(missing_docs)]
impl PathVertex<Pixels> {
    pub fn scale(&self, factor: f32) -> PathVertex<ScaledPixels> {
        PathVertex {
            xy_position: self.xy_position.scale(factor),
            st_position: self.st_position,
            content_mask: self.content_mask.scale(factor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{red, size};

    #[test]
    fn planning_rejection_describe_names_provenance_limit_and_suggestion() {
        let rejection = RenderGroupPlanningRejection {
            effect: RenderGroupRejectedEffect::DropShadow,
            reason: RenderGroupPlanningRejectionReason::LimitExceeded {
                limit: ScaledPixels(8.),
                requested: ScaledPixels(12.),
                unit: RenderGroupLimitUnit::GaussianSigma,
            },
            suggestion: "use an explicitly approximate blur tier or reduce the exact blur radius",
        };
        assert_eq!(
            rejection.describe(),
            concat!(
                "derived.content_alpha.shadow: requested 12 exceeds the exact-effect limit 8 ",
                "[gaussian sigma (scaled px)]; use an explicitly approximate blur tier or ",
                "reduce the exact blur radius"
            ),
        );
    }

    #[test]
    fn rejected_drop_shadow_over_sigma_cap_is_absent_from_plan_but_described() {
        let effects = vec![CompositeEffect::drop_shadow(
            point(crate::px(0.), crate::px(4.)),
            crate::px(6.),
            crate::hsla(0., 0., 0., 0.1),
        )];
        // Retina: 6px blur scales to sigma 12 > MAX_EXACT_GAUSSIAN_SIGMA.
        let plan = LogicalVisualPlan::from_effects(2.0, 1.0, effects);
        assert_eq!(plan.normalized_effects().drop_shadows().len(), 0);
        assert_eq!(plan.planning_rejections().len(), 1);
        assert!(
            plan.planning_rejections()[0]
                .describe()
                .contains("derived.content_alpha.shadow")
        );
    }

    fn sp(value: f32) -> ScaledPixels {
        ScaledPixels(value)
    }

    fn scaled_bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
        Bounds::new(point(sp(x), sp(y)), size(sp(width), sp(height)))
    }

    fn mask(bounds: Bounds<ScaledPixels>) -> ContentMask<ScaledPixels> {
        ContentMask { bounds }
    }

    fn support_counters(
        plan: &LogicalVisualPlan,
        bounds: Bounds<ScaledPixels>,
    ) -> RenderGroupSupportCounters {
        plan.support_counters(bounds, bounds, mask(bounds))
    }

    fn scaled_group_silhouette(corner_radius: f32) -> SurfaceSilhouette<ScaledPixels> {
        SurfaceSilhouette::GroupShape {
            corner_radii: Corners::all(ScaledPixels(corner_radius)),
            shape_kind: GroupShapeKind::RoundedRect,
        }
    }

    fn group(
        bounds: Bounds<ScaledPixels>,
        capture_bounds: Bounds<ScaledPixels>,
        content_mask: ContentMask<ScaledPixels>,
    ) -> PaintGroup {
        PaintGroup::new(
            0,
            bounds,
            capture_bounds,
            content_mask,
            LogicalVisualPlan::from_effects(1., 0.5, vec![CompositeEffect::opacity(0.5)]),
            Scene::default(),
        )
    }

    fn apply_filter(filter: SourceColorFilter, rgb: [f32; 3]) -> [f32; 3] {
        let (matrix, offset) = filter.components();
        [
            matrix[0][0] * rgb[0] + matrix[0][1] * rgb[1] + matrix[0][2] * rgb[2] + offset[0],
            matrix[1][0] * rgb[0] + matrix[1][1] * rgb[1] + matrix[1][2] * rgb[2] + offset[1],
            matrix[2][0] * rgb[0] + matrix[2][1] * rgb[1] + matrix[2][2] * rgb[2] + offset[2],
        ]
    }

    #[test]
    fn group_batches_order_against_other_primitives() {
        let mut scene = Scene::default();
        let bounds = scaled_bounds(0., 0., 10., 10.);
        let content_mask = mask(scaled_bounds(0., 0., 100., 100.));

        scene.insert_primitive(Quad {
            order: 0,
            bounds,
            content_mask,
            ..Default::default()
        });
        scene.insert_primitive(group(bounds, bounds, content_mask));
        scene.finish();

        let batches = scene.batches().collect::<Vec<_>>();
        assert!(matches!(batches[0], PrimitiveBatch::Quads(_)));
        assert!(matches!(batches[1], PrimitiveBatch::Groups(_)));
    }

    #[test]
    fn replay_preserves_group_primitives() {
        let mut scene = Scene::default();
        let bounds = scaled_bounds(0., 0., 10., 10.);
        scene.insert_primitive(group(bounds, bounds, mask(bounds)));

        let mut replayed = Scene::default();
        replayed.replay(0..scene.len(), &scene);

        assert_eq!(replayed.groups.len(), 1);
        assert_eq!(replayed.groups[0].plan.normalized_effects().opacity(), 0.25);
        assert_eq!(replayed.groups[0].plan.effects().len(), 1);
    }

    #[test]
    fn visual_bounds_include_group_capture_bounds() {
        let mut scene = Scene::default();
        let bounds = scaled_bounds(10., 10., 20., 20.);
        let capture_bounds = scaled_bounds(5., 5., 40., 40.);
        let content_mask = mask(scaled_bounds(0., 0., 100., 100.));
        scene.insert_primitive(group(bounds, capture_bounds, content_mask));

        assert_eq!(scene.visual_bounds(), Some(capture_bounds));
    }

    #[test]
    fn composite_effect_plan_multiplies_group_opacity() {
        let plan = CompositeEffectPlan::from_effects(
            1.,
            0.5,
            &[CompositeEffect::opacity(0.5), CompositeEffect::opacity(0.5)],
        );

        assert_eq!(plan.opacity(), 0.125);
        assert!(plan.source_color_filter().is_identity());
    }

    #[test]
    fn composite_effect_plan_composes_source_color_filters_in_order() {
        let brightness_then_invert = CompositeEffectPlan::from_effects(
            1.,
            1.,
            &[
                CompositeEffect::brightness(0.5),
                CompositeEffect::invert(1.),
            ],
        );
        let invert_then_brightness = CompositeEffectPlan::from_effects(
            1.,
            1.,
            &[
                CompositeEffect::invert(1.),
                CompositeEffect::brightness(0.5),
            ],
        );

        assert_eq!(
            apply_filter(brightness_then_invert.source_color_filter(), [1., 0., 0.]),
            [0.5, 1., 1.]
        );
        assert_eq!(
            apply_filter(invert_then_brightness.source_color_filter(), [1., 0., 0.]),
            [0., 0.5, 0.5]
        );
    }

    #[test]
    fn source_color_matrix_composes_with_named_filters_in_order() {
        let matrix = SourceColorFilter::color_matrix(
            [[0., 0., 0.], [1., 0., 0.], [0., 0., 0.]],
            [0., 0., 0.],
        );
        let plan = CompositeEffectPlan::from_effects(
            1.,
            1.,
            &[
                CompositeEffect::SourceColorFilter(matrix),
                CompositeEffect::brightness(0.5),
            ],
        );

        assert_eq!(
            apply_filter(plan.source_color_filter(), [1., 0., 0.]),
            [0., 0.5, 0.]
        );
    }

    #[test]
    fn composite_effect_plan_scales_source_effect_geometry() {
        let shadow_shape = GroupShape::rounded_rect(Corners::all(Pixels(7.)));
        let plan = CompositeEffectPlan::from_effects(
            2.,
            1.,
            &[
                CompositeEffect::source_blur(Pixels(3.)),
                CompositeEffect::drop_shadow(point(Pixels(4.), Pixels(-2.)), Pixels(5.), red()),
                CompositeEffect::surface_shadow(
                    shadow_shape,
                    point(Pixels(-3.), Pixels(6.)),
                    Pixels(4.),
                    red(),
                ),
                CompositeEffect::rounded_mask(Corners::all(Pixels(6.))),
            ],
        );

        assert_eq!(plan.source_blur_radius(), ScaledPixels(6.));
        assert_eq!(
            plan.drop_shadows()[0].shadow.offset,
            point(ScaledPixels(8.), ScaledPixels(-4.))
        );
        assert_eq!(plan.drop_shadows()[0].shadow.blur_radius, ScaledPixels(10.));
        assert_eq!(
            plan.surface_shadows()[0].silhouette,
            scaled_group_silhouette(14.)
        );
        assert_eq!(
            plan.surface_shadows()[0].shadow.offset,
            point(ScaledPixels(-6.), ScaledPixels(12.))
        );
        assert_eq!(
            plan.surface_shadows()[0].shadow.blur_radius,
            ScaledPixels(8.)
        );
        assert_eq!(plan.source_mask(), Some(Corners::all(ScaledPixels(12.))));
        assert_eq!(plan.material_shape(), Some(&scaled_group_silhouette(12.)));
        assert_eq!(plan.rounded_mask(), Some(Corners::all(ScaledPixels(12.))));
        assert_eq!(
            CompositeEffectPlan::visual_outset(
                2.,
                &[
                    CompositeEffect::source_blur(Pixels(3.)),
                    CompositeEffect::drop_shadow(point(Pixels(4.), Pixels(-2.)), Pixels(5.), red()),
                ],
            ),
            ScaledPixels(32.)
        );
    }

    #[test]
    fn composite_effect_plan_keeps_source_mask_and_material_shape_distinct() {
        let plan = CompositeEffectPlan::from_effects(
            2.,
            1.,
            &[
                CompositeEffect::source_mask(GroupShape::rounded_rect(Corners::all(Pixels(4.)))),
                CompositeEffect::material_shape(GroupShape::rounded_rect(Corners::all(Pixels(9.)))),
            ],
        );

        assert_eq!(plan.source_mask(), Some(Corners::all(ScaledPixels(8.))));
        assert_eq!(plan.material_shape(), Some(&scaled_group_silhouette(18.)));
        assert_eq!(plan.rounded_mask(), None);
    }

    #[test]
    fn surface_silhouette_union_scales_material_and_surface_shadow_geometry() {
        let slab = SurfacePrimitive::new(
            Bounds::new(
                point(Pixels(0.), Pixels(0.)),
                size(Pixels(100.), Pixels(32.)),
            ),
            GroupShape::rounded_rect(Corners::all(Pixels(6.))),
        );
        let selected_tab = SurfacePrimitive::new(
            Bounds::new(
                point(Pixels(24.), Pixels(30.)),
                size(Pixels(36.), Pixels(14.)),
            ),
            GroupShape::rounded_rect(Corners {
                top_left: Pixels(0.),
                top_right: Pixels(0.),
                bottom_right: Pixels(6.),
                bottom_left: Pixels(6.),
            }),
        );
        let silhouette = SurfaceSilhouette::union([slab, selected_tab]).unwrap();
        let plan = CompositeEffectPlan::from_effects(
            2.,
            1.,
            &[
                CompositeEffect::material_shape(silhouette.clone()),
                CompositeEffect::surface_shadow(
                    silhouette,
                    point(Pixels(0.), Pixels(3.)),
                    Pixels(4.),
                    red(),
                ),
            ],
        );

        let material_primitives = plan.material_shape().unwrap().primitives();
        assert_eq!(material_primitives.len(), 2);
        assert_eq!(
            material_primitives[0].bounds(),
            scaled_bounds(0., 0., 200., 64.)
        );
        assert_eq!(
            material_primitives[1].bounds(),
            scaled_bounds(48., 60., 72., 28.)
        );
        assert_eq!(
            material_primitives[1].corner_radii(),
            Corners {
                top_left: ScaledPixels(0.),
                top_right: ScaledPixels(0.),
                bottom_right: ScaledPixels(12.),
                bottom_left: ScaledPixels(12.),
            }
        );

        let shadow_primitives = plan.surface_shadows()[0].silhouette.primitives();
        assert_eq!(shadow_primitives, material_primitives);
        assert_eq!(
            plan.surface_shadows()[0].shadow.offset,
            point(ScaledPixels(0.), ScaledPixels(6.))
        );
    }

    #[test]
    fn surface_silhouette_union_can_include_group_bounds_shape() {
        let selected_tab = SurfacePrimitive::new(
            Bounds::new(
                point(Pixels(24.), Pixels(30.)),
                size(Pixels(36.), Pixels(14.)),
            ),
            GroupShape::rounded_rect(Corners {
                top_left: Pixels(0.),
                top_right: Pixels(0.),
                bottom_right: Pixels(6.),
                bottom_left: Pixels(6.),
            }),
        );
        let silhouette = SurfaceSilhouette::union_with_group_shape(
            GroupShape::rounded_rect(Corners::all(Pixels(6.))),
            [selected_tab],
        )
        .unwrap();
        let plan = CompositeEffectPlan::from_effects(
            2.,
            1.,
            &[CompositeEffect::surface_shadow(
                silhouette,
                point(Pixels(0.), Pixels(3.)),
                Pixels(4.),
                red(),
            )],
        );

        assert_eq!(plan.surface_shadows()[0].silhouette.primitives().len(), 1);
        let sprite_data = plan.surface_shadows()[0]
            .silhouette
            .sprite_data(scaled_bounds(0., 0., 200., 64.));
        assert_eq!(sprite_data.count, 2);
        assert_eq!(sprite_data.bounds[0], scaled_bounds(0., 0., 200., 64.));
        assert_eq!(sprite_data.corner_radii[0], Corners::all(ScaledPixels(12.)));
        assert_eq!(sprite_data.bounds[1], scaled_bounds(48., 60., 72., 28.));
        assert_eq!(
            sprite_data.corner_radii[1],
            Corners {
                top_left: ScaledPixels(0.),
                top_right: ScaledPixels(0.),
                bottom_right: ScaledPixels(12.),
                bottom_left: ScaledPixels(12.),
            }
        );
    }

    #[test]
    fn surface_silhouette_union_rejects_empty_and_oversized_inputs() {
        assert_eq!(
            SurfaceSilhouette::union([]),
            Err(SurfaceSilhouetteError::Empty)
        );

        let primitive = SurfacePrimitive::new(
            Bounds::new(point(Pixels(0.), Pixels(0.)), size(Pixels(1.), Pixels(1.))),
            GroupShape::rectangle(),
        );
        assert_eq!(
            SurfaceSilhouette::union(std::iter::repeat_n(
                primitive,
                MAX_SURFACE_SILHOUETTE_PRIMITIVES + 1
            )),
            Err(SurfaceSilhouetteError::TooManyPrimitives {
                limit: MAX_SURFACE_SILHOUETTE_PRIMITIVES,
                requested: MAX_SURFACE_SILHOUETTE_PRIMITIVES + 1,
            })
        );
        assert_eq!(
            SurfaceSilhouette::union_with_group_shape(
                GroupShape::rectangle(),
                std::iter::repeat_n(primitive, MAX_SURFACE_SILHOUETTE_PRIMITIVES),
            ),
            Err(SurfaceSilhouetteError::TooManyPrimitives {
                limit: MAX_SURFACE_SILHOUETTE_PRIMITIVES,
                requested: MAX_SURFACE_SILHOUETTE_PRIMITIVES + 1,
            })
        );
    }

    #[test]
    fn derived_surface_shape_shadow_lowers_with_surface_provenance() {
        let shape = GroupShape::rounded_rect(Corners::all(Pixels(10.)));
        let input = RenderGroupInput::Semantic(SemanticRenderGroupSpec::from_layers(
            None,
            ContentLayer::default(),
            [DerivedLayer::from_surface_shape(shape).shadow(
                point(Pixels(2.), Pixels(3.)),
                Pixels(4.),
                red(),
            )],
            Composite::normal(),
        ));

        let plan = LogicalVisualPlan::from_input(2., 1., &input);

        assert_eq!(
            plan.accepted_effects(),
            &[CompositeEffect::surface_shadow(
                shape,
                point(Pixels(2.), Pixels(3.)),
                Pixels(4.),
                red()
            )]
        );
        assert_eq!(plan.normalized_effects().drop_shadows(), []);
        assert_eq!(plan.normalized_effects().surface_shadows().len(), 1);
        assert_eq!(
            plan.normalized_effects().surface_shadows()[0].silhouette,
            scaled_group_silhouette(20.)
        );
        assert_eq!(
            plan.normalized_effects().surface_shadows()[0].shadow.offset,
            point(ScaledPixels(4.), ScaledPixels(6.))
        );
        assert_eq!(
            plan.normalized_effects().surface_shadows()[0]
                .shadow
                .blur_radius,
            ScaledPixels(8.)
        );
        assert_eq!(plan.requirements().output_outset, ScaledPixels(30.));
    }

    #[test]
    fn material_shape_is_identity_without_a_material_effect() {
        assert!(
            CompositeEffect::material_shape(GroupShape::rounded_rect(Corners::all(Pixels(9.))))
                .is_identity()
        );
    }

    #[test]
    fn empty_glass_surface_lowers_to_no_effects() {
        let shape = GroupShape::rounded_rect(Corners::all(Pixels(9.)));
        let mut effects = Vec::new();
        GlassSurface::for_shape(shape).push_effects(&mut effects);
        assert_eq!(effects, []);

        GlassSurface::for_shape(shape)
            .frost(Pixels(4.))
            .push_effects(&mut effects);
        assert_eq!(
            effects,
            [
                CompositeEffect::material_shape(shape),
                CompositeEffect::backdrop_blur(Pixels(4.))
            ]
        );
    }

    #[test]
    fn logical_visual_plan_lowers_semantic_input_once() {
        let shape = GroupShape::rounded_rect(Corners::all(Pixels(8.)));
        let input = RenderGroupInput::Semantic(SemanticRenderGroupSpec::from_layers(
            Some(GlassSurface::for_shape(shape).frost(Pixels(4.))),
            ContentLayer::clipped_to(shape).blur(Pixels(2.)),
            [],
            Composite::normal().opacity(0.5),
        ));

        let plan = LogicalVisualPlan::from_input(2., 0.75, &input);

        assert_eq!(plan.normalized_effects().opacity(), 0.375);
        assert_eq!(
            plan.normalized_effects().source_blur_radius(),
            ScaledPixels(4.)
        );
        assert_eq!(
            plan.normalized_effects().source_mask(),
            Some(Corners::all(ScaledPixels(16.)))
        );
        assert_eq!(
            plan.normalized_effects().material_shape(),
            Some(&scaled_group_silhouette(16.))
        );
        assert!(plan.requirements().reads_backdrop);
        assert_eq!(plan.requirements().source_outset, ScaledPixels(12.));
        assert_eq!(plan.requirements().backdrop_outset, ScaledPixels(24.));
        assert_eq!(plan.physical_plan().backdrop_copies, 1);
    }

    #[test]
    fn render_group_content_stages_preserve_mask_blur_order() {
        let shape = GroupShape::rounded_rect(Corners::all(Pixels(8.)));
        let clip_then_blur = RenderGroupInput::Semantic(SemanticRenderGroupSpec::from_layers(
            None,
            ContentLayer::staged([
                ContentStage::clip_to(shape),
                ContentStage::exact_blur(Pixels(2.)),
            ]),
            [],
            Composite::normal(),
        ));
        let blur_then_clip = RenderGroupInput::Semantic(SemanticRenderGroupSpec::from_layers(
            None,
            ContentLayer::staged([
                ContentStage::exact_blur(Pixels(2.)),
                ContentStage::clip_to(shape),
            ]),
            [],
            Composite::normal(),
        ));

        let clip_then_blur = LogicalVisualPlan::from_input(1., 1., &clip_then_blur);
        let blur_then_clip = LogicalVisualPlan::from_input(1., 1., &blur_then_clip);

        assert_eq!(
            clip_then_blur.accepted_effects(),
            &[
                CompositeEffect::source_mask_before_blur(shape),
                CompositeEffect::source_blur(Pixels(2.)),
            ]
        );
        assert_eq!(
            clip_then_blur.normalized_effects().source_mask_blur_order(),
            SourceMaskBlurOrder::BeforeBlur
        );
        assert_eq!(
            blur_then_clip.accepted_effects(),
            &[
                CompositeEffect::source_blur(Pixels(2.)),
                CompositeEffect::source_mask(shape),
            ]
        );
        assert_eq!(
            blur_then_clip.normalized_effects().source_mask_blur_order(),
            SourceMaskBlurOrder::AfterBlur
        );
    }

    #[test]
    fn processed_content_glow_lowers_with_content_pixel_provenance() {
        let input = RenderGroupInput::Semantic(SemanticRenderGroupSpec::from_layers(
            None,
            ContentLayer::default(),
            [DerivedLayer::from_processed_content([
                DerivedStage::threshold_luma(LumaThreshold::above(0.8)),
                DerivedStage::exact_blur(Pixels(3.)),
            ])
            .glow(Glow::tinted(red()))],
            Composite::normal(),
        ));

        let plan = LogicalVisualPlan::from_input(2., 1., &input);

        assert_eq!(plan.planning_rejections(), []);
        assert_eq!(plan.normalized_effects().processed_content_glows().len(), 1);
        assert_eq!(
            plan.normalized_effects().processed_content_glows()[0],
            CompositeProcessedContentGlowPlan {
                luma_threshold: 0.8,
                blur_radius: ScaledPixels(6.),
                color: red(),
            }
        );
        assert_eq!(plan.requirements().output_outset, ScaledPixels(18.));
    }

    #[test]
    fn processed_content_glow_rejects_unsupported_stage_order() {
        let input = RenderGroupInput::Semantic(SemanticRenderGroupSpec::from_layers(
            None,
            ContentLayer::default(),
            [DerivedLayer::from_processed_content([
                DerivedStage::exact_blur(Pixels(3.)),
                DerivedStage::threshold_luma(LumaThreshold::above(0.8)),
            ])
            .glow(Glow::tinted(red()))],
            Composite::normal(),
        ));

        let plan = LogicalVisualPlan::from_input(1., 1., &input);

        assert_eq!(plan.accepted_effects(), []);
        assert_eq!(plan.normalized_effects().processed_content_glows(), []);
        assert_eq!(plan.planning_rejections().len(), 1);
        assert_eq!(
            plan.planning_rejections()[0].effect.provenance(),
            "derived.processed_content.glow"
        );
        assert_eq!(
            plan.planning_rejections()[0].reason,
            RenderGroupPlanningRejectionReason::UnsupportedStageSequence {
                expected: "threshold_luma(...) followed by exact_blur(...)",
            }
        );
    }

    #[test]
    fn logical_visual_plan_reports_support_counters_for_visible_work() {
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![
                CompositeEffect::source_blur(Pixels(2.)),
                CompositeEffect::backdrop_blur(Pixels(4.)),
            ],
        );

        let counters = support_counters(&plan, scaled_bounds(10., 20., 100., 40.));

        assert_eq!(
            counters,
            RenderGroupSupportCounters {
                rendered_groups: 1,
                source_capture_groups: 1,
                source_capture_pixels: 4_000,
                backdrop_read_pixels: 7_936,
                logical_passes: 3,
                physical_passes: 3,
                intermediate_textures: 2,
                backdrop_copies: 1,
                ..RenderGroupSupportCounters::default()
            }
        );
    }

    #[test]
    fn logical_visual_plan_reports_source_dependencies() {
        let shape = GroupShape::rounded_rect(Corners::all(Pixels(8.)));
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![
                CompositeEffect::source_blur(Pixels(2.)),
                CompositeEffect::source_mask(shape),
                CompositeEffect::drop_shadow(point(Pixels(0.), Pixels(4.)), Pixels(3.), red()),
            ],
        );

        assert_eq!(
            plan.dependencies(),
            RenderGroupDependencies {
                source_pixels: true,
                source_alpha: true,
                source_mask: true,
                backdrop_pixels: false,
                destination_pixels: false,
                material_shape: false,
                material_normal: false,
            }
        );
    }

    #[test]
    fn logical_visual_plan_reports_surface_and_destination_dependencies() {
        let shape = GroupShape::rounded_rect(Corners::all(Pixels(8.)));
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![
                CompositeEffect::material_shape(shape),
                CompositeEffect::backdrop_lens(
                    Pixels(4.),
                    Pixels(12.),
                    Pixels(1.),
                    0.4,
                    0.2,
                    point(-0.5, -1.),
                ),
                CompositeEffect::blend_mode(CompositeBlendMode::Multiply),
            ],
        );

        assert_eq!(
            plan.dependencies(),
            RenderGroupDependencies {
                source_pixels: true,
                source_alpha: false,
                source_mask: false,
                backdrop_pixels: true,
                destination_pixels: true,
                material_shape: true,
                material_normal: true,
            }
        );
    }

    #[test]
    fn logical_visual_plan_reports_surface_shadow_shape_dependency() {
        let shape = GroupShape::rounded_rect(Corners::all(Pixels(8.)));
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![CompositeEffect::surface_shadow(
                shape,
                point(Pixels(0.), Pixels(4.)),
                Pixels(3.),
                red(),
            )],
        );

        assert_eq!(
            plan.dependencies(),
            RenderGroupDependencies {
                source_pixels: false,
                source_alpha: false,
                source_mask: false,
                backdrop_pixels: false,
                destination_pixels: false,
                material_shape: true,
                material_normal: false,
            }
        );
    }

    #[test]
    fn logical_visual_plan_counts_surface_shadow_source_and_mode_separately() {
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![CompositeEffect::surface_shadow(
                GroupShape::rounded_rect(Corners::all(Pixels(8.))),
                point(Pixels(0.), Pixels(4.)),
                Pixels(3.),
                red(),
            )],
        );

        let counters = support_counters(&plan, scaled_bounds(0., 0., 100., 40.));

        assert_eq!(
            plan.physical_plan(),
            PhysicalRenderGroupPlan {
                kind: RenderGroupPhysicalPlanKind::DirectSurfaceEffects,
                pass_count: 1,
                intermediate_textures: 0,
                backdrop_copies: 0,
                content_alpha_shadow_blur_passes: 0,
            }
        );
        assert_eq!(
            counters,
            RenderGroupSupportCounters {
                rendered_groups: 1,
                direct_surface_groups: 1,
                surface_shadow_draw_pixels: 4_000,
                logical_passes: 1,
                physical_passes: 1,
                shadow_sources: RenderGroupShadowSourceCounters {
                    surface_geometry: 1,
                    ..RenderGroupShadowSourceCounters::default()
                },
                shadow_modes: RenderGroupShadowModeCounters {
                    exact: 1,
                    ..RenderGroupShadowModeCounters::default()
                },
                ..RenderGroupSupportCounters::default()
            }
        );
    }

    #[test]
    fn logical_visual_plan_dependency_metadata_elides_rejected_work() {
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![CompositeEffect::drop_shadow(
                point(Pixels(0.), Pixels(4.)),
                Pixels(20.),
                red(),
            )],
        );

        assert_eq!(plan.accepted_effects(), []);
        assert_eq!(plan.dependencies(), RenderGroupDependencies::default());
    }

    #[test]
    fn logical_visual_plan_support_counters_separate_rejected_work() {
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![CompositeEffect::source_blur(Pixels(20.))],
        );

        let counters = support_counters(&plan, scaled_bounds(0., 0., 100., 40.));

        assert_eq!(plan.accepted_effects(), []);
        assert_eq!(
            counters,
            RenderGroupSupportCounters {
                rejected_groups: 1,
                ..RenderGroupSupportCounters::default()
            }
        );
    }

    #[test]
    fn logical_visual_plan_support_counters_elide_identity_work() {
        let plan = LogicalVisualPlan::from_effects(1., 1., Vec::new());

        let counters = support_counters(&plan, scaled_bounds(0., 0., 100., 40.));

        assert_eq!(
            counters,
            RenderGroupSupportCounters {
                elided_groups: 1,
                ..RenderGroupSupportCounters::default()
            }
        );
    }

    #[test]
    fn logical_visual_plan_counts_separable_content_alpha_shadow_work() {
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![CompositeEffect::drop_shadow(
                point(Pixels(0.), Pixels(4.)),
                Pixels(3.),
                red(),
            )],
        );

        let counters = support_counters(&plan, scaled_bounds(0., 0., 100., 40.));

        assert_eq!(
            plan.physical_plan(),
            PhysicalRenderGroupPlan {
                kind: RenderGroupPhysicalPlanKind::SourceCapture,
                pass_count: 4,
                intermediate_textures: 2,
                backdrop_copies: 0,
                content_alpha_shadow_blur_passes: 1,
            }
        );
        assert_eq!(
            counters,
            RenderGroupSupportCounters {
                rendered_groups: 1,
                source_capture_groups: 1,
                source_capture_pixels: 4_000,
                logical_passes: 3,
                physical_passes: 4,
                intermediate_textures: 2,
                shadow_sources: RenderGroupShadowSourceCounters {
                    content_alpha: 1,
                    ..RenderGroupShadowSourceCounters::default()
                },
                shadow_modes: RenderGroupShadowModeCounters {
                    separable: 1,
                    ..RenderGroupShadowModeCounters::default()
                },
                content_alpha_shadow_max_kernel_radius: 9,
                content_alpha_shadow_sample_count_estimate: 152_000,
                ..RenderGroupSupportCounters::default()
            }
        );
    }

    #[test]
    fn logical_visual_plan_counts_explicit_exact_content_alpha_shadow_work() {
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![CompositeEffect::drop_shadow_with_mode(
                point(Pixels(0.), Pixels(4.)),
                Pixels(3.),
                red(),
                RenderGroupShadowMode::Exact,
            )],
        );

        let counters = support_counters(&plan, scaled_bounds(0., 0., 100., 40.));

        assert_eq!(
            plan.physical_plan(),
            PhysicalRenderGroupPlan {
                kind: RenderGroupPhysicalPlanKind::SourceCapture,
                pass_count: 3,
                intermediate_textures: 1,
                backdrop_copies: 0,
                content_alpha_shadow_blur_passes: 0,
            }
        );
        assert_eq!(
            counters,
            RenderGroupSupportCounters {
                rendered_groups: 1,
                source_capture_groups: 1,
                source_capture_pixels: 4_000,
                logical_passes: 3,
                physical_passes: 3,
                intermediate_textures: 1,
                shadow_sources: RenderGroupShadowSourceCounters {
                    content_alpha: 1,
                    ..RenderGroupShadowSourceCounters::default()
                },
                shadow_modes: RenderGroupShadowModeCounters {
                    exact: 1,
                    ..RenderGroupShadowModeCounters::default()
                },
                content_alpha_shadow_max_kernel_radius: 9,
                content_alpha_shadow_sample_count_estimate: 1_444_000,
                ..RenderGroupSupportCounters::default()
            }
        );
    }

    #[test]
    fn logical_visual_plan_keeps_zero_blur_content_alpha_shadow_single_pass() {
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![CompositeEffect::drop_shadow(
                point(Pixels(0.), Pixels(4.)),
                Pixels(0.),
                red(),
            )],
        );

        let counters = support_counters(&plan, scaled_bounds(0., 0., 100., 40.));

        assert_eq!(
            plan.physical_plan(),
            PhysicalRenderGroupPlan {
                kind: RenderGroupPhysicalPlanKind::SourceCapture,
                pass_count: 3,
                intermediate_textures: 1,
                backdrop_copies: 0,
                content_alpha_shadow_blur_passes: 0,
            }
        );
        assert_eq!(
            counters,
            RenderGroupSupportCounters {
                rendered_groups: 1,
                source_capture_groups: 1,
                source_capture_pixels: 4_000,
                logical_passes: 3,
                physical_passes: 3,
                intermediate_textures: 1,
                shadow_sources: RenderGroupShadowSourceCounters {
                    content_alpha: 1,
                    ..RenderGroupShadowSourceCounters::default()
                },
                shadow_modes: RenderGroupShadowModeCounters {
                    separable: 1,
                    ..RenderGroupShadowModeCounters::default()
                },
                content_alpha_shadow_max_kernel_radius: 0,
                content_alpha_shadow_sample_count_estimate: 4_000,
                ..RenderGroupSupportCounters::default()
            }
        );
    }

    #[test]
    fn logical_visual_plan_shares_separable_blur_profile_across_matching_shadows() {
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![
                CompositeEffect::drop_shadow(point(Pixels(0.), Pixels(4.)), Pixels(3.), red()),
                CompositeEffect::drop_shadow(point(Pixels(2.), Pixels(6.)), Pixels(3.), red()),
            ],
        );

        let counters = support_counters(&plan, scaled_bounds(0., 0., 100., 40.));

        assert_eq!(
            plan.physical_plan(),
            PhysicalRenderGroupPlan {
                kind: RenderGroupPhysicalPlanKind::SourceCapture,
                pass_count: 4,
                intermediate_textures: 2,
                backdrop_copies: 0,
                content_alpha_shadow_blur_passes: 1,
            }
        );
        assert_eq!(
            counters,
            RenderGroupSupportCounters {
                rendered_groups: 1,
                source_capture_groups: 1,
                source_capture_pixels: 4_000,
                logical_passes: 3,
                physical_passes: 4,
                intermediate_textures: 2,
                shadow_sources: RenderGroupShadowSourceCounters {
                    content_alpha: 2,
                    ..RenderGroupShadowSourceCounters::default()
                },
                shadow_modes: RenderGroupShadowModeCounters {
                    separable: 2,
                    ..RenderGroupShadowModeCounters::default()
                },
                content_alpha_shadow_max_kernel_radius: 9,
                content_alpha_shadow_sample_count_estimate: 228_000,
                ..RenderGroupSupportCounters::default()
            }
        );
    }

    #[test]
    fn logical_visual_plan_rejects_unimplemented_downsampled_content_alpha_shadow() {
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![CompositeEffect::drop_shadow_with_mode(
                point(Pixels(0.), Pixels(4.)),
                Pixels(3.),
                red(),
                RenderGroupShadowMode::Downsampled,
            )],
        );

        assert_eq!(plan.accepted_effects(), []);
        assert_eq!(plan.normalized_effects().drop_shadows(), []);
        assert_eq!(
            plan.planning_rejections(),
            &[RenderGroupPlanningRejection {
                effect: RenderGroupRejectedEffect::DropShadow,
                reason: RenderGroupPlanningRejectionReason::UnsupportedRenderGroupShadowMode {
                    mode: RenderGroupShadowMode::Downsampled,
                },
                suggestion: "use RenderGroupShadowMode::Separable for content-alpha shadows until the downsampled backend lands",
            }]
        );
        assert_eq!(
            plan.planning_rejections()[0].describe(),
            concat!(
                "derived.content_alpha.shadow: unsupported render-group shadow mode downsampled; ",
                "use RenderGroupShadowMode::Separable for content-alpha shadows until the downsampled backend lands"
            )
        );
    }

    #[test]
    fn logical_visual_plan_rejects_unimplemented_surface_shadow_mode() {
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![CompositeEffect::surface_shadow_with_mode(
                GroupShape::rectangle(),
                point(Pixels(0.), Pixels(4.)),
                Pixels(3.),
                red(),
                RenderGroupShadowMode::Separable,
            )],
        );

        assert_eq!(plan.accepted_effects(), []);
        assert_eq!(plan.normalized_effects().surface_shadows(), []);
        assert_eq!(
            plan.planning_rejections(),
            &[RenderGroupPlanningRejection {
                effect: RenderGroupRejectedEffect::SurfaceShadow,
                reason: RenderGroupPlanningRejectionReason::UnsupportedRenderGroupShadowMode {
                    mode: RenderGroupShadowMode::Separable,
                },
                suggestion: "use RenderGroupShadowMode::Exact for surface shadows until a surface-specific optimized mode lands",
            }]
        );
        assert_eq!(
            plan.planning_rejections()[0].describe(),
            concat!(
                "derived.surface_shape.shadow: unsupported render-group shadow mode separable; ",
                "use RenderGroupShadowMode::Exact for surface shadows until a surface-specific optimized mode lands"
            )
        );
    }

    #[test]
    fn logical_visual_plan_support_counters_count_boundary_opacity() {
        let plan = LogicalVisualPlan::from_effects(1., 0.5, Vec::new());

        let counters = support_counters(&plan, scaled_bounds(0., 0., 100., 40.));

        assert_eq!(counters.rendered_groups, 1);
        assert_eq!(counters.elided_groups, 0);
        assert_eq!(counters.source_capture_groups, 1);
        assert_eq!(counters.source_capture_pixels, 4_000);
        assert_eq!(counters.logical_passes, 2);
        assert_eq!(counters.physical_passes, 2);
    }

    #[test]
    fn render_group_input_keeps_semantic_and_raw_modes_disjoint() {
        let shape = GroupShape::rectangle();
        let mut semantic_then_raw = RenderGroupInput::default();
        semantic_then_raw
            .semantic_mut()
            .content(ContentLayer::default());
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                semantic_then_raw
                    .raw_mut()
                    .push(CompositeEffect::opacity(0.5));
            }))
            .is_err()
        );

        let mut raw_then_semantic = RenderGroupInput::default();
        raw_then_semantic
            .raw_mut()
            .push(CompositeEffect::opacity(0.5));
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                raw_then_semantic
                    .semantic_mut()
                    .surface(GlassSurface::for_shape(shape));
            }))
            .is_err()
        );
    }

    #[test]
    fn visual_outset_matches_capped_shader_blur_kernel() {
        assert_eq!(
            CompositeEffectPlan::visual_outset(1., &[CompositeEffect::source_blur(Pixels(20.))]),
            ScaledPixels(24.)
        );
        assert_eq!(
            CompositeEffectPlan::visual_outset(1., &[CompositeEffect::backdrop_blur(Pixels(20.))]),
            ScaledPixels(24.)
        );
        assert_eq!(
            CompositeEffectPlan::visual_outset(
                1.,
                &[CompositeEffect::drop_shadow(
                    point(Pixels(5.), Pixels(-3.)),
                    Pixels(20.),
                    red(),
                )],
            ),
            ScaledPixels(29.)
        );
        assert_eq!(
            CompositeEffectPlan::visual_outset(
                1.,
                &[CompositeEffect::surface_shadow(
                    GroupShape::rectangle(),
                    point(Pixels(-4.), Pixels(7.)),
                    Pixels(20.),
                    red(),
                )],
            ),
            ScaledPixels(31.)
        );
        // Directional blur dilates by its axis-aligned reach when under the cap,
        // and saturates at the same 24px cap as the shader's tap limit so an
        // over-long streak never over-allocates the captured intermediate.
        assert_eq!(
            CompositeEffectPlan::visual_outset(
                1.,
                &[CompositeEffect::directional_blur(0., Pixels(10.))],
            ),
            ScaledPixels(10.)
        );
        assert_eq!(
            CompositeEffectPlan::visual_outset(
                1.,
                &[CompositeEffect::directional_blur(0., Pixels(100.))],
            ),
            ScaledPixels(24.)
        );
    }

    fn semantic_plan(
        surface: Option<GlassSurface>,
        content: ContentLayer,
        derived_layers: impl IntoIterator<Item = DerivedLayer>,
    ) -> LogicalVisualPlan {
        let input = RenderGroupInput::Semantic(SemanticRenderGroupSpec::from_layers(
            surface,
            content,
            derived_layers,
            Composite::normal(),
        ));
        LogicalVisualPlan::from_input(1., 1., &input)
    }

    fn assert_single_exact_limit_rejection(
        plan: &LogicalVisualPlan,
        rejected_effect: RenderGroupRejectedEffect,
        requested: f32,
    ) {
        assert_eq!(plan.planning_rejections().len(), 1);
        assert_eq!(
            plan.planning_rejections()[0],
            RenderGroupPlanningRejection {
                effect: rejected_effect,
                reason: RenderGroupPlanningRejectionReason::LimitExceeded {
                    limit: ScaledPixels(8.),
                    requested: ScaledPixels(requested),
                    unit: RenderGroupLimitUnit::GaussianSigma,
                },
                suggestion: "use an explicitly approximate blur tier or reduce the exact blur radius",
            }
        );
    }

    #[test]
    fn semantic_content_blur_rejects_exact_radius_beyond_kernel_limit() {
        let plan = semantic_plan(None, ContentLayer::unclipped().blur(Pixels(9.)), []);

        assert_single_exact_limit_rejection(&plan, RenderGroupRejectedEffect::SourceBlur, 9.);
        assert!(
            !plan
                .accepted_effects()
                .iter()
                .any(|effect| matches!(effect, CompositeEffect::SourceBlur(_)))
        );
        assert_eq!(
            plan.normalized_effects().source_blur_radius(),
            ScaledPixels(0.)
        );
    }

    #[test]
    fn semantic_surface_frost_rejects_exact_radius_beyond_kernel_limit() {
        let shape = GroupShape::rectangle();
        let plan = semantic_plan(
            Some(GlassSurface::for_shape(shape).frost(Pixels(9.))),
            ContentLayer::default(),
            [],
        );

        assert_single_exact_limit_rejection(&plan, RenderGroupRejectedEffect::BackdropBlur, 9.);
        assert!(
            !plan
                .accepted_effects()
                .iter()
                .any(|effect| matches!(effect, CompositeEffect::BackdropBlur(_)))
        );
        assert_eq!(
            plan.normalized_effects().backdrop_blur_radius(),
            ScaledPixels(0.)
        );
    }

    #[test]
    fn semantic_content_alpha_shadow_rejects_exact_radius_beyond_kernel_limit() {
        let plan = semantic_plan(
            None,
            ContentLayer::default(),
            [DerivedLayer::from_content_alpha().shadow(
                point(Pixels(0.), Pixels(4.)),
                Pixels(9.),
                red(),
            )],
        );

        assert_single_exact_limit_rejection(&plan, RenderGroupRejectedEffect::DropShadow, 9.);
        assert!(
            !plan
                .accepted_effects()
                .iter()
                .any(|effect| matches!(effect, CompositeEffect::DropShadow(_)))
        );
        assert_eq!(plan.normalized_effects().drop_shadows(), []);
    }

    #[test]
    fn semantic_surface_shape_shadow_rejects_exact_radius_beyond_kernel_limit() {
        let shape = GroupShape::rounded_rect(Corners::all(Pixels(8.)));
        let plan = semantic_plan(
            None,
            ContentLayer::default(),
            [DerivedLayer::from_surface_shape(shape).shadow(
                point(Pixels(0.), Pixels(4.)),
                Pixels(9.),
                red(),
            )],
        );

        assert_single_exact_limit_rejection(&plan, RenderGroupRejectedEffect::SurfaceShadow, 9.);
        assert!(
            !plan
                .accepted_effects()
                .iter()
                .any(|effect| matches!(effect, CompositeEffect::SurfaceShadow(_)))
        );
        assert_eq!(plan.normalized_effects().surface_shadows(), []);
    }

    #[test]
    fn semantic_processed_content_glow_rejects_exact_radius_beyond_kernel_limit() {
        let plan = semantic_plan(
            None,
            ContentLayer::default(),
            [DerivedLayer::from_processed_content([
                DerivedStage::threshold_luma(LumaThreshold::above(0.8)),
                DerivedStage::exact_blur(Pixels(9.)),
            ])
            .glow(Glow::tinted(red()))],
        );

        assert_single_exact_limit_rejection(
            &plan,
            RenderGroupRejectedEffect::ProcessedContentGlow,
            9.,
        );
        assert!(
            !plan
                .accepted_effects()
                .iter()
                .any(|effect| matches!(effect, CompositeEffect::ProcessedContentGlow(_)))
        );
        assert_eq!(plan.normalized_effects().processed_content_glows(), []);
    }

    #[test]
    fn logical_visual_plan_rejects_exact_blur_beyond_kernel_limit() {
        let shape = GroupShape::rectangle();
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![
                CompositeEffect::source_blur(Pixels(20.)),
                CompositeEffect::backdrop_blur(Pixels(9.)),
                CompositeEffect::drop_shadow(point(Pixels(0.), Pixels(4.)), Pixels(12.), red()),
                CompositeEffect::surface_shadow(
                    shape,
                    point(Pixels(0.), Pixels(4.)),
                    Pixels(10.),
                    red(),
                ),
                CompositeEffect::opacity(0.5),
            ],
        );

        assert_eq!(plan.accepted_effects(), &[CompositeEffect::opacity(0.5)]);
        assert_eq!(
            plan.normalized_effects().source_blur_radius(),
            ScaledPixels(0.)
        );
        assert_eq!(
            plan.normalized_effects().backdrop_blur_radius(),
            ScaledPixels(0.)
        );
        assert_eq!(plan.requirements().source_outset, ScaledPixels(0.));
        assert_eq!(plan.requirements().backdrop_outset, ScaledPixels(0.));
        assert_eq!(plan.planning_rejections().len(), 4);
        assert_eq!(
            plan.planning_rejections()[0].effect.provenance(),
            "content.blur"
        );
        assert_eq!(
            plan.planning_rejections()[0].reason,
            RenderGroupPlanningRejectionReason::LimitExceeded {
                limit: ScaledPixels(8.),
                requested: ScaledPixels(20.),
                unit: RenderGroupLimitUnit::GaussianSigma,
            }
        );
        assert_eq!(
            plan.planning_rejections()[1].effect.provenance(),
            "surface.frost"
        );
        assert_eq!(
            plan.planning_rejections()[2].effect.provenance(),
            "derived.content_alpha.shadow"
        );
        assert_eq!(
            plan.planning_rejections()[3].effect.provenance(),
            "derived.surface_shape.shadow"
        );
    }

    #[test]
    fn logical_visual_plan_rejects_cumulative_exact_blur_limit() {
        let plan = LogicalVisualPlan::from_effects(
            1.,
            1.,
            vec![
                CompositeEffect::source_blur(Pixels(6.)),
                CompositeEffect::source_blur(Pixels(6.)),
            ],
        );

        assert_eq!(
            plan.accepted_effects(),
            &[CompositeEffect::source_blur(Pixels(6.))]
        );
        assert_eq!(
            plan.normalized_effects().source_blur_radius(),
            ScaledPixels(6.)
        );
        assert_eq!(plan.planning_rejections().len(), 1);
        assert_eq!(
            plan.planning_rejections()[0].reason,
            RenderGroupPlanningRejectionReason::LimitExceeded {
                limit: ScaledPixels(8.),
                requested: ScaledPixels(72_f32.sqrt()),
                unit: RenderGroupLimitUnit::GaussianSigma,
            }
        );
    }

    #[test]
    fn render_group_capability_reports_rendered_current_primitives() {
        let report = RenderGroupCapabilityProbe::BackdropLens.report();

        assert_eq!(report.status, RenderGroupCapabilityStatus::Rendered);
        assert_eq!(report.rejection, None);
        assert!(report.evidence.contains("backend path"));
    }

    #[test]
    fn render_group_capability_reports_non_blur_capability_gaps() {
        let destination_mask = RenderGroupCapabilityProbe::DestinationMask.report();
        assert_eq!(
            destination_mask.status,
            RenderGroupCapabilityStatus::Unsupported
        );
        assert_eq!(
            destination_mask.rejection,
            Some(RenderGroupCapabilityRejectionReason::DestinationReadMissing)
        );

        let temporal = RenderGroupCapabilityProbe::TemporalInteraction.report();
        assert_eq!(temporal.status, RenderGroupCapabilityStatus::Unsupported);
        assert_eq!(
            temporal.rejection,
            Some(RenderGroupCapabilityRejectionReason::TemporalDependencyMissing)
        );

        let diagnostics = RenderGroupCapabilityProbe::BackendDiagnostics.report();
        assert_eq!(
            diagnostics.status,
            RenderGroupCapabilityStatus::InspectorOnly
        );
        assert_eq!(
            diagnostics.rejection,
            Some(RenderGroupCapabilityRejectionReason::BackendDiagnosticsMissing)
        );
    }

    #[test]
    fn composite_effect_plan_scales_backdrop_lens_geometry() {
        let plan = CompositeEffectPlan::from_effects(
            2.,
            1.,
            &[CompositeEffect::backdrop_lens(
                Pixels(4.),
                Pixels(8.),
                Pixels(1.5),
                0.4,
                0.2,
                point(-0.5, -1.),
            )],
        );

        let lens = plan.backdrop_lens().expect("backdrop lens");
        assert_eq!(lens.refraction_radius(), ScaledPixels(8.));
        assert_eq!(lens.rim_width(), ScaledPixels(16.));
        assert_eq!(lens.chromatic_aberration(), ScaledPixels(3.));
        assert_eq!(lens.highlight_strength(), 0.4);
        assert_eq!(lens.shadow_strength(), 0.2);
        assert_eq!(lens.light_direction(), point(-0.5, -1.));
        assert!(plan.reads_backdrop());
        assert!(plan.has_backdrop_material());
    }
}
