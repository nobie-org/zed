use crate::{CompositorGpuHint, WgpuAtlas, WgpuContext};
use bytemuck::{Pod, Zeroable};
use gpui::{
    AtlasTextureId, Background, Bounds, CompositeEffectPlan, Corners, DevicePixels, GpuSpecs,
    MonochromeSprite, PaintGroup, Path, Point, PolychromeSprite, PrimitiveBatch, Quad,
    ScaledPixels, Scene, Shadow, Size, SubpixelSprite, Underline, get_gamma_correction_ratios,
    point,
};
use log::warn;
#[cfg(not(target_family = "wasm"))]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::cell::RefCell;
use std::num::NonZeroU64;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlobalParams {
    viewport_size: [f32; 2],
    premultiplied_alpha: u32,
    pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PodBounds {
    origin: [f32; 2],
    size: [f32; 2],
}

impl From<Bounds<ScaledPixels>> for PodBounds {
    fn from(bounds: Bounds<ScaledPixels>) -> Self {
        Self {
            origin: [bounds.origin.x.0, bounds.origin.y.0],
            size: [bounds.size.width.0, bounds.size.height.0],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SurfaceParams {
    bounds: PodBounds,
    content_mask: PodBounds,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GammaParams {
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
    is_bgr: u32,
    _pad: u32,
}

#[derive(Clone, Debug)]
#[repr(C)]
struct PathSprite {
    bounds: Bounds<ScaledPixels>,
}

#[derive(Clone, Debug)]
#[repr(C)]
struct GroupSprite {
    bounds: Bounds<ScaledPixels>,
    opacity: f32,
    effect_kind: u32,
    source_blur_radius: f32,
    source_mask_enabled: f32,
    shadow_offset: [f32; 2],
    shadow_blur_radius: f32,
    backdrop_blur_radius: f32,
    shadow_color: [f32; 4],
    source_mask_bounds: Bounds<ScaledPixels>,
    source_mask_corner_radii: Corners<ScaledPixels>,
    material_shape_bounds: Bounds<ScaledPixels>,
    material_shape_corner_radii: Corners<ScaledPixels>,
    color_matrix: [[f32; 4]; 4],
    color_offset: [f32; 4],
    backdrop_active: f32,
    blend_mode: u32,
    _pad0: [u32; 2],
    backdrop_tint: [f32; 4],
    backdrop_color_matrix: [[f32; 4]; 4],
    backdrop_color_offset: [f32; 4],
    backdrop_lens: [f32; 4],
    backdrop_lens_lighting: [f32; 4],
}

#[derive(Clone, Debug)]
#[repr(C)]
struct PathRasterizationVertex {
    xy_position: Point<ScaledPixels>,
    st_position: Point<f32>,
    color: Background,
    bounds: Bounds<ScaledPixels>,
}

pub struct WgpuSurfaceConfig {
    pub size: Size<DevicePixels>,
    pub transparent: bool,
    /// Preferred presentation mode. When `Some`, the renderer will use this
    /// mode if supported by the surface, falling back to `Fifo`.
    /// When `None`, defaults to `Fifo` (VSync).
    ///
    /// Mobile platforms may prefer `Mailbox` (triple-buffering) to avoid
    /// blocking in `get_current_texture()` during lifecycle transitions.
    pub preferred_present_mode: Option<wgpu::PresentMode>,
}

struct WgpuPipelines {
    quads: wgpu::RenderPipeline,
    shadows: wgpu::RenderPipeline,
    path_rasterization: wgpu::RenderPipeline,
    paths: wgpu::RenderPipeline,
    groups: wgpu::RenderPipeline,
    underlines: wgpu::RenderPipeline,
    mono_sprites: wgpu::RenderPipeline,
    subpixel_sprites: Option<wgpu::RenderPipeline>,
    poly_sprites: wgpu::RenderPipeline,
    #[allow(dead_code)]
    surfaces: wgpu::RenderPipeline,
}

struct WgpuBindGroupLayouts {
    globals: wgpu::BindGroupLayout,
    instances: wgpu::BindGroupLayout,
    instances_with_texture: wgpu::BindGroupLayout,
    group_composite: wgpu::BindGroupLayout,
    surfaces: wgpu::BindGroupLayout,
}

/// Shared GPU context reference, used to coordinate device recovery across multiple windows.
pub type GpuContext = Rc<RefCell<Option<WgpuContext>>>;

/// GPU resources that must be dropped together during device recovery.
struct WgpuResources {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    /// `None` for a headless offscreen renderer (visual-regression capture),
    /// which renders to its own offscreen texture and never presents to a window.
    surface: Option<wgpu::Surface<'static>>,
    pipelines: WgpuPipelines,
    bind_group_layouts: WgpuBindGroupLayouts,
    atlas_sampler: wgpu::Sampler,
    group_sampler: wgpu::Sampler,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    path_globals_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    path_intermediate_texture: Option<wgpu::Texture>,
    path_intermediate_view: Option<wgpu::TextureView>,
    path_msaa_texture: Option<wgpu::Texture>,
    path_msaa_view: Option<wgpu::TextureView>,
}

impl WgpuResources {
    fn invalidate_intermediate_textures(&mut self) {
        self.path_intermediate_texture = None;
        self.path_intermediate_view = None;
        self.path_msaa_texture = None;
        self.path_msaa_view = None;
    }
}

pub struct WgpuRenderer {
    /// Shared GPU context for device recovery coordination (unused on WASM).
    #[allow(dead_code)]
    context: Option<GpuContext>,
    /// Compositor GPU hint for adapter selection (unused on WASM).
    #[allow(dead_code)]
    compositor_gpu: Option<CompositorGpuHint>,
    resources: Option<WgpuResources>,
    surface_config: wgpu::SurfaceConfiguration,
    atlas: Arc<WgpuAtlas>,
    path_globals_offset: u64,
    gamma_offset: u64,
    instance_buffer_capacity: u64,
    max_buffer_size: u64,
    storage_buffer_alignment: u64,
    rendering_params: RenderingParameters,
    is_bgr: bool,
    dual_source_blending: bool,
    adapter_info: wgpu::AdapterInfo,
    transparent_alpha_mode: wgpu::CompositeAlphaMode,
    opaque_alpha_mode: wgpu::CompositeAlphaMode,
    max_texture_size: u32,
    last_error: Arc<Mutex<Option<String>>>,
    failed_frame_count: u32,
    device_lost: std::sync::Arc<std::sync::atomic::AtomicBool>,
    surface_configured: bool,
    needs_redraw: bool,
}

impl WgpuRenderer {
    fn resources(&self) -> &WgpuResources {
        self.resources
            .as_ref()
            .expect("GPU resources not available")
    }

    fn resources_mut(&mut self) -> &mut WgpuResources {
        self.resources
            .as_mut()
            .expect("GPU resources not available")
    }

    /// Creates a new WgpuRenderer from raw window handles.
    ///
    /// The `gpu_context` is a shared reference that coordinates GPU context across
    /// multiple windows. The first window to create a renderer will initialize the
    /// context; subsequent windows will share it.
    ///
    /// # Safety
    /// The caller must ensure that the window handle remains valid for the lifetime
    /// of the returned renderer.
    #[cfg(not(target_family = "wasm"))]
    pub fn new<W>(
        gpu_context: GpuContext,
        window: &W,
        config: WgpuSurfaceConfig,
        compositor_gpu: Option<CompositorGpuHint>,
    ) -> anyhow::Result<Self>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
    {
        let window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;

        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            // Fall back to the display handle already provided via InstanceDescriptor::display.
            raw_display_handle: None,
            raw_window_handle: window_handle.as_raw(),
        };

        // Use the existing context's instance if available, otherwise create a new one.
        // The surface must be created with the same instance that will be used for
        // adapter selection, otherwise wgpu will panic.
        let instance = gpu_context
            .borrow()
            .as_ref()
            .map(|ctx| ctx.instance.clone())
            .unwrap_or_else(|| WgpuContext::instance(Box::new(window.clone())));

        // Safety: The caller guarantees that the window handle is valid for the
        // lifetime of this renderer. In practice, the RawWindow struct is created
        // from the native window handles and the surface is dropped before the window.
        let surface = unsafe {
            instance
                .create_surface_unsafe(target)
                .map_err(|e| anyhow::anyhow!("Failed to create surface: {e}"))?
        };

        let mut ctx_ref = gpu_context.borrow_mut();
        let context = match ctx_ref.as_mut() {
            Some(context) => {
                context.check_compatible_with_surface(&surface)?;
                context
            }
            None => ctx_ref.insert(WgpuContext::new(instance, &surface, compositor_gpu)?),
        };

        let atlas = Arc::new(WgpuAtlas::from_context(context));

        Self::new_internal(
            Some(Rc::clone(&gpu_context)),
            context,
            Some(surface),
            config,
            compositor_gpu,
            atlas,
        )
    }

    #[cfg(target_family = "wasm")]
    pub fn new_from_canvas(
        context: &WgpuContext,
        canvas: &web_sys::HtmlCanvasElement,
        config: WgpuSurfaceConfig,
    ) -> anyhow::Result<Self> {
        let surface = context
            .instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(|e| anyhow::anyhow!("Failed to create surface: {e}"))?;

        let atlas = Arc::new(WgpuAtlas::from_context(context));

        Self::new_internal(None, context, Some(surface), config, None, atlas)
    }

    fn new_internal(
        gpu_context: Option<GpuContext>,
        context: &WgpuContext,
        surface: Option<wgpu::Surface<'static>>,
        config: WgpuSurfaceConfig,
        compositor_gpu: Option<CompositorGpuHint>,
        atlas: Arc<WgpuAtlas>,
    ) -> anyhow::Result<Self> {
        // A headless renderer (visual-regression capture) has no surface: it
        // renders into its own offscreen texture and never presents. To keep
        // capture byte-identical across backends (macOS Metal vs Linux
        // Vulkan/lavapipe) it pins a fixed format and opaque alpha instead of
        // negotiating capabilities with a surface.
        let (surface_format, transparent_alpha_mode, opaque_alpha_mode, present_mode) =
            if let Some(surface) = surface.as_ref() {
                let surface_caps = surface.get_capabilities(&context.adapter);
                let preferred_formats = [
                    wgpu::TextureFormat::Bgra8Unorm,
                    wgpu::TextureFormat::Rgba8Unorm,
                ];
                let surface_format = preferred_formats
                    .iter()
                    .find(|f| surface_caps.formats.contains(f))
                    .copied()
                    .or_else(|| surface_caps.formats.iter().find(|f| !f.is_srgb()).copied())
                    .or_else(|| surface_caps.formats.first().copied())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Surface reports no supported texture formats for adapter {:?}",
                            context.adapter.get_info().name
                        )
                    })?;

                let pick_alpha_mode = |preferences: &[wgpu::CompositeAlphaMode]| -> anyhow::Result<wgpu::CompositeAlphaMode> {
                    preferences
                        .iter()
                        .find(|p| surface_caps.alpha_modes.contains(p))
                        .copied()
                        .or_else(|| surface_caps.alpha_modes.first().copied())
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "Surface reports no supported alpha modes for adapter {:?}",
                                context.adapter.get_info().name
                            )
                        })
                };

                let transparent_alpha_mode = pick_alpha_mode(&[
                    wgpu::CompositeAlphaMode::PreMultiplied,
                    wgpu::CompositeAlphaMode::Inherit,
                ])?;
                let opaque_alpha_mode = pick_alpha_mode(&[
                    wgpu::CompositeAlphaMode::Opaque,
                    wgpu::CompositeAlphaMode::Inherit,
                ])?;
                let present_mode = config
                    .preferred_present_mode
                    .filter(|mode| surface_caps.present_modes.contains(mode))
                    .unwrap_or(wgpu::PresentMode::Fifo);
                (
                    surface_format,
                    transparent_alpha_mode,
                    opaque_alpha_mode,
                    present_mode,
                )
            } else {
                // Headless capture renders into an `Rgba32Float` target so all
                // alpha compositing accumulates at full f32 precision. An 8-bit
                // `Unorm` target re-quantizes after *every* blend, and that
                // per-blend rounding is backend-defined: Metal and Vulkan/lavapipe
                // round it differently, producing ±1 LSB cross-platform drift at
                // every anti-aliased/blended edge. A 16-bit float target removes
                // most of that but the blend itself is only specified to "at
                // least f16" precision, so a backend that blends in a wider
                // intermediate still differs by ±1 LSB. f32 is the widest blend
                // precision, so both backends round identically. Quantization to
                // 8-bit happens once, deterministically on the CPU, in
                // `render_scene_to_image`. Requires `FLOAT32_BLENDABLE` (asserted
                // in `WgpuContext::new_headless`).
                (
                    wgpu::TextureFormat::Rgba32Float,
                    wgpu::CompositeAlphaMode::Opaque,
                    wgpu::CompositeAlphaMode::Opaque,
                    wgpu::PresentMode::Fifo,
                )
            };

        let alpha_mode = if config.transparent {
            transparent_alpha_mode
        } else {
            opaque_alpha_mode
        };

        let device = Arc::clone(&context.device);
        let max_texture_size = device.limits().max_texture_dimension_2d;

        let requested_width = config.size.width.0 as u32;
        let requested_height = config.size.height.0 as u32;
        let clamped_width = requested_width.min(max_texture_size);
        let clamped_height = requested_height.min(max_texture_size);

        if clamped_width != requested_width || clamped_height != requested_height {
            warn!(
                "Requested surface size ({}, {}) exceeds maximum texture dimension {}. \
                 Clamping to ({}, {}). Window content may not fill the entire window.",
                requested_width, requested_height, max_texture_size, clamped_width, clamped_height
            );
        }

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: clamped_width.max(1),
            height: clamped_height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        };
        // Configure the surface immediately when present. The adapter selection
        // process already validated that this adapter can configure this surface.
        if let Some(surface) = surface.as_ref() {
            surface.configure(&context.device, &surface_config);
        }

        let queue = Arc::clone(&context.queue);
        let dual_source_blending = if surface.is_some() {
            context.supports_dual_source_blending()
        } else {
            // Headless capture paints grayscale monochrome glyphs only
            // (TestWindow reports subpixel rendering unsupported), so the
            // subpixel pipeline and its adapter-dependence are dropped.
            false
        };

        let rendering_params = if surface.is_some() {
            RenderingParameters::new(&context.adapter, surface_format)
        } else {
            // Fixed sample count (no MSAA): MSAA sample positions are
            // backend-defined, and the render-parity contract forbids relying on
            // backend-defined behavior for cross-platform byte-identity.
            RenderingParameters::headless()
        };
        let bind_group_layouts = Self::create_bind_group_layouts(&device);
        let pipelines = Self::create_pipelines(
            &device,
            &bind_group_layouts,
            surface_format,
            alpha_mode,
            rendering_params.path_sample_count,
            dual_source_blending,
        );

        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let group_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("group_sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let uniform_alignment = device.limits().min_uniform_buffer_offset_alignment as u64;
        let globals_size = std::mem::size_of::<GlobalParams>() as u64;
        let gamma_size = std::mem::size_of::<GammaParams>() as u64;
        let path_globals_offset = globals_size.next_multiple_of(uniform_alignment);
        let gamma_offset = (path_globals_offset + globals_size).next_multiple_of(uniform_alignment);

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals_buffer"),
            size: gamma_offset + gamma_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let max_buffer_size = device.limits().max_buffer_size;
        let storage_buffer_alignment = device.limits().min_storage_buffer_offset_alignment as u64;
        let initial_instance_buffer_capacity = 2 * 1024 * 1024;
        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance_buffer"),
            size: initial_instance_buffer_capacity,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals_bind_group"),
            layout: &bind_group_layouts.globals,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: 0,
                        size: Some(NonZeroU64::new(globals_size).unwrap()),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: gamma_offset,
                        size: Some(NonZeroU64::new(gamma_size).unwrap()),
                    }),
                },
            ],
        });

        let path_globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("path_globals_bind_group"),
            layout: &bind_group_layouts.globals,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: path_globals_offset,
                        size: Some(NonZeroU64::new(globals_size).unwrap()),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &globals_buffer,
                        offset: gamma_offset,
                        size: Some(NonZeroU64::new(gamma_size).unwrap()),
                    }),
                },
            ],
        });

        let adapter_info = context.adapter.get_info();

        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let last_error_clone = Arc::clone(&last_error);
        device.on_uncaptured_error(Arc::new(move |error| {
            let mut guard = last_error_clone.lock().unwrap();
            *guard = Some(error.to_string());
        }));

        let surface_configured = surface.is_some();
        let resources = WgpuResources {
            device,
            queue,
            surface,
            pipelines,
            bind_group_layouts,
            atlas_sampler,
            group_sampler,
            globals_buffer,
            globals_bind_group,
            path_globals_bind_group,
            instance_buffer,
            // Defer intermediate texture creation to first draw call via ensure_intermediate_textures().
            // This avoids panics when the device/surface is in an invalid state during initialization.
            path_intermediate_texture: None,
            path_intermediate_view: None,
            path_msaa_texture: None,
            path_msaa_view: None,
        };

        Ok(Self {
            context: gpu_context,
            compositor_gpu,
            resources: Some(resources),
            surface_config,
            atlas,
            path_globals_offset,
            gamma_offset,
            instance_buffer_capacity: initial_instance_buffer_capacity,
            max_buffer_size,
            storage_buffer_alignment,
            rendering_params,
            is_bgr: false,
            dual_source_blending,
            adapter_info,
            transparent_alpha_mode,
            opaque_alpha_mode,
            max_texture_size,
            last_error,
            failed_frame_count: 0,
            device_lost: context.device_lost_flag(),
            surface_configured,
            needs_redraw: false,
        })
    }

    /// Build a surfaceless renderer for headless offscreen capture (visual
    /// regression). `initial_size` only sizes the first allocation;
    /// [`render_scene_to_image`](Self::render_scene_to_image) resizes per capture.
    #[cfg(all(not(target_family = "wasm"), feature = "test-support"))]
    pub fn new_headless(
        context: &WgpuContext,
        initial_size: Size<DevicePixels>,
    ) -> anyhow::Result<Self> {
        let atlas = Arc::new(WgpuAtlas::from_context(context));
        Self::new_internal(
            None,
            context,
            None,
            WgpuSurfaceConfig {
                size: initial_size,
                transparent: false,
                preferred_present_mode: None,
            },
            None,
            atlas,
        )
    }

    /// Render `scene` to an offscreen texture at `size` and read it back as an
    /// RGBA image. The single canonical headless capture path: deterministic and
    /// byte-identical across backends (macOS Metal, Linux Vulkan/lavapipe).
    #[cfg(all(not(target_family = "wasm"), feature = "test-support"))]
    pub fn render_scene_to_image(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> anyhow::Result<image::RgbaImage> {
        if size.width.0 <= 0 || size.height.0 <= 0 {
            anyhow::bail!("invalid headless capture size: {size:?}");
        }
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;
        let max = self.max_texture_size;
        if width > max || height > max {
            anyhow::bail!("headless capture size {width}x{height} exceeds max texture dim {max}");
        }

        // Size the globals viewport and intermediate path textures to this capture.
        if width != self.surface_config.width || height != self.surface_config.height {
            self.surface_config.width = width;
            self.surface_config.height = height;
            if let Some(res) = self.resources.as_mut() {
                res.invalidate_intermediate_textures();
            }
        }

        self.atlas.before_frame();
        self.ensure_intermediate_textures();

        let format = self.surface_config.format;
        let target_texture = self
            .resources()
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("headless_capture_target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
        let target_view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());

        if !self.encode_scene_to_view(scene, Some(&target_texture), true, &target_view) {
            anyhow::bail!("headless scene encode failed");
        }

        // Copy the rendered texture into a CPU-readable buffer with rows padded
        // to COPY_BYTES_PER_ROW_ALIGNMENT, then read back, un-pad, and quantize
        // to 8-bit RGBA.
        let bytes_per_pixel: u32 = match format {
            wgpu::TextureFormat::Rgba32Float => 16,
            wgpu::TextureFormat::Rgba16Float => 8,
            wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Rgba8UnormSrgb
            | wgpu::TextureFormat::Bgra8Unorm
            | wgpu::TextureFormat::Bgra8UnormSrgb => 4,
            other => anyhow::bail!("unsupported headless capture format {other:?}"),
        };
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
        let buffer_size = (padded_bytes_per_row as u64) * (height as u64);

        let read_buffer = self
            .resources()
            .device
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("headless_capture_readback"),
                size: buffer_size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

        let mut encoder =
            self.resources()
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("headless_capture_copy"),
                });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &read_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.resources()
            .queue
            .submit(std::iter::once(encoder.finish()));

        let (sender, receiver) = std::sync::mpsc::channel();
        read_buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.resources()
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|e| anyhow::anyhow!("headless capture device poll failed: {e:?}"))?;
        receiver
            .recv()
            .map_err(|_| anyhow::anyhow!("headless capture map channel disconnected"))?
            .map_err(|e| anyhow::anyhow!("headless capture buffer map failed: {e:?}"))?;

        if let Some(error) = self.last_error.lock().unwrap().take() {
            anyhow::bail!("GPU error during headless scene encode: {error}");
        }

        let row = unpadded_bytes_per_row as usize;
        let padded = padded_bytes_per_row as usize;
        let mut raw = vec![0u8; row * height as usize];
        {
            let mapped = read_buffer.slice(..).get_mapped_range();
            for y in 0..height as usize {
                let src = y * padded;
                let dst = y * row;
                raw[dst..dst + row].copy_from_slice(&mapped[src..src + row]);
            }
        }
        read_buffer.unmap();

        // Quantize the captured texels to 8-bit RGBA with a single deterministic
        // CPU rounding step. For the float capture target this is the *only*
        // quantization, so the result never depends on backend-defined 8-bit
        // blend rounding (Metal vs Vulkan/lavapipe).
        let rgba = quantize_headless_capture_to_rgba8(&raw, format)?;
        image::RgbaImage::from_raw(width, height, rgba)
            .ok_or_else(|| anyhow::anyhow!("failed to build RgbaImage from headless capture"))
    }

    fn create_bind_group_layouts(device: &wgpu::Device) -> WgpuBindGroupLayouts {
        let globals =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("globals_layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<GlobalParams>() as u64
                            ),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(
                                std::mem::size_of::<GammaParams>() as u64
                            ),
                        },
                        count: None,
                    },
                ],
            });

        let storage_buffer_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let instances = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("instances_layout"),
            entries: &[storage_buffer_entry(0)],
        });

        let instances_with_texture =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("instances_with_texture_layout"),
                entries: &[
                    storage_buffer_entry(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let unfiltered_texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };

        let group_composite = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("group_composite_layout"),
            entries: &[
                storage_buffer_entry(0),
                unfiltered_texture_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                unfiltered_texture_entry(3),
            ],
        });

        let surfaces = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surfaces_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<SurfaceParams>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        WgpuBindGroupLayouts {
            globals,
            instances,
            instances_with_texture,
            group_composite,
            surfaces,
        }
    }

    fn create_pipelines(
        device: &wgpu::Device,
        layouts: &WgpuBindGroupLayouts,
        surface_format: wgpu::TextureFormat,
        alpha_mode: wgpu::CompositeAlphaMode,
        path_sample_count: u32,
        dual_source_blending: bool,
    ) -> WgpuPipelines {
        // Diagnostic guard: verify the device actually has
        // DUAL_SOURCE_BLENDING. We have a crash report (ZED-5G1) where a
        // feature mismatch caused a wgpu-hal abort, but we haven't
        // identified the code path that produces the mismatch. This
        // guard prevents the crash and logs more evidence.
        // Remove this check once:
        // a) We find and fix the root cause, or
        // b) There are no reports of this warning appearing for some time.
        let device_has_feature = device
            .features()
            .contains(wgpu::Features::DUAL_SOURCE_BLENDING);
        if dual_source_blending && !device_has_feature {
            log::error!(
                "BUG: dual_source_blending flag is true but device does not \
                 have DUAL_SOURCE_BLENDING enabled (device features: {:?}). \
                 Falling back to mono text rendering. Please report this at \
                 https://github.com/zed-industries/zed/issues",
                device.features(),
            );
        }
        let dual_source_blending = dual_source_blending && device_has_feature;

        let base_shader_source = include_str!("shaders.wgsl");
        let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui_shaders"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(base_shader_source)),
        });

        let subpixel_shader_source = include_str!("shaders_subpixel.wgsl");
        let subpixel_shader_module = if dual_source_blending {
            let combined = format!(
                "enable dual_source_blending;\n{base_shader_source}\n{subpixel_shader_source}"
            );
            Some(device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("gpui_subpixel_shaders"),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Owned(combined)),
            }))
        } else {
            None
        };

        let blend_mode = match alpha_mode {
            wgpu::CompositeAlphaMode::PreMultiplied => {
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
            }
            _ => wgpu::BlendState::ALPHA_BLENDING,
        };

        let color_target = wgpu::ColorTargetState {
            format: surface_format,
            blend: Some(blend_mode),
            write_mask: wgpu::ColorWrites::ALL,
        };

        let create_pipeline = |name: &str,
                               vs_entry: &str,
                               fs_entry: &str,
                               globals_layout: &wgpu::BindGroupLayout,
                               data_layout: &wgpu::BindGroupLayout,
                               topology: wgpu::PrimitiveTopology,
                               color_targets: &[Option<wgpu::ColorTargetState>],
                               sample_count: u32,
                               module: &wgpu::ShaderModule| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&format!("{name}_layout")),
                bind_group_layouts: &[Some(globals_layout), Some(data_layout)],
                immediate_size: 0,
            });

            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some(vs_entry),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fs_entry),
                    targets: color_targets,
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    unclipped_depth: false,
                    conservative: false,
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState {
                    count: sample_count,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview_mask: None,
                cache: None,
            })
        };

        let quads = create_pipeline(
            "quads",
            "vs_quad",
            "fs_quad",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let shadows = create_pipeline(
            "shadows",
            "vs_shadow",
            "fs_shadow",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let path_rasterization = create_pipeline(
            "path_rasterization",
            "vs_path_rasterization",
            "fs_path_rasterization",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleList,
            &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            path_sample_count,
            &shader_module,
        );

        let paths_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let paths = create_pipeline(
            "paths",
            "vs_path",
            "fs_path",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(paths_blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            1,
            &shader_module,
        );

        let groups = create_pipeline(
            "groups",
            "vs_group",
            "fs_group",
            &layouts.globals,
            &layouts.group_composite,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            1,
            &shader_module,
        );

        let underlines = create_pipeline(
            "underlines",
            "vs_underline",
            "fs_underline",
            &layouts.globals,
            &layouts.instances,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let mono_sprites = create_pipeline(
            "mono_sprites",
            "vs_mono_sprite",
            "fs_mono_sprite",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let subpixel_sprites = if let Some(subpixel_module) = &subpixel_shader_module {
            let subpixel_blend = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Src1,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc1,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            };

            Some(create_pipeline(
                "subpixel_sprites",
                "vs_subpixel_sprite",
                "fs_subpixel_sprite",
                &layouts.globals,
                &layouts.instances_with_texture,
                wgpu::PrimitiveTopology::TriangleStrip,
                &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(subpixel_blend),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
                1,
                subpixel_module,
            ))
        } else {
            None
        };

        let poly_sprites = create_pipeline(
            "poly_sprites",
            "vs_poly_sprite",
            "fs_poly_sprite",
            &layouts.globals,
            &layouts.instances_with_texture,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target.clone())],
            1,
            &shader_module,
        );

        let surfaces = create_pipeline(
            "surfaces",
            "vs_surface",
            "fs_surface",
            &layouts.globals,
            &layouts.surfaces,
            wgpu::PrimitiveTopology::TriangleStrip,
            &[Some(color_target)],
            1,
            &shader_module,
        );

        WgpuPipelines {
            quads,
            shadows,
            path_rasterization,
            paths,
            groups,
            underlines,
            mono_sprites,
            subpixel_sprites,
            poly_sprites,
            surfaces,
        }
    }

    fn create_path_intermediate(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("path_intermediate"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn create_msaa_if_needed(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        sample_count: u32,
    ) -> Option<(wgpu::Texture, wgpu::TextureView)> {
        if sample_count <= 1 {
            return None;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("path_msaa"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Some((texture, view))
    }

    pub fn update_drawable_size(&mut self, size: Size<DevicePixels>) {
        let width = size.width.0 as u32;
        let height = size.height.0 as u32;

        if width != self.surface_config.width || height != self.surface_config.height {
            let clamped_width = width.min(self.max_texture_size);
            let clamped_height = height.min(self.max_texture_size);

            if clamped_width != width || clamped_height != height {
                warn!(
                    "Requested surface size ({}, {}) exceeds maximum texture dimension {}. \
                     Clamping to ({}, {}). Window content may not fill the entire window.",
                    width, height, self.max_texture_size, clamped_width, clamped_height
                );
            }

            self.surface_config.width = clamped_width.max(1);
            self.surface_config.height = clamped_height.max(1);
            let surface_config = self.surface_config.clone();

            let resources = self.resources_mut();

            // Wait for any in-flight GPU work to complete before destroying textures
            if let Err(e) = resources.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            }) {
                warn!("Failed to poll device during resize: {e:?}");
            }

            // Destroy old textures before allocating new ones to avoid GPU memory spikes
            if let Some(ref texture) = resources.path_intermediate_texture {
                texture.destroy();
            }
            if let Some(ref texture) = resources.path_msaa_texture {
                texture.destroy();
            }

            if let Some(surface) = resources.surface.as_ref() {
                surface.configure(&resources.device, &surface_config);
            }

            // Invalidate intermediate textures - they will be lazily recreated
            // in draw() after we confirm the surface is healthy. This avoids
            // panics when the device/surface is in an invalid state during resize.
            resources.invalidate_intermediate_textures();
        }
    }

    fn ensure_intermediate_textures(&mut self) {
        if self.resources().path_intermediate_texture.is_some() {
            return;
        }

        let format = self.surface_config.format;
        let width = self.surface_config.width;
        let height = self.surface_config.height;
        let path_sample_count = self.rendering_params.path_sample_count;
        let resources = self.resources_mut();

        let (t, v) = Self::create_path_intermediate(&resources.device, format, width, height);
        resources.path_intermediate_texture = Some(t);
        resources.path_intermediate_view = Some(v);

        let (path_msaa_texture, path_msaa_view) = Self::create_msaa_if_needed(
            &resources.device,
            format,
            width,
            height,
            path_sample_count,
        )
        .map(|(t, v)| (Some(t), Some(v)))
        .unwrap_or((None, None));
        resources.path_msaa_texture = path_msaa_texture;
        resources.path_msaa_view = path_msaa_view;
    }

    pub fn set_subpixel_layout(&mut self, is_bgr: bool) {
        self.is_bgr = is_bgr;
    }

    pub fn update_transparency(&mut self, transparent: bool) {
        let new_alpha_mode = if transparent {
            self.transparent_alpha_mode
        } else {
            self.opaque_alpha_mode
        };

        if new_alpha_mode != self.surface_config.alpha_mode {
            self.surface_config.alpha_mode = new_alpha_mode;
            let surface_config = self.surface_config.clone();
            let path_sample_count = self.rendering_params.path_sample_count;
            let dual_source_blending = self.dual_source_blending;
            let resources = self.resources_mut();
            if let Some(surface) = resources.surface.as_ref() {
                surface.configure(&resources.device, &surface_config);
            }
            resources.pipelines = Self::create_pipelines(
                &resources.device,
                &resources.bind_group_layouts,
                surface_config.format,
                surface_config.alpha_mode,
                path_sample_count,
                dual_source_blending,
            );
        }
    }

    #[allow(dead_code)]
    pub fn viewport_size(&self) -> Size<DevicePixels> {
        Size {
            width: DevicePixels(self.surface_config.width as i32),
            height: DevicePixels(self.surface_config.height as i32),
        }
    }

    pub fn sprite_atlas(&self) -> &Arc<WgpuAtlas> {
        &self.atlas
    }

    /// The wgpu backend of the adapter this renderer drives. Used by the
    /// headless capture path to report which backend produced a screenshot.
    pub fn adapter_backend(&self) -> wgpu::Backend {
        self.adapter_info.backend
    }

    pub fn supports_dual_source_blending(&self) -> bool {
        self.dual_source_blending
    }

    pub fn gpu_specs(&self) -> GpuSpecs {
        GpuSpecs {
            is_software_emulated: self.adapter_info.device_type == wgpu::DeviceType::Cpu,
            device_name: self.adapter_info.name.clone(),
            driver_name: self.adapter_info.driver.clone(),
            driver_info: self.adapter_info.driver_info.clone(),
        }
    }

    pub fn max_texture_size(&self) -> u32 {
        self.max_texture_size
    }

    pub fn draw(&mut self, scene: &Scene) -> bool {
        // Bail out early if the surface has been unconfigured (e.g. during
        // Android background/rotation transitions).  Attempting to acquire
        // a texture from an unconfigured surface can block indefinitely on
        // some drivers (Adreno).
        if !self.surface_configured {
            return false;
        }

        let last_error = self.last_error.lock().unwrap().take();
        if let Some(error) = last_error {
            self.failed_frame_count += 1;
            log::error!(
                "GPU error during frame (failure {} of 10): {error}",
                self.failed_frame_count
            );

            // TBD. Does retrying more actually help?
            if self.failed_frame_count > 10 {
                panic!("Too many consecutive GPU errors. Last error: {error}");
            } else if self.failed_frame_count > 5 {
                if let Some(res) = self.resources.as_mut() {
                    res.invalidate_intermediate_textures();
                }
                self.atlas.clear();
                self.needs_redraw = true;
                self.failed_frame_count = 0;
                return false;
            }
        } else {
            self.failed_frame_count = 0;
        }

        self.atlas.before_frame();

        let current = match self.resources().surface.as_ref() {
            Some(surface) => surface.get_current_texture(),
            // A headless renderer has no surface; it captures via
            // `render_scene_to_image` and `draw()` is never called on it.
            None => return false,
        };
        let frame = match current {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                // Textures must be destroyed before the surface can be reconfigured.
                drop(frame);
                let surface_config = self.surface_config.clone();
                let resources = self.resources_mut();
                if let Some(surface) = resources.surface.as_ref() {
                    surface.configure(&resources.device, &surface_config);
                }
                return false;
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                let surface_config = self.surface_config.clone();
                let resources = self.resources_mut();
                if let Some(surface) = resources.surface.as_ref() {
                    surface.configure(&resources.device, &surface_config);
                }
                return false;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return false;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                *self.last_error.lock().unwrap() =
                    Some("Surface texture validation error".to_string());
                return false;
            }
        };

        // Now that we know the surface is healthy, ensure intermediate textures exist
        self.ensure_intermediate_textures();

        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let encoded = self.encode_scene_to_view(
            scene,
            Some(&frame.texture),
            self.surface_config
                .usage
                .contains(wgpu::TextureUsages::COPY_SRC),
            &frame_view,
        );
        frame.present();
        encoded
    }

    /// Encode `scene` into `target_view` and submit it (does **not** present).
    /// Shared by the windowed `draw` path (target = surface texture; caller
    /// presents) and the headless `render_scene_to_image` path (target =
    /// offscreen texture; caller reads it back). Callers must run
    /// `before_frame`/`ensure_intermediate_textures` and set `surface_config`
    /// width/height before calling. Returns whether a frame was encoded.
    fn encode_scene_to_view(
        &mut self,
        scene: &Scene,
        target_texture: Option<&wgpu::Texture>,
        target_can_copy: bool,
        target_view: &wgpu::TextureView,
    ) -> bool {
        let gamma_params = GammaParams {
            gamma_ratios: self.rendering_params.gamma_ratios,
            grayscale_enhanced_contrast: self.rendering_params.grayscale_enhanced_contrast,
            subpixel_enhanced_contrast: self.rendering_params.subpixel_enhanced_contrast,
            is_bgr: self.is_bgr as u32,
            _pad: 0,
        };

        let globals = GlobalParams {
            viewport_size: [
                self.surface_config.width as f32,
                self.surface_config.height as f32,
            ],
            premultiplied_alpha: if self.surface_config.alpha_mode
                == wgpu::CompositeAlphaMode::PreMultiplied
            {
                1
            } else {
                0
            },
            pad: 0,
        };

        let path_globals = GlobalParams {
            premultiplied_alpha: 0,
            ..globals
        };

        {
            let resources = self.resources();
            resources.queue.write_buffer(
                &resources.globals_buffer,
                0,
                bytemuck::bytes_of(&globals),
            );
            resources.queue.write_buffer(
                &resources.globals_buffer,
                self.path_globals_offset,
                bytemuck::bytes_of(&path_globals),
            );
            resources.queue.write_buffer(
                &resources.globals_buffer,
                self.gamma_offset,
                bytemuck::bytes_of(&gamma_params),
            );
        }

        loop {
            let mut instance_offset: u64 = 0;

            let mut encoder =
                self.resources()
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("main_encoder"),
                    });
            let mut retained_textures = Vec::new();

            let needs_root_intermediate = scene.requires_backdrop_effects() && !target_can_copy;
            let overflow = if needs_root_intermediate {
                let (root_texture, root_view) = self.create_group_intermediate();
                let encoded_root = self.encode_scene_batches_to_view(
                    scene,
                    Some(&root_texture),
                    true,
                    &root_view,
                    &mut encoder,
                    &mut instance_offset,
                    true,
                    &mut retained_textures,
                );
                let drew_root = encoded_root
                    && self.draw_texture_to_view(
                        &root_view,
                        target_view,
                        &mut encoder,
                        &mut instance_offset,
                        true,
                    );
                retained_textures.push(root_texture);
                !drew_root
            } else {
                !self.encode_scene_batches_to_view(
                    scene,
                    target_texture,
                    target_can_copy,
                    target_view,
                    &mut encoder,
                    &mut instance_offset,
                    true,
                    &mut retained_textures,
                )
            };

            if overflow {
                drop(encoder);
                if self.instance_buffer_capacity >= self.max_buffer_size {
                    log::error!(
                        "instance buffer size grew too large: {}",
                        self.instance_buffer_capacity
                    );
                    return true;
                }
                self.grow_instance_buffer();
                continue;
            }

            self.resources()
                .queue
                .submit(std::iter::once(encoder.finish()));
            drop(retained_textures);
            return true;
        }
    }

    fn encode_scene_batches_to_view(
        &self,
        scene: &Scene,
        target_texture: Option<&wgpu::Texture>,
        target_can_copy: bool,
        target_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        instance_offset: &mut u64,
        clear: bool,
        retained_textures: &mut Vec<wgpu::Texture>,
    ) -> bool {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scene_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        for batch in scene.batches() {
            let ok = match batch {
                PrimitiveBatch::Quads(range) => {
                    self.draw_quads(&scene.quads[range], instance_offset, &mut pass)
                }
                PrimitiveBatch::Shadows(range) => {
                    self.draw_shadows(&scene.shadows[range], instance_offset, &mut pass)
                }
                PrimitiveBatch::Paths(range) => {
                    let paths = &scene.paths[range];
                    if paths.is_empty() {
                        continue;
                    }

                    drop(pass);

                    let did_draw = self.draw_paths_to_intermediate(encoder, paths, instance_offset);

                    pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("scene_pass_continued"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: target_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        ..Default::default()
                    });

                    if did_draw {
                        self.draw_paths_from_intermediate(paths, instance_offset, &mut pass)
                    } else {
                        false
                    }
                }
                PrimitiveBatch::Underlines(range) => {
                    self.draw_underlines(&scene.underlines[range], instance_offset, &mut pass)
                }
                PrimitiveBatch::MonochromeSprites { texture_id, range } => self
                    .draw_monochrome_sprites(
                        &scene.monochrome_sprites[range],
                        texture_id,
                        instance_offset,
                        &mut pass,
                    ),
                PrimitiveBatch::SubpixelSprites { texture_id, range } => self
                    .draw_subpixel_sprites(
                        &scene.subpixel_sprites[range],
                        texture_id,
                        instance_offset,
                        &mut pass,
                    ),
                PrimitiveBatch::PolychromeSprites { texture_id, range } => self
                    .draw_polychrome_sprites(
                        &scene.polychrome_sprites[range],
                        texture_id,
                        instance_offset,
                        &mut pass,
                    ),
                PrimitiveBatch::Surfaces(_surfaces) => {
                    // Surfaces are macOS-only for video playback.
                    true
                }
                PrimitiveBatch::Groups(range) => {
                    drop(pass);

                    let did_draw = self.draw_groups(
                        &scene.groups[range],
                        target_texture,
                        target_can_copy,
                        target_view,
                        encoder,
                        instance_offset,
                        retained_textures,
                    );

                    pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("scene_pass_continued"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: target_view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        ..Default::default()
                    });

                    did_draw
                }
            };
            if !ok {
                return false;
            }
        }

        true
    }

    fn draw_quads(
        &self,
        quads: &[Quad],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(quads) };
        self.draw_instances(
            data,
            quads.len() as u32,
            &self.resources().pipelines.quads,
            instance_offset,
            pass,
        )
    }

    fn draw_shadows(
        &self,
        shadows: &[Shadow],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(shadows) };
        self.draw_instances(
            data,
            shadows.len() as u32,
            &self.resources().pipelines.shadows,
            instance_offset,
            pass,
        )
    }

    fn draw_underlines(
        &self,
        underlines: &[Underline],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let data = unsafe { Self::instance_bytes(underlines) };
        self.draw_instances(
            data,
            underlines.len() as u32,
            &self.resources().pipelines.underlines,
            instance_offset,
            pass,
        )
    }

    fn draw_monochrome_sprites(
        &self,
        sprites: &[MonochromeSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            &self.resources().pipelines.mono_sprites,
            instance_offset,
            pass,
        )
    }

    fn draw_subpixel_sprites(
        &self,
        sprites: &[SubpixelSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        let resources = self.resources();
        let pipeline = resources
            .pipelines
            .subpixel_sprites
            .as_ref()
            .unwrap_or(&resources.pipelines.mono_sprites);
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            pipeline,
            instance_offset,
            pass,
        )
    }

    fn draw_polychrome_sprites(
        &self,
        sprites: &[PolychromeSprite],
        texture_id: AtlasTextureId,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let tex_info = self.atlas.get_texture_info(texture_id);
        let data = unsafe { Self::instance_bytes(sprites) };
        self.draw_instances_with_texture(
            data,
            sprites.len() as u32,
            &tex_info.view,
            &self.resources().pipelines.poly_sprites,
            instance_offset,
            pass,
        )
    }

    fn draw_instances(
        &self,
        data: &[u8],
        instance_count: u32,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        if instance_count == 0 {
            return true;
        }
        let Some((offset, size)) = self.write_to_instance_buffer(instance_offset, data) else {
            return false;
        };
        let resources = self.resources();
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &resources.bind_group_layouts.instances,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.instance_binding(offset, size),
                }],
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..instance_count);
        true
    }

    fn draw_instances_with_texture(
        &self,
        data: &[u8],
        instance_count: u32,
        texture_view: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        self.draw_instances_with_texture_and_sampler(
            data,
            instance_count,
            texture_view,
            &self.resources().atlas_sampler,
            pipeline,
            instance_offset,
            pass,
        )
    }

    fn draw_instances_with_texture_and_sampler(
        &self,
        data: &[u8],
        instance_count: u32,
        texture_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        if instance_count == 0 {
            return true;
        }
        let Some((offset, size)) = self.write_to_instance_buffer(instance_offset, data) else {
            return false;
        };
        let resources = self.resources();
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &resources.bind_group_layouts.instances_with_texture,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.instance_binding(offset, size),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..instance_count);
        true
    }

    fn draw_group_instances(
        &self,
        data: &[u8],
        instance_count: u32,
        group_view: &wgpu::TextureView,
        backdrop_view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        pipeline: &wgpu::RenderPipeline,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        if instance_count == 0 {
            return true;
        }
        let Some((offset, size)) = self.write_to_instance_buffer(instance_offset, data) else {
            return false;
        };
        let resources = self.resources();
        let bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &resources.bind_group_layouts.group_composite,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.instance_binding(offset, size),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(group_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(backdrop_view),
                    },
                ],
            });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..4, 0..instance_count);
        true
    }

    unsafe fn instance_bytes<T>(instances: &[T]) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                instances.as_ptr() as *const u8,
                std::mem::size_of_val(instances),
            )
        }
    }

    fn draw_texture_to_view(
        &self,
        texture_view: &wgpu::TextureView,
        target_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        instance_offset: &mut u64,
        clear: bool,
    ) -> bool {
        let sprite = PathSprite {
            bounds: Bounds::new(
                point(ScaledPixels(0.), ScaledPixels(0.)),
                Size {
                    width: ScaledPixels(self.surface_config.width as f32),
                    height: ScaledPixels(self.surface_config.height as f32),
                },
            ),
        };
        let sprite_data = unsafe { Self::instance_bytes(std::slice::from_ref(&sprite)) };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("texture_to_view_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if clear {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        self.draw_instances_with_texture(
            sprite_data,
            1,
            texture_view,
            &self.resources().pipelines.paths,
            instance_offset,
            &mut pass,
        )
    }

    fn draw_groups(
        &self,
        groups: &[PaintGroup],
        target_texture: Option<&wgpu::Texture>,
        target_can_copy: bool,
        target_view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
        instance_offset: &mut u64,
        retained_textures: &mut Vec<wgpu::Texture>,
    ) -> bool {
        for group in groups {
            let effect_plan = group.plan.normalized_effects().clone();
            if effect_plan.opacity() <= 0. {
                continue;
            }

            let (group_texture, group_view) = self.create_group_intermediate();

            if !self.encode_scene_batches_to_view(
                group.scene.as_ref(),
                Some(&group_texture),
                true,
                &group_view,
                encoder,
                instance_offset,
                true,
                retained_textures,
            ) {
                return false;
            }

            let backdrop_view_storage;
            let backdrop_view = if effect_plan.reads_backdrop() {
                let Some(target_texture) = target_texture else {
                    *self.last_error.lock().unwrap() =
                        Some("Render-group backdrop effect has no readable target texture".into());
                    return false;
                };
                if !target_can_copy {
                    *self.last_error.lock().unwrap() = Some(
                        "Render-group backdrop effect needs COPY_SRC support for the target texture"
                            .into(),
                    );
                    return false;
                }

                let (backdrop_texture, backdrop_view) = self.create_group_intermediate();
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: target_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &backdrop_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: self.surface_config.width.max(1),
                        height: self.surface_config.height.max(1),
                        depth_or_array_layers: 1,
                    },
                );
                retained_textures.push(backdrop_texture);
                backdrop_view_storage = backdrop_view;
                &backdrop_view_storage
            } else {
                &group_view
            };

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("group_composite_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            if !self.draw_group_from_intermediate(
                group,
                effect_plan,
                &group_view,
                backdrop_view,
                instance_offset,
                &mut pass,
            ) {
                return false;
            }

            retained_textures.push(group_texture);
        }

        true
    }

    fn create_group_intermediate(&self) -> (wgpu::Texture, wgpu::TextureView) {
        let resources = self.resources();
        let texture = resources.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("group_intermediate"),
            size: wgpu::Extent3d {
                width: self.surface_config.width.max(1),
                height: self.surface_config.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.surface_config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn draw_group_from_intermediate(
        &self,
        group: &PaintGroup,
        effect_plan: CompositeEffectPlan,
        group_view: &wgpu::TextureView,
        backdrop_view: &wgpu::TextureView,
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let (color_matrix, color_offset) = Self::group_source_color_filter(&effect_plan);
        let (backdrop_color_matrix, backdrop_color_offset) =
            Self::group_backdrop_color_filter(&effect_plan);
        let backdrop_tint = effect_plan.backdrop_tint().to_rgb();
        let (backdrop_lens, backdrop_lens_lighting) = Self::group_backdrop_lens(&effect_plan);
        let mut sprites = Vec::with_capacity(
            effect_plan.drop_shadows().len() + effect_plan.surface_shadows().len() + 1,
        );
        let (source_mask_enabled, source_mask_corner_radii) = match effect_plan.source_mask() {
            Some(corner_radii) => (1., corner_radii),
            None => (0., Corners::all(ScaledPixels(0.))),
        };
        let material_shape_corner_radii = effect_plan
            .material_shape()
            .unwrap_or_else(|| Corners::all(ScaledPixels(0.)));

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
                shadow_color: [color.r, color.g, color.b, color.a],
                source_mask_bounds: group.bounds,
                source_mask_corner_radii,
                material_shape_bounds: group.bounds,
                material_shape_corner_radii,
                color_matrix,
                color_offset,
                backdrop_active: 0.,
                blend_mode: 0,
                _pad0: [0; 2],
                backdrop_tint: [0., 0., 0., 0.],
                backdrop_color_matrix,
                backdrop_color_offset,
                backdrop_lens: [0., 0., 0., 0.],
                backdrop_lens_lighting: [0., 0., 0., 0.],
            });
        }

        for shadow in effect_plan.surface_shadows() {
            let color = shadow.color.to_rgb();
            sprites.push(GroupSprite {
                bounds: group.capture_bounds,
                opacity: effect_plan.opacity(),
                effect_kind: 2,
                source_blur_radius: 0.,
                source_mask_enabled: 0.,
                shadow_offset: [shadow.offset.x.0, shadow.offset.y.0],
                shadow_blur_radius: shadow.blur_radius.0,
                backdrop_blur_radius: 0.,
                shadow_color: [color.r, color.g, color.b, color.a],
                source_mask_bounds: group.bounds,
                source_mask_corner_radii: Corners::all(ScaledPixels(0.)),
                material_shape_bounds: group.bounds,
                material_shape_corner_radii: shadow.shape,
                color_matrix,
                color_offset,
                backdrop_active: 0.,
                blend_mode: 0,
                _pad0: [0; 2],
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
            shadow_color: [0., 0., 0., 0.],
            source_mask_bounds: group.bounds,
            source_mask_corner_radii,
            material_shape_bounds: group.bounds,
            material_shape_corner_radii,
            color_matrix,
            color_offset,
            backdrop_active: if effect_plan.has_backdrop_material() {
                1.
            } else {
                0.
            },
            blend_mode: effect_plan.blend_mode().shader_code(),
            _pad0: [0; 2],
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
        let sprite_data = unsafe { Self::instance_bytes(&sprites) };
        self.draw_group_instances(
            sprite_data,
            sprites.len() as u32,
            group_view,
            backdrop_view,
            &self.resources().group_sampler,
            &self.resources().pipelines.groups,
            instance_offset,
            pass,
        )
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

    fn draw_paths_from_intermediate(
        &self,
        paths: &[Path<ScaledPixels>],
        instance_offset: &mut u64,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> bool {
        let first_path = &paths[0];
        let sprites: Vec<PathSprite> = if paths.last().map(|p| &p.order) == Some(&first_path.order)
        {
            paths
                .iter()
                .map(|p| PathSprite {
                    bounds: p.clipped_bounds(),
                })
                .collect()
        } else {
            let mut bounds = first_path.clipped_bounds();
            for path in paths.iter().skip(1) {
                bounds = bounds.union(&path.clipped_bounds());
            }
            vec![PathSprite { bounds }]
        };

        let resources = self.resources();
        let Some(path_intermediate_view) = resources.path_intermediate_view.as_ref() else {
            return true;
        };

        let sprite_data = unsafe { Self::instance_bytes(&sprites) };
        self.draw_instances_with_texture(
            sprite_data,
            sprites.len() as u32,
            path_intermediate_view,
            &resources.pipelines.paths,
            instance_offset,
            pass,
        )
    }

    fn draw_paths_to_intermediate(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        paths: &[Path<ScaledPixels>],
        instance_offset: &mut u64,
    ) -> bool {
        let mut vertices = Vec::new();
        for path in paths {
            let bounds = path.clipped_bounds();
            vertices.extend(path.vertices.iter().map(|v| PathRasterizationVertex {
                xy_position: v.xy_position,
                st_position: v.st_position,
                color: path.color,
                bounds,
            }));
        }

        if vertices.is_empty() {
            return true;
        }

        let vertex_data = unsafe { Self::instance_bytes(&vertices) };
        let Some((vertex_offset, vertex_size)) =
            self.write_to_instance_buffer(instance_offset, vertex_data)
        else {
            return false;
        };

        let resources = self.resources();
        let data_bind_group = resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("path_rasterization_bind_group"),
                layout: &resources.bind_group_layouts.instances,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.instance_binding(vertex_offset, vertex_size),
                }],
            });

        let Some(path_intermediate_view) = resources.path_intermediate_view.as_ref() else {
            return true;
        };

        let (target_view, resolve_target) = if let Some(ref msaa_view) = resources.path_msaa_view {
            (msaa_view, Some(path_intermediate_view))
        } else {
            (path_intermediate_view, None)
        };

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("path_rasterization_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            pass.set_pipeline(&resources.pipelines.path_rasterization);
            pass.set_bind_group(0, &resources.path_globals_bind_group, &[]);
            pass.set_bind_group(1, &data_bind_group, &[]);
            pass.draw(0..vertices.len() as u32, 0..1);
        }

        true
    }

    fn grow_instance_buffer(&mut self) {
        let new_capacity = (self.instance_buffer_capacity * 2).min(self.max_buffer_size);
        log::info!("increased instance buffer size to {}", new_capacity);
        let resources = self.resources_mut();
        resources.instance_buffer = resources.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance_buffer"),
            size: new_capacity,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.instance_buffer_capacity = new_capacity;
    }

    fn write_to_instance_buffer(
        &self,
        instance_offset: &mut u64,
        data: &[u8],
    ) -> Option<(u64, NonZeroU64)> {
        let offset = (*instance_offset).next_multiple_of(self.storage_buffer_alignment);
        let size = (data.len() as u64).max(16);
        if offset + size > self.instance_buffer_capacity {
            return None;
        }
        let resources = self.resources();
        resources
            .queue
            .write_buffer(&resources.instance_buffer, offset, data);
        *instance_offset = offset + size;
        Some((offset, NonZeroU64::new(size).expect("size is at least 16")))
    }

    fn instance_binding(&self, offset: u64, size: NonZeroU64) -> wgpu::BindingResource<'_> {
        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &self.resources().instance_buffer,
            offset,
            size: Some(size),
        })
    }

    /// Mark the surface as unconfigured so rendering is skipped until a new
    /// surface is provided via [`replace_surface`](Self::replace_surface).
    ///
    /// This does **not** drop the renderer — the device, queue, atlas, and
    /// pipelines stay alive.  Use this when the native window is destroyed
    /// (e.g. Android `TerminateWindow`) but you intend to re-create the
    /// surface later without losing cached atlas textures.
    pub fn unconfigure_surface(&mut self) {
        self.surface_configured = false;
        // Drop intermediate textures since they reference the old surface size.
        if let Some(res) = self.resources.as_mut() {
            res.invalidate_intermediate_textures();
        }
    }

    /// Replace the wgpu surface with a new one (e.g. after Android destroys
    /// and recreates the native window).  Keeps the device, queue, atlas, and
    /// all pipelines intact so cached `AtlasTextureId`s remain valid.
    ///
    /// The `instance` **must** be the same [`wgpu::Instance`] that was used to
    /// create the adapter and device (i.e. from the [`WgpuContext`]).  Using a
    /// different instance will cause a "Device does not exist" panic because
    /// the wgpu device is bound to its originating instance.
    #[cfg(not(target_family = "wasm"))]
    pub fn replace_surface<W: HasWindowHandle>(
        &mut self,
        window: &W,
        config: WgpuSurfaceConfig,
        instance: &wgpu::Instance,
    ) -> anyhow::Result<()> {
        let window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;

        let surface = create_surface(instance, window_handle.as_raw())?;

        let width = (config.size.width.0 as u32).max(1);
        let height = (config.size.height.0 as u32).max(1);

        let alpha_mode = if config.transparent {
            self.transparent_alpha_mode
        } else {
            self.opaque_alpha_mode
        };

        self.surface_config.width = width;
        self.surface_config.height = height;
        self.surface_config.alpha_mode = alpha_mode;
        if let Some(mode) = config.preferred_present_mode {
            self.surface_config.present_mode = mode;
        }

        {
            let res = self
                .resources
                .as_mut()
                .expect("GPU resources not available");
            surface.configure(&res.device, &self.surface_config);
            res.surface = Some(surface);

            // Invalidate intermediate textures — they'll be recreated lazily.
            res.invalidate_intermediate_textures();
        }

        self.surface_configured = true;

        Ok(())
    }

    pub fn destroy(&mut self) {
        // Release surface-bound GPU resources eagerly so the underlying native
        // window can be destroyed before the renderer itself is dropped.
        self.resources.take();
    }

    /// Returns true if the GPU device was lost and recovery is needed.
    pub fn device_lost(&self) -> bool {
        self.device_lost.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Returns true if a redraw is needed because GPU state was cleared.
    /// Calling this method clears the flag.
    pub fn needs_redraw(&mut self) -> bool {
        std::mem::take(&mut self.needs_redraw)
    }

    /// Recovers from a lost GPU device by recreating the renderer with a new context.
    ///
    /// Call this after detecting `device_lost()` returns true.
    ///
    /// This method coordinates recovery across multiple windows:
    /// - The first window to call this will recreate the shared context
    /// - Subsequent windows will adopt the already-recovered context
    #[cfg(not(target_family = "wasm"))]
    pub fn recover<W>(&mut self, window: &W) -> anyhow::Result<()>
    where
        W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
    {
        let gpu_context = self.context.as_ref().expect("recover requires gpu_context");

        // Check if another window already recovered the context
        let needs_new_context = gpu_context
            .borrow()
            .as_ref()
            .is_none_or(|ctx| ctx.device_lost());

        let window_handle = window
            .window_handle()
            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;

        let surface = if needs_new_context {
            log::warn!("GPU device lost, recreating context...");

            // Drop old resources to release Arc<Device>/Arc<Queue> and GPU resources
            self.resources = None;
            *gpu_context.borrow_mut() = None;

            // Wait briefly for the GPU driver to stabilize, then try to
            // recreate the context without software renderers. If this fails
            // the caller should request another frame and retry — the real GPU
            // may need more time to come back (e.g. after suspend/resume).
            std::thread::sleep(std::time::Duration::from_millis(350));

            let instance = WgpuContext::instance(Box::new(window.clone()));
            let surface = create_surface(&instance, window_handle.as_raw())?;
            let new_context =
                WgpuContext::new_rejecting_software(instance, &surface, self.compositor_gpu)?;
            *gpu_context.borrow_mut() = Some(new_context);
            surface
        } else {
            let ctx_ref = gpu_context.borrow();
            let instance = &ctx_ref.as_ref().unwrap().instance;
            create_surface(instance, window_handle.as_raw())?
        };

        let config = WgpuSurfaceConfig {
            size: gpui::Size {
                width: gpui::DevicePixels(self.surface_config.width as i32),
                height: gpui::DevicePixels(self.surface_config.height as i32),
            },
            transparent: self.surface_config.alpha_mode != wgpu::CompositeAlphaMode::Opaque,
            preferred_present_mode: Some(self.surface_config.present_mode),
        };
        let gpu_context = Rc::clone(gpu_context);
        let ctx_ref = gpu_context.borrow();
        let context = ctx_ref.as_ref().expect("context should exist");

        self.resources = None;
        self.atlas.handle_device_lost(context);

        *self = Self::new_internal(
            Some(gpu_context.clone()),
            context,
            Some(surface),
            config,
            self.compositor_gpu,
            self.atlas.clone(),
        )?;

        log::info!("GPU recovery complete");
        Ok(())
    }
}

#[cfg(not(target_family = "wasm"))]
fn create_surface(
    instance: &wgpu::Instance,
    raw_window_handle: raw_window_handle::RawWindowHandle,
) -> anyhow::Result<wgpu::Surface<'static>> {
    unsafe {
        instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                // Fall back to the display handle already provided via InstanceDescriptor::display.
                raw_display_handle: None,
                raw_window_handle,
            })
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
}

struct RenderingParameters {
    path_sample_count: u32,
    gamma_ratios: [f32; 4],
    grayscale_enhanced_contrast: f32,
    subpixel_enhanced_contrast: f32,
}

impl RenderingParameters {
    fn new(adapter: &wgpu::Adapter, surface_format: wgpu::TextureFormat) -> Self {
        let format_features = adapter.get_texture_format_features(surface_format);
        let path_sample_count = [4, 2, 1]
            .into_iter()
            .find(|&n| format_features.flags.sample_count_supported(n))
            .unwrap_or(1);

        Self::from_env(path_sample_count)
    }

    /// Deterministic parameters for headless offscreen capture. Forces
    /// `path_sample_count = 1`: MSAA sample positions are backend-defined, and
    /// the render-parity contract forbids relying on backend-defined behavior,
    /// so cross-platform byte-identity (macOS Metal vs Linux Vulkan/lavapipe)
    /// requires single-sampled path rasterization. Gamma/contrast use the same
    /// env-driven defaults as the windowed path.
    fn headless() -> Self {
        Self::from_env(1)
    }

    fn from_env(path_sample_count: u32) -> Self {
        use std::env;

        let gamma = env::var("ZED_FONTS_GAMMA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.8_f32)
            .clamp(1.0, 2.2);
        let gamma_ratios = get_gamma_correction_ratios(gamma);

        let grayscale_enhanced_contrast = env::var("ZED_FONTS_GRAYSCALE_ENHANCED_CONTRAST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0_f32)
            .max(0.0);

        let subpixel_enhanced_contrast = env::var("ZED_FONTS_SUBPIXEL_ENHANCED_CONTRAST")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.5_f32)
            .max(0.0);

        Self {
            path_sample_count,
            gamma_ratios,
            grayscale_enhanced_contrast,
            subpixel_enhanced_contrast,
        }
    }
}

/// Convert tightly packed captured texels (no row padding) to 8-bit RGBA,
/// quantizing the float capture target with a single deterministic round so the
/// readback is byte-identical across GPU backends. 8-bit `Unorm` inputs are
/// already quantized and pass through (with a BGRA→RGBA swizzle when needed).
#[cfg(all(not(target_family = "wasm"), feature = "test-support"))]
fn quantize_headless_capture_to_rgba8(
    raw: &[u8],
    format: wgpu::TextureFormat,
) -> anyhow::Result<Vec<u8>> {
    match format {
        wgpu::TextureFormat::Rgba32Float => {
            let mut out = Vec::with_capacity(raw.len() / 4);
            for texel in raw.chunks_exact(16) {
                for channel in 0..4 {
                    let base = channel * 4;
                    let value = f32::from_le_bytes([
                        texel[base],
                        texel[base + 1],
                        texel[base + 2],
                        texel[base + 3],
                    ]);
                    out.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
            Ok(out)
        }
        wgpu::TextureFormat::Rgba16Float => {
            let mut out = Vec::with_capacity(raw.len() / 2);
            for texel in raw.chunks_exact(8) {
                for channel in 0..4 {
                    let bits = u16::from_le_bytes([texel[channel * 2], texel[channel * 2 + 1]]);
                    let value = half_bits_to_f32(bits);
                    out.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
            Ok(out)
        }
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => Ok(raw.to_vec()),
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
            let mut out = raw.to_vec();
            for px in out.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
            Ok(out)
        }
        other => anyhow::bail!("unsupported headless capture format {other:?}"),
    }
}

/// Decode an IEEE 754 binary16 (half) bit pattern to `f32` using only integer
/// arithmetic and `f32::from_bits`. Deterministic and identical across
/// platforms — no transcendental or `powi` dependence.
#[cfg(all(not(target_family = "wasm"), feature = "test-support"))]
fn half_bits_to_f32(bits: u16) -> f32 {
    let sign = ((bits as u32) & 0x8000) << 16;
    let exponent = ((bits >> 10) & 0x1f) as u32;
    let mantissa = (bits & 0x3ff) as u32;
    if exponent == 0 {
        if mantissa == 0 {
            // Signed zero.
            return f32::from_bits(sign);
        }
        // Subnormal half: normalize into a normal f32.
        let mut exp = -1i32;
        let mut mant = mantissa;
        loop {
            exp += 1;
            mant <<= 1;
            if mant & 0x400 != 0 {
                break;
            }
        }
        let f32_exponent = (127 - 15 - exp) as u32;
        f32::from_bits(sign | (f32_exponent << 23) | ((mant & 0x3ff) << 13))
    } else if exponent == 0x1f {
        // Inf / NaN.
        f32::from_bits(sign | 0x7f80_0000 | (mantissa << 13))
    } else {
        let f32_exponent = exponent + (127 - 15);
        f32::from_bits(sign | (f32_exponent << 23) | (mantissa << 13))
    }
}

/// Headless renderer for visual-regression capture: renders a GPUI scene to an
/// offscreen wgpu texture and reads it back as RGBA. This is the single canonical
/// cross-platform headless capture backend (macOS Metal, Linux Vulkan/lavapipe),
/// wired into `gpui_platform::current_headless_renderer`.
#[cfg(all(not(target_family = "wasm"), feature = "test-support"))]
pub struct WgpuHeadlessRenderer {
    renderer: WgpuRenderer,
}

#[cfg(all(not(target_family = "wasm"), feature = "test-support"))]
impl WgpuHeadlessRenderer {
    pub fn new() -> anyhow::Result<Self> {
        let context = WgpuContext::new_headless()?;
        let renderer = WgpuRenderer::new_headless(
            &context,
            Size {
                width: DevicePixels(1),
                height: DevicePixels(1),
            },
        )?;
        Ok(Self { renderer })
    }
}

#[cfg(all(not(target_family = "wasm"), feature = "test-support"))]
impl gpui::PlatformHeadlessRenderer for WgpuHeadlessRenderer {
    fn render_scene_to_image(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> anyhow::Result<image::RgbaImage> {
        self.renderer.render_scene_to_image(scene, size)
    }

    fn sprite_atlas(&self) -> Arc<dyn gpui::PlatformAtlas> {
        self.renderer.sprite_atlas().clone()
    }

    fn capture_backend(&self) -> gpui::SceneCaptureBackend {
        match self.renderer.adapter_backend() {
            wgpu::Backend::Metal => gpui::SceneCaptureBackend::Metal,
            wgpu::Backend::Vulkan => gpui::SceneCaptureBackend::Vulkan,
            wgpu::Backend::Dx12 => gpui::SceneCaptureBackend::Dx12,
            wgpu::Backend::Gl => gpui::SceneCaptureBackend::Gl,
            _ => gpui::SceneCaptureBackend::Other,
        }
    }
}
