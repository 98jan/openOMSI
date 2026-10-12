//! `OMSI_SCENE_DIGEST=1`: a hash of what the renderer holds of the scene, logged before each
//! picture an offscreen run takes - its instances in their order (mesh, place, materials,
//! per-slot parameters, visibility), and how many meshes, textures, materials and interior
//! lamps there are. Two runs whose renderers saw the same calls in the same order log the
//! same digests: a change that is meant to leave the renderer's calls as they were (the
//! view sync's events, for one) is checked with it against the commit before it.

use omsi_render::Scene;
use std::hash::{Hash, Hasher};

/// The digest of `scene` (see the module).
pub(crate) fn scene_digest(scene: &Scene) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (scene.instances.len(), scene.meshes.len(), scene.textures.len(), scene.materials.len(), scene.interior_lights.len()).hash(&mut h);
    let bits = |v: &[f32], h: &mut std::collections::hash_map::DefaultHasher| {
        for x in v {
            x.to_bits().hash(h);
        }
    };
    for i in &scene.instances {
        i.mesh.hash(&mut h);
        bits(&i.transform.to_cols_array(), &mut h);
        (i.origin.x.to_bits(), i.origin.y.to_bits(), i.origin.z.to_bits()).hash(&mut h);
        i.materials.hash(&mut h);
        bits(&i.slot_alpha, &mut h);
        bits(&i.slot_light, &mut h);
        bits(&i.slot_night, &mut h);
        for uv in &i.slot_uv {
            bits(uv, &mut h);
        }
        (i.visible, i.interior.to_bits(), i.interior_lamps).hash(&mut h);
    }
    for l in &scene.interior_lights {
        (l.position.x.to_bits(), l.position.y.to_bits(), l.position.z.to_bits(), l.intensity.to_bits()).hash(&mut h);
    }
    h.finish()
}

/// Log the digest of `scene` under `what` when `OMSI_SCENE_DIGEST` asks for it.
pub(crate) fn log_scene_digest(scene: &Scene, what: &str) {
    if omsi_cfg::flags::OMSI_SCENE_DIGEST.is_set() {
        log::info!("scene digest {what}: {:016x} ({} instances)", scene_digest(scene), scene.instances.len());
    }
}
