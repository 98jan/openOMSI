//! GPU dedicated-memory detection for the rendering backends.
//! The OpenGL and ANGLE paths may omit PCI IDs even on a discrete GPU.

use super::*;

/// The card's own memory in MB where the system tells it: Windows, through DXGI, for
/// whichever backend draws; Linux, through the DRM driver's sysfs (amdgpu; not
/// NVIDIA's own driver, whose memory [`vulkan_vram_mb`] reads from Vulkan instead).
fn dedicated_vram_mb(info: &wgpu::AdapterInfo) -> Option<u64> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
        let f: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut i = 0;
        let mut name_match = None;
        let mut ambiguous_name = false;
        // ANGLE wraps the GPU name and embeds the PCI device ID as "(0x00006613)":
        // check both the name and that ID, when present.
        let angle_device = info.name
            .split("(0x")
            .nth(1)
            .and_then(|s| s.split(')').next())
            .and_then(|s| u32::from_str_radix(s, 16).ok());
        while let Ok(a) = f.EnumAdapters1(i) {
            i += 1;
            let Ok(d) = a.GetDesc1() else { continue };
            let mb = d.DedicatedVideoMemory as u64 >> 20;
            if mb == 0 {
                continue;
            }
            // OpenGL/ANGLE often report vendor/device as 0 even on a discrete GPU.
            // Prefer matching PCI IDs when they are available.
            if info.vendor != 0 && info.device != 0
                && d.VendorId == info.vendor && d.DeviceId == info.device
            {
                return Some(mb);
            }
            // DXGI still knows the card's name. Use it only if it identifies exactly
            // one adapter; never borrow another card's budget on a multi-GPU PC.
            let dxgi_name = String::from_utf16_lossy(&d.Description);
            let dxgi_name = dxgi_name.trim_end_matches('\0').trim();
            let angle_matches = info.name.starts_with("ANGLE (")
                && info.name.contains(dxgi_name)
                && angle_device == Some(d.DeviceId)
                && (info.vendor == 0 || info.vendor == d.VendorId);
            if (info.vendor == 0 || info.device == 0)
                && (dxgi_name.eq_ignore_ascii_case(info.name.trim()) || angle_matches)
            {
                ambiguous_name |= name_match.replace(mb).is_some();
            }
        }
        if ambiguous_name { None } else { name_match }
    }
    #[cfg(target_os = "linux")]
    {
        let hex = |p: std::path::PathBuf| {
            let t = std::fs::read_to_string(p).ok()?;
            u32::from_str_radix(t.trim().trim_start_matches("0x"), 16).ok()
        };
        for e in std::fs::read_dir("/sys/class/drm").ok()?.flatten() {
            // (card0, card1, ...; not their connectors, card1-DP-1)
            let name = e.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("card") || name.contains('-') {
                continue;
            }
            let dev = e.path().join("device");
            if hex(dev.join("vendor")) != Some(info.vendor) || hex(dev.join("device")) != Some(info.device) {
                continue;
            }
            let bytes = std::fs::read_to_string(dev.join("mem_info_vram_total"))
                .ok()
                .and_then(|t| t.trim().parse::<u64>().ok());
            if let Some(b) = bytes.filter(|b| *b > 0) {
                return Some(b >> 20);
            }
        }
        None
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = info;
        None
    }
}

/// The largest device-local memory heap of a Vulkan adapter (MB).
#[cfg(target_os = "linux")]
fn vulkan_vram_mb(adapter: &wgpu::Adapter) -> Option<u64> {
    // SAFETY: the adapter outlives the borrow, and only its memory properties are read
    let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }?;
    // SAFETY: the physical device belongs to this instance
    let props = unsafe { hal.shared_instance().raw_instance().get_physical_device_memory_properties(hal.raw_physical_device()) };
    props.memory_heaps[..props.memory_heap_count as usize]
        .iter()
        .filter(|h| h.flags.contains(ash::vk::MemoryHeapFlags::DEVICE_LOCAL))
        .map(|h| h.size >> 20)
        .max()
}

#[cfg(not(target_os = "linux"))]
fn vulkan_vram_mb(_adapter: &wgpu::Adapter) -> Option<u64> {
    None
}

/// The card's own memory in MB: wgpu's report on DirectX 12 (the adapter's DXGI
/// `DedicatedVideoMemory`, the number [`dedicated_vram_mb`] reads), else what the system
/// tells. (wgpu's Vulkan report sums every device-local heap where [`vulkan_vram_mb`] takes
/// the largest, and Metal's is the working set the system recommends, not the card's own
/// memory: those keep their own paths, so the texture budgets stay as they were.)
pub(super) fn adapter_vram_mb(adapter: &wgpu::Adapter, info: &wgpu::AdapterInfo, mem: Option<&wgpu::AdapterMemoryInfo>) -> Option<u64> {
    match mem {
        Some(m) if info.backend == wgpu::Backend::Dx12 => Some(m.dedicated_bytes >> 20),
        _ => dedicated_vram_mb(info).or_else(|| vulkan_vram_mb(adapter)),
    }
}

/// What the driver says it really hands out (MB): the memory budget, the amount the
/// adapter may take all together. None where nothing is told, as on the older phones
/// whose allowance stays the 1000 MB guess (#323).
fn budget_mb(mem: Option<&wgpu::AdapterMemoryInfo>) -> Option<u64> {
    let m = mem?;
    let budget = m.budget_bytes? >> 20;
    (budget > 0).then_some(budget)
}

/// Conservative texture allowance in MB, distinct from physical VRAM.
/// Low-memory discrete GPUs need room for render targets and driver allocations.
pub(super) fn texture_allowance_mb(info: &wgpu::AdapterInfo, vram: Option<u64>, mem: Option<&wgpu::AdapterMemoryInfo>) -> u64 {
    let discrete_allowance = |fallback| {
        vram.filter(|v| *v >= 512).map_or(fallback, |v| {
            if v <= 2560 { v * 35 / 100 }
            else if v <= 6144 { (v / 2).min(1600) }
            else { v * 3 / 10 }
        })
    };
    match info.device_type {
        wgpu::DeviceType::DiscreteGpu => discrete_allowance(1600),
        wgpu::DeviceType::IntegratedGpu if info.backend == wgpu::Backend::Metal => 3000,
        // (an APU's memory is the system's own and its driver tells how much it really
        // hands out: 1000 was a guess for the chips that report nothing, and on a desktop
        // APU with 11 GB of budget it kept a mod's 4k livery on its low mip levels -
        // the discrete branch's 3/10 share now, up to 4000)
        wgpu::DeviceType::IntegratedGpu | wgpu::DeviceType::VirtualGpu => {
            let guess = 1000;
            budget_mb(mem).filter(|b| *b >= 2_600).map_or(guess, |b| (b * 3 / 10).min(4_000))
        }
        _ => discrete_allowance(800),
    }
}

/// How much lighter than the settings ask a chip draws.
#[derive(Debug, PartialEq)]
enum Picture {
    /// a small or shared chip: no SSAO, no MSAA, shadow maps of at most 1024
    Weak,
    /// a card of up to 4 GB: no SSAO, at most 2x MSAA and 2048 shadow maps
    Modest,
    AsAsked,
}

/// A small or shared graphics chip (the processor's graphics outside a Mac, a phone, a
/// card of up to 2.5 GB, OpenGL whose memory nothing tells) is a weak one; a card of up
/// to 4 GB, and an iPhone's or an iPad's, a modest one. (The settings' "High" on such a machine ran out of memory or
/// at a dozen frames a second.)
fn picture(backend: wgpu::Backend, device_type: wgpu::DeviceType, vram: Option<u64>) -> Picture {
    let weak = (backend == wgpu::Backend::Gl && vram.is_none())
        // (a phone's chip, whatever type its driver reports: some say "other")
        || cfg!(target_os = "android")
        || (device_type == wgpu::DeviceType::IntegratedGpu && backend != wgpu::Backend::Metal)
        || vram.is_some_and(|v| v <= 2560);
    if weak {
        Picture::Weak
    } else if vram.is_some_and(|v| v <= 4200) || cfg!(target_os = "ios") {
        Picture::Modest
    } else {
        Picture::AsAsked
    }
}

/// The settings `options` lightened for this chip (see [`picture`]), and whether they
/// were; SSAO is off on OpenGL whatever its memory (#422). OMSI_FULL_GPU=1 keeps the
/// settings as they are.
pub(super) fn lighter_picture(info: &wgpu::AdapterInfo, vram: Option<u64>, options: RenderOptions) -> (RenderOptions, bool) {
    let options = RenderOptions { ssao: options.ssao && info.backend != wgpu::Backend::Gl, ..options };
    if omsi_cfg::flags::OMSI_FULL_GPU.is_set() {
        return (options, false);
    }
    match picture(info.backend, info.device_type, vram) {
        Picture::Weak => {
            log::warn!("{}: a small or shared graphics chip - no SSAO, no MSAA, shadow maps of at most 1024 (OMSI_FULL_GPU=1 keeps the settings)", info.name);
            (RenderOptions { msaa: 1, ssao: false, shadow_size: options.shadow_size.min(1024), ..options }, true)
        }
        Picture::Modest => {
            log::info!("{}: {} MB of its own - no SSAO, at most 2x MSAA and 2048 shadow maps (OMSI_FULL_GPU=1 keeps the settings)", info.name, vram.unwrap_or(0));
            (RenderOptions { msaa: options.msaa.min(2), ssao: false, shadow_size: options.shadow_size.min(2048), ..options }, true)
        }
        Picture::AsAsked => (options, false),
    }
}

/// What the game holds on the graphics card now (bytes), see [`Renderer::gpu_memory`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuMemory {
    /// Every texture of the scene.
    pub textures: u64,
    /// The meshes' vertex and index buffers.
    pub meshes: u64,
    /// Everything else: the render targets and shadow maps, the uniform and instance
    /// buffers, the driver's own.
    pub other: u64,
    /// What the driver lets the game have now (the DXGI or Vulkan memory budget, Metal's
    /// recommended working set), where it tells.
    pub budget: Option<u64>,
    /// `other` is the driver's own count less the textures and meshes; without one it is
    /// the render targets' and shadow maps' size worked out.
    pub measured: bool,
}

impl GpuMemory {
    pub fn total(&self) -> u64 {
        self.textures + self.meshes + self.other
    }

    /// What the game may hold on the card: `limit` (bytes, 0 = none set) and nine tenths of
    /// the driver's budget, the smaller; 0 = no bound known.
    pub fn cap(&self, limit: u64) -> u64 {
        match (limit, self.budget.map(|b| b / 10 * 9)) {
            (0, None) => 0,
            (0, Some(b)) => b,
            (l, None) => l,
            (l, Some(b)) => l.min(b),
        }
    }
}

/// What the driver counts on the card beyond the textures and meshes: its count less the room
/// the allocator holds free in its blocks (which the driver counts as used) and less what the
/// game counts itself.
fn rest_of_card(used: u64, free_in_blocks: u64, counted: u64) -> u64 {
    used.saturating_sub(free_in_blocks).saturating_sub(counted)
}

/// The driver's last counts (see [`Renderer::gpu_memory`]).
#[derive(Default)]
pub(crate) struct Seen {
    at: Option<std::time::Instant>,
    /// The last three seconds' `other`, newest last.
    other: Vec<u64>,
    budget: Option<u64>,
    measured: bool,
    /// The allocator's free room in its blocks as last counted, on a worker: its report
    /// lists every allocation, milliseconds with thousands of textures, which once a second
    /// on the frame's thread would be a hitch.
    free_in_blocks: std::sync::Arc<std::sync::atomic::AtomicU64>,
    counting: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Seen {
    /// The allocator's free room as last counted, and a new count started on a worker (none
    /// while one runs).
    fn free_in_blocks(&self, device: &wgpu::Device) -> u64 {
        use std::sync::atomic::Ordering::Relaxed;
        if !self.counting.swap(true, Relaxed) {
            let (device, free, counting) = (device.clone(), self.free_in_blocks.clone(), self.counting.clone());
            // (a thread of its own, not the shared pool whose workers the frame waits for)
            let spawned = std::thread::Builder::new().name("gpu memory count".into()).spawn(move || {
                let room = device.generate_allocator_report().map_or(0, |r| r.total_reserved_bytes.saturating_sub(r.total_allocated_bytes));
                free.store(room, Relaxed);
                counting.store(false, Relaxed);
            });
            if spawned.is_err() {
                self.counting.store(false, Relaxed);
            }
        }
        self.free_in_blocks.load(Relaxed)
    }

    fn note(&mut self, other: u64) {
        if self.other.len() == 3 {
            self.other.remove(0);
        }
        self.other.push(other);
    }
}

impl Renderer {
    /// What the game holds on the graphics card now: the textures and meshes as counted, and
    /// the rest as the driver tells it (DirectX 12, Vulkan with VK_EXT_memory_budget, Metal),
    /// asked at most once a second, less the room wgpu's allocator holds free in its blocks:
    /// DirectX 12 and Vulkan suballocate from blocks of 128-256 MB and give a block back only
    /// once it is empty, so a texture made smaller leaves a hole the driver still counts - it
    /// read as more of the rest, kept the total over the bound and the far textures small for
    /// good. The rest is the least of the last three seconds' counts: a texture made smaller
    /// is out of the count at once but out of the driver's only once the GPU let it go.
    /// Where the driver tells nothing (OpenGL), the render targets and shadow maps are
    /// worked out from their sizes.
    pub fn gpu_memory(&self, scene: &Scene) -> GpuMemory {
        let textures = self.texture_bytes(scene);
        let meshes = self.mesh_bytes(scene);
        let mut seen = self.gpu_memory_seen.borrow_mut();
        if seen.at.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(1)) {
            seen.at = Some(std::time::Instant::now());
            let info = self.adapter.as_ref().and_then(|a| a.memory_info());
            seen.budget = info.and_then(|m| m.budget_bytes).filter(|b| *b > 0);
            match info.and_then(|m| m.usage_bytes).filter(|u| *u > 0) {
                Some(used) => {
                    seen.measured = true;
                    let free_in_blocks = seen.free_in_blocks(&self.device);
                    seen.note(rest_of_card(used, free_in_blocks, textures + meshes));
                }
                None => {
                    seen.measured = false;
                    let targets = self.target_bytes();
                    seen.note(targets);
                }
            }
        }
        GpuMemory {
            textures,
            meshes,
            other: seen.other.iter().copied().min().unwrap_or(0),
            budget: seen.budget,
            measured: seen.measured,
        }
    }

    /// The render targets' and the shadow maps' bytes, from their sizes.
    fn target_bytes(&self) -> u64 {
        fn view(v: &wgpu::TextureView) -> u64 {
            let t = v.texture();
            let s = t.size();
            texture_bytes(t.format(), s.width, s.height, t.mip_level_count()) * t.sample_count() as u64 * s.depth_or_array_layers as u64
        }
        let some = |v: &Option<wgpu::TextureView>| v.as_ref().map_or(0, view);
        let t = &self.targets;
        let mut bytes = view(&self.shadows.shadow_view) + view(&self.shadows.shadow_view_far);
        bytes += t.depth.as_ref().map_or(0, |d| view(&d.1));
        bytes += t.msaa_targets.values().map(|(c, d)| some(c) + view(d)).sum::<u64>();
        if let Some(ao) = &t.ao {
            bytes += view(&ao.depth_view) + view(&ao.ao_view) + view(&ao.blur_view) + view(&ao.fog_view);
        }
        for h in t.hdr_targets.values() {
            bytes += some(&h.msaa_view) + view(&h.view) + some(&h.mask_msaa) + view(&h.mask) + view(&h.ldr);
            bytes += h.gbuf.iter().flatten().map(|(m, r)| some(m) + view(r)).sum::<u64>();
            bytes += h.down.iter().chain(&h.up).map(view).sum::<u64>();
        }
        bytes += t.scale_targets.values().map(|(v, _)| view(v)).sum::<u64>();
        bytes += t.triple_targets.iter().flat_map(|(_, v)| v.iter()).map(|(v, _)| view(v)).sum::<u64>();
        bytes + some(&t.glass_picture)
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::{picture, rest_of_card, GpuMemory, Picture};
    use wgpu::{Backend, DeviceType};

    /// The game's bound on the card: the setting and nine tenths of the driver's budget,
    /// the smaller; nothing known, none.
    #[test]
    fn video_memory_cap_takes_the_smaller_of_setting_and_budget() {
        let m = |budget| GpuMemory { budget, ..Default::default() };
        assert_eq!(m(None).cap(0), 0);
        assert_eq!(m(None).cap(4_000), 4_000);
        assert_eq!(m(Some(10_000)).cap(0), 9_000);
        assert_eq!(m(Some(10_000)).cap(4_000), 4_000);
        assert_eq!(m(Some(4_000)).cap(8_000), 3_600);
    }

    /// A texture made smaller on DirectX 12 or Vulkan leaves a hole in the allocator's
    /// block: the driver's count stays, the game's falls, the hole is the allocator's free
    /// room - the rest of the card stays what it was, not what the texture gave up.
    #[test]
    fn a_hole_in_an_allocator_block_is_not_taken_for_more_of_the_rest() {
        let rest = 1_000;
        assert_eq!(rest_of_card(6_000, 0, 5_000), rest);
        // 800 of textures freed into a block that stays: the driver still says 6000
        assert_eq!(rest_of_card(6_000, 800, 4_200), rest);
        // the block given back: the driver's count falls as well
        assert_eq!(rest_of_card(5_200, 0, 4_200), rest);
        assert_eq!(rest_of_card(100, 500, 4_200), 0);
    }

    /// OpenGL and ANGLE report a discrete card as "other": its memory, once DXGI tells
    /// it, decides, as on Vulkan; unknown memory on OpenGL stays the weak profile.
    #[test]
    fn opengl_card_with_known_memory_is_not_taken_for_a_weak_chip() {
        assert_eq!(picture(Backend::Gl, DeviceType::Other, None), Picture::Weak);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, Some(4076)), Picture::Modest);
        assert_eq!(picture(Backend::Vulkan, DeviceType::DiscreteGpu, Some(4076)), Picture::Modest);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, Some(16304)), Picture::AsAsked);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, Some(2048)), Picture::Weak);
        assert_eq!(picture(Backend::Dx12, DeviceType::IntegratedGpu, Some(8192)), Picture::Weak);
    }
}
