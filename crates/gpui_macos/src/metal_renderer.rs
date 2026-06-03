use crate::metal_atlas::MetalAtlas;
use anyhow::Result;
use block::ConcreteBlock;
use cocoa::{
    base::{NO, YES},
    foundation::{NSSize, NSUInteger},
    quartzcore::AutoresizingMask,
};
use gpui::{
    AtlasTextureId, Background, Bounds, ContentMask, Corners, DevicePixels, Point, ScaledPixels,
    Size, Surface, point,
    scene_protocol::{
        CompositeEffectPlan, MonochromeSprite, PaintGroup, PaintSurface, Path, PolychromeSprite,
        PrimitiveBatch, Quad, RenderGroupBackendCounters, Scene, Shadow, Underline,
    },
    size,
};
#[cfg(any(test, feature = "test-support"))]
use gpui::{SceneCapture, SceneCaptureBackend};
#[cfg(any(test, feature = "test-support"))]
use image::RgbaImage;

use core_foundation::base::TCFType;
use core_video::{
    metal_texture::CVMetalTextureGetTexture,
    metal_texture_cache::CVMetalTextureCache,
    pixel_buffer::{kCVPixelFormatType_32BGRA, kCVPixelFormatType_420YpCbCr8BiPlanarFullRange},
};
use foreign_types::{ForeignType, ForeignTypeRef};
use gpui::nobie_platform_trace;
use metal::{
    CAMetalLayer, CommandQueue, MTLGPUFamily, MTLPixelFormat, MTLResourceOptions, NSRange,
    RenderPassColorAttachmentDescriptorRef,
};
use objc::{self, msg_send, sel, sel_impl};
use parking_lot::Mutex;

use crate::quartzcore_time::ca_current_media_time;

use std::{cell::Cell, ffi::c_void, mem, ptr, sync::Arc};

// Exported to metal
pub(crate) type PointF = gpui::Point<f32>;

#[cfg(not(feature = "runtime_shaders"))]
const SHADERS_METALLIB: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shaders.metallib"));
#[cfg(feature = "runtime_shaders")]
const SHADERS_SOURCE_FILE: &str = include_str!(concat!(env!("OUT_DIR"), "/stitched_shaders.metal"));
// Use 4x MSAA, all devices support it.
// https://developer.apple.com/documentation/metal/mtldevice/1433355-supportstexturesamplecount
const PATH_SAMPLE_COUNT: u32 = 4;

pub(crate) type Context = Arc<Mutex<InstanceBufferPool>>;
pub(crate) type Renderer = MetalRenderer;

pub(crate) unsafe fn new_renderer(
    context: self::Context,
    _native_window: *mut c_void,
    _native_view: *mut c_void,
    _bounds: gpui::Size<f32>,
    transparent: bool,
) -> Renderer {
    MetalRenderer::new(context, transparent)
}

pub(crate) struct InstanceBufferPool {
    buffer_size: usize,
    buffers: Vec<metal::Buffer>,
}

impl Default for InstanceBufferPool {
    fn default() -> Self {
        Self {
            buffer_size: 2 * 1024 * 1024,
            buffers: Vec::new(),
        }
    }
}

pub(crate) struct InstanceBuffer {
    metal_buffer: metal::Buffer,
    size: usize,
}

impl InstanceBufferPool {
    pub(crate) fn reset(&mut self, buffer_size: usize) {
        self.buffer_size = buffer_size;
        self.buffers.clear();
    }

    pub(crate) fn acquire(
        &mut self,
        device: &metal::Device,
        unified_memory: bool,
    ) -> InstanceBuffer {
        let buffer = self.buffers.pop().unwrap_or_else(|| {
            let options = if unified_memory {
                MTLResourceOptions::StorageModeShared
                    // Buffers are write only which can benefit from the combined cache
                    // https://developer.apple.com/documentation/metal/mtlresourceoptions/cpucachemodewritecombined
                    | MTLResourceOptions::CPUCacheModeWriteCombined
            } else {
                MTLResourceOptions::StorageModeManaged
            };

            device.new_buffer(self.buffer_size as u64, options)
        });
        InstanceBuffer {
            metal_buffer: buffer,
            size: self.buffer_size,
        }
    }

    pub(crate) fn release(&mut self, buffer: InstanceBuffer) {
        if buffer.size == self.buffer_size {
            self.buffers.push(buffer.metal_buffer)
        }
    }
}

pub(crate) struct MetalRenderer {
    device: metal::Device,
    layer: Option<metal::MetalLayer>,
    is_apple_gpu: bool,
    is_unified_memory: bool,
    presents_with_transaction: bool,
    /// For headless rendering, tracks whether output should be opaque
    opaque: bool,
    command_queue: CommandQueue,
    paths_rasterization_pipeline_state: metal::RenderPipelineState,
    path_sprites_pipeline_state: metal::RenderPipelineState,
    group_sprites_pipeline_state: metal::RenderPipelineState,
    shadows_pipeline_state: metal::RenderPipelineState,
    quads_pipeline_state: metal::RenderPipelineState,
    underlines_pipeline_state: metal::RenderPipelineState,
    monochrome_sprites_pipeline_state: metal::RenderPipelineState,
    polychrome_sprites_pipeline_state: metal::RenderPipelineState,
    surfaces_pipeline_state: metal::RenderPipelineState,
    bgra_surfaces_pipeline_state: metal::RenderPipelineState,
    unit_vertices: metal::Buffer,
    #[allow(clippy::arc_with_non_send_sync)]
    instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>,
    sprite_atlas: Arc<MetalAtlas>,
    core_video_texture_cache: core_video::metal_texture_cache::CVMetalTextureCache,
    path_intermediate_texture: Option<metal::Texture>,
    path_intermediate_msaa_texture: Option<metal::Texture>,
    path_sample_count: u32,
    /// Measured render-group resource counts from the most recent scene render.
    last_render_group_counters: Option<RenderGroupBackendCounters>,
}

#[repr(C)]
pub struct PathRasterizationVertex {
    pub xy_position: Point<ScaledPixels>,
    pub st_position: Point<f32>,
    pub color: Background,
    pub bounds: Bounds<ScaledPixels>,
}

impl MetalRenderer {
    /// Creates a new MetalRenderer with a CAMetalLayer for window-based rendering.
    pub fn new(instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>, transparent: bool) -> Self {
        let device = Self::create_device();

        let layer = metal::MetalLayer::new();
        layer.set_device(&device);
        layer.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        // Support direct-to-display rendering if the window is not transparent
        // https://developer.apple.com/documentation/metal/managing-your-game-window-for-metal-in-macos
        layer.set_opaque(!transparent);
        layer.set_maximum_drawable_count(3);
        // Allow texture reading for visual tests (captures screenshots without ScreenCaptureKit)
        #[cfg(any(test, feature = "test-support"))]
        layer.set_framebuffer_only(false);
        unsafe {
            let _: () = msg_send![&*layer, setAllowsNextDrawableTimeout: NO];
            let _: () = msg_send![&*layer, setNeedsDisplayOnBoundsChange: YES];
            let _: () = msg_send![
                &*layer,
                setAutoresizingMask: AutoresizingMask::WIDTH_SIZABLE
                    | AutoresizingMask::HEIGHT_SIZABLE
            ];
        }

        Self::new_internal(device, Some(layer), !transparent, instance_buffer_pool)
    }

    fn create_device() -> metal::Device {
        // Prefer low‐power integrated GPUs on Intel Mac. On Apple
        // Silicon, there is only ever one GPU, so this is equivalent to
        // `metal::Device::system_default()`.
        if let Some(d) = metal::Device::all()
            .into_iter()
            .min_by_key(|d| (d.is_removable(), !d.is_low_power()))
        {
            d
        } else {
            // For some reason `all()` can return an empty list, see https://github.com/zed-industries/zed/issues/37689
            // In that case, we fall back to the system default device.
            log::error!(
                "Unable to enumerate Metal devices; attempting to use system default device"
            );
            metal::Device::system_default().unwrap_or_else(|| {
                log::error!("unable to access a compatible graphics device");
                std::process::exit(1);
            })
        }
    }

    fn new_internal(
        device: metal::Device,
        layer: Option<metal::MetalLayer>,
        opaque: bool,
        instance_buffer_pool: Arc<Mutex<InstanceBufferPool>>,
    ) -> Self {
        #[cfg(feature = "runtime_shaders")]
        let library = device
            .new_library_with_source(&SHADERS_SOURCE_FILE, &metal::CompileOptions::new())
            .expect("error building metal library");
        #[cfg(not(feature = "runtime_shaders"))]
        let library = device
            .new_library_with_data(SHADERS_METALLIB)
            .expect("error building metal library");

        fn to_float2_bits(point: PointF) -> u64 {
            let mut output = point.y.to_bits() as u64;
            output <<= 32;
            output |= point.x.to_bits() as u64;
            output
        }

        // Shared memory can be used only if CPU and GPU share the same memory space.
        // https://developer.apple.com/documentation/metal/setting-resource-storage-modes
        let is_unified_memory = device.has_unified_memory();
        // Apple GPU families support memoryless textures, which can significantly reduce
        // memory usage by keeping render targets in on-chip tile memory instead of
        // allocating backing store in system memory.
        // https://developer.apple.com/documentation/metal/mtlgpufamily
        let is_apple_gpu = device.supports_family(MTLGPUFamily::Apple1);

        let unit_vertices = [
            to_float2_bits(point(0., 0.)),
            to_float2_bits(point(1., 0.)),
            to_float2_bits(point(0., 1.)),
            to_float2_bits(point(0., 1.)),
            to_float2_bits(point(1., 0.)),
            to_float2_bits(point(1., 1.)),
        ];
        let unit_vertices = device.new_buffer_with_data(
            unit_vertices.as_ptr() as *const c_void,
            mem::size_of_val(&unit_vertices) as u64,
            if is_unified_memory {
                MTLResourceOptions::StorageModeShared
                    | MTLResourceOptions::CPUCacheModeWriteCombined
            } else {
                MTLResourceOptions::StorageModeManaged
            },
        );

        let paths_rasterization_pipeline_state = build_path_rasterization_pipeline_state(
            &device,
            &library,
            "paths_rasterization",
            "path_rasterization_vertex",
            "path_rasterization_fragment",
            MTLPixelFormat::BGRA8Unorm,
            PATH_SAMPLE_COUNT,
        );
        let path_sprites_pipeline_state = build_path_sprite_pipeline_state(
            &device,
            &library,
            "path_sprites",
            "path_sprite_vertex",
            "path_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let group_sprites_pipeline_state = build_path_sprite_pipeline_state(
            &device,
            &library,
            "group_sprites",
            "group_sprite_vertex",
            "group_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let shadows_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "shadows",
            "shadow_vertex",
            "shadow_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let quads_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "quads",
            "quad_vertex",
            "quad_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let underlines_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "underlines",
            "underline_vertex",
            "underline_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let monochrome_sprites_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "monochrome_sprites",
            "monochrome_sprite_vertex",
            "monochrome_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let polychrome_sprites_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "polychrome_sprites",
            "polychrome_sprite_vertex",
            "polychrome_sprite_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let surfaces_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "surfaces",
            "surface_vertex",
            "surface_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );
        let bgra_surfaces_pipeline_state = build_pipeline_state(
            &device,
            &library,
            "bgra_surfaces",
            "surface_vertex",
            "surface_bgra_fragment",
            MTLPixelFormat::BGRA8Unorm,
        );

        let command_queue = device.new_command_queue();
        let sprite_atlas = Arc::new(MetalAtlas::new(device.clone(), is_apple_gpu));
        let core_video_texture_cache =
            CVMetalTextureCache::new(None, device.clone(), None).unwrap();

        Self {
            device,
            layer,
            presents_with_transaction: false,
            is_apple_gpu,
            is_unified_memory,
            opaque,
            command_queue,
            paths_rasterization_pipeline_state,
            path_sprites_pipeline_state,
            group_sprites_pipeline_state,
            shadows_pipeline_state,
            quads_pipeline_state,
            underlines_pipeline_state,
            monochrome_sprites_pipeline_state,
            polychrome_sprites_pipeline_state,
            surfaces_pipeline_state,
            bgra_surfaces_pipeline_state,
            unit_vertices,
            instance_buffer_pool,
            sprite_atlas,
            core_video_texture_cache,
            last_render_group_counters: None,
            path_intermediate_texture: None,
            path_intermediate_msaa_texture: None,
            path_sample_count: PATH_SAMPLE_COUNT,
        }
    }

    pub fn layer(&self) -> Option<&metal::MetalLayerRef> {
        self.layer.as_ref().map(|l| l.as_ref())
    }

    pub fn layer_ptr(&self) -> *mut CAMetalLayer {
        self.layer
            .as_ref()
            .map(|l| l.as_ptr())
            .unwrap_or(ptr::null_mut())
    }

    pub fn sprite_atlas(&self) -> &Arc<MetalAtlas> {
        &self.sprite_atlas
    }

    pub fn set_presents_with_transaction(&mut self, presents_with_transaction: bool) {
        self.presents_with_transaction = presents_with_transaction;
        if let Some(layer) = &self.layer {
            layer.set_presents_with_transaction(presents_with_transaction);
        }
    }

    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        if let Some(layer) = &self.layer {
            let ns_size = NSSize {
                width: size.width.0 as f64,
                height: size.height.0 as f64,
            };
            unsafe {
                let _: () = msg_send![
                    layer.as_ref(),
                    setDrawableSize: ns_size
                ];
            }
        }
        self.update_path_intermediate_textures(size);
    }

    fn update_path_intermediate_textures(&mut self, size: Size<DevicePixels>) {
        // We are uncertain when this happens, but sometimes size can be 0 here. Most likely before
        // the layout pass on window creation. Zero-sized texture creation causes SIGABRT.
        // https://github.com/zed-industries/zed/issues/36229
        if size.width.0 <= 0 || size.height.0 <= 0 {
            self.path_intermediate_texture = None;
            self.path_intermediate_msaa_texture = None;
            return;
        }

        let texture_descriptor = metal::TextureDescriptor::new();
        texture_descriptor.set_width(size.width.0 as u64);
        texture_descriptor.set_height(size.height.0 as u64);
        texture_descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        texture_descriptor.set_storage_mode(metal::MTLStorageMode::Private);
        texture_descriptor
            .set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        self.path_intermediate_texture = Some(self.device.new_texture(&texture_descriptor));

        if self.path_sample_count > 1 {
            // https://developer.apple.com/documentation/metal/choosing-a-resource-storage-mode-for-apple-gpus
            // Rendering MSAA textures are done in a single pass, so we can use memory-less storage on Apple Silicon
            let storage_mode = if self.is_apple_gpu {
                metal::MTLStorageMode::Memoryless
            } else {
                metal::MTLStorageMode::Private
            };

            let msaa_descriptor = texture_descriptor;
            msaa_descriptor.set_texture_type(metal::MTLTextureType::D2Multisample);
            msaa_descriptor.set_storage_mode(storage_mode);
            msaa_descriptor.set_sample_count(self.path_sample_count as _);
            self.path_intermediate_msaa_texture = Some(self.device.new_texture(&msaa_descriptor));
        } else {
            self.path_intermediate_msaa_texture = None;
        }
    }

    pub fn update_transparency(&mut self, transparent: bool) {
        self.opaque = !transparent;
        if let Some(layer) = &self.layer {
            layer.set_opaque(!transparent);
        }
    }

    pub fn destroy(&self) {
        // nothing to do
    }

    pub fn draw(&mut self, scene: &Scene) {
        let metal_draw_id = nobie_platform_trace::next_metal_draw_id();
        let draw_id = nobie_platform_trace::current_draw_id();
        let present_id = nobie_platform_trace::current_present_id();
        nobie_platform_trace::trace(
            "metal_draw_start",
            format_args!(
                "metal_draw_id={} draw_id={} present_id={} scene_ops={} surfaces={}",
                metal_draw_id,
                draw_id,
                present_id,
                scene.len(),
                scene.surfaces.len()
            ),
        );
        let layer = match &self.layer {
            Some(l) => l.clone(),
            None => {
                log::error!(
                    "draw() called on headless renderer - use render_scene_to_image() instead"
                );
                return;
            }
        };
        let viewport_size = layer.drawable_size();
        let viewport_size: Size<DevicePixels> = size(
            (viewport_size.width.ceil() as i32).into(),
            (viewport_size.height.ceil() as i32).into(),
        );
        let drawable = if let Some(drawable) = layer.next_drawable() {
            nobie_platform_trace::trace(
                "metal_next_drawable",
                format_args!(
                    "metal_draw_id={} draw_id={} present_id={} drawable_id={}",
                    metal_draw_id,
                    draw_id,
                    present_id,
                    drawable.drawable_id()
                ),
            );
            drawable
        } else {
            log::error!(
                "failed to retrieve next drawable, drawable size: {:?}",
                viewport_size
            );
            nobie_platform_trace::trace(
                "metal_next_drawable_failed",
                format_args!(
                    "metal_draw_id={} draw_id={} present_id={} viewport_width={} viewport_height={}",
                    metal_draw_id,
                    draw_id,
                    present_id,
                    viewport_size.width.0,
                    viewport_size.height.0
                ),
            );
            return;
        };

        loop {
            let mut instance_buffer = self
                .instance_buffer_pool
                .lock()
                .acquire(&self.device, self.is_unified_memory);

            let command_buffer =
                self.draw_primitives(scene, &mut instance_buffer, drawable, viewport_size);

            match command_buffer {
                Ok(command_buffer) => {
                    let instance_buffer_pool = self.instance_buffer_pool.clone();
                    let instance_buffer = Cell::new(Some(instance_buffer));
                    let completed_metal_draw_id = metal_draw_id;
                    let completed_draw_id = draw_id;
                    let completed_present_id = present_id;
                    let block = ConcreteBlock::new(move |_| {
                        if let Some(instance_buffer) = instance_buffer.take() {
                            instance_buffer_pool.lock().release(instance_buffer);
                        }
                        nobie_platform_trace::trace(
                            "metal_command_buffer_completed",
                            format_args!(
                                "metal_draw_id={} draw_id={} present_id={} callback_ca_time={:.9}",
                                completed_metal_draw_id,
                                completed_draw_id,
                                completed_present_id,
                                ca_current_media_time()
                            ),
                        );
                    });
                    let block = block.copy();
                    command_buffer.add_completed_handler(&block);

                    if nobie_platform_trace::enabled() {
                        let presented_metal_draw_id = metal_draw_id;
                        let presented_draw_id = draw_id;
                        let presented_present_id = present_id;
                        let presented_block = ConcreteBlock::new(
                            move |drawable: &metal::DrawableRef| {
                                let presented_time = drawable.presented_time();
                                let callback_ca_time = ca_current_media_time();
                                nobie_platform_trace::trace(
                                    "metal_drawable_presented",
                                    format_args!(
                                        "metal_draw_id={} draw_id={} present_id={} drawable_id={} presented_time={:.9} callback_ca_time={:.9}",
                                        presented_metal_draw_id,
                                        presented_draw_id,
                                        presented_present_id,
                                        drawable.drawable_id(),
                                        presented_time,
                                        callback_ca_time
                                    ),
                                );
                            },
                        );
                        let presented_block = presented_block.copy();
                        drawable.add_presented_handler(&presented_block);
                    }

                    if self.presents_with_transaction {
                        nobie_platform_trace::trace(
                            "metal_command_buffer_commit",
                            format_args!(
                                "metal_draw_id={} draw_id={} present_id={} present_mode=transaction drawable_id={} ca_time={:.9}",
                                metal_draw_id,
                                draw_id,
                                present_id,
                                drawable.drawable_id(),
                                ca_current_media_time()
                            ),
                        );
                        command_buffer.commit();
                        command_buffer.wait_until_scheduled();
                        nobie_platform_trace::trace(
                            "metal_drawable_present_schedule",
                            format_args!(
                                "metal_draw_id={} draw_id={} present_id={} present_mode=transaction drawable_id={} ca_time={:.9}",
                                metal_draw_id,
                                draw_id,
                                present_id,
                                drawable.drawable_id(),
                                ca_current_media_time()
                            ),
                        );
                        drawable.present();
                    } else {
                        nobie_platform_trace::trace(
                            "metal_drawable_present_schedule",
                            format_args!(
                                "metal_draw_id={} draw_id={} present_id={} present_mode=command_buffer drawable_id={} ca_time={:.9}",
                                metal_draw_id,
                                draw_id,
                                present_id,
                                drawable.drawable_id(),
                                ca_current_media_time()
                            ),
                        );
                        command_buffer.present_drawable(drawable);
                        nobie_platform_trace::trace(
                            "metal_command_buffer_commit",
                            format_args!(
                                "metal_draw_id={} draw_id={} present_id={} present_mode=command_buffer drawable_id={} ca_time={:.9}",
                                metal_draw_id,
                                draw_id,
                                present_id,
                                drawable.drawable_id(),
                                ca_current_media_time()
                            ),
                        );
                        command_buffer.commit();
                    }
                    nobie_platform_trace::trace(
                        "metal_draw_finish",
                        format_args!(
                            "metal_draw_id={} draw_id={} present_id={} drawable_id={}",
                            metal_draw_id,
                            draw_id,
                            present_id,
                            drawable.drawable_id()
                        ),
                    );
                    return;
                }
                Err(err) => {
                    log::error!(
                        "failed to render: {}. retrying with larger instance buffer size",
                        err
                    );
                    nobie_platform_trace::trace(
                        "metal_draw_retry",
                        format_args!(
                            "metal_draw_id={} draw_id={} present_id={} error={:?}",
                            metal_draw_id, draw_id, present_id, err
                        ),
                    );
                    let mut instance_buffer_pool = self.instance_buffer_pool.lock();
                    let buffer_size = instance_buffer_pool.buffer_size;
                    if buffer_size >= 256 * 1024 * 1024 {
                        log::error!("instance buffer size grew too large: {}", buffer_size);
                        break;
                    }
                    instance_buffer_pool.reset(buffer_size * 2);
                    log::info!(
                        "increased instance buffer size to {}",
                        instance_buffer_pool.buffer_size
                    );
                }
            }
        }
    }

    /// Renders the scene to a texture and returns the pixel data as an RGBA image.
    /// This does not present the frame to screen - useful for visual testing
    /// where we want to capture what would be rendered without displaying it.
    ///
    /// Note: This requires a layer-backed renderer. For headless rendering,
    /// use `render_scene_to_image()` instead.
    #[cfg(any(test, feature = "test-support"))]
    pub fn render_to_image(&mut self, scene: &Scene) -> Result<RgbaImage> {
        let layer = self
            .layer
            .clone()
            .ok_or_else(|| anyhow::anyhow!("render_to_image requires a layer-backed renderer"))?;
        let viewport_size = layer.drawable_size();
        let viewport_size: Size<DevicePixels> = size(
            (viewport_size.width.ceil() as i32).into(),
            (viewport_size.height.ceil() as i32).into(),
        );
        let drawable = layer
            .next_drawable()
            .ok_or_else(|| anyhow::anyhow!("Failed to get drawable for render_to_image"))?;

        loop {
            let mut instance_buffer = self
                .instance_buffer_pool
                .lock()
                .acquire(&self.device, self.is_unified_memory);

            let command_buffer =
                self.draw_primitives(scene, &mut instance_buffer, drawable, viewport_size);

            match command_buffer {
                Ok(command_buffer) => {
                    let instance_buffer_pool = self.instance_buffer_pool.clone();
                    let instance_buffer = Cell::new(Some(instance_buffer));
                    let block = ConcreteBlock::new(move |_| {
                        if let Some(instance_buffer) = instance_buffer.take() {
                            instance_buffer_pool.lock().release(instance_buffer);
                        }
                    });
                    let block = block.copy();
                    command_buffer.add_completed_handler(&block);

                    // Commit and wait for completion without presenting
                    command_buffer.commit();
                    command_buffer.wait_until_completed();

                    // Read pixels from the texture
                    let texture = drawable.texture();
                    let width = texture.width() as u32;
                    let height = texture.height() as u32;
                    let bytes_per_row = width as usize * 4;
                    let buffer_size = height as usize * bytes_per_row;

                    let mut pixels = vec![0u8; buffer_size];

                    let region = metal::MTLRegion {
                        origin: metal::MTLOrigin { x: 0, y: 0, z: 0 },
                        size: metal::MTLSize {
                            width: width as u64,
                            height: height as u64,
                            depth: 1,
                        },
                    };

                    texture.get_bytes(
                        pixels.as_mut_ptr() as *mut std::ffi::c_void,
                        bytes_per_row as u64,
                        region,
                        0,
                    );

                    // Convert BGRA to RGBA (swap B and R channels)
                    for chunk in pixels.chunks_exact_mut(4) {
                        chunk.swap(0, 2);
                    }

                    return RgbaImage::from_raw(width, height, pixels).ok_or_else(|| {
                        anyhow::anyhow!("Failed to create RgbaImage from pixel data")
                    });
                }
                Err(err) => {
                    log::error!(
                        "failed to render: {}. retrying with larger instance buffer size",
                        err
                    );
                    let mut instance_buffer_pool = self.instance_buffer_pool.lock();
                    let buffer_size = instance_buffer_pool.buffer_size;
                    if buffer_size >= 256 * 1024 * 1024 {
                        anyhow::bail!("instance buffer size grew too large: {}", buffer_size);
                    }
                    instance_buffer_pool.reset(buffer_size * 2);
                    log::info!(
                        "increased instance buffer size to {}",
                        instance_buffer_pool.buffer_size
                    );
                }
            }
        }
    }

    /// Renders the full scene to CPU-readable RGBA bytes for deterministic automation.
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture_scene(&mut self, scene: &Scene) -> Result<SceneCapture> {
        let layer = self
            .layer
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("capture_scene requires a layer-backed renderer"))?;
        let drawable_size = layer.drawable_size();
        let size: Size<DevicePixels> = size(
            (drawable_size.width.ceil() as i32).into(),
            (drawable_size.height.ceil() as i32).into(),
        );
        let image = self.render_scene_to_image(scene, size)?;
        let width_px = image.width();
        let height_px = image.height();
        Ok(SceneCapture {
            rgba: image.into_raw(),
            width_px,
            height_px,
            backend: SceneCaptureBackend::Metal,
        })
    }

    /// Renders a scene to an image without requiring a window or CAMetalLayer.
    ///
    /// This is the primary method for headless rendering. It creates an offscreen
    /// texture, renders the scene to it, and returns the pixel data as an RGBA image.
    #[cfg(any(test, feature = "test-support"))]
    pub fn render_scene_to_image(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> Result<RgbaImage> {
        if size.width.0 <= 0 || size.height.0 <= 0 {
            anyhow::bail!("Invalid size for render_scene_to_image: {:?}", size);
        }

        // Update path intermediate textures for this size
        self.update_path_intermediate_textures(size);

        // Create an offscreen texture as render target
        let texture_descriptor = metal::TextureDescriptor::new();
        texture_descriptor.set_width(size.width.0 as u64);
        texture_descriptor.set_height(size.height.0 as u64);
        texture_descriptor.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        texture_descriptor
            .set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        texture_descriptor.set_storage_mode(metal::MTLStorageMode::Managed);
        let target_texture = self.device.new_texture(&texture_descriptor);

        loop {
            let mut instance_buffer = self
                .instance_buffer_pool
                .lock()
                .acquire(&self.device, self.is_unified_memory);

            let command_buffer =
                self.draw_primitives_to_texture(scene, &mut instance_buffer, &target_texture, size);

            match command_buffer {
                Ok(command_buffer) => {
                    let instance_buffer_pool = self.instance_buffer_pool.clone();
                    let instance_buffer = Cell::new(Some(instance_buffer));
                    let block = ConcreteBlock::new(move |_| {
                        if let Some(instance_buffer) = instance_buffer.take() {
                            instance_buffer_pool.lock().release(instance_buffer);
                        }
                    });
                    let block = block.copy();
                    command_buffer.add_completed_handler(&block);

                    // On discrete GPUs (non-unified memory), Managed textures
                    // require an explicit blit synchronize before the CPU can
                    // read back the rendered data. Without this, get_bytes
                    // returns stale zeros.
                    if !self.is_unified_memory {
                        let blit = command_buffer.new_blit_command_encoder();
                        blit.synchronize_resource(&target_texture);
                        blit.end_encoding();
                    }

                    // Commit and wait for completion
                    command_buffer.commit();
                    command_buffer.wait_until_completed();

                    // Read pixels from the texture
                    let width = size.width.0 as u32;
                    let height = size.height.0 as u32;
                    let bytes_per_row = width as usize * 4;
                    let buffer_size = height as usize * bytes_per_row;

                    let mut pixels = vec![0u8; buffer_size];

                    let region = metal::MTLRegion {
                        origin: metal::MTLOrigin { x: 0, y: 0, z: 0 },
                        size: metal::MTLSize {
                            width: width as u64,
                            height: height as u64,
                            depth: 1,
                        },
                    };

                    target_texture.get_bytes(
                        pixels.as_mut_ptr() as *mut std::ffi::c_void,
                        bytes_per_row as u64,
                        region,
                        0,
                    );

                    // Convert BGRA to RGBA (swap B and R channels)
                    for chunk in pixels.chunks_exact_mut(4) {
                        chunk.swap(0, 2);
                    }

                    return RgbaImage::from_raw(width, height, pixels).ok_or_else(|| {
                        anyhow::anyhow!("Failed to create RgbaImage from pixel data")
                    });
                }
                Err(err) => {
                    log::error!(
                        "failed to render: {}. retrying with larger instance buffer size",
                        err
                    );
                    let mut instance_buffer_pool = self.instance_buffer_pool.lock();
                    let buffer_size = instance_buffer_pool.buffer_size;
                    if buffer_size >= 256 * 1024 * 1024 {
                        anyhow::bail!("instance buffer size grew too large: {}", buffer_size);
                    }
                    instance_buffer_pool.reset(buffer_size * 2);
                    log::info!(
                        "increased instance buffer size to {}",
                        instance_buffer_pool.buffer_size
                    );
                }
            }
        }
    }

    fn draw_primitives(
        &mut self,
        scene: &Scene,
        instance_buffer: &mut InstanceBuffer,
        drawable: &metal::MetalDrawableRef,
        viewport_size: Size<DevicePixels>,
    ) -> Result<metal::CommandBuffer> {
        if scene.requires_backdrop_effects() {
            let Some(root_texture) = self.new_group_intermediate_texture(viewport_size) else {
                anyhow::bail!("invalid viewport for backdrop render group: {viewport_size:?}");
            };
            let command_queue = self.command_queue.clone();
            let command_buffer = command_queue.new_command_buffer();
            let alpha = if self.opaque { 1. } else { 0. };
            let mut instance_offset = 0;

            self.last_render_group_counters = Some(RenderGroupBackendCounters::default());
            let ok = self.encode_primitives_to_texture(
                scene,
                instance_buffer,
                &mut instance_offset,
                &root_texture,
                viewport_size,
                command_buffer,
                Some(alpha),
            ) && self.draw_texture_to_target(
                &root_texture,
                instance_buffer,
                &mut instance_offset,
                drawable.texture(),
                viewport_size,
                command_buffer,
                Some(alpha),
            );

            if !ok {
                anyhow::bail!(
                    "scene too large: {} paths, {} shadows, {} quads, {} underlines, {} mono, {} poly, {} surfaces, {} groups",
                    scene.paths.len(),
                    scene.shadows.len(),
                    scene.quads.len(),
                    scene.underlines.len(),
                    scene.monochrome_sprites.len(),
                    scene.polychrome_sprites.len(),
                    scene.surfaces.len(),
                    scene.groups.len(),
                );
            }

            if !self.is_unified_memory {
                instance_buffer.metal_buffer.did_modify_range(NSRange {
                    location: 0,
                    length: instance_offset as NSUInteger,
                });
            }

            return Ok(command_buffer.to_owned());
        }

        self.draw_primitives_to_texture(scene, instance_buffer, drawable.texture(), viewport_size)
    }

    fn draw_primitives_to_texture(
        &mut self,
        scene: &Scene,
        instance_buffer: &mut InstanceBuffer,
        texture: &metal::TextureRef,
        viewport_size: Size<DevicePixels>,
    ) -> Result<metal::CommandBuffer> {
        let command_queue = self.command_queue.clone();
        let command_buffer = command_queue.new_command_buffer();
        let alpha = if self.opaque { 1. } else { 0. };
        let mut instance_offset = 0;

        self.last_render_group_counters = Some(RenderGroupBackendCounters::default());
        let ok = self.encode_primitives_to_texture(
            scene,
            instance_buffer,
            &mut instance_offset,
            texture,
            viewport_size,
            command_buffer,
            Some(alpha),
        );

        if !ok {
            anyhow::bail!(
                "scene too large: {} paths, {} shadows, {} quads, {} underlines, {} mono, {} poly, {} surfaces, {} groups",
                scene.paths.len(),
                scene.shadows.len(),
                scene.quads.len(),
                scene.underlines.len(),
                scene.monochrome_sprites.len(),
                scene.polychrome_sprites.len(),
                scene.surfaces.len(),
                scene.groups.len(),
            );
        }

        if !self.is_unified_memory {
            // Sync the instance buffer to the GPU
            instance_buffer.metal_buffer.did_modify_range(NSRange {
                location: 0,
                length: instance_offset as NSUInteger,
            });
        }

        Ok(command_buffer.to_owned())
    }

    fn encode_primitives_to_texture(
        &mut self,
        scene: &Scene,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        texture: &metal::TextureRef,
        viewport_size: Size<DevicePixels>,
        command_buffer: &metal::CommandBufferRef,
        clear_alpha: Option<f64>,
    ) -> bool {
        let mut command_encoder = new_command_encoder_for_texture(
            command_buffer,
            texture,
            viewport_size,
            |color_attachment| {
                if let Some(alpha) = clear_alpha {
                    color_attachment.set_load_action(metal::MTLLoadAction::Clear);
                    color_attachment.set_clear_color(metal::MTLClearColor::new(0., 0., 0., alpha));
                } else {
                    color_attachment.set_load_action(metal::MTLLoadAction::Load);
                }
            },
        );

        for batch in scene.batches() {
            let ok = match batch {
                PrimitiveBatch::Shadows(range) => self.draw_shadows(
                    &scene.shadows[range],
                    instance_buffer,
                    instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::Quads(range) => self.draw_quads(
                    &scene.quads[range],
                    instance_buffer,
                    instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::Paths(range) => {
                    let paths = &scene.paths[range];
                    command_encoder.end_encoding();

                    let did_draw = self.draw_paths_to_intermediate(
                        paths,
                        instance_buffer,
                        instance_offset,
                        viewport_size,
                        command_buffer,
                    );

                    command_encoder = new_command_encoder_for_texture(
                        command_buffer,
                        texture,
                        viewport_size,
                        |color_attachment| {
                            color_attachment.set_load_action(metal::MTLLoadAction::Load);
                        },
                    );

                    if did_draw {
                        self.draw_paths_from_intermediate(
                            paths,
                            instance_buffer,
                            instance_offset,
                            viewport_size,
                            command_encoder,
                        )
                    } else {
                        false
                    }
                }
                PrimitiveBatch::Underlines(range) => self.draw_underlines(
                    &scene.underlines[range],
                    instance_buffer,
                    instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::MonochromeSprites { texture_id, range } => self
                    .draw_monochrome_sprites(
                        texture_id,
                        &scene.monochrome_sprites[range],
                        instance_buffer,
                        instance_offset,
                        viewport_size,
                        command_encoder,
                    ),
                PrimitiveBatch::PolychromeSprites { texture_id, range } => self
                    .draw_polychrome_sprites(
                        texture_id,
                        &scene.polychrome_sprites[range],
                        instance_buffer,
                        instance_offset,
                        viewport_size,
                        command_encoder,
                    ),
                PrimitiveBatch::Surfaces(range) => self.draw_surfaces(
                    &scene.surfaces[range],
                    instance_buffer,
                    instance_offset,
                    viewport_size,
                    command_encoder,
                ),
                PrimitiveBatch::Groups(range) => {
                    command_encoder.end_encoding();
                    let did_draw = self.draw_groups(
                        &scene.groups[range],
                        instance_buffer,
                        instance_offset,
                        texture,
                        viewport_size,
                        command_buffer,
                    );

                    command_encoder = new_command_encoder_for_texture(
                        command_buffer,
                        texture,
                        viewport_size,
                        |color_attachment| {
                            color_attachment.set_load_action(metal::MTLLoadAction::Load);
                        },
                    );

                    did_draw
                }
                PrimitiveBatch::SubpixelSprites { .. } => unreachable!(),
                unknown => panic!("unsupported GPUI primitive batch: {unknown:?}"),
            };
            if !ok {
                command_encoder.end_encoding();
                return false;
            }
        }

        command_encoder.end_encoding();
        true
    }

    fn draw_texture_to_target(
        &self,
        source_texture: &metal::TextureRef,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        target_texture: &metal::TextureRef,
        viewport_size: Size<DevicePixels>,
        command_buffer: &metal::CommandBufferRef,
        clear_alpha: Option<f64>,
    ) -> bool {
        let command_encoder = new_command_encoder_for_texture(
            command_buffer,
            target_texture,
            viewport_size,
            |color_attachment| {
                if let Some(alpha) = clear_alpha {
                    color_attachment.set_load_action(metal::MTLLoadAction::Clear);
                    color_attachment.set_clear_color(metal::MTLClearColor::new(0., 0., 0., alpha));
                } else {
                    color_attachment.set_load_action(metal::MTLLoadAction::Load);
                }
            },
        );
        command_encoder.set_render_pipeline_state(&self.path_sprites_pipeline_state);
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder
            .set_fragment_texture(SpriteInputIndex::AtlasTexture as u64, Some(source_texture));

        let sprite = PathSprite {
            bounds: Bounds::new(
                point(ScaledPixels(0.), ScaledPixels(0.)),
                size(
                    ScaledPixels(viewport_size.width.0 as f32),
                    ScaledPixels(viewport_size.height.0 as f32),
                ),
            ),
        };
        align_offset(instance_offset);
        let sprite_bytes_len = mem::size_of::<PathSprite>();
        let next_offset = *instance_offset + sprite_bytes_len;
        if next_offset > instance_buffer.size {
            command_encoder.end_encoding();
            return false;
        }

        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };
        unsafe {
            ptr::copy_nonoverlapping(
                &sprite as *const PathSprite as *const u8,
                buffer_contents,
                sprite_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(metal::MTLPrimitiveType::Triangle, 0, 6, 1);
        command_encoder.end_encoding();
        *instance_offset = next_offset;
        true
    }

    /// Measured render-group resource counts from the most recent scene render
    /// (group intermediate textures + backdrop copies), or `None` if no scene has
    /// rendered. Validated against the planner's `RenderGroupSupportCounters` in
    /// the metal render-group tests.
    pub fn render_group_backend_counters(&self) -> Option<RenderGroupBackendCounters> {
        self.last_render_group_counters
    }

    fn draw_groups(
        &mut self,
        groups: &[PaintGroup],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        target_texture: &metal::TextureRef,
        viewport_size: Size<DevicePixels>,
        command_buffer: &metal::CommandBufferRef,
    ) -> bool {
        for group in groups {
            let effect_plan = group.plan.normalized_effects().clone();
            if effect_plan.opacity() <= 0. {
                continue;
            }

            let Some(group_texture) = self.new_group_intermediate_texture(viewport_size) else {
                return false;
            };
            if let Some(counters) = self.last_render_group_counters.as_mut() {
                counters.intermediate_textures += 1;
            }

            if !self.encode_primitives_to_texture(
                group.scene.as_ref(),
                instance_buffer,
                instance_offset,
                &group_texture,
                viewport_size,
                command_buffer,
                Some(0.),
            ) {
                return false;
            }

            let backdrop_texture;
            let backdrop_texture_ref = if effect_plan.reads_backdrop() {
                let Some(texture) =
                    self.copy_backdrop_texture(target_texture, viewport_size, command_buffer)
                else {
                    return false;
                };
                backdrop_texture = texture;
                if let Some(counters) = self.last_render_group_counters.as_mut() {
                    counters.intermediate_textures += 1;
                    counters.backdrop_copies += 1;
                }
                &backdrop_texture
            } else {
                &group_texture
            };

            // Arbitrary-path source mask pre-pass: rasterize the group's path into
            // a coverage mask in the same screen/device space as all group
            // geometry, parallel to the backdrop copy. For groups without a path
            // mask, bind `&group_texture` (a real allocated texture); the
            // `source_mask_mode` flag gates the sample so the bound-but-unread
            // texture is harmless. The mask is a path-rasterization intermediate,
            // which the planner excludes from per-group counts, so it is NOT
            // counted here.
            let source_mask_texture;
            let source_mask_texture_ref = if let Some(path) = effect_plan.source_mask_path() {
                let Some(texture) = self.rasterize_source_mask_path(
                    path,
                    instance_buffer,
                    instance_offset,
                    viewport_size,
                    command_buffer,
                ) else {
                    return false;
                };
                source_mask_texture = texture;
                &source_mask_texture
            } else {
                &group_texture
            };

            let command_encoder = new_command_encoder_for_texture(
                command_buffer,
                target_texture,
                viewport_size,
                |color_attachment| {
                    color_attachment.set_load_action(metal::MTLLoadAction::Load);
                },
            );
            let ok = self.draw_group_from_texture(
                group,
                effect_plan,
                &group_texture,
                backdrop_texture_ref,
                source_mask_texture_ref,
                instance_buffer,
                instance_offset,
                viewport_size,
                command_encoder,
            );
            command_encoder.end_encoding();

            if !ok {
                return false;
            }
        }

        true
    }

    fn copy_backdrop_texture(
        &self,
        target_texture: &metal::TextureRef,
        viewport_size: Size<DevicePixels>,
        command_buffer: &metal::CommandBufferRef,
    ) -> Option<metal::Texture> {
        let texture = self.new_group_intermediate_texture(viewport_size)?;
        let blit = command_buffer.new_blit_command_encoder();
        blit.copy_from_texture(
            target_texture,
            0,
            0,
            metal::MTLOrigin { x: 0, y: 0, z: 0 },
            metal::MTLSize {
                width: viewport_size.width.0 as u64,
                height: viewport_size.height.0 as u64,
                depth: 1,
            },
            &texture,
            0,
            0,
            metal::MTLOrigin { x: 0, y: 0, z: 0 },
        );
        blit.end_encoding();
        Some(texture)
    }

    fn new_group_intermediate_texture(
        &self,
        viewport_size: Size<DevicePixels>,
    ) -> Option<metal::Texture> {
        if viewport_size.width.0 <= 0 || viewport_size.height.0 <= 0 {
            return None;
        }

        let texture_descriptor = metal::TextureDescriptor::new();
        texture_descriptor.set_width(viewport_size.width.0 as u64);
        texture_descriptor.set_height(viewport_size.height.0 as u64);
        texture_descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
        texture_descriptor.set_storage_mode(metal::MTLStorageMode::Private);
        texture_descriptor
            .set_usage(metal::MTLTextureUsage::RenderTarget | metal::MTLTextureUsage::ShaderRead);
        Some(self.device.new_texture(&texture_descriptor))
    }

    fn draw_group_from_texture(
        &self,
        group: &PaintGroup,
        effect_plan: CompositeEffectPlan,
        group_texture: &metal::TextureRef,
        backdrop_texture: &metal::TextureRef,
        source_mask_texture: &metal::TextureRef,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        command_encoder.set_render_pipeline_state(&self.group_sprites_pipeline_state);
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        command_encoder
            .set_fragment_texture(SpriteInputIndex::AtlasTexture as u64, Some(group_texture));
        command_encoder.set_fragment_texture(
            SpriteInputIndex::BackdropTexture as u64,
            Some(backdrop_texture),
        );
        command_encoder.set_fragment_texture(
            SpriteInputIndex::SourceMaskTexture as u64,
            Some(source_mask_texture),
        );

        let (color_matrix, color_offset) = Self::group_source_color_filter(&effect_plan);
        let (source_tone_op, source_tone_param) = effect_plan
            .source_color_filter()
            .tone()
            .shader_code_and_param();
        let (backdrop_color_matrix, backdrop_color_offset) =
            Self::group_backdrop_color_filter(&effect_plan);
        let backdrop_tint = effect_plan.backdrop_tint().to_rgb();
        let (backdrop_lens, backdrop_lens_lighting) = Self::group_backdrop_lens(&effect_plan);
        let mut sprites = Vec::with_capacity(
            effect_plan.drop_shadows().len()
                + effect_plan.surface_shadows().len()
                + effect_plan.processed_content_glows().len()
                + 1,
        );
        // A path source mask supersedes the analytic SDF: enable masking and
        // select sampled mode so the shader reads the rasterized coverage texture.
        let source_mask_mode: u32 = if effect_plan.source_mask_path().is_some() {
            1
        } else {
            0
        };
        let (source_mask_enabled, source_mask_corner_radii) = match effect_plan.source_mask() {
            Some(corner_radii) => (1., corner_radii),
            None if source_mask_mode == 1 => (1., Corners::all(ScaledPixels(0.))),
            None => (0., Corners::all(ScaledPixels(0.))),
        };
        let material_shape_corner_radii = effect_plan
            .material_shape()
            .unwrap_or_else(|| Corners::all(ScaledPixels(0.)));
        let (smk, sme) = effect_plan.source_mask_shape().shader_params();
        let (mmk, mme) = effect_plan.material_shape_shape().shader_params();
        let group_shape_params = [smk as f32, sme, mmk as f32, mme];
        let db = effect_plan.source_directional_blur();
        let source_directional_blur = [db.x.0, db.y.0, 0., 0.];

        for shadow in effect_plan.drop_shadows() {
            let color = shadow.color.to_rgb();
            sprites.push(GroupSprite {
                bounds: group.capture_bounds,
                opacity: effect_plan.opacity(),
                effect_kind: 1,
                source_blur_radius: 0.,
                source_mask_enabled,
                shadow_offset: [shadow.offset.x.0, shadow.offset.y.0],
                shadow_blur_radius: shadow.blur_radius.0,
                backdrop_blur_radius: 0.,
                source_mask_blur_order: effect_plan.source_mask_blur_order().shader_code(),
                derived_luma_threshold: 0.,
                source_mask_mode,
                _pad1: 0,
                shadow_color: [color.r, color.g, color.b, color.a],
                source_mask_bounds: group.bounds,
                source_mask_corner_radii,
                material_shape_bounds: group.bounds,
                material_shape_corner_radii,
                group_shape_params,
                source_directional_blur,
                color_matrix,
                color_offset,
                backdrop_active: 0.,
                blend_mode: 0,
                source_tone_op: 0,
                source_tone_param: 0.,
                backdrop_tint: [0., 0., 0., 0.],
                backdrop_color_matrix,
                backdrop_color_offset,
                backdrop_lens: [0., 0., 0., 0.],
                backdrop_lens_lighting: [0., 0., 0., 0.],
            });
        }

        for shadow in effect_plan.surface_shadows() {
            let color = shadow.color.to_rgb();
            // The surface-shadow SDF reads the material slot; drive it from the
            // shadow's own shape so a superellipse shadow gets squircle corners.
            let (shadow_shape_kind, shadow_shape_exponent) = shadow.shape_kind.shader_params();
            let shadow_group_shape_params =
                [0., 0., shadow_shape_kind as f32, shadow_shape_exponent];
            sprites.push(GroupSprite {
                bounds: group.capture_bounds,
                opacity: effect_plan.opacity(),
                effect_kind: 2,
                source_blur_radius: 0.,
                source_mask_enabled: 0.,
                shadow_offset: [shadow.offset.x.0, shadow.offset.y.0],
                shadow_blur_radius: shadow.blur_radius.0,
                backdrop_blur_radius: 0.,
                source_mask_blur_order: effect_plan.source_mask_blur_order().shader_code(),
                derived_luma_threshold: 0.,
                source_mask_mode: 0,
                _pad1: 0,
                shadow_color: [color.r, color.g, color.b, color.a],
                source_mask_bounds: group.bounds,
                source_mask_corner_radii: Corners::all(ScaledPixels(0.)),
                material_shape_bounds: group.bounds,
                material_shape_corner_radii: shadow.shape,
                group_shape_params: shadow_group_shape_params,
                source_directional_blur,
                color_matrix,
                color_offset,
                backdrop_active: 0.,
                blend_mode: 0,
                source_tone_op: 0,
                source_tone_param: 0.,
                backdrop_tint: [0., 0., 0., 0.],
                backdrop_color_matrix,
                backdrop_color_offset,
                backdrop_lens: [0., 0., 0., 0.],
                backdrop_lens_lighting: [0., 0., 0., 0.],
            });
        }

        for glow in effect_plan.processed_content_glows() {
            let color = glow.color.to_rgb();
            sprites.push(GroupSprite {
                bounds: group.capture_bounds,
                opacity: effect_plan.opacity(),
                effect_kind: 3,
                source_blur_radius: effect_plan.source_blur_radius().0,
                source_mask_enabled,
                shadow_offset: [0., 0.],
                shadow_blur_radius: glow.blur_radius.0,
                backdrop_blur_radius: 0.,
                source_mask_blur_order: effect_plan.source_mask_blur_order().shader_code(),
                derived_luma_threshold: glow.luma_threshold,
                source_mask_mode,
                _pad1: 0,
                shadow_color: [color.r, color.g, color.b, color.a],
                source_mask_bounds: group.bounds,
                source_mask_corner_radii,
                material_shape_bounds: group.bounds,
                material_shape_corner_radii,
                group_shape_params,
                source_directional_blur,
                color_matrix,
                color_offset,
                backdrop_active: 0.,
                blend_mode: 0,
                source_tone_op: 0,
                source_tone_param: 0.,
                backdrop_tint: [0., 0., 0., 0.],
                backdrop_color_matrix,
                backdrop_color_offset,
                backdrop_lens: [0., 0., 0., 0.],
                backdrop_lens_lighting: [0., 0., 0., 0.],
            });
        }

        sprites.push(GroupSprite {
            bounds: group.capture_bounds,
            opacity: effect_plan.opacity(),
            effect_kind: 0,
            source_blur_radius: effect_plan.source_blur_radius().0,
            source_mask_enabled,
            shadow_offset: [0., 0.],
            shadow_blur_radius: 0.,
            backdrop_blur_radius: effect_plan.backdrop_blur_radius().0,
            source_mask_blur_order: effect_plan.source_mask_blur_order().shader_code(),
            derived_luma_threshold: 0.,
            source_mask_mode,
            _pad1: 0,
            shadow_color: [0., 0., 0., 0.],
            source_mask_bounds: group.bounds,
            source_mask_corner_radii,
            material_shape_bounds: group.bounds,
            material_shape_corner_radii,
            group_shape_params,
            source_directional_blur,
            color_matrix,
            color_offset,
            backdrop_active: if effect_plan.has_backdrop_material() {
                1.
            } else {
                0.
            },
            blend_mode: effect_plan.blend_mode().shader_code(),
            source_tone_op,
            source_tone_param,
            backdrop_tint: [
                backdrop_tint.r,
                backdrop_tint.g,
                backdrop_tint.b,
                backdrop_tint.a,
            ],
            backdrop_color_matrix,
            backdrop_color_offset,
            backdrop_lens,
            backdrop_lens_lighting,
        });

        align_offset(instance_offset);
        let sprite_bytes_len = mem::size_of_val(sprites.as_slice());
        let next_offset = *instance_offset + sprite_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };
        unsafe {
            ptr::copy_nonoverlapping(
                sprites.as_ptr() as *const u8,
                buffer_contents,
                sprite_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            sprites.len() as u64,
        );
        *instance_offset = next_offset;

        true
    }

    fn group_source_color_filter(effect_plan: &CompositeEffectPlan) -> ([[f32; 4]; 4], [f32; 4]) {
        let (matrix, offset) = effect_plan.source_color_filter().components();
        (
            [
                [matrix[0][0], matrix[0][1], matrix[0][2], 0.],
                [matrix[1][0], matrix[1][1], matrix[1][2], 0.],
                [matrix[2][0], matrix[2][1], matrix[2][2], 0.],
                [0., 0., 0., 1.],
            ],
            [offset[0], offset[1], offset[2], 0.],
        )
    }

    fn group_backdrop_color_filter(effect_plan: &CompositeEffectPlan) -> ([[f32; 4]; 4], [f32; 4]) {
        let (matrix, offset) = effect_plan.backdrop_color_filter().components();
        (
            [
                [matrix[0][0], matrix[0][1], matrix[0][2], 0.],
                [matrix[1][0], matrix[1][1], matrix[1][2], 0.],
                [matrix[2][0], matrix[2][1], matrix[2][2], 0.],
                [0., 0., 0., 1.],
            ],
            [offset[0], offset[1], offset[2], 0.],
        )
    }

    fn group_backdrop_lens(effect_plan: &CompositeEffectPlan) -> ([f32; 4], [f32; 4]) {
        let Some(lens) = effect_plan.backdrop_lens() else {
            return ([0., 0., 0., 0.], [0., 0., 0., 0.]);
        };
        let light_direction = lens.light_direction();
        (
            [
                lens.refraction_radius().0,
                lens.rim_width().0,
                lens.chromatic_aberration().0,
                if lens.is_identity() { 0. } else { 1. },
            ],
            [
                lens.highlight_strength(),
                lens.shadow_strength(),
                light_direction.x,
                light_direction.y,
            ],
        )
    }

    fn draw_paths_to_intermediate(
        &self,
        paths: &[Path<ScaledPixels>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_buffer: &metal::CommandBufferRef,
    ) -> bool {
        if paths.is_empty() {
            return true;
        }
        let Some(intermediate_texture) = &self.path_intermediate_texture else {
            return false;
        };

        let render_pass_descriptor = metal::RenderPassDescriptor::new();
        let color_attachment = render_pass_descriptor
            .color_attachments()
            .object_at(0)
            .unwrap();
        color_attachment.set_load_action(metal::MTLLoadAction::Clear);
        color_attachment.set_clear_color(metal::MTLClearColor::new(0., 0., 0., 0.));

        if let Some(msaa_texture) = &self.path_intermediate_msaa_texture {
            color_attachment.set_texture(Some(msaa_texture));
            color_attachment.set_resolve_texture(Some(intermediate_texture));
            color_attachment.set_store_action(metal::MTLStoreAction::MultisampleResolve);
        } else {
            color_attachment.set_texture(Some(intermediate_texture));
            color_attachment.set_store_action(metal::MTLStoreAction::Store);
        }

        let command_encoder = command_buffer.new_render_command_encoder(render_pass_descriptor);
        command_encoder.set_render_pipeline_state(&self.paths_rasterization_pipeline_state);

        align_offset(instance_offset);
        let mut vertices = Vec::new();
        for path in paths {
            vertices.extend(path.vertices.iter().map(|v| PathRasterizationVertex {
                xy_position: v.xy_position,
                st_position: v.st_position,
                color: path.color,
                bounds: path.bounds.intersect(&path.content_mask.bounds),
            }));
        }
        let vertices_bytes_len = mem::size_of_val(vertices.as_slice());
        let next_offset = *instance_offset + vertices_bytes_len;
        if next_offset > instance_buffer.size {
            command_encoder.end_encoding();
            return false;
        }
        command_encoder.set_vertex_buffer(
            PathRasterizationInputIndex::Vertices as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_vertex_bytes(
            PathRasterizationInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_buffer(
            PathRasterizationInputIndex::Vertices as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };
        unsafe {
            ptr::copy_nonoverlapping(
                vertices.as_ptr() as *const u8,
                buffer_contents,
                vertices_bytes_len,
            );
        }
        command_encoder.draw_primitives(
            metal::MTLPrimitiveType::Triangle,
            0,
            vertices.len() as u64,
        );
        *instance_offset = next_offset;

        command_encoder.end_encoding();
        true
    }

    /// Rasterizes a render group's arbitrary source-mask path into a fresh
    /// coverage-mask texture using the shared path rasterization pipeline (and
    /// the same MSAA sample count) so its anti-aliased coverage matches the
    /// analytic source-mask SDF.
    ///
    /// The path is rasterized in screen/device space with no group-local remap;
    /// its forced-opaque-white fill makes the cleared-transparent target's alpha
    /// channel hold pure coverage (`mask.a == coverage`).
    fn rasterize_source_mask_path(
        &self,
        path: &Path<ScaledPixels>,
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_buffer: &metal::CommandBufferRef,
    ) -> Option<metal::Texture> {
        let mask_texture = self.new_group_intermediate_texture(viewport_size)?;

        let render_pass_descriptor = metal::RenderPassDescriptor::new();
        let color_attachment = render_pass_descriptor
            .color_attachments()
            .object_at(0)
            .unwrap();
        color_attachment.set_load_action(metal::MTLLoadAction::Clear);
        color_attachment.set_clear_color(metal::MTLClearColor::new(0., 0., 0., 0.));

        // Allocate a per-group MSAA target matching the shared path pipeline's
        // sample count so anti-aliased edges match the analytic oracle.
        let msaa_texture = if self.path_sample_count > 1 {
            let storage_mode = if self.is_apple_gpu {
                metal::MTLStorageMode::Memoryless
            } else {
                metal::MTLStorageMode::Private
            };
            let descriptor = metal::TextureDescriptor::new();
            descriptor.set_width(viewport_size.width.0 as u64);
            descriptor.set_height(viewport_size.height.0 as u64);
            descriptor.set_pixel_format(metal::MTLPixelFormat::BGRA8Unorm);
            descriptor.set_texture_type(metal::MTLTextureType::D2Multisample);
            descriptor.set_storage_mode(storage_mode);
            descriptor.set_sample_count(self.path_sample_count as _);
            descriptor.set_usage(metal::MTLTextureUsage::RenderTarget);
            Some(self.device.new_texture(&descriptor))
        } else {
            None
        };

        if let Some(msaa_texture) = &msaa_texture {
            color_attachment.set_texture(Some(msaa_texture));
            color_attachment.set_resolve_texture(Some(&mask_texture));
            color_attachment.set_store_action(metal::MTLStoreAction::MultisampleResolve);
        } else {
            color_attachment.set_texture(Some(&mask_texture));
            color_attachment.set_store_action(metal::MTLStoreAction::Store);
        }

        let command_encoder = command_buffer.new_render_command_encoder(render_pass_descriptor);
        command_encoder.set_render_pipeline_state(&self.paths_rasterization_pipeline_state);

        align_offset(instance_offset);
        let bounds = path.bounds.intersect(&path.content_mask.bounds);
        let vertices: Vec<PathRasterizationVertex> = path
            .vertices
            .iter()
            .map(|v| PathRasterizationVertex {
                xy_position: v.xy_position,
                st_position: v.st_position,
                color: path.color,
                bounds,
            })
            .collect();
        if vertices.is_empty() {
            command_encoder.end_encoding();
            return Some(mask_texture);
        }
        let vertices_bytes_len = mem::size_of_val(vertices.as_slice());
        let next_offset = *instance_offset + vertices_bytes_len;
        if next_offset > instance_buffer.size {
            command_encoder.end_encoding();
            return None;
        }
        command_encoder.set_vertex_buffer(
            PathRasterizationInputIndex::Vertices as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_vertex_bytes(
            PathRasterizationInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_buffer(
            PathRasterizationInputIndex::Vertices as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };
        unsafe {
            ptr::copy_nonoverlapping(
                vertices.as_ptr() as *const u8,
                buffer_contents,
                vertices_bytes_len,
            );
        }
        command_encoder.draw_primitives(
            metal::MTLPrimitiveType::Triangle,
            0,
            vertices.len() as u64,
        );
        *instance_offset = next_offset;

        command_encoder.end_encoding();
        Some(mask_texture)
    }

    fn draw_shadows(
        &self,
        shadows: &[Shadow],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if shadows.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        command_encoder.set_render_pipeline_state(&self.shadows_pipeline_state);
        command_encoder.set_vertex_buffer(
            ShadowInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            ShadowInputIndex::Shadows as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_buffer(
            ShadowInputIndex::Shadows as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        command_encoder.set_vertex_bytes(
            ShadowInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        let shadow_bytes_len = mem::size_of_val(shadows);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + shadow_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(
                shadows.as_ptr() as *const u8,
                buffer_contents,
                shadow_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            shadows.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_quads(
        &self,
        quads: &[Quad],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if quads.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        command_encoder.set_render_pipeline_state(&self.quads_pipeline_state);
        command_encoder.set_vertex_buffer(
            QuadInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            QuadInputIndex::Quads as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_buffer(
            QuadInputIndex::Quads as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        command_encoder.set_vertex_bytes(
            QuadInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        let quad_bytes_len = mem::size_of_val(quads);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + quad_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(quads.as_ptr() as *const u8, buffer_contents, quad_bytes_len);
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            quads.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_paths_from_intermediate(
        &self,
        paths: &[Path<ScaledPixels>],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        let Some(first_path) = paths.first() else {
            return true;
        };

        let Some(ref intermediate_texture) = self.path_intermediate_texture else {
            return false;
        };

        command_encoder.set_render_pipeline_state(&self.path_sprites_pipeline_state);
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        command_encoder.set_fragment_texture(
            SpriteInputIndex::AtlasTexture as u64,
            Some(intermediate_texture),
        );

        // When copying paths from the intermediate texture to the drawable,
        // each pixel must only be copied once, in case of transparent paths.
        //
        // If all paths have the same draw order, then their bounds are all
        // disjoint, so we can copy each path's bounds individually. If this
        // batch combines different draw orders, we perform a single copy
        // for a minimal spanning rect.
        let sprites;
        if paths.last().unwrap().order == first_path.order {
            sprites = paths
                .iter()
                .map(|path| PathSprite {
                    bounds: path.clipped_bounds(),
                })
                .collect();
        } else {
            let mut bounds = first_path.clipped_bounds();
            for path in paths.iter().skip(1) {
                bounds = bounds.union(&path.clipped_bounds());
            }
            sprites = vec![PathSprite { bounds }];
        }

        align_offset(instance_offset);
        let sprite_bytes_len = mem::size_of_val(sprites.as_slice());
        let next_offset = *instance_offset + sprite_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };
        unsafe {
            ptr::copy_nonoverlapping(
                sprites.as_ptr() as *const u8,
                buffer_contents,
                sprite_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            sprites.len() as u64,
        );
        *instance_offset = next_offset;

        true
    }

    fn draw_underlines(
        &self,
        underlines: &[Underline],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if underlines.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        command_encoder.set_render_pipeline_state(&self.underlines_pipeline_state);
        command_encoder.set_vertex_buffer(
            UnderlineInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            UnderlineInputIndex::Underlines as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_buffer(
            UnderlineInputIndex::Underlines as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );

        command_encoder.set_vertex_bytes(
            UnderlineInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        let underline_bytes_len = mem::size_of_val(underlines);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + underline_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(
                underlines.as_ptr() as *const u8,
                buffer_contents,
                underline_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            underlines.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_monochrome_sprites(
        &self,
        texture_id: AtlasTextureId,
        sprites: &[MonochromeSprite],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if sprites.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        let sprite_bytes_len = mem::size_of_val(sprites);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + sprite_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        let texture = self.sprite_atlas.metal_texture(texture_id);
        let texture_size = size(
            DevicePixels(texture.width() as i32),
            DevicePixels(texture.height() as i32),
        );
        command_encoder.set_render_pipeline_state(&self.monochrome_sprites_pipeline_state);
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::AtlasTextureSize as u64,
            mem::size_of_val(&texture_size) as u64,
            &texture_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_texture(SpriteInputIndex::AtlasTexture as u64, Some(&texture));

        unsafe {
            ptr::copy_nonoverlapping(
                sprites.as_ptr() as *const u8,
                buffer_contents,
                sprite_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            sprites.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_polychrome_sprites(
        &self,
        texture_id: AtlasTextureId,
        sprites: &[PolychromeSprite],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        if sprites.is_empty() {
            return true;
        }
        align_offset(instance_offset);

        let texture = self.sprite_atlas.metal_texture(texture_id);
        let texture_size = size(
            DevicePixels(texture.width() as i32),
            DevicePixels(texture.height() as i32),
        );
        command_encoder.set_render_pipeline_state(&self.polychrome_sprites_pipeline_state);
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_vertex_bytes(
            SpriteInputIndex::AtlasTextureSize as u64,
            mem::size_of_val(&texture_size) as u64,
            &texture_size as *const Size<DevicePixels> as *const _,
        );
        command_encoder.set_fragment_buffer(
            SpriteInputIndex::Sprites as u64,
            Some(&instance_buffer.metal_buffer),
            *instance_offset as u64,
        );
        command_encoder.set_fragment_texture(SpriteInputIndex::AtlasTexture as u64, Some(&texture));

        let sprite_bytes_len = mem::size_of_val(sprites);
        let buffer_contents =
            unsafe { (instance_buffer.metal_buffer.contents() as *mut u8).add(*instance_offset) };

        let next_offset = *instance_offset + sprite_bytes_len;
        if next_offset > instance_buffer.size {
            return false;
        }

        unsafe {
            ptr::copy_nonoverlapping(
                sprites.as_ptr() as *const u8,
                buffer_contents,
                sprite_bytes_len,
            );
        }

        command_encoder.draw_primitives_instanced(
            metal::MTLPrimitiveType::Triangle,
            0,
            6,
            sprites.len() as u64,
        );
        *instance_offset = next_offset;
        true
    }

    fn draw_surfaces(
        &mut self,
        surfaces: &[PaintSurface],
        instance_buffer: &mut InstanceBuffer,
        instance_offset: &mut usize,
        viewport_size: Size<DevicePixels>,
        command_encoder: &metal::RenderCommandEncoderRef,
    ) -> bool {
        command_encoder.set_vertex_buffer(
            SurfaceInputIndex::Vertices as u64,
            Some(&self.unit_vertices),
            0,
        );
        command_encoder.set_vertex_bytes(
            SurfaceInputIndex::ViewportSize as u64,
            mem::size_of_val(&viewport_size) as u64,
            &viewport_size as *const Size<DevicePixels> as *const _,
        );

        for surface in surfaces {
            let texture_size = size(
                DevicePixels::from(surface.image_buffer.get_width() as i32),
                DevicePixels::from(surface.image_buffer.get_height() as i32),
            );

            align_offset(instance_offset);
            let next_offset = *instance_offset + mem::size_of::<Surface>();
            if next_offset > instance_buffer.size {
                return false;
            }

            command_encoder.set_vertex_buffer(
                SurfaceInputIndex::Surfaces as u64,
                Some(&instance_buffer.metal_buffer),
                *instance_offset as u64,
            );
            command_encoder.set_vertex_bytes(
                SurfaceInputIndex::TextureSize as u64,
                mem::size_of_val(&texture_size) as u64,
                &texture_size as *const Size<DevicePixels> as *const _,
            );
            let pixel_format = surface.image_buffer.get_pixel_format();
            match pixel_format {
                pixel_format if pixel_format == kCVPixelFormatType_32BGRA => {
                    command_encoder.set_render_pipeline_state(&self.bgra_surfaces_pipeline_state);
                    let bgra_texture = self
                        .core_video_texture_cache
                        .create_texture_from_image(
                            surface.image_buffer.as_concrete_TypeRef(),
                            None,
                            MTLPixelFormat::BGRA8Unorm,
                            surface.image_buffer.get_width(),
                            surface.image_buffer.get_height(),
                            0,
                        )
                        .unwrap();
                    command_encoder.set_fragment_texture(
                        SurfaceInputIndex::YTexture as u64,
                        unsafe {
                            let texture =
                                CVMetalTextureGetTexture(bgra_texture.as_concrete_TypeRef());
                            Some(metal::TextureRef::from_ptr(texture as *mut _))
                        },
                    );
                    command_encoder
                        .set_fragment_texture(SurfaceInputIndex::CbCrTexture as u64, None);
                }
                pixel_format if pixel_format == kCVPixelFormatType_420YpCbCr8BiPlanarFullRange => {
                    command_encoder.set_render_pipeline_state(&self.surfaces_pipeline_state);
                    let y_texture = self
                        .core_video_texture_cache
                        .create_texture_from_image(
                            surface.image_buffer.as_concrete_TypeRef(),
                            None,
                            MTLPixelFormat::R8Unorm,
                            surface.image_buffer.get_width_of_plane(0),
                            surface.image_buffer.get_height_of_plane(0),
                            0,
                        )
                        .unwrap();
                    let cb_cr_texture = self
                        .core_video_texture_cache
                        .create_texture_from_image(
                            surface.image_buffer.as_concrete_TypeRef(),
                            None,
                            MTLPixelFormat::RG8Unorm,
                            surface.image_buffer.get_width_of_plane(1),
                            surface.image_buffer.get_height_of_plane(1),
                            1,
                        )
                        .unwrap();

                    command_encoder.set_fragment_texture(
                        SurfaceInputIndex::YTexture as u64,
                        unsafe {
                            let texture = CVMetalTextureGetTexture(y_texture.as_concrete_TypeRef());
                            Some(metal::TextureRef::from_ptr(texture as *mut _))
                        },
                    );
                    command_encoder.set_fragment_texture(
                        SurfaceInputIndex::CbCrTexture as u64,
                        unsafe {
                            let texture =
                                CVMetalTextureGetTexture(cb_cr_texture.as_concrete_TypeRef());
                            Some(metal::TextureRef::from_ptr(texture as *mut _))
                        },
                    );
                }
                pixel_format => panic!("unsupported CVPixelBuffer pixel format {pixel_format}"),
            }

            unsafe {
                let buffer_contents = (instance_buffer.metal_buffer.contents() as *mut u8)
                    .add(*instance_offset)
                    as *mut SurfaceBounds;
                ptr::write(
                    buffer_contents,
                    SurfaceBounds {
                        bounds: surface.bounds,
                        content_mask: surface.content_mask,
                    },
                );
            }

            command_encoder.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 6);
            *instance_offset = next_offset;
        }
        true
    }
}

fn new_command_encoder_for_texture<'a>(
    command_buffer: &'a metal::CommandBufferRef,
    texture: &'a metal::TextureRef,
    viewport_size: Size<DevicePixels>,
    configure_color_attachment: impl Fn(&RenderPassColorAttachmentDescriptorRef),
) -> &'a metal::RenderCommandEncoderRef {
    let render_pass_descriptor = metal::RenderPassDescriptor::new();
    let color_attachment = render_pass_descriptor
        .color_attachments()
        .object_at(0)
        .unwrap();
    color_attachment.set_texture(Some(texture));
    color_attachment.set_store_action(metal::MTLStoreAction::Store);
    configure_color_attachment(color_attachment);

    let command_encoder = command_buffer.new_render_command_encoder(render_pass_descriptor);
    command_encoder.set_viewport(metal::MTLViewport {
        originX: 0.0,
        originY: 0.0,
        width: i32::from(viewport_size.width) as f64,
        height: i32::from(viewport_size.height) as f64,
        znear: 0.0,
        zfar: 1.0,
    });
    command_encoder
}

fn build_pipeline_state(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: metal::MTLPixelFormat,
) -> metal::RenderPipelineState {
    let vertex_fn = library
        .get_function(vertex_fn_name, None)
        .expect("error locating vertex function");
    let fragment_fn = library
        .get_function(fragment_fn_name, None)
        .expect("error locating fragment function");

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label(label);
    descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
    descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
    let color_attachment = descriptor.color_attachments().object_at(0).unwrap();
    color_attachment.set_pixel_format(pixel_format);
    color_attachment.set_blending_enabled(true);
    color_attachment.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_source_rgb_blend_factor(metal::MTLBlendFactor::SourceAlpha);
    color_attachment.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    color_attachment.set_destination_alpha_blend_factor(metal::MTLBlendFactor::One);

    device
        .new_render_pipeline_state(&descriptor)
        .expect("could not create render pipeline state")
}

fn build_path_sprite_pipeline_state(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: metal::MTLPixelFormat,
) -> metal::RenderPipelineState {
    let vertex_fn = library
        .get_function(vertex_fn_name, None)
        .expect("error locating vertex function");
    let fragment_fn = library
        .get_function(fragment_fn_name, None)
        .expect("error locating fragment function");

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label(label);
    descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
    descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
    let color_attachment = descriptor.color_attachments().object_at(0).unwrap();
    color_attachment.set_pixel_format(pixel_format);
    color_attachment.set_blending_enabled(true);
    color_attachment.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_source_rgb_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    color_attachment.set_destination_alpha_blend_factor(metal::MTLBlendFactor::One);

    device
        .new_render_pipeline_state(&descriptor)
        .expect("could not create render pipeline state")
}

fn build_path_rasterization_pipeline_state(
    device: &metal::DeviceRef,
    library: &metal::LibraryRef,
    label: &str,
    vertex_fn_name: &str,
    fragment_fn_name: &str,
    pixel_format: metal::MTLPixelFormat,
    path_sample_count: u32,
) -> metal::RenderPipelineState {
    let vertex_fn = library
        .get_function(vertex_fn_name, None)
        .expect("error locating vertex function");
    let fragment_fn = library
        .get_function(fragment_fn_name, None)
        .expect("error locating fragment function");

    let descriptor = metal::RenderPipelineDescriptor::new();
    descriptor.set_label(label);
    descriptor.set_vertex_function(Some(vertex_fn.as_ref()));
    descriptor.set_fragment_function(Some(fragment_fn.as_ref()));
    if path_sample_count > 1 {
        descriptor.set_raster_sample_count(path_sample_count as _);
        descriptor.set_alpha_to_coverage_enabled(false);
    }
    let color_attachment = descriptor.color_attachments().object_at(0).unwrap();
    color_attachment.set_pixel_format(pixel_format);
    color_attachment.set_blending_enabled(true);
    color_attachment.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
    color_attachment.set_source_rgb_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    color_attachment.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    color_attachment.set_destination_alpha_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);

    device
        .new_render_pipeline_state(&descriptor)
        .expect("could not create render pipeline state")
}

// Align to multiples of 256 make Metal happy.
fn align_offset(offset: &mut usize) {
    *offset = (*offset).div_ceil(256) * 256;
}

#[repr(C)]
enum ShadowInputIndex {
    Vertices = 0,
    Shadows = 1,
    ViewportSize = 2,
}

#[repr(C)]
enum QuadInputIndex {
    Vertices = 0,
    Quads = 1,
    ViewportSize = 2,
}

#[repr(C)]
enum UnderlineInputIndex {
    Vertices = 0,
    Underlines = 1,
    ViewportSize = 2,
}

#[repr(C)]
enum SpriteInputIndex {
    Vertices = 0,
    Sprites = 1,
    ViewportSize = 2,
    AtlasTextureSize = 3,
    AtlasTexture = 4,
    BackdropTexture = 5,
    SourceMaskTexture = 6,
}

#[repr(C)]
enum SurfaceInputIndex {
    Vertices = 0,
    Surfaces = 1,
    ViewportSize = 2,
    TextureSize = 3,
    YTexture = 4,
    CbCrTexture = 5,
}

#[repr(C)]
enum PathRasterizationInputIndex {
    Vertices = 0,
    ViewportSize = 1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct PathSprite {
    pub bounds: Bounds<ScaledPixels>,
}

#[derive(Clone, Debug, PartialEq)]
#[repr(C)]
pub struct GroupSprite {
    pub bounds: Bounds<ScaledPixels>,
    pub opacity: f32,
    pub effect_kind: u32,
    pub source_blur_radius: f32,
    pub source_mask_enabled: f32,
    pub shadow_offset: [f32; 2],
    pub shadow_blur_radius: f32,
    pub backdrop_blur_radius: f32,
    pub source_mask_blur_order: u32,
    pub derived_luma_threshold: f32,
    // 0 = analytic SDF source mask, 1 = sampled arbitrary-path coverage mask.
    pub source_mask_mode: u32,
    pub _pad1: u32,
    pub shadow_color: [f32; 4],
    pub source_mask_bounds: Bounds<ScaledPixels>,
    pub source_mask_corner_radii: Corners<ScaledPixels>,
    pub material_shape_bounds: Bounds<ScaledPixels>,
    pub material_shape_corner_radii: Corners<ScaledPixels>,
    pub group_shape_params: [f32; 4],
    pub source_directional_blur: [f32; 4],
    pub color_matrix: [[f32; 4]; 4],
    pub color_offset: [f32; 4],
    pub backdrop_active: f32,
    pub blend_mode: u32,
    pub source_tone_op: u32,
    pub source_tone_param: f32,
    pub backdrop_tint: [f32; 4],
    pub backdrop_color_matrix: [[f32; 4]; 4],
    pub backdrop_color_offset: [f32; 4],
    pub backdrop_lens: [f32; 4],
    pub backdrop_lens_lighting: [f32; 4],
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct SurfaceBounds {
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        BorderStyle, CompositeBlendMode, CompositeEffect, Edges, GroupShape, Hsla,
        LogicalVisualPlan, px, rgba,
        scene_protocol::{PaintGroup, RenderGroupBackendCounters},
        transparent_black,
    };
    use image::RgbaImage;
    use std::sync::Arc;

    const IMAGE_SIZE: i32 = 32;

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

    fn red() -> Hsla {
        rgba(0xff0000ff).into()
    }

    fn green() -> Hsla {
        rgba(0x00ff00ff).into()
    }

    fn gray() -> Hsla {
        rgba(0x808080ff).into()
    }

    fn white() -> Hsla {
        rgba(0xffffffff).into()
    }

    #[test]
    fn render_group_difference_blend_mode_metal() {
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), rgba(0x0000ffff)));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(8., 8., 8., 8.),
            vec![CompositeEffect::blend_mode(CompositeBlendMode::Difference)],
            finished_scene([quad(0, rect(8., 8., 8., 8.), white())]),
        ));
        grouped.finish();

        assert_eq!(pixel(&render(&grouped), 12, 12), [255, 255, 0, 255]);
    }

    #[test]
    fn render_group_exclusion_blend_mode_metal() {
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), rgba(0xffffffff)));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(8., 8., 8., 8.),
            vec![CompositeEffect::blend_mode(CompositeBlendMode::Exclusion)],
            finished_scene([quad(0, rect(8., 8., 8., 8.), white())]),
        ));
        grouped.finish();

        assert_eq!(pixel(&render(&grouped), 12, 12), [0, 0, 0, 255]);
    }

    #[test]
    fn render_group_hard_light_blend_mode_metal() {
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), rgba(0x808080ff)));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(8., 8., 8., 8.),
            vec![CompositeEffect::blend_mode(CompositeBlendMode::HardLight)],
            finished_scene([quad(0, rect(8., 8., 8., 8.), white())]),
        ));
        grouped.finish();

        assert_eq!(pixel(&render(&grouped), 12, 12), [255, 255, 255, 255]);
    }

    #[test]
    fn render_group_posterize_quantizes_source_metal() {
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), black()));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(8., 8., 8., 8.),
            vec![CompositeEffect::posterize(3.)],
            finished_scene([quad(0, rect(8., 8., 8., 8.), gray())]),
        ));
        grouped.finish();

        // 3 bands: gray(0.502) → floor(1.506)=1 → 1/2 = 0.5.
        assert_eq!(pixel(&render(&grouped), 12, 12), [128, 128, 128, 255]);
    }

    #[test]
    fn render_group_threshold_splits_source_by_luma_metal() {
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), black()));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(8., 8., 8., 8.),
            vec![CompositeEffect::threshold(0.5)],
            finished_scene([quad(0, rect(8., 8., 8., 8.), red())]),
        ));
        grouped.finish();

        // red luma 0.2126 < 0.5 → black.
        assert_eq!(pixel(&render(&grouped), 12, 12), [0, 0, 0, 255]);
    }

    #[test]
    fn render_group_solarize_inverts_above_threshold_metal() {
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), black()));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(8., 8., 8., 8.),
            vec![CompositeEffect::solarize(0.5)],
            finished_scene([quad(0, rect(8., 8., 8., 8.), white())]),
        ));
        grouped.finish();

        // white ≥ 0.5 → inverted to black.
        assert_eq!(pixel(&render(&grouped), 12, 12), [0, 0, 0, 255]);
    }

    #[test]
    fn render_group_sepia_tones_source_red_metal() {
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), black()));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(8., 8., 8., 8.),
            vec![CompositeEffect::sepia()],
            finished_scene([quad(0, rect(8., 8., 8., 8.), red())]),
        ));
        grouped.finish();

        assert_eq!(pixel(&render(&grouped), 12, 12), [100, 89, 69, 255]);
    }

    #[test]
    fn render_group_duotone_black_white_is_grayscale_metal() {
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), black()));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(8., 8., 8., 8.),
            vec![CompositeEffect::duotone(rgba(0x000000ff), rgba(0xffffffff))],
            finished_scene([quad(0, rect(8., 8., 8., 8.), gray())]),
        ));
        grouped.finish();

        // duotone(black, white) == grayscale; gray(0.502) luma stays 128.
        assert_eq!(pixel(&render(&grouped), 12, 12), [128, 128, 128, 255]);
    }

    #[test]
    fn render_group_duotone_maps_luma_to_color_gradient_metal() {
        // duotone(blue, yellow): white source (luma 1) -> yellow endpoint.
        let mut light = Scene::default();
        light.insert_primitive(quad(0, viewport(), black()));
        light.insert_primitive(paint_group_with_effects(
            1,
            rect(8., 8., 8., 8.),
            vec![CompositeEffect::duotone(rgba(0x0000ffff), rgba(0xffff00ff))],
            finished_scene([quad(0, rect(8., 8., 8., 8.), white())]),
        ));
        light.finish();
        assert_eq!(pixel(&render(&light), 12, 12), [255, 255, 0, 255]);

        // Black source (luma 0) -> blue endpoint.
        let mut dark = Scene::default();
        dark.insert_primitive(quad(0, viewport(), black()));
        dark.insert_primitive(paint_group_with_effects(
            1,
            rect(8., 8., 8., 8.),
            vec![CompositeEffect::duotone(rgba(0x0000ffff), rgba(0xffff00ff))],
            finished_scene([quad(0, rect(8., 8., 8., 8.), black())]),
        ));
        dark.finish();
        assert_eq!(pixel(&render(&dark), 12, 12), [0, 0, 255, 255]);
    }

    #[test]
    fn render_group_hue_rotate_keeps_gray_fixed_metal() {
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), black()));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(8., 8., 8., 8.),
            vec![CompositeEffect::hue_rotate(120.)],
            finished_scene([quad(0, rect(8., 8., 8., 8.), gray())]),
        ));
        grouped.finish();

        // Neutral gray is a fixed point of hue rotation.
        assert_eq!(pixel(&render(&grouped), 12, 12), [128, 128, 128, 255]);
    }

    #[test]
    fn render_group_directional_blur_smears_along_x_only_metal() {
        // A small bright green square inside a group with a horizontal directional
        // (motion) blur. The streak must smear the source along +x but leave the
        // cross-axis (+y) untouched.
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), black()));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(6., 6., 20., 20.),
            vec![CompositeEffect::directional_blur(0.0, px(8.))],
            finished_scene([quad(0, rect(14., 14., 4., 4.), green())]),
        ));
        grouped.finish();

        let image = render(&grouped);
        // +x of the square edge (square spans x in [14,18)): smeared green.
        let smeared_x = pixel(&image, 20, 16);
        // +y of the square edge: NOT smeared vertically -> stays black.
        let cross_axis_y = pixel(&image, 16, 20);

        assert!(
            smeared_x[1] > 0,
            "horizontal blur should smear green into +x neighbor: {smeared_x:?}"
        );
        assert_eq!(
            cross_axis_y,
            [0, 0, 0, 255],
            "horizontal blur must not smear vertically: {cross_axis_y:?}"
        );
    }

    #[test]
    fn render_group_directional_blur_smears_along_y_only_metal() {
        // Same source, but a vertical directional blur (angle = 90 degrees). The
        // streak must now smear along +y and leave +x untouched, proving the angle
        // parameter rotates the streak.
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), black()));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(6., 6., 20., 20.),
            vec![CompositeEffect::directional_blur(
                std::f32::consts::FRAC_PI_2,
                px(8.),
            )],
            finished_scene([quad(0, rect(14., 14., 4., 4.), green())]),
        ));
        grouped.finish();

        let image = render(&grouped);
        // +y of the square edge: smeared green.
        let smeared_y = pixel(&image, 16, 20);
        // +x of the square edge: NOT smeared horizontally -> stays black.
        let cross_axis_x = pixel(&image, 20, 16);

        assert!(
            smeared_y[1] > 0,
            "vertical blur should smear green into +y neighbor: {smeared_y:?}"
        );
        assert_eq!(
            cross_axis_x,
            [0, 0, 0, 255],
            "vertical blur must not smear horizontally: {cross_axis_x:?}"
        );
    }

    fn render(scene: &Scene) -> RgbaImage {
        let pool = Arc::new(Mutex::new(InstanceBufferPool::default()));
        let device = MetalRenderer::create_device();
        let mut renderer = MetalRenderer::new_internal(device, None, true, pool);
        renderer
            .render_scene_to_image(
                scene,
                size(DevicePixels(IMAGE_SIZE), DevicePixels(IMAGE_SIZE)),
            )
            .expect("render metal scene")
    }

    fn finished_scene(primitives: impl IntoIterator<Item = Quad>) -> Scene {
        let mut scene = Scene::default();
        for primitive in primitives {
            scene.insert_primitive(primitive);
        }
        scene.finish();
        scene
    }

    /// Render `scene` headlessly and return the metal backend's measured counters.
    fn backend_counters(scene: &Scene) -> RenderGroupBackendCounters {
        let pool = Arc::new(Mutex::new(InstanceBufferPool::default()));
        let device = MetalRenderer::create_device();
        let mut renderer = MetalRenderer::new_internal(device, None, true, pool);
        renderer
            .render_scene_to_image(
                scene,
                size(DevicePixels(IMAGE_SIZE), DevicePixels(IMAGE_SIZE)),
            )
            .expect("render metal scene");
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

    #[test]
    fn backend_counters_match_planner_for_opacity_group_metal() {
        let group = paint_group_with_effects(
            1,
            rect(4., 4., 16., 16.),
            vec![CompositeEffect::opacity(0.5)],
            finished_scene([quad(0, rect(4., 4., 16., 16.), black())]),
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
    fn backend_counters_exclude_source_mask_path_intermediate_metal() {
        // Parity with the wgpu law: the path source-mask intermediate is a
        // path-rasterization target, not a group-composite/backdrop-copy target,
        // so the planner does not predict it and the renderer does not count it.
        // A path-mask group (no backdrop) stays at intermediate_textures == 1;
        // this locks the intentional exclusion against drift.
        let group_bounds = rect(4., 4., 20., 20.);
        let group = paint_group_with_effects(
            1,
            group_bounds,
            vec![CompositeEffect::source_mask_path(Arc::new(rectangle_path(
                group_bounds,
            )))],
            finished_scene([quad(0, group_bounds, green())]),
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
    fn backend_counters_match_planner_for_backdrop_blur_group_metal() {
        let group = paint_group_with_effects(
            1,
            rect(4., 4., 16., 16.),
            vec![CompositeEffect::backdrop_blur(px(4.))],
            finished_scene([quad(0, rect(6., 6., 8., 8.), black())]),
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
    fn backend_counters_sum_across_sibling_groups_metal() {
        let opacity_group = paint_group_with_effects(
            1,
            rect(2., 2., 10., 10.),
            vec![CompositeEffect::opacity(0.5)],
            finished_scene([quad(0, rect(2., 2., 10., 10.), black())]),
        );
        let backdrop_group = paint_group_with_effects(
            2,
            rect(14., 14., 12., 12.),
            vec![CompositeEffect::backdrop_blur(px(4.))],
            finished_scene([quad(0, rect(16., 16., 6., 6.), black())]),
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
    fn backend_counters_zero_for_elided_opacity_zero_group_metal() {
        let group = paint_group_with_effects(
            1,
            rect(4., 4., 16., 16.),
            vec![CompositeEffect::opacity(0.0)],
            finished_scene([quad(0, rect(4., 4., 16., 16.), black())]),
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
    fn backend_counters_match_planner_for_nested_groups_metal() {
        let inner = paint_group_with_effects(
            0,
            rect(6., 6., 12., 12.),
            vec![CompositeEffect::backdrop_blur(px(4.))],
            finished_scene([quad(0, rect(8., 8., 6., 6.), black())]),
        );
        let inner_predicted = predicted_counters(&inner);

        let mut inner_scene = Scene::default();
        inner_scene.insert_primitive(quad(0, rect(4., 4., 20., 20.), black()));
        inner_scene.insert_primitive(inner);
        inner_scene.finish();

        let outer = paint_group_with_effects(
            1,
            rect(4., 4., 20., 20.),
            vec![CompositeEffect::opacity(0.5)],
            inner_scene,
        );
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
    fn render_group_backdrop_lens_metal_lights_continuous_bevel_profile() {
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
    fn render_group_backdrop_lens_metal_reflects_backdrop_color_along_material_edge() {
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
            reflected_edge[2] > reflected_edge[0] + 16
                && reflected_edge[2] > reflected_edge[1] + 16,
            "edge reflection should carry backdrop color along the tangent instead of only whitening the bevel: {reflected_edge:?}"
        );
        assert_eq!(
            stable_center,
            [0, 0, 0, 255],
            "center pane should not pick up the edge reflection sample"
        );
    }

    fn superellipse_clip_scene(shape: GroupShape) -> Scene {
        let group_scene = finished_scene([quad(0, rect(4., 4., 20., 20.), rgba(0x00ff00ff))]);
        let mut grouped = Scene::default();
        grouped.insert_primitive(quad(0, viewport(), black()));
        grouped.insert_primitive(paint_group_with_effects(
            1,
            rect(4., 4., 20., 20.),
            vec![CompositeEffect::source_mask(shape)],
            group_scene,
        ));
        grouped.finish();
        grouped
    }

    /// A superellipse with exponent 2.0 must render byte-identical to a rounded
    /// rectangle with the same radii: this guards the shared corner-SDF refactor.
    #[test]
    fn render_group_superellipse_n2_matches_rounded_rect_metal() {
        let radii = Corners::all(px(8.));
        let rounded = render(&superellipse_clip_scene(GroupShape::rounded_rect(radii)));
        let superellipse = render(&superellipse_clip_scene(GroupShape::superellipse(
            radii, 2.0,
        )));
        assert_eq!(rounded.as_raw(), superellipse.as_raw());
    }

    /// A fuller superellipse corner (exponent 4) keeps a near-corner pixel that
    /// the circular rounded-rect corner (exponent 2) clips to background.
    #[test]
    fn render_group_superellipse_keeps_fuller_corner_metal() {
        let radii = Corners::all(px(8.));
        let rounded = render(&superellipse_clip_scene(GroupShape::rounded_rect(radii)));
        let superellipse = render(&superellipse_clip_scene(GroupShape::superellipse(
            radii, 4.0,
        )));

        // Pixel (6, 5) sits in the corner crescent that the circular corner clips
        // away to background black but the squircle (n=4) keeps as opaque source.
        assert_eq!(pixel(&rounded, 6, 5), [0, 0, 0, 255]);
        assert_eq!(pixel(&superellipse, 6, 5), [0, 255, 0, 255]);
    }

    /// Builds a filled-rectangle `Path` (two solid triangles) in logical pixels,
    /// matching `GroupShape::rectangle()` over the same bounds. The plan forces
    /// the fill to opaque white so coverage lands in the mask's alpha channel.
    fn rectangle_path(bounds: Bounds<ScaledPixels>) -> Path<gpui::Pixels> {
        let x0 = px(bounds.origin.x.0);
        let y0 = px(bounds.origin.y.0);
        let x1 = px(bounds.origin.x.0 + bounds.size.width.0);
        let y1 = px(bounds.origin.y.0 + bounds.size.height.0);

        let top_left = point(x0, y0);
        let top_right = point(x1, y0);
        let bottom_right = point(x1, y1);
        let bottom_left = point(x0, y1);

        // st = (0, 1) at every vertex => solid coverage across each triangle.
        let solid = (point(0., 1.), point(0., 1.), point(0., 1.));

        let mut path = Path::new(top_left);
        path.content_mask = ContentMask {
            bounds: Bounds::new(
                point(px(0.), px(0.)),
                size(px(IMAGE_SIZE as f32), px(IMAGE_SIZE as f32)),
            ),
        };
        path.push_triangle((top_left, top_right, bottom_right), solid);
        path.push_triangle((top_left, bottom_right, bottom_left), solid);
        path
    }

    /// ORACLE (metal): an arbitrary-path source mask tracing a rectangle must
    /// produce the SAME coverage as the analytic `GroupShape::rectangle()` source
    /// mask. AfterBlur, no source blur => the mask multiplies once at integer
    /// pixel centers where the rasterized path coverage and analytic SDF agree.
    #[test]
    fn render_group_source_mask_path_matches_analytic_rectangle_mask_metal() {
        let group_bounds = rect(4., 4., 20., 20.);

        let analytic = {
            let mut scene = Scene::default();
            scene.insert_primitive(quad(0, viewport(), black()));
            scene.insert_primitive(paint_group_with_effects(
                1,
                group_bounds,
                vec![CompositeEffect::source_mask(GroupShape::rectangle())],
                finished_scene([quad(0, group_bounds, green())]),
            ));
            scene.finish();
            render(&scene)
        };

        let path_mask = {
            let mut scene = Scene::default();
            scene.insert_primitive(quad(0, viewport(), black()));
            scene.insert_primitive(paint_group_with_effects(
                1,
                group_bounds,
                vec![CompositeEffect::source_mask_path(Arc::new(rectangle_path(
                    group_bounds,
                )))],
                finished_scene([quad(0, group_bounds, green())]),
            ));
            scene.finish();
            render(&scene)
        };

        // Interior lit green (coverage 1); exterior clipped to black (coverage 0).
        assert_eq!(pixel(&analytic, 14, 14), [0, 255, 0, 255]);
        assert_eq!(pixel(&path_mask, 14, 14), [0, 255, 0, 255]);
        assert_eq!(pixel(&analytic, 1, 1), [0, 0, 0, 255]);
        assert_eq!(pixel(&path_mask, 1, 1), [0, 0, 0, 255]);
        assert_eq!(pixel(&analytic, 2, 14), [0, 0, 0, 255]);
        assert_eq!(pixel(&path_mask, 2, 14), [0, 0, 0, 255]);

        for (x, y) in [(14u32, 14u32), (6, 6), (20, 20), (1, 1), (2, 14), (30, 30)] {
            assert_eq!(
                pixel(&path_mask, x, y),
                pixel(&analytic, x, y),
                "path mask != analytic mask at ({x}, {y})"
            );
        }
    }
}
