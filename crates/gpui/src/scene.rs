// todo("windows"): remove
#![cfg_attr(windows, allow(dead_code))]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AtlasTextureId, AtlasTile, Background, Bounds, ContentMask, Corners, Edges, Hsla, Pixels,
    Point, Radians, Rgba, ScaledPixels, Size, bounds_tree::BoundsTree, point, transparent_black,
};
use std::{
    fmt::Debug,
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
    pub surfaces: Vec<PaintSurface>,
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
            group.effects.iter().any(CompositeEffect::reads_backdrop)
                || group.scene.requires_backdrop_effects()
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
pub enum Primitive {
    Shadow(Shadow),
    Quad(Quad),
    Path(Path<ScaledPixels>),
    Underline(Underline),
    MonochromeSprite(MonochromeSprite),
    SubpixelSprite(SubpixelSprite),
    PolychromeSprite(PolychromeSprite),
    Surface(PaintSurface),
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
    surfaces_iter: Peekable<slice::Iter<'a, PaintSurface>>,
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
pub struct PaintSurface {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    #[cfg(target_os = "macos")]
    pub image_buffer: core_video::pixel_buffer::CVPixelBuffer,
}

impl From<PaintSurface> for Primitive {
    fn from(surface: PaintSurface) -> Self {
        Primitive::Surface(surface)
    }
}

#[derive(Clone)]
#[allow(missing_docs)]
pub struct PaintGroup {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub capture_bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub scale_factor: f32,
    pub boundary_opacity: f32,
    pub effects: Vec<CompositeEffect>,
    pub scene: Arc<Scene>,
}

impl From<PaintGroup> for Primitive {
    fn from(group: PaintGroup) -> Self {
        Primitive::Group(group)
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
#[allow(missing_docs)]
pub enum CompositeEffect {
    Opacity(f32),
    SourceColorFilter(SourceColorFilter),
    SourceBlur(Pixels),
    BackdropColorFilter(SourceColorFilter),
    BackdropBlur(Pixels),
    BackdropTint(Hsla),
    DropShadow(CompositeDropShadow<Pixels>),
    RoundedMask(Corners<Pixels>),
    BlendMode(CompositeBlendMode),
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

    /// Applies a Gaussian blur to the already-rendered backdrop under the group.
    pub fn backdrop_blur(radius: Pixels) -> Self {
        Self::BackdropBlur(Pixels(radius.0.max(0.)))
    }

    /// Draws a translucent tint over the backdrop material under the group.
    pub fn backdrop_tint(color: Hsla) -> Self {
        Self::BackdropTint(color)
    }

    /// Draws a drop shadow from the composited source image's alpha channel.
    pub fn drop_shadow(offset: Point<Pixels>, blur_radius: Pixels, color: Hsla) -> Self {
        Self::DropShadow(CompositeDropShadow {
            offset,
            blur_radius: Pixels(blur_radius.0.max(0.)),
            color,
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
            Self::BackdropColorFilter(filter) => filter.is_identity(),
            Self::BackdropBlur(radius) => radius.0 <= f32::EPSILON,
            Self::BackdropTint(color) => color.a <= f32::EPSILON,
            Self::DropShadow(shadow) => shadow.color.a <= f32::EPSILON,
            Self::RoundedMask(_) => false,
            Self::BlendMode(mode) => *mode == CompositeBlendMode::Normal,
        }
    }

    /// Returns whether this effect needs pixels from the parent render target.
    pub fn reads_backdrop(&self) -> bool {
        match self {
            Self::BackdropColorFilter(filter) => !filter.is_identity(),
            Self::BackdropBlur(radius) => radius.0 > f32::EPSILON,
            Self::BackdropTint(color) => color.a > f32::EPSILON,
            Self::BlendMode(mode) => *mode != CompositeBlendMode::Normal,
            Self::Opacity(_)
            | Self::SourceColorFilter(_)
            | Self::SourceBlur(_)
            | Self::DropShadow(_)
            | Self::RoundedMask(_) => false,
        }
    }
}

/// A drop shadow effect derived from a render group's composited source alpha.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct CompositeDropShadow<P: Clone + Debug + Default + PartialEq> {
    pub offset: Point<P>,
    pub blur_radius: P,
    pub color: Hsla,
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
        }
    }
}

/// An affine source-color transform applied to an already-composited render
/// group image.
///
/// The transform operates on unpremultiplied linear RGB and preserves alpha.
/// This keeps coverage separate from source color effects and lets opacity
/// remain a distinct group effect.
#[derive(Copy, Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct SourceColorFilter {
    matrix: [[f32; 3]; 3],
    offset: [f32; 3],
}

impl SourceColorFilter {
    /// Returns the identity source color filter.
    pub fn identity() -> Self {
        Self {
            matrix: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            offset: [0., 0., 0.],
        }
    }

    /// Returns a filter that multiplies source color channels by `factor`.
    pub fn brightness(factor: f32) -> Self {
        let factor = factor.max(0.);
        Self {
            matrix: [[factor, 0., 0.], [0., factor, 0.], [0., 0., factor]],
            offset: [0., 0., 0.],
        }
    }

    /// Returns a filter that scales source color distance from mid-gray by `factor`.
    pub fn contrast(factor: f32) -> Self {
        let factor = factor.max(0.);
        let offset = 0.5 * (1. - factor);
        Self {
            matrix: [[factor, 0., 0.], [0., factor, 0.], [0., 0., factor]],
            offset: [offset, offset, offset],
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
        }
    }

    /// Returns an affine source-color matrix over unpremultiplied RGB.
    pub fn color_matrix(matrix: [[f32; 3]; 3], offset: [f32; 3]) -> Self {
        Self { matrix, offset }
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

        Self { matrix, offset }
    }

    /// Returns whether this filter leaves source colors unchanged.
    pub fn is_identity(&self) -> bool {
        *self == Self::identity()
    }

    /// Returns the affine matrix and offset for renderer consumption.
    pub fn components(&self) -> ([[f32; 3]; 3], [f32; 3]) {
        (self.matrix, self.offset)
    }
}

/// Renderer-facing normalization of a render group's ordered effect list.
#[derive(Clone, Debug, PartialEq)]
#[allow(missing_docs)]
pub struct CompositeEffectPlan {
    opacity: f32,
    source_color_filter: SourceColorFilter,
    source_blur_radius: ScaledPixels,
    backdrop_color_filter: SourceColorFilter,
    backdrop_blur_radius: ScaledPixels,
    backdrop_tint: Hsla,
    drop_shadows: Vec<CompositeDropShadow<ScaledPixels>>,
    rounded_mask: Option<Corners<ScaledPixels>>,
    blend_mode: CompositeBlendMode,
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
            backdrop_color_filter: SourceColorFilter::identity(),
            backdrop_blur_radius: ScaledPixels(0.),
            backdrop_tint: transparent_black(),
            drop_shadows: Vec::new(),
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
                CompositeEffect::BackdropColorFilter(filter) => {
                    plan.backdrop_color_filter = plan.backdrop_color_filter.then(*filter);
                }
                CompositeEffect::BackdropBlur(radius) => {
                    let radius = radius.scale(scale_factor);
                    plan.backdrop_blur_radius = ScaledPixels(
                        (plan.backdrop_blur_radius.0.powi(2) + radius.0.powi(2)).sqrt(),
                    );
                }
                CompositeEffect::BackdropTint(color) => {
                    plan.backdrop_tint = composite_tint(plan.backdrop_tint, *color);
                }
                CompositeEffect::DropShadow(shadow) => {
                    plan.drop_shadows.push(CompositeDropShadow {
                        offset: shadow.offset.scale(scale_factor),
                        blur_radius: shadow.blur_radius.scale(scale_factor),
                        color: shadow.color,
                    });
                }
                CompositeEffect::RoundedMask(corner_radii) => {
                    plan.rounded_mask = Some(corner_radii.scale(scale_factor));
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

    /// Returns the normalized backdrop color filter.
    pub fn backdrop_color_filter(&self) -> SourceColorFilter {
        self.backdrop_color_filter
    }

    /// Returns the normalized Gaussian backdrop blur radius in device pixels.
    pub fn backdrop_blur_radius(&self) -> ScaledPixels {
        self.backdrop_blur_radius
    }

    /// Returns the normalized backdrop tint.
    pub fn backdrop_tint(&self) -> Hsla {
        self.backdrop_tint
    }

    /// Returns whether the plan needs a backdrop material layer.
    pub fn has_backdrop_material(&self) -> bool {
        self.backdrop_blur_radius.0 > f32::EPSILON
            || !self.backdrop_color_filter.is_identity()
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

    /// Returns source-alpha drop shadows in declared order.
    pub fn drop_shadows(&self) -> &[CompositeDropShadow<ScaledPixels>] {
        &self.drop_shadows
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
                    outset = ScaledPixels(outset.0.max(radius.scale(scale_factor).0 * 3.));
                }
                CompositeEffect::DropShadow(shadow) => {
                    let offset = shadow.offset.scale(scale_factor);
                    let blur_outset = shadow.blur_radius.scale(scale_factor).0 * 3.;
                    let shadow_outset = offset.x.0.abs().max(offset.y.0.abs()) + blur_outset;
                    outset = ScaledPixels(outset.0.max(shadow_outset));
                }
                CompositeEffect::BackdropBlur(radius) => {
                    outset = ScaledPixels(outset.0.max(radius.scale(scale_factor).0 * 3.));
                }
                CompositeEffect::Opacity(_)
                | CompositeEffect::SourceColorFilter(_)
                | CompositeEffect::BackdropColorFilter(_)
                | CompositeEffect::BackdropTint(_)
                | CompositeEffect::BlendMode(_)
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
#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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

    fn sp(value: f32) -> ScaledPixels {
        ScaledPixels(value)
    }

    fn scaled_bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
        Bounds::new(point(sp(x), sp(y)), size(sp(width), sp(height)))
    }

    fn mask(bounds: Bounds<ScaledPixels>) -> ContentMask<ScaledPixels> {
        ContentMask { bounds }
    }

    fn group(
        bounds: Bounds<ScaledPixels>,
        capture_bounds: Bounds<ScaledPixels>,
        content_mask: ContentMask<ScaledPixels>,
    ) -> PaintGroup {
        PaintGroup {
            order: 0,
            bounds,
            capture_bounds,
            content_mask,
            scale_factor: 1.,
            boundary_opacity: 0.5,
            effects: vec![CompositeEffect::opacity(0.5)],
            scene: Arc::new(Scene::default()),
        }
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
        assert_eq!(replayed.groups[0].boundary_opacity, 0.5);
        assert_eq!(replayed.groups[0].effects.len(), 1);
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
        let plan = CompositeEffectPlan::from_effects(
            2.,
            1.,
            &[
                CompositeEffect::source_blur(Pixels(3.)),
                CompositeEffect::drop_shadow(point(Pixels(4.), Pixels(-2.)), Pixels(5.), red()),
                CompositeEffect::rounded_mask(Corners::all(Pixels(6.))),
            ],
        );

        assert_eq!(plan.source_blur_radius(), ScaledPixels(6.));
        assert_eq!(
            plan.drop_shadows()[0].offset,
            point(ScaledPixels(8.), ScaledPixels(-4.))
        );
        assert_eq!(plan.drop_shadows()[0].blur_radius, ScaledPixels(10.));
        assert_eq!(plan.rounded_mask(), Some(Corners::all(ScaledPixels(12.))));
        assert_eq!(
            CompositeEffectPlan::visual_outset(
                2.,
                &[
                    CompositeEffect::source_blur(Pixels(3.)),
                    CompositeEffect::drop_shadow(point(Pixels(4.), Pixels(-2.)), Pixels(5.), red()),
                ],
            ),
            ScaledPixels(38.)
        );
    }
}
