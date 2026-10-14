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
        let mut named = Vec::new();
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
            // one card; never borrow another card's budget on a multi-GPU PC.
            let dxgi_name = String::from_utf16_lossy(&d.Description);
            if (info.vendor == 0 || info.device == 0) && dxgi_names_card(&info.name, info.vendor, dxgi_name.trim_end_matches('\0').trim(), d.VendorId, d.DeviceId) {
                named.push((d.VendorId, d.DeviceId, mb));
            }
        }
        one_card(&named)
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

fn angle_device_id(name: &str) -> Option<u32> {
    name.split("(0x").nth(1).and_then(|s| s.split(')').next()).and_then(|s| u32::from_str_radix(s, 16).ok())
}

fn chip_name(name: &str) -> &str {
    let name = name.trim();
    let name = name.strip_prefix("ANGLE (").map_or(name, |rest| rest.split(", ").nth(1).unwrap_or(rest));
    let name = name.split(" (").next().unwrap_or(name);
    name.split('/').next().unwrap_or(name).trim()
}

fn dxgi_names_card(name: &str, gl_vendor: u32, dxgi_name: &str, vendor: u32, device: u32) -> bool {
    if dxgi_name.is_empty() || (gl_vendor != 0 && gl_vendor != vendor) {
        return false;
    }
    if name.trim_start().starts_with("ANGLE (") {
        return name.contains(dxgi_name) && angle_device_id(name) == Some(device);
    }
    let chip = chip_name(name).to_ascii_lowercase();
    let dxgi = dxgi_name.to_ascii_lowercase();
    !chip.is_empty() && (dxgi == chip || dxgi.ends_with(&format!(" {chip}")))
}

fn one_card(named: &[(u32, u32, u64)]) -> Option<u64> {
    let (first, rest) = named.split_first()?;
    rest.iter().all(|c| c == first).then_some(first.2)
}

fn shared_chip_by_name(name: &str) -> Option<bool> {
    let full = name.to_ascii_lowercase();
    if ["llvmpipe", "softpipe", "lavapipe", "swiftshader", "basic render", "software rasterizer"].iter().any(|s| full.contains(s)) {
        return Some(true);
    }
    let chip = chip_name(name).to_ascii_lowercase().replace("(tm)", " ").replace("(r)", " ");
    let words: Vec<&str> = chip.split_whitespace().collect();
    let has = |w: &str| words.contains(&w);
    let apu_model = |w: &str| {
        let (digits, suffix) = if w.is_ascii() { w.split_at(w.len().saturating_sub(1)) } else { ("", "") };
        digits.chars().all(|c| c.is_ascii_digit()) && matches!((digits.len(), suffix), (3, "m") | (4, "s") | (4, "g"))
    };
    if ["mali", "adreno", "powervr", "tegra", "apple", "videocore"].iter().any(|w| chip.contains(w)) {
        Some(true)
    } else if has("intel") || has("iris") || has("uhd") {
        let arc_card = words.windows(2).any(|w| {
            w[0] == "arc" && w[1].len() > 1 && w[1].starts_with(['a', 'b']) && w[1][1..].starts_with(|c: char| c.is_ascii_digit())
        });
        Some(!arc_card)
    } else if has("radeon") || has("amd") || has("ati") {
        Some(words.last() == Some(&"graphics") || (!has("rx") && words.iter().any(|w| apu_model(w))))
    } else if has("geforce") || has("quadro") || has("nvidia") || has("rtx") || has("firepro") {
        Some(false)
    } else {
        None
    }
}

fn amd_opengl_on_windows(info: &wgpu::AdapterInfo) -> bool {
    let name = info.name.to_ascii_lowercase();
    cfg!(windows) && info.backend == wgpu::Backend::Gl && !name.contains("angle") && (name.contains("radeon") || name.starts_with("amd "))
}

pub(super) fn own_card_vram_mb(info: &wgpu::AdapterInfo, vram: Option<u64>) -> u64 {
    let card = match info.device_type {
        wgpu::DeviceType::DiscreteGpu => true,
        wgpu::DeviceType::Other => info.backend == wgpu::Backend::Gl && picture(info.backend, info.device_type, &info.name, vram) != Picture::Weak,
        _ => false,
    };
    vram.filter(|_| card).unwrap_or(0)
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
/// card of up to 2.5 GB, on OpenGL one whose name tells it shared or whose memory nothing
/// tells) is a weak one; a card of up to 4 GB, and an iPhone's or an iPad's, a modest one.
/// (The settings' "High" on such a machine ran out of memory or at a dozen frames a second.)
fn picture(backend: wgpu::Backend, device_type: wgpu::DeviceType, name: &str, vram: Option<u64>) -> Picture {
    let gl = backend == wgpu::Backend::Gl;
    let shared = if gl { shared_chip_by_name(name) } else { None };
    let weak = (gl && (shared == Some(true) || vram.is_none()))
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
/// were; SSAO is off on OpenGL whatever its memory (#422), and MSAA on AMD's OpenGL driver
/// for Windows. OMSI_FULL_GPU=1 keeps the settings as they are.
pub(super) fn lighter_picture(info: &wgpu::AdapterInfo, vram: Option<u64>, options: RenderOptions) -> (RenderOptions, bool) {
    let options = RenderOptions { ssao: options.ssao && info.backend != wgpu::Backend::Gl, ..options };
    if omsi_cfg::flags::OMSI_FULL_GPU.is_set() {
        return (options, false);
    }
    let options = if amd_opengl_on_windows(info) && options.msaa > 1 {
        log::warn!("{}: no MSAA on AMD's OpenGL driver, which closes the game with it (OMSI_FULL_GPU=1 keeps the settings)", info.name);
        RenderOptions { msaa: 1, ..options }
    } else {
        options
    };
    match picture(info.backend, info.device_type, &info.name, vram) {
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

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::{chip_name, dxgi_names_card, one_card, picture, shared_chip_by_name, Picture};
    use wgpu::{Backend, DeviceType};

    /// OpenGL and ANGLE report a discrete card as "other": its memory, once DXGI tells
    /// it, decides, as on Vulkan; unknown memory on OpenGL stays the weak profile.
    #[test]
    fn opengl_card_with_known_memory_is_not_taken_for_a_weak_chip() {
        assert_eq!(picture(Backend::Gl, DeviceType::Other, "", None), Picture::Weak);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, "", Some(4076)), Picture::Modest);
        assert_eq!(picture(Backend::Vulkan, DeviceType::DiscreteGpu, "", Some(4076)), Picture::Modest);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, "", Some(16304)), Picture::AsAsked);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, "", Some(2048)), Picture::Weak);
        assert_eq!(picture(Backend::Dx12, DeviceType::IntegratedGpu, "", Some(8192)), Picture::Weak);
    }

    const ANGLE_7900: &str = "ANGLE (AMD, AMD Radeon RX 7900 XT (0x0000744C) Direct3D11 vs_5_0 ps_5_0, D3D11-32.0.31041.1004)";

    #[test]
    fn graphics_cards_and_shared_chips_are_told_apart_by_name() {
        for card in [
            "AMD Radeon RX 7900 XT",
            ANGLE_7900,
            "AMD Radeon RX 580 Series",
            "AMD Radeon Pro W6800",
            "AMD Radeon RX 7600M XT",
            "AMD Radeon HD 7970M",
            "AMD Radeon RX 7600 (radeonsi, navi33, LLVM 17.0.6, DRM 3.57, 6.8.0)",
            "NVIDIA GeForce RTX 3060/PCIe/SSE2",
            "GeForce GT 530/PCIe/SSE2",
            "ANGLE (NVIDIA, NVIDIA GeForce RTX 4070 (0x00002786) Direct3D11 vs_5_0 ps_5_0, D3D11-32.0.15.6094)",
            "Quadro P1000/PCIe/SSE2",
            "Intel(R) Arc(TM) A770 Graphics",
            "Mesa Intel(R) Arc(TM) B580 Graphics (BMG G21)",
            "AMD Radeon RX 6800S",
            "AMD Radeon RX 7600S",
        ] {
            assert_eq!(shared_chip_by_name(card), Some(false), "{card}");
        }
        for chip in [
            "AMD Radeon(TM) Graphics",
            "ANGLE (AMD, AMD Radeon(TM) Graphics (0x0000164E) Direct3D11 vs_5_0 ps_5_0, D3D11-32.0.21045.5002)",
            "AMD Radeon 780M Graphics",
            "AMD Radeon RX Vega 11 Graphics",
            "AMD Radeon Graphics (radeonsi, renoir, LLVM 15.0.7, DRM 3.49, 6.2.0)",
            "AMD Radeon 890M",
            "AMD Radeon HD 8650G",
            "AMD Radeon 8060S",
            "Intel(R) UHD Graphics 630",
            "Intel(R) Iris(R) Xe Graphics",
            "Intel(R) Arc(TM) Graphics",
            "Intel(R) Arc(TM) 140V GPU (16GB)",
            "ANGLE (Intel, Intel(R) UHD Graphics 620 (0x00005917) Direct3D11 vs_5_0 ps_5_0, D3D11-31.0.101.2111)",
            "Mesa Intel(R) UHD Graphics 620 (KBL GT2)",
            "ANGLE (Microsoft, Microsoft Basic Render Driver (0x0000008C) Direct3D11 vs_5_0 ps_5_0, D3D11)",
            "llvmpipe (LLVM 15.0.7, 256 bits)",
            "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver)",
            "Mali-G78",
            "Adreno (TM) 650",
        ] {
            assert_eq!(shared_chip_by_name(chip), Some(true), "{chip}");
        }
        assert_eq!(shared_chip_by_name(""), None);
        assert_eq!(chip_name(ANGLE_7900), "AMD Radeon RX 7900 XT");
    }

    #[test]
    fn opengl_picture_follows_the_name_where_the_memory_is_not_told() {
        assert_eq!(picture(Backend::Gl, DeviceType::Other, ANGLE_7900, Some(20421)), Picture::AsAsked);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, "AMD Radeon RX 7900 XT", Some(20421)), Picture::AsAsked);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, "AMD Radeon RX 7900 XT", None), Picture::Weak);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, "AMD Radeon(TM) Graphics", Some(8192)), Picture::Weak);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, "AMD Radeon(TM) Graphics", None), Picture::Weak);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, "Intel(R) UHD Graphics 630", None), Picture::Weak);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, "Microsoft Basic Render Driver", None), Picture::Weak);
        assert_eq!(picture(Backend::Gl, DeviceType::Cpu, "llvmpipe (LLVM 15.0.7, 256 bits)", None), Picture::Weak);
        assert_eq!(picture(Backend::Gl, DeviceType::Other, "GeForce GT 530/PCIe/SSE2", Some(1024)), Picture::Weak);
        assert_eq!(picture(Backend::Vulkan, DeviceType::DiscreteGpu, "AMD Radeon(TM) Graphics", Some(8192)), Picture::AsAsked);
    }

    #[test]
    fn dxgi_lends_its_memory_to_the_one_card_the_name_tells() {
        assert!(dxgi_names_card("AMD Radeon RX 7900 XT", 0, "AMD Radeon RX 7900 XT", 0x1002, 0x744c));
        assert!(dxgi_names_card(ANGLE_7900, 0x1002, "AMD Radeon RX 7900 XT", 0x1002, 0x744c));
        assert!(!dxgi_names_card(ANGLE_7900, 0x1002, "AMD Radeon RX 7900 XT", 0x1002, 0x744d));
        assert!(!dxgi_names_card(ANGLE_7900, 0x10de, "AMD Radeon RX 7900 XT", 0x1002, 0x744c));
        assert!(dxgi_names_card("GeForce GT 530/PCIe/SSE2", 0, "NVIDIA GeForce GT 530", 0x10de, 0x0de4));
        assert!(!dxgi_names_card("AMD Radeon RX 7900 XT", 0, "AMD Radeon RX 7900 XTX", 0x1002, 0x744c));
        assert!(!dxgi_names_card("AMD Radeon RX 7900 XT", 0, "AMD Radeon(TM) Graphics", 0x1002, 0x164e));
        assert!(!dxgi_names_card("", 0, "AMD Radeon RX 7900 XT", 0x1002, 0x744c));
        assert_eq!(one_card(&[(0x1002, 0x744c, 20421), (0x1002, 0x744c, 20421)]), Some(20421));
        assert_eq!(one_card(&[(0x10de, 0x2786, 12282), (0x10de, 0x2504, 12288)]), None);
        assert_eq!(one_card(&[]), None);
    }
}
