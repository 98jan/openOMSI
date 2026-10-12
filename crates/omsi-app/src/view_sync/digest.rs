//! `OMSI_SCENE_DIGEST=1`: hashes of what the renderer holds of the scene, logged before each
//! picture an offscreen run takes, part by part - so that a part that moves with the wall
//! clock (textures finishing their upload on worker threads, tiles streamed in) does not
//! hide whether the others are the same:
//!
//! * `counts`: how many instances, meshes, textures, materials and interior lamps;
//! * `places`: every instance's mesh, transform and origin, in their order;
//! * `materials`: every instance's materials;
//! * `params`: every instance's per-slot alpha, light map, night map and texture offsets,
//!   visibility and interior light, and the interior lamps;
//! * `sim`: the instances the view sync owns (the AI cars' bodies and coupled parts, the
//!   people's meshes) - places and params - in the cars' and the people's order.
//!
//! Two runs whose renderers saw the same calls in the same order log the same digests: a
//! change that is meant to leave the renderer's calls as they were (the view sync's events,
//! for one) is checked with it against the commit before it.

use super::SimView;
use omsi_render::Scene;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn bits(v: &[f32], h: &mut DefaultHasher) {
    for x in v {
        x.to_bits().hash(h);
    }
}

fn place(scene: &Scene, i: usize, h: &mut DefaultHasher) {
    let Some(i) = scene.instances.get(i) else { return };
    i.mesh.hash(h);
    bits(&i.transform.to_cols_array(), h);
    (i.origin.x.to_bits(), i.origin.y.to_bits(), i.origin.z.to_bits()).hash(h);
}

fn params(scene: &Scene, i: usize, h: &mut DefaultHasher) {
    let Some(i) = scene.instances.get(i) else { return };
    bits(&i.slot_alpha, h);
    bits(&i.slot_light, h);
    bits(&i.slot_night, h);
    for uv in &i.slot_uv {
        bits(uv, h);
    }
    (i.visible, i.interior.to_bits(), i.interior_lamps).hash(h);
}

/// The digests of `scene` (see the module): (counts, places, materials, params, sim).
pub(crate) fn scene_digest(scene: &Scene, view: Option<&SimView>) -> [u64; 5] {
    let mut counts = DefaultHasher::new();
    (scene.instances.len(), scene.meshes.len(), scene.textures.len(), scene.materials.len(), scene.interior_lights.len()).hash(&mut counts);
    let (mut places, mut materials, mut pars) = (DefaultHasher::new(), DefaultHasher::new(), DefaultHasher::new());
    for k in 0..scene.instances.len() {
        place(scene, k, &mut places);
        scene.instances[k].materials.hash(&mut materials);
        params(scene, k, &mut pars);
    }
    for l in &scene.interior_lights {
        (l.position.x.to_bits(), l.position.y.to_bits(), l.position.z.to_bits(), l.intensity.to_bits()).hash(&mut pars);
    }
    let mut sim = DefaultHasher::new();
    if let Some(v) = view {
        for (id, inst) in v.traffic.instances().into_iter().chain(v.people.instances()) {
            (id, inst).hash(&mut sim);
            place(scene, inst, &mut sim);
            params(scene, inst, &mut sim);
        }
    }
    [counts.finish(), places.finish(), materials.finish(), pars.finish(), sim.finish()]
}

/// Log the digests of `scene` (and of the instances of `view`) under `what` when
/// `OMSI_SCENE_DIGEST` asks for them.
pub(crate) fn log_scene_digest(scene: &Scene, view: Option<&SimView>, what: &str) {
    if omsi_cfg::flags::OMSI_SCENE_DIGEST.is_set() {
        let [c, p, m, a, s] = scene_digest(scene, view);
        log::info!("scene digest {what}: counts {c:016x} places {p:016x} materials {m:016x} params {a:016x} sim {s:016x} ({} instances)", scene.instances.len());
    }
}
