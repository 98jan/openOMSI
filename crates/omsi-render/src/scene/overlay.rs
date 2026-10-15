//! Which of an object's surfaces lie on an earlier one in its own plane - the instruments
//! printed on a dashboard, the lines on a plate - and so take the draw order's depth step
//! (`draw_order_level`, `scene_clip`). Omsi.exe draws a model mesh by mesh and subset by
//! subset with LESSEQUAL, so of two such surfaces the later one is seen; everything else
//! keeps its own depth.

use glam::{Mat4, Vec3};
use std::collections::HashMap;

use crate::MeshData;

/// The most flat surfaces kept of one material slot of a mesh (the largest).
const FLATS_PER_SLOT: usize = 32;
/// How far apart two planes may lie and still be one (the meshes' own rounding).
const PLANE_GAP: f32 = 2e-4;
/// How far apart two surfaces' boxes may be and still overlap.
const BOX_GAP: f32 = 1e-3;
/// The highest step: a surface on a surface on a surface ... is rare beyond that.
pub(crate) const MAX_LEVEL: u8 = 3;

/// A flat surface of a mesh's material slot: its plane (unit normal, its sign made the same
/// for both faces, and the plane's offset along it) and the box its triangles lie in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Flat {
    pub(crate) slot: u32,
    pub(crate) normal: Vec3,
    pub(crate) offset: f32,
    pub(crate) lo: Vec3,
    pub(crate) hi: Vec3,
}

/// The normal with the sign that makes its largest component positive, and the offset to
/// go with it (a surface and one facing the other way in the same plane are one plane).
fn canonical(n: Vec3, offset: f32) -> (Vec3, f32) {
    let a = n.abs();
    let big = if a.x >= a.y && a.x >= a.z { n.x } else if a.y >= a.z { n.y } else { n.z };
    if big < 0.0 { (-n, -offset) } else { (n, offset) }
}

/// The flat surfaces of `data`, slot by slot: its triangles grouped by plane, the largest
/// `FLATS_PER_SLOT` of each slot kept.
pub(crate) fn mesh_flats(data: &MeshData) -> Vec<Flat> {
    let mut out = Vec::new();
    for &(first, count, slot) in &data.ranges {
        // (plane key -> flat, area)
        let mut planes: HashMap<(i32, i32, i32, i32), (Flat, f32)> = HashMap::new();
        let end = (first + count).min(data.indices.len() as u32);
        let mut k = first as usize;
        while k + 2 < end as usize {
            let idx = [data.indices[k], data.indices[k + 1], data.indices[k + 2]];
            k += 3;
            let Some(p) = idx.iter().map(|&i| data.positions.get(i as usize).copied()).collect::<Option<Vec<Vec3>>>() else { continue };
            let cross = (p[1] - p[0]).cross(p[2] - p[0]);
            let area = cross.length() * 0.5;
            if area < 1e-8 {
                continue;
            }
            let (n, offset) = canonical(cross / (2.0 * area), (cross / (2.0 * area)).dot(p[0]));
            let key = ((n.x * 1000.0).round() as i32, (n.y * 1000.0).round() as i32, (n.z * 1000.0).round() as i32, (offset / PLANE_GAP).round() as i32);
            let (lo, hi) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
            let e = planes.entry(key).or_insert((Flat { slot, normal: n, offset, lo, hi }, 0.0));
            e.0.lo = e.0.lo.min(lo);
            e.0.hi = e.0.hi.max(hi);
            e.1 += area;
        }
        let mut flats: Vec<(Flat, f32)> = planes.into_values().collect();
        flats.sort_by(|a, b| b.1.total_cmp(&a.1));
        out.extend(flats.into_iter().take(FLATS_PER_SLOT).map(|(f, _)| f));
    }
    out
}

/// The object whose meshes are being added: where it stands, the creation order its next
/// mesh would have, and its surfaces so far (in its frame) with their steps, found by plane.
#[derive(Default)]
pub(crate) struct ObjectRun {
    /// An object was begun (`begin`) and none of another has been added since.
    open: bool,
    /// Its first mesh is still to come.
    fresh: bool,
    origin: [u64; 3],
    next: u64,
    flats: Vec<(Flat, u8)>,
    by_plane: HashMap<(i32, i32, i32, i32), Vec<usize>>,
}

fn plane_key(n: Vec3, offset: f32) -> (i32, i32, i32, i32) {
    ((n.x * 100.0).round() as i32, (n.y * 100.0).round() as i32, (n.z * 100.0).round() as i32, (offset / 1e-3).floor() as i32)
}

impl ObjectRun {
    /// A new object (a model) begins: the meshes added from now on, one after another from
    /// one place, are its own. Without it a mesh takes no step: only the scenery objects and
    /// the vehicles mark where their models begin - a vehicle's meshes all stand at the
    /// origin, so two in a row would otherwise share one history.
    pub(crate) fn begin(&mut self) {
        self.open = true;
        self.fresh = true;
        self.flats.clear();
        self.by_plane.clear();
    }

    /// The steps of the slots (`slots` of them) of a mesh with surfaces `flats`, placed with
    /// `transform` at `origin` (its key) as instance number `draw_order`: a slot lying on an
    /// earlier surface of the same object (one plane, the boxes overlapping) takes one step
    /// more than the highest of those, at most `MAX_LEVEL`; any other slot 0. Outside an
    /// object begun with `begin` every slot is 0, and a mesh that is not the next one of the
    /// object (another place, a gap in the order) ends it.
    pub(crate) fn levels(&mut self, origin: [u64; 3], draw_order: u64, transform: Mat4, flats: &[Flat], slots: usize) -> Vec<u8> {
        if !self.fresh && (origin != self.origin || draw_order != self.next) {
            self.open = false;
            self.flats.clear();
            self.by_plane.clear();
        }
        self.fresh = false;
        self.origin = origin;
        self.next = draw_order + 1;
        if !self.open {
            return vec![0; slots];
        }
        let normal_m = transform.inverse().transpose();
        let placed: Vec<Flat> = flats
            .iter()
            .filter_map(|f| {
                let n = normal_m.transform_vector3(f.normal).try_normalize()?;
                let p = transform.transform_point3(f.normal * f.offset);
                let (normal, offset) = canonical(n, n.dot(p));
                let corners = (0..8).map(|c| {
                    let pick = |bit: usize, lo: f32, hi: f32| if c & bit != 0 { hi } else { lo };
                    transform.transform_point3(Vec3::new(pick(1, f.lo.x, f.hi.x), pick(2, f.lo.y, f.hi.y), pick(4, f.lo.z, f.hi.z)))
                });
                let (lo, hi) = corners.fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), c| (lo.min(c), hi.max(c)));
                Some(Flat { slot: f.slot, normal, offset, lo, hi })
            })
            .collect();
        let mut levels = vec![0u8; slots];
        for slot in 0..slots {
            let mine: Vec<&Flat> = placed.iter().filter(|f| f.slot as usize == slot).collect();
            let mut level = 0u8;
            for f in &mine {
                let (kx, ky, kz, ko) = plane_key(f.normal, f.offset);
                for dx in -1..=1 { for dy in -1..=1 { for dz in -1..=1 { for d_o in -1..=1 {
                    let Some(list) = self.by_plane.get(&(kx + dx, ky + dy, kz + dz, ko + d_o)) else { continue };
                    for &i in list {
                        let (g, gl) = &self.flats[i];
                        let same_plane = f.normal.dot(g.normal) > 0.9995 && (f.offset - g.offset).abs() < PLANE_GAP;
                        let overlap = (f.lo - Vec3::splat(BOX_GAP)).cmple(g.hi).all() && (g.lo - Vec3::splat(BOX_GAP)).cmple(f.hi).all();
                        if same_plane && overlap {
                            level = level.max(gl.saturating_add(1).min(MAX_LEVEL));
                        }
                    }
                }}}}
            }
            levels[slot] = level;
            // (the slot's surfaces count for the slots after it, of this mesh and the next)
            for f in mine {
                let key = plane_key(f.normal, f.offset);
                self.by_plane.entry(key).or_default().push(self.flats.len());
                self.flats.push((*f, level));
            }
        }
        levels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(z: f32, half: f32, slot: u32, cut_other_way: bool) -> MeshData {
        let positions = vec![Vec3::new(-half, -half, z), Vec3::new(half, -half, z), Vec3::new(half, half, z), Vec3::new(-half, half, z)];
        let indices = if cut_other_way { vec![0, 1, 3, 1, 2, 3] } else { vec![0, 1, 2, 0, 2, 3] };
        MeshData { positions, normals: vec![Vec3::Z; 4], uvs: vec![glam::Vec2::ZERO; 4], ranges: vec![(0, 6, slot)], indices, one_sided: false }
    }

    fn box_mesh(at: Vec3) -> MeshData {
        // (an unrelated mesh: a box somewhere else)
        let mut m = quad(0.0, 0.5, 0, false);
        for p in &mut m.positions {
            *p += at;
        }
        m
    }

    /// The bus case (#1488): its meshes all stand at one place, the dashboard and its
    /// instruments far down the list. Unrelated meshes stay at 0, the instruments lying on
    /// the dashboard take 1, a sticker on the instruments 2; capped at `MAX_LEVEL`.
    #[test]
    fn a_surface_on_an_earlier_one_of_its_object_takes_a_step() {
        let mut run = ObjectRun::default();
        run.begin();
        let origin = [1, 2, 3];
        let mut order = 0;
        let mut add = |run: &mut ObjectRun, m: &MeshData, t: Mat4| {
            let l = run.levels(origin, order, t, &mesh_flats(m), 1);
            order += 1;
            l[0]
        };
        for k in 0..6 {
            assert_eq!(add(&mut run, &box_mesh(Vec3::new(10.0 * k as f32, 5.0, 2.0)), Mat4::IDENTITY), 0);
        }
        assert_eq!(add(&mut run, &quad(1.0, 0.4, 0, false), Mat4::IDENTITY), 0, "the dashboard");
        assert_eq!(add(&mut run, &quad(1.0, 0.1, 0, true), Mat4::IDENTITY), 1, "instruments on it");
        assert_eq!(add(&mut run, &quad(1.0, 0.05, 0, false), Mat4::IDENTITY), 2, "a sticker on those");
        assert_eq!(add(&mut run, &quad(1.0, 0.02, 0, false), Mat4::IDENTITY), 3);
        assert_eq!(add(&mut run, &quad(1.0, 0.01, 0, false), Mat4::IDENTITY), MAX_LEVEL);
        // (a millimetre over it is not in its plane: its height decides, as without the step)
        assert_eq!(add(&mut run, &quad(1.001, 0.1, 0, false), Mat4::IDENTITY), 0);
        // (in the plane but beside it: no overlap)
        assert_eq!(add(&mut run, &moved(quad(1.0, 0.1, 0, false), Vec3::new(3.0, 0.0, 0.0)), Mat4::IDENTITY), 0);
        // (placed by its transform onto the dashboard's plane, in a corner of it)
        assert_eq!(add(&mut run, &quad(0.0, 0.05, 0, false), Mat4::from_translation(Vec3::new(0.3, 0.3, 1.0))), 1);
    }

    /// Slots of one mesh: the later one on the earlier one takes the step.
    #[test]
    fn a_later_slot_on_an_earlier_one_takes_a_step() {
        let mut m = quad(0.0, 1.0, 0, false);
        let over = quad(0.0, 0.5, 1, true);
        m.positions.extend(&over.positions);
        m.normals.extend(&over.normals);
        m.uvs.extend(&over.uvs);
        m.indices.extend(over.indices.iter().map(|i| i + 4));
        m.ranges = vec![(0, 6, 0), (6, 6, 1)];
        let mut run = ObjectRun::default();
        run.begin();
        assert_eq!(run.levels([0; 3], 0, Mat4::IDENTITY, &mesh_flats(&m), 2), vec![0, 1]);
    }

    /// Another object starts afresh: a new place, or a gap in the creation order, ends it;
    /// meshes added outside a begun object take no step.
    #[test]
    fn another_object_starts_at_zero() {
        let mut run = ObjectRun::default();
        let (a, b) = (quad(0.0, 1.0, 0, false), quad(0.0, 0.5, 0, true));
        assert_eq!(run.levels([1; 3], 0, Mat4::IDENTITY, &mesh_flats(&a), 1), vec![0]);
        assert_eq!(run.levels([1; 3], 1, Mat4::IDENTITY, &mesh_flats(&b), 1), vec![0], "no object begun");
        run.begin();
        assert_eq!(run.levels([1; 3], 2, Mat4::IDENTITY, &mesh_flats(&a), 1), vec![0]);
        assert_eq!(run.levels([2; 3], 3, Mat4::IDENTITY, &mesh_flats(&b), 1), vec![0], "another place");
        assert_eq!(run.levels([2; 3], 4, Mat4::IDENTITY, &mesh_flats(&b), 1), vec![0], "ended there");
        run.begin();
        assert_eq!(run.levels([2; 3], 5, Mat4::IDENTITY, &mesh_flats(&a), 1), vec![0]);
        assert_eq!(run.levels([2; 3], 9, Mat4::IDENTITY, &mesh_flats(&b), 1), vec![0], "a gap in the order");
    }

    /// Two vehicles of one model in a row: all their meshes at the origin, created one after
    /// the other with no gap. Each begins its own object and keeps its own steps (#2603).
    #[test]
    fn models_in_a_row_at_one_place_keep_their_own_steps() {
        let layers = [quad(0.0, 1.0, 0, false), quad(0.0, 0.5, 0, true), quad(0.0, 0.2, 0, false)];
        let mut run = ObjectRun::default();
        let mut order = 0;
        for vehicle in 0..2 {
            run.begin();
            let levels: Vec<u8> = layers
                .iter()
                .map(|m| {
                    order += 1;
                    run.levels([0; 3], order, Mat4::IDENTITY, &mesh_flats(m), 1)[0]
                })
                .collect();
            assert_eq!(levels, vec![0, 1, 2], "vehicle {vehicle}");
        }
    }

    fn moved(mut m: MeshData, by: Vec3) -> MeshData {
        for p in &mut m.positions {
            *p += by;
        }
        m
    }
}
