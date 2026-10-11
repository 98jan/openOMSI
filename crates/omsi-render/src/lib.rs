//! wgpu renderer.

pub mod angle;
pub mod atmosphere;
pub mod clouds;
mod cookies;
pub use cookies::{Cookie, COOKIE_LAYERS};
mod gpu_memory;
mod passes;
mod device_selection;
use passes::{Encoders, FrameArgs, FrameEnv, PassTimers, StageClock};
use gpu_memory::adapter_vram_mb;
pub mod pipeline_cache;
mod pipelines;
mod puddles;
mod rt;
mod shaders;
mod srgb_encode;
mod triple;
mod frame_plan;
mod gpu;
mod lighting;
mod render;
mod resources;
mod scene;
mod sky;
mod surface;
mod targets;
mod view;
pub(crate) use frame_plan::*;
pub use gpu::*;
pub use lighting::*;
pub(crate) use render::*;
pub use resources::*;
pub use scene::*;
pub(crate) use shaders::*;
pub use sky::*;
pub use surface::*;
pub(crate) use targets::*;
pub use view::*;
pub use triple::{panel_width, ScreenView, TripleScreen};

#[derive(Default)]
struct ViewCulling {
    drawn: Vec<u64>,
    sizes: hashbrown::HashMap<[u64; 4], f32>,
    scratch: hashbrown::HashMap<[u64; 4], f32>,
}

use anyhow::{anyhow, Context, Result};
use glam::{DVec3, Mat4, Vec3, Vec4};
use omsi_geometry::MeshData;
use std::collections::HashMap;
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    cam_pos: [f32; 4],
    /// World-space render origin modulo 1000 m (the procedural patterns' period), added back
    /// for procedural patterns that must not jump when the floating origin moves to the next
    /// 100 m cell.
    world_origin: [f32; 4],
    sun_dir: [f32; 4],
    ambient: [f32; 4],
    fog: [f32; 4],
    sun_color: [f32; 4],
    sky_color: [f32; 4],
    light_grid: [f32; 4],
    sky: [f32; 4],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    clouds: [f32; 4],
    light_view_proj: [[f32; 4]; 4],
    light_view_proj_far: [[f32; 4]; 4],
    shadow: [f32; 4],
    /// x enhanced graphics (0/1), y seconds since start, z sun ndc x, w sun ndc y
    post: [f32; 4],
    /// The player's vehicle as a box the weather does not reach into: origin (render-origin
    /// relative) and sin(heading); cos(heading) and the half extents; the box centre offset
    /// and whether there is one.
    inside_a: [f32; 4],
    inside_b: [f32; 4],
    inside_c: [f32; 4],
    /// x procedural detail texturing (0/1), y soft shadow penumbra (0/1), z aerial
    /// perspective strength, w the close shadow cascade's half range
    flags: [f32; 4],
    /// The close shadow cascade (right half of the near map).
    light_view_proj_close: [[f32; 4]; 4],
    /// The player's vehicle's velocity (m/s, world) and 1: the airstream the rain on its
    /// glass meets (see `Lighting::glass_wind`).
    wind: [f32; 4],
    /// Enhanced: the street lamps' shadow maps (`LAMP_SHADOWS` tiles under the far map),
    /// and the light indices they belong to (-1: none).
    lamp_view_proj: [[[f32; 4]; 4]; 4],
    lamp_shadow: [f32; 4],
    /// Windy trees: xy the weather's wind (m/s, world; 0 with the setting off), zw how far
    /// the air has carried the gusts since the start (m, modulo the shaders' PATTERN_PERIOD).
    tree_wind: [f32; 4],
    /// The rear section of the player's articulated vehicle as `inside_*` (w of the third
    /// 0 without one): the weather stays out of it as well (#1967: it snowed and rained in
    /// the back of an articulated bus). Last, so that the shaders that do not read it can
    /// leave it out of their copy of this struct.
    inside2_a: [f32; 4],
    inside2_b: [f32; 4],
    inside2_c: [f32; 4],
    /// Enhanced: the field each light cookie layer covers (`cookies::CookieAtlas::fields`).
    /// After the rest: the shaders that light nothing by a cookie leave it out.
    cookies: [[f32; 4]; COOKIE_LAYERS],
}

/// The period the sky's cloud patterns repeat with (m): 5 x the cloud field (14 km), 8 x
/// its billow detail (8.75 km) and 28 x the high layer's (2.5 km) - the render origin is
/// taken modulo this for them, which keeps centimetres of precision in 32 bits.
const CLOUD_ORIGIN_PERIOD: f64 = 70000.0;

/// The enhanced path's post passes (post.wgsl `PostParams`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniform {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
}

/// The enhanced lighting (enhanced_common.wgsl `Enhanced`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct EnhancedUniform {
    exposure: [f32; 4],
    sun: [f32; 4],
    sh: [[f32; 4]; 9],
    ground: [f32; 4],
    fog: [f32; 4],
    fog_color: [f32; 4],
    weather: [f32; 4],
    lights: [f32; 4],
    sun_disc: [f32; 4],
    debug: [f32; 4],
    /// xyz where the sky cube was drawn from, relative to the camera (the dome looks the
    /// clouds up through it with the parallax taken out); w the clouds' march steps
    /// (`Lighting::low_clouds`)
    eye: [f32; 4],
    /// x how bright an LED panel's dots burn (`Lighting::led_glow`), y how much of the
    /// mip chain an LED panel is held at (`Lighting::led_mips`)
    led: [f32; 4],
    /// xyz towards the moon, w its angular radius
    moon: [f32; 4],
    /// rgb the moon disc's irradiance at the ground before the clouds, w how much of the
    /// starry sky shows (the city's lamps and the haze wash the faint stars out)
    moon_disc: [f32; 4],
    /// rgb the sun's irradiance at the clouds' heights (`atmosphere::CLOUD_SUN_HEIGHTS`)
    cloud_sun: [[f32; 4]; 4],
    /// rgb the moonlight on a surface facing the moon (after the clouds), w 1 while the
    /// shadow maps are the moon's (`Lighting::casts_moon_shadows`)
    moon_light: [f32; 4],
    /// `Lighting::condensation`
    condensation: [f32; 4],
}

/// Steps of the enhanced clouds' march through their layer (sky_enhanced.wgsl
/// `cloud_layer`), and with the low cloud quality (`Lighting::low_clouds`).
const CLOUD_STEPS: u32 = 56;
const CLOUD_STEPS_LOW: u32 = 20;

/// Face size of the enhanced sky cube (see `Probe::cube_view`): about as many texels per
/// degree as a 1600-pixel-wide picture has pixels at half its size (at 512 the clouds'
/// edges stood in blocks of three or four pixels).
const SKY_CUBE_SIZE: u32 = 1024;
/// Redraw rounds of a sky cube face: each starts the clouds' steps elsewhere, and the
/// rounds are averaged (all of them at once for a picture on its own or a new sky).
const SKY_CUBE_ROUNDS: u32 = 8;
/// The old picture's share when a face is redrawn in the window.
const SKY_CUBE_HISTORY: f64 = 0.8;
/// A face is redrawn every this many window frames: the clouds drift by a pixel of the
/// cube in seconds, and a face a frame (a 32-step march of 1024x1024 texels) cost 2.9 ms of
/// every enhanced frame on an M4.
const SKY_CUBE_EVERY: u32 = 4;

const PROBE_SIZE: u32 = 64;
const PROBE_MIPS: u32 = 6;
/// Glow levels of the enhanced post path.
const GLOW_LEVELS: usize = 6;
/// The illuminance a light's core gives (maplight colour 1, in the sky model's units:
/// 55 lux, so the street under a lamp gets its 15-25 lux).
const LAMP_E: f32 = 0.0055;
/// Illuminance of a bus saloon's lamps on the seats and the floor (300 lux).
const CABIN_E: f32 = 0.03;
/// A lit window's radiance at night.
const WINDOW_RADIANCE: f32 = 0.0022;
/// The metering (see `meter_tuning`).
const METER_GAIN: f32 = 0.4;
const METER_TARGET: f32 = -2.84;
// (how far the eye adapts to what it looks at beyond what the light model knows: the sun
// in view, a dark cab or an underpass by day - two stops and a half and more for the
// eye; at the metering's gain a snow field still comes out darkened by well under a stop)
const METER_DARKEN: f32 = 2.0;
const METER_BRIGHTEN: f32 = 1.6;
/// The tone curve's contrast about mid grey by day and at night (see `tone_contrast`).
const TONE_CONTRAST_DAY: f32 = 1.22;
const TONE_CONTRAST_NIGHT: f32 = 0.94;
/// How far night vision takes the colour out of a dark scene (post.wgsl `night_vision`).
const NIGHT_VISION: f32 = 0.3;
/// The sun's angular radius as drawn (a little larger than the real 0.27°).
const SUN_RADIUS: f32 = 0.0065;
/// The moon's, as drawn (the real one is as large as the sun's; a little more, as the
/// eye sees it).
const MOON_RADIUS: f32 = 0.0062;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FogLampUniform {
    inv_view_proj: [[f32; 4]; 4],
    size: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SsaoUniform {
    inv_proj: [[f32; 4]; 4],
    params: [f32; 4],
    /// xy: the projection's off-centre shift (0 for the window's own symmetric frustum;
    /// a headset eye or a triple screen's side panel has one)
    shift: [f32; 4],
}

pub type MeshId = usize;
pub type TextureId = usize;
pub type MaterialId = usize;

/// The enhanced path's post pipelines (post.wgsl).
struct PostPipelines {
    down_first: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
    meter: wgpu::RenderPipeline,
    adapt: wgpu::RenderPipeline,
    /// Straight into the target, or gamma-encoded for FXAA.
    tonemap: wgpu::RenderPipeline,
    tonemap_encoded: wgpu::RenderPipeline,
    fxaa: wgpu::RenderPipeline,
}

/// The pipelines of the main pass for one colour target format: the swap chain's, and
/// the high-range one of the enhanced path.
struct PassPipelines {
    /// Single-sampled films, drawn after resolving the scene and its reflections.
    rain_pipelines: Vec<wgpu::RenderPipeline>,
    /// Indexed by `pipe_code`: 4 depth/blend kinds x culled x depth-biased.
    pipelines: Vec<wgpu::RenderPipeline>,
    corona_pipeline: wgpu::RenderPipeline,
    /// Smoke particles (`[smoke]`): the corona sprite alpha-blended with the smoke texture.
    smoke_pipeline: wgpu::RenderPipeline,
    /// The snowfall (snow.wgsl), opaque over the scene (premultiplied); none with
    /// `basic_pipelines`.
    snow_pipeline: Option<wgpu::RenderPipeline>,
    sky_pipeline: wgpu::RenderPipeline,
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter_name: String,
    camera_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    pass: PassPipelines,
    hdr_pass: Option<PassPipelines>,
    /// The enhanced sky tone-mapped into the plain pass's target: the sky of the mirrors
    /// of an Enhanced picture (sky_enhanced.wgsl `fs_enhanced_mirror`).
    sky_mirror_pipeline: Option<wgpu::RenderPipeline>,
    reflection_pass: Option<PassPipelines>,
    corona_bind_group: wgpu::BindGroup,
    /// The snowfall's parameters (snow.wgsl `SnowParams`) and their bind group.
    snow_buf: wgpu::Buffer,
    snow_bind_group: wgpu::BindGroup,
    /// The smoke texture (`Texture/rauch.tga`, see [`Renderer::set_smoke_texture`]).
    smoke_bind_group: wgpu::BindGroup,
    /// The coronas' pictures besides the standard glow (index = `Corona::texture`; entry 0
    /// unused).
    corona_textures: Vec<Option<wgpu::BindGroup>>,
    /// The light cookies' pictures and the frame's layers of them.
    cookies: cookies::CookieAtlas,
    corona_layout: wgpu::BindGroupLayout,
    corona_sampler: wgpu::Sampler,
    sky_layout: wgpu::BindGroupLayout,
    sky_sampler: wgpu::Sampler,
    /// The classic sky's weather (`VanillaSkyUniform`, sky bind group binding 10).
    vanilla_sky_buf: wgpu::Buffer,
    /// The enhanced clouds' noise (clouds.rs): the shape map, the detail volume and their
    /// repeating, mip-mapped sampler (sky bind group bindings 6-8).
    cloud_shape_view: wgpu::TextureView,
    cloud_detail_view: wgpu::TextureView,
    cloud_sampler: wgpu::Sampler,
    /// The shape map's second level (RGBA8), for the clouds' shadow on the street, and how
    /// much of the sun the clouds let through over the camera now (smoothed).
    cloud_shape_cpu: Vec<u8>,
    cloud_sun: Option<f32>,
    sky_mesh: (wgpu::Buffer, wgpu::Buffer, u32),
    overlay_pipeline: wgpu::RenderPipeline,
    overlay_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    camera_buf: wgpu::Buffer,
    white_texture: GpuTexture,
    black_texture: GpuTexture,
    /// A tangent-space normal pointing straight out (the PBR normal map's stand-in).
    flat_normal_texture: GpuTexture,
    format: wgpu::TextureFormat,
    depth: Option<(wgpu::Texture, wgpu::TextureView, u32, u32)>,
    /// Multisampled colour and depth targets per size (the window, each mirror).
    msaa_targets: HashMap<(u32, u32), (Option<wgpu::TextureView>, wgpu::TextureView)>,
    /// Screen-space ambient occlusion: depth prepass and AO textures of the window's size.
    ao: Option<AoTargets>,
    ao_sampler: wgpu::Sampler,
    ao_layout: wgpu::BindGroupLayout,
    ao_buf: wgpu::Buffer,
    /// Depth prepass pipelines (plain, alpha-tested; each two-sided and culled),
    /// single-sampled, camera projection.
    /// Plain opaque, alpha-tested, and the opaque portions of blended transmap materials;
    /// each has a two-sided and a culled variant.
    prepass_pipelines: [wgpu::RenderPipeline; 6],
    /// The same, multisampled: the enhanced main pass's own depth laid first (see
    /// `render_inner`), so that its costly shading runs once per visible surface.
    prepass_msaa_pipelines: Option<[wgpu::RenderPipeline; 6]>,
    /// Depth-only pipelines compatible with the existing HDR colour pass.
    presurface_msaa_pipelines: Option<[wgpu::RenderPipeline; 4]>,
    /// Ambient occlusion and its blur; none on OpenGL (GLES), whose shading language cannot
    /// read a depth texture texel by texel - the pipelines failed there, AO off or not (#422).
    ssao_pipeline: Option<wgpu::RenderPipeline>,
    blur_pipeline: Option<wgpu::RenderPipeline>,
    /// Enhanced: the lamps' light the fog scatters, added over the drawn picture (none on
    /// OpenGL and without storage buffers), its bind group layout and parameters.
    fog_lamps_pipeline: Option<[wgpu::RenderPipeline; 2]>,
    fog_lamps_layout: wgpu::BindGroupLayout,
    fog_lamps_buf: wgpu::Buffer,
    shadow_view: wgpu::TextureView,
    shadow_view_far: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    shadow_layout: wgpu::BindGroupLayout,
    /// [near opaque, near alpha-tested, far opaque, far alpha-tested, close opaque, close
    /// alpha-tested], then the street lamps' (6 + 2 k + kind; not with `basic_pipelines`)
    shadow_pipelines: Vec<wgpu::RenderPipeline>,
    /// The settings this renderer was built with.
    pub options: RenderOptions,
    /// Enhanced path: the HDR targets per size, the post pipelines and their resources.
    hdr_targets: HashMap<(u32, u32), HdrTargets>,
    puddles: Option<puddles::Pipelines>,
    post: PostPipelines,
    post_layout: wgpu::BindGroupLayout,
    post_buf: wgpu::Buffer,
    post_sampler: wgpu::Sampler,
    /// The metered mean log luminance (1x1), and the adapted value in two textures drawn
    /// into by turns (`adapt_front` is the current one), with their bind groups.
    meter_view: wgpu::TextureView,
    adapt_views: [wgpu::TextureView; 2],
    adapt_bg: [wgpu::BindGroup; 2],
    adapt_front: usize,
    /// OMSI_DEBUG_EXPOSURE: the adapted metering read back now and then, and logged.
    exposure_log: Option<ExposureLog>,
    /// Enhanced lighting: its uniform, the sky table, the sampler both use, the reflection
    /// probe, the sky last computed and the exposure as it follows it.
    enh_buf: wgpu::Buffer,
    sky_lut: wgpu::Texture,
    sky_lut_view: wgpu::TextureView,
    lin_sampler: wgpu::Sampler,
    probe: Option<Probe>,
    sky_state: Option<atmosphere::SkyState>,
    /// How much the lamps round the camera light the night sky, relative to a city's
    /// (`lamp_sky_glow`), as the sky is computed with it.
    city_glow: Option<f32>,
    /// The lamps' and headlights' light on what the camera looks at (`view_lamp_light`),
    /// which the eye adapts to by night.
    view_lamps: Option<f32>,
    /// A sky being computed on a helper thread, for this input.
    sky_job: Option<(
        atmosphere::SkyInput,
        std::sync::mpsc::Receiver<atmosphere::SkyState>,
    )>,
    /// The pre-exposure (natural log) as it follows the sky's.
    exposure: Option<f32>,
    /// The projection's width over height while a picture is drawn into a texture (see
    /// `render_to_texture`), whatever the texture's own shape.
    texture_aspect: Option<f32>,
    last_frame: Option<std::time::Instant>,
    /// The next frame stands alone (an offscreen picture): the exposure is there at once.
    pub instant_exposure: bool,
    /// Overlay pipeline without multisampling, for drawing the HUD after the post pass.
    overlay_pipeline_1x: wgpu::RenderPipeline,
    xr_ui_pipeline: wgpu::RenderPipeline,
    started: std::time::Instant,
    /// Windy trees: how far the wind has carried its gusts (m, modulo 1000) and when.
    tree_gust_drift: std::cell::Cell<(glam::DVec2, f64)>,
    /// `[matl_texadress_clamp]` (and border, mirror-once) and `[matl_texadress_mirror]`.
    clamp_sampler: wgpu::Sampler,
    mirror_sampler: wgpu::Sampler,
    /// The addressing of the next material's textures (`[matl_texadress_*]`).
    pub address_next: std::cell::Cell<TexAddressing>,
    /// The next material is lit at night by the tile light maps (`[LightMapMapping]`, the
    /// splines) instead of the map's lamps.
    pub light_map_next: std::cell::Cell<bool>,
    /// The tile light maps (`.map.LM.bmp`) of the 5x5 tiles around the camera, north up, and
    /// where that square lies (world x, y of its south-west corner, its side in metres).
    lm_atlas: wgpu::Texture,
    lm_atlas_view: wgpu::TextureView,
    lm_uniform: wgpu::Buffer,
    lm_place: std::cell::Cell<(f64, f64, f64)>,
    /// Mipmap generation on the GPU (a blit per level).
    mip_pipeline: wgpu::RenderPipeline,
    mip_layout: wgpu::BindGroupLayout,
    mip_sampler: wgpu::Sampler,
    /// Set by the device's error handler when something failed with multisampling on.
    gpu_error: Arc<std::sync::atomic::AtomicBool>,
    /// A GPU error while Enhanced+ traces rays: its ray tracing is left out from the next
    /// frame on (a frame whose commands the device refused shows nothing at all - left to
    /// go on, the picture stood still on the loading screen while the game ran behind it).
    rt_error: Arc<std::sync::atomic::AtomicBool>,
    /// The headset's pictures: the heading (degrees) the `[matl_envmap]` sphere maps are
    /// laid out by instead of each eye's view (see `set_env_heading`).
    env_heading: std::cell::Cell<Option<f32>>,
    /// The card ran out of memory since the last `take_out_of_memory`.
    out_of_memory: Arc<std::sync::atomic::AtomicBool>,
    /// Why the graphics device was lost (a driver reset, the card removed), once it was.
    device_lost: Arc<std::sync::Mutex<Option<String>>>,
    /// Render scale: the pipeline that scales the 3D picture up to the window, its
    /// parameters, and the smaller targets per size (with their bind groups).
    upscale_pipeline: wgpu::RenderPipeline,
    copy_pipeline: wgpu::RenderPipeline,
    /// Triple screen: a panel's picture copied 1:1 to its place in the window.
    panel_pipeline: wgpu::RenderPipeline,
    upscale_layout: wgpu::BindGroupLayout,
    upscale_buf: wgpu::Buffer,
    scale_targets: HashMap<(u32, u32), (wgpu::TextureView, wgpu::BindGroup)>,
    triple_targets: Option<((u32, u32), Vec<(wgpu::TextureView, wgpu::BindGroup)>)>,
    triple_culling: [ViewCulling; 3],
    /// Full-resolution current scene before rain films, in its original colour format.
    glass_picture: Option<wgpu::TextureView>,
    /// When each size of the size-keyed targets (scale, MSAA, HDR) was last asked for.
    target_use: HashMap<(u32, u32), std::time::Instant>,
    /// The game's frame-rate governor on top of the render scale (1 = none; see
    /// `set_dynamic_scale`).
    dynamic_scale: std::cell::Cell<f32>,
    /// `OMSI_DEBUG_FLICKER`: which near instances in view were drawn last frame.
    flicker: std::cell::RefCell<HashMap<usize, bool>>,
    /// The window's main-pass bundles as last recorded, with what they were made of (see
    /// `record_main_bundles`).
    main_bundles: std::cell::RefCell<Option<passes::batch::MainBundles>>,
    /// Instances the main view drew last frame (a bit each): they keep being drawn a little
    /// past the size and distance limits, so that an object right at a limit does not pop
    /// in and out as the camera sways.
    cull_drawn: std::cell::RefCell<Vec<u64>>,
    /// The screen size each object (origin and radius) was judged by in the main view's last
    /// frame: all its meshes and LOD levels take the same one, so exactly one level of an
    /// object is drawn and it does not flip between levels with the view's jitter.
    object_sizes: std::cell::RefCell<hashbrown::HashMap<[u64; 4], f32>>,
    /// Cleared and reused as the next main view's object-size history.
    object_sizes_scratch: std::cell::RefCell<hashbrown::HashMap<[u64; 4], f32>>,
    /// The far shadow cascade as last drawn: its light matrix, frames since, the render
    /// origin and the sun it was drawn for.
    shadow_far_cache: std::cell::Cell<(Mat4, u32, DVec3, Vec3)>,
    /// The same for the near cascade, drawn every other frame (see `render_inner`).
    shadow_near_cache: std::cell::Cell<(Mat4, u32, DVec3, Vec3)>,
    /// Shadow atlas matrices from the first OpenXR eye, reused by the second eye.
    xr_shadow_cache: std::cell::Cell<Option<(DVec3, Vec3, Mat4, Mat4, Mat4)>>,
    /// Depth-only pipeline that fills its viewport with the far depth: clears the close
    /// cascade's part of the atlas when the near part is kept from the frame before.
    shadow_clear_pipeline: wgpu::RenderPipeline,
    /// Sort the blended draws by origin distance alone, as before the camera-enclosing
    /// objects were drawn last (only for before/after pictures, `OMSI_BLEND_AB`).
    pub blend_by_origin: bool,
    /// Draw the models' `[isshadow]` shadow blobs (see [`RenderOptions::shadow_blobs`]).
    /// Settable while the game runs, so the graphics list can switch it off at once.
    pub shadow_blobs: bool,
    /// GPU time per pass (OMSI_GPU_TIMERS, when the device has timestamp queries): the
    /// mirrors and the window's picture apart, each timed on its own.
    gpu_timers: [Option<GpuTimers>; 2],
    /// Seconds spent per stage of `render_inner` since start (OMSI_PROFILE).
    pub stats: std::cell::RefCell<std::collections::BTreeMap<&'static str, f64>>,
    /// What the window's pictures drew since start, summed (OMSI_PROFILE): instances that
    /// passed the culling and draws per pass.
    pub counts: std::cell::RefCell<std::collections::BTreeMap<&'static str, f64>>,
    profiling: bool,
    draw_audit_at: std::time::Instant,
    /// Reuse a small set of encoding workers instead of creating OS threads for each
    /// main/mirror picture. Keep these separate from simulation's worker queue.
    encoding_pool: Option<rayon::ThreadPool>,
    _device_poller: Option<DevicePoller>,
    /// Vertex data of changed meshes (skinned people, the driver) waiting for the next
    /// picture: (mesh, bytes). Written with one staging buffer and a copy each at the start
    /// of the frame - a `write_buffer` per mesh made wgpu create a staging buffer for every
    /// one of them, forty a frame with a crowd at a stop.
    pending_meshes: std::cell::RefCell<Vec<(MeshId, Vec<u8>)>>,
    /// What a freed mesh and a freed material hold (see `free_mesh`), made once.
    freed: std::cell::OnceCell<Freed>,
    /// Enhanced+: the ray tracer (on a device with ray queries, see `RenderOptions::ray_tracing`).
    rt: Option<rt::RayTracer>,
    /// Meshes share pages of buffers (the adapter draws with a base vertex).
    mesh_pages: bool,
}

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Samples per pixel of the scene passes when nothing else is asked for (the shadow maps
/// stay single-sampled). Four samples take the staircase off every edge; the multisampled
/// colour target resolves into the window or the mirror texture at the end of the main pass.
pub const MSAA: u32 = 4;
/// Sun shadow map resolution when nothing else is asked for.
pub const SHADOW_SIZE: u32 = 2048;

/// What the renderer is built with: the user's graphics settings.
#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    /// Samples per pixel: 1 (off), 2, 4 or 8.
    pub msaa: u32,
    /// Anisotropic filtering: 1 (off) .. 16.
    pub anisotropy: u16,
    /// Sun shadow map size per cascade (1024, 2048, 4096).
    pub shadow_size: u32,
    /// Screen-space ambient occlusion.
    pub ssao: bool,
    /// The 3D picture of the window drawn at this fraction of its size and scaled up
    /// (0.5..1); 0 = automatic: full size up to `AUTO_SCALE_PIXELS`, smaller above.
    pub render_scale: f32,
    /// Uncompressed texture files (BMP, TGA, JPG) are compressed to BC1/BC3 on loading
    /// where the device takes block formats and the result is close to the picture
    /// (DXT files always go up as blocks there).
    pub compress_textures: bool,
    /// DXT/BC textures may stay compressed on the GPU (where it takes them); off, the
    /// device is opened without block formats and every texture goes up as RGBA, as with
    /// `OMSI_NO_BC=1` (the settings' `gpu_texture_compression`, for a driver that
    /// mishandles them).
    pub gpu_texture_compression: bool,
    /// FXAA over the enhanced path's tone-mapped picture.
    pub fxaa: bool,
    /// The original's `performance_minObjSize` (see `Lighting::min_obj_size`, which may
    /// raise it for one picture, as the mirrors do).
    pub min_obj_size: f32,
    /// The original's `performance_maxObjDist` (m): objects farther away are not drawn
    /// unless they say `[noDistanceCheck]`. 0 = no limit.
    pub max_obj_dist: f32,
    /// Only the meshes the models mark `[shadow]` cast sun shadows, as in OMSI 2 (else every
    /// solid mesh does).
    pub omsi_shadow_casters: bool,
    /// Draw the models' `[isshadow]` shadow meshes - OMSI's flat blob under a vehicle,
    /// standing in for the sky light the body keeps off the road (see [`Instance::blob`]).
    /// Off, the sun shadow map is all the shading under a vehicle, and the blob (which
    /// OMSI draws whatever the depth) cannot be seen at all.
    pub shadow_blobs: bool,
    /// The materials' reflection maps (`[matl_envmap]`: the shine of paint, chrome and
    /// glass). Off, nothing mirrors the sky photo - some players find it too strong.
    pub reflections: bool,
    /// Enhanced graphics are not asked for: a phone or OpenGL then leaves their pipelines,
    /// sky shader, cloud noise and probe out (compiling the ray-marched clouds' sky killed
    /// Mali and Adreno drivers before the first frame, #364, #333, #316, #371). Asked for,
    /// they are built where the texture layout supports them. Native desktop backends
    /// keep them available for switching modes in-game.
    pub no_enhanced: bool,
    /// The launcher's preview (the showroom), which always draws Vanilla+ (see
    /// `showroom::lighting_for`): no enhanced pipelines, reflection probe or cloud noise
    /// are made for it, on any device.
    pub preview_only: bool,
    /// Enhanced+: the enhanced path with ray-traced sun shadows, ambient occlusion and
    /// reflections, where the device can trace rays (hardware ray queries); elsewhere the
    /// enhanced picture as it is.
    pub ray_tracing: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            msaa: MSAA,
            anisotropy: 8,
            shadow_size: SHADOW_SIZE,
            ssao: true,
            render_scale: 0.0,
            compress_textures: true,
            gpu_texture_compression: true,
            fxaa: true,
            min_obj_size: 0.013,
            max_obj_dist: 0.0,
            omsi_shadow_casters: false,
            shadow_blobs: true,
            reflections: true,
            no_enhanced: false,
            preview_only: false,
            ray_tracing: false,
        }
    }
}

/// Automatic render scale: a window of up to this many pixels is drawn at full size (the
/// default 1600x900 window and a 2560x1080 screen are); a bigger one - a Retina window has
/// four times the pixels of its size in points - gets a 3D picture of about this many
/// pixels, scaled up. The HUD is always drawn at full size. Elsewhere (a desktop card on a
/// 1440p or 4K screen) the picture is drawn at full size up to 4K: scaled down to 2.8
/// million pixels, a 4K screen showed a picture of 58 % its size, and the enhanced
/// graphics looked like textures of low quality; the frame-rate governor still steps down
/// on a card that cannot keep up.
pub const AUTO_SCALE_PIXELS: f32 = if cfg!(target_os = "macos") || cfg!(target_os = "android") { 2_800_000.0 } else { 8_400_000.0 };

/// Interior lamps one mesh may be lit by (OMSI: four; a model may list more in its
/// `[illumination_interior]`), and the step of the lamp code sent to the shaders (`first *
/// stride + count`, exact in the f32 it travels in for a quarter of a million lamp slots).
pub const MAX_LAMPS_PER_MESH: u32 = 63;
pub const LAMP_CODE_STRIDE: u32 = 64;

/// The enhanced pass's second target: r is 1 where the bus's own screens are
/// (`MaterialExtra::screen`), 0 elsewhere - the glow takes no light from them and FXAA
/// passes them through; g is 1 on an LED panel's own dots (`MaterialExtra::led`), which the
/// glow's source keeps and multiplies up (see `post.wgsl`). b carries the puddle's
/// reflected-light weight; a is blend coverage. Sharing this attachment avoids another
/// geometry pass or a full normal/material buffer just for water.
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Enhanced+: the enhanced pass's third and fourth targets, what the ray-traced reflections
/// start from (rt.wgsl): the surface's normal (xyz) and how much it reflects (w), and its
/// view depth and roughness (rg) - all weighted by that reflection (premultiplied), so that
/// a pane blended over a wet road leaves the stronger reflection's surface in them.
const GBUF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const AUX_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;
/// The renderer is built for Enhanced+: its HDR pipelines and targets have the two above.
static RT_GBUF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn rt_gbuf() -> bool {
    RT_GBUF.load(std::sync::atomic::Ordering::Relaxed)
}
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The colour targets of a pipeline drawing into `format`: Enhanced and classic puddle
/// shading draw into `HDR_FORMAT` with the screen mask beside it,
/// written by the scene's own shader only (`mask`), coverage-blended where the colour is.
fn color_targets(format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>, write: wgpu::ColorWrites, mask: bool, gbuf: bool) -> Vec<Option<wgpu::ColorTargetState>> {
    let mut v = vec![Some(wgpu::ColorTargetState { format, blend, write_mask: write })];
    if format == HDR_FORMAT {
        v.push(Some(wgpu::ColorTargetState {
            format: MASK_FORMAT,
            blend: blend.map(|_| wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent::REPLACE,
            }),
            write_mask: if mask { wgpu::ColorWrites::ALL } else { wgpu::ColorWrites::empty() },
        }));
        if rt_gbuf() {
            // (premultiplied by the reflection's weight, the shader's alpha)
            let over = blend.map(|_| wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            });
            let write = if gbuf { wgpu::ColorWrites::ALL } else { wgpu::ColorWrites::empty() };
            v.push(Some(wgpu::ColorTargetState { format: GBUF_FORMAT, blend: over, write_mask: write }));
            v.push(Some(wgpu::ColorTargetState { format: AUX_FORMAT, blend: over, write_mask: write }));
        }
    }
    v
}
/// Half size of the area around the camera covered by the near shadow cascade (m).
pub const SHADOW_RANGE: f32 = 140.0;
/// Half size of the far cascade (m): coarser, but reaches the whole visible street.
pub const SHADOW_RANGE_FAR: f32 = 700.0;
/// Half size of the close cascade around the camera (m). The near cascade's texel is
/// 280 m / 2048 = 14 cm, and a moving bus's shadow edge stepped from texel to texel: the
/// shadow trembled while driving. The close map (the right half of the near map's atlas)
/// has 3 cm texels for the bus and everything next to it.
pub const SHADOW_RANGE_CLOSE: f32 = 32.0;
/// The close cascade's map is drawn no bigger than this (texels a side), whatever the shadow
/// setting: 3 cm texels. At the 4096 setting it had been 1.6 cm, and drawing it was a third
/// of the shadow pass's 4.6 ms.
const SHADOW_CLOSE_MAX: u32 = 2048;

impl Renderer {

    fn main_pass(&self, enhanced: bool, reflections: bool) -> &PassPipelines {
        if enhanced { self.hdr_pass.as_ref().unwrap() }
        else if reflections { self.reflection_pass.as_ref().unwrap() }
        else { &self.pass }
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// The fraction of a window of this size the 3D picture is drawn at (`render_scale`).
    pub fn scene_scale(&self, width: u32, height: u32) -> f32 {
        let s = scene_scale_for(self.options.render_scale, width, height);
        // (a fixed render scale is the player's choice: the governor works on automatic)
        if self.options.render_scale > 0.0 {
            s
        } else {
            (s * self.dynamic_scale.get()).clamp(0.5, 1.0)
        }
    }

    /// Draw the 3D picture at this fraction of what the render scale says (0.6..1): the
    /// game lowers it while a slow graphics chip cannot keep the frame rate up, and raises
    /// it again when there is room. Steps of a twentieth, so that the few sizes it takes
    /// keep their render targets.
    pub fn set_dynamic_scale(&self, s: f32) {
        // (1, 0.85, 0.7 or 0.55 - the last for a phone's chip that is still too slow: see
        // the game's governor)
        let s = s.clamp(0.55, 1.0);
        let level = [1.0f32, 0.85, 0.7, 0.55].into_iter().min_by(|a, b| (a - s).abs().total_cmp(&(b - s).abs())).unwrap_or(1.0);
        self.dynamic_scale.set(level);
    }

    pub fn dynamic_scale(&self) -> f32 {
        self.dynamic_scale.get()
    }

    /// The size the 3D scene is drawn at for a window of this size.
    pub fn scene_size(&self, width: u32, height: u32) -> (u32, u32) {
        let s = self.scene_scale(width, height);
        if s >= 0.999 {
            return (width, height);
        }
        (
            ((width as f32 * s).round() as u32).max(1),
            ((height as f32 * s).round() as u32).max(1),
        )
    }
}

/// The render scale for a picture of this size: the requested one (0.5..1), or with 0
/// (automatic) full size up to `AUTO_SCALE_PIXELS` and that many pixels above it.
/// A requested one keeps to the same budget on a Mac or a phone: the Low preset's fixed
/// 0.75 of a Retina window drew more pixels than the budget allows (3.24 of 2.8 million
/// on a 3200x1800 window), and the automatic scale would never have drawn that many.
fn scene_scale_for(requested: f32, width: u32, height: u32) -> f32 {
    let pixels = width as f32 * height as f32;
    if requested > 0.0 {
        let requested = requested.clamp(0.5, 1.0);
        if (cfg!(target_os = "macos") || cfg!(target_os = "android")) && pixels > AUTO_SCALE_PIXELS {
            return requested.min((AUTO_SCALE_PIXELS / pixels).sqrt().clamp(0.5, 1.0));
        }
        return requested;
    }
    if pixels <= AUTO_SCALE_PIXELS {
        1.0
    } else {
        (AUTO_SCALE_PIXELS / pixels).sqrt().clamp(0.5, 1.0)
    }
}

/// Is `p` inside a vehicle's `[boundingbox]` (origin, heading in degrees, w l h cx cy cz)?
/// The same box the shader keeps the weather out of, without its margins.
fn point_in_vehicle_box(p: DVec3, (origin, heading, bb): &(DVec3, f64, [f32; 6])) -> bool {
    let d = (p - *origin).as_vec3();
    let (sh, ch) = (*heading as f32).to_radians().sin_cos();
    let x = d.x * ch - d.y * sh - bb[3];
    let y = d.x * sh + d.y * ch - bb[4];
    let z = d.z - bb[5];
    x.abs() < bb[0] * 0.5 && y.abs() < bb[1] * 0.5 && z.abs() < bb[2] * 0.5
}

/// An overlay's rectangle (physical pixels) moved onto whole pixels, its size kept. The
/// overlays are pictures drawn texel for pixel - a text, a plate - and the linear filter
/// blended every pixel of one placed between pixels with its neighbour: the interface's texts
/// were soft at every size whose layout fell between them (most but 100 %, and the timetable's
/// rows at that too). A line thinner than a pixel stays one pixel wide or high.
fn snap_rect(r: [f32; 4]) -> [f32; 4] {
    // (half up the same way left of the window as right of it: `round` goes away from zero,
    // and a rectangle across the left edge came out a pixel wider)
    let snap = |v: f32| (v + 0.5).floor();
    let (x0, y0) = (snap(r[0]), snap(r[1]));
    let x1 = if r[2] > r[0] { snap(r[2]).max(x0 + 1.0) } else { snap(r[2]) };
    let y1 = if r[3] > r[1] { snap(r[3]).max(y0 + 1.0) } else { snap(r[3]) };
    [x0, y0, x1, y1]
}

#[cfg(test)]
mod tests;

thread_local! {
    /// A panic on this thread now is caught and handled (see [`catching`]).
    static CATCHING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether a panic on this thread is being caught by the renderer (the game's panic hook
/// does not report it as the end of the game).
pub fn catching() -> bool {
    CATCHING.with(|c| c.get())
}

/// Run `f`; a panic in it (a driver wgpu cannot use, taking it down in a way it does not
/// turn into an error) is caught and gives None, and is not reported as the end of the game.
pub fn catch<R>(f: impl FnOnce() -> R) -> Option<R> {
    let was = CATCHING.with(|c| c.replace(true));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    CATCHING.with(|c| c.set(was));
    r.ok()
}
