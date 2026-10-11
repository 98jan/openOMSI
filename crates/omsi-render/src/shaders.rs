//! The WGSL sources that are too long for one file, kept as parts in `shaders/<module>/` and
//! put back together at compile time, byte for byte: `SCENE` is what was `shader.wgsl`, and
//! `ENHANCED` what was `enhanced.wgsl` (comments elsewhere still call them so).
//!
//! The parts are plain cuts between top-level items, joined with nothing in between, so the
//! GPU receives exactly the text it did from the single files. That matters: Apple's Metal
//! compiler (M4) is at its limit with the scene module, and any change to the text it gets,
//! however harmless it looks, has to be tried there. The tests below hold the assembled text
//! to the old files' size and hash; a deliberate edit of a part updates them.

/// The scene module's own part: the vanilla path, the shadow passes, the rain on the glass
/// (`shader.wgsl`). Joined with the shared parts in `scene_shader_text`.
pub(crate) const SCENE: &str = concat!(
    include_str!("shaders/scene/01_camera.wgsl"),
    include_str!("shaders/scene/02_bindings.wgsl"),
    include_str!("shaders/scene/03_vertex.wgsl"),
    include_str!("shaders/scene/04_shadow_passes.wgsl"),
    include_str!("shaders/scene/05_sun_shadow.wgsl"),
    include_str!("shaders/scene/06_lights.wgsl"),
    include_str!("shaders/scene/07_patterns_weather.wgsl"),
    include_str!("shaders/scene/08_rain_glass.wgsl"),
    include_str!("shaders/scene/09_vanilla.wgsl"),
);

/// The enhanced fragment shader (`enhanced.wgsl`): its lighting model, the lamps, the
/// surface's shading and the puddles' reflections.
pub(crate) const ENHANCED: &str = concat!(
    include_str!("shaders/enhanced/01_brdf_shadow.wgsl"),
    include_str!("shaders/enhanced/02_lamps.wgsl"),
    include_str!("shaders/enhanced/03_surface.wgsl"),
    include_str!("shaders/enhanced/04_shade.wgsl"),
    include_str!("shaders/enhanced/05_puddles.wgsl"),
);

#[cfg(test)]
mod tests {
    /// FNV-1a, 64 bits: enough to tell one text from another, and no dependency for it.
    fn fnv1a(text: &str) -> u64 {
        text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
    }

    /// The assembled modules are byte for byte the single files they were cut from (sizes
    /// and hashes taken of `shader.wgsl` and `enhanced.wgsl` before the cut).
    #[test]
    fn assembled_shaders_are_the_old_files() {
        assert_eq!((super::SCENE.len(), fnv1a(super::SCENE)), (116_173, 0x5efe_f9d5_1092_98c6));
        assert_eq!((super::ENHANCED.len(), fnv1a(super::ENHANCED)), (83_366, 0x8e9f_e48c_fe3d_40f5));
    }
}
