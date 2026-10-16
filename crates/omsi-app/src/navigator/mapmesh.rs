//! The map's own picture (`omsi_map::layers`, read off the map by `omsi_scene::mapgen`) as
//! triangles for the small map and the city map: the ground, water and woods, every paved
//! surface as it lies (with a fine edge round it, so a car park reads as one grey yard and
//! not as ten road ribbons), tracks, trees, and the houses - flat from far, standing in 3D with
//! lit walls and soft shadows from near.

use super::style::{Palette, SHADOW_DIR};
use super::*;
use omsi_map::layers::{self, Layers};

/// The layers cut into square cells, so that the part around a place is found at once.
pub(crate) struct MapData {
    pub layers: std::sync::Arc<Layers>,
    cells: HashMap<(i32, i32), Cell>,
    /// Lines with their boxes (they run across many cells).
    lines: Vec<(Vec2, Vec2, usize)>,
}

#[derive(Default)]
struct Cell {
    land: Vec<Vec<Vec2>>,
    paved: Vec<Vec<Vec2>>,
    buildings: Vec<u32>,
    trees: Vec<u32>,
}

const CELL: f32 = 200.0;

fn cell_of(p: Vec2) -> (i32, i32) {
    ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32)
}

/// What to draw of the ground.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct GroundLook {
    /// Pavements and car parks as surfaces (near); else carriageways only.
    pub surfaces: bool,
    pub trees: bool,
    /// The railways' dashes (lost in a blur from far).
    pub dashes: bool,
}

impl MapData {
    pub fn new(layers: std::sync::Arc<Layers>) -> MapData {
        let mut cells: HashMap<(i32, i32), Cell> = HashMap::new();
        let new_cell = || Cell { land: vec![Vec::new(); layers::LAND_CLASSES], paved: vec![Vec::new(); layers::PAVE_CLASSES], ..Default::default() };
        for (class, tris) in layers.land.iter().enumerate() {
            for t in tris.chunks_exact(3) {
                let c = cell_of((t[0] + t[1] + t[2]) / 3.0);
                cells.entry(c).or_insert_with(new_cell).land[class].extend_from_slice(t);
            }
        }
        for (class, tris) in layers.paved.iter().enumerate() {
            for t in tris.chunks_exact(3) {
                let c = cell_of((t[0] + t[1] + t[2]) / 3.0);
                cells.entry(c).or_insert_with(new_cell).paved[class].extend_from_slice(t);
            }
        }
        for (i, b) in layers.buildings.iter().enumerate() {
            let c = cell_of(b.outline.iter().fold(Vec2::ZERO, |a, p| a + *p) / b.outline.len().max(1) as f32);
            cells.entry(c).or_insert_with(new_cell).buildings.push(i as u32);
        }
        for (i, t) in layers.trees.iter().enumerate() {
            cells.entry(cell_of(Vec2::new(t[0], t[1]))).or_insert_with(new_cell).trees.push(i as u32);
        }
        let lines = layers
            .lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let lo = l.points.iter().fold(Vec2::splat(f32::MAX), |a, p| a.min(*p));
                let hi = l.points.iter().fold(Vec2::splat(f32::MIN), |a, p| a.max(*p));
                (lo, hi, i)
            })
            .collect();
        MapData { layers, cells, lines }
    }

    /// The cells within `r` metres of `c` (layer coordinates).
    fn cells_near(&self, c: Vec2, r: f32) -> impl Iterator<Item = &Cell> {
        let (lo, hi) = (cell_of(c - Vec2::splat(r)), cell_of(c + Vec2::splat(r)));
        (lo.1..=hi.1).flat_map(move |y| (lo.0..=hi.0).filter_map(move |x| self.cells.get(&(x, y))))
    }

    /// From the layers' frame to one anchored at `anchor` (world).
    fn shift(&self, anchor: DVec2) -> Vec2 {
        (self.layers.origin - anchor).as_vec2()
    }

    /// Everything of the ground within `radius` of `centre` (world), relative to `anchor`: the
    /// land, the paved surfaces and the tracks.
    #[allow(dead_code)]
    pub fn build_ground(&self, p: &mut Painter, anchor: DVec2, centre: DVec2, radius: f64, pal: &Palette, look: GroundLook) {
        self.build_land(p, anchor, centre, radius, pal, look);
        self.build_paved(p, anchor, centre, radius, pal, look);
        self.build_lines(p, anchor, centre, radius, pal, look);
    }

    /// The tiles' ground, water, woods and trees.
    pub fn build_land(&self, p: &mut Painter, anchor: DVec2, centre: DVec2, radius: f64, pal: &Palette, look: GroundLook) {
        let k = self.shift(anchor);
        let c = (centre - self.layers.origin).as_vec2();
        let v = |q: Vec2| (q + k).extend(0.0);
        let cells: Vec<&Cell> = self.cells_near(c, radius as f32).collect();
        let land_order = [
            (layers::LAND_GRASS, pal.grass),
            (layers::LAND_FIELD, pal.field),
            (layers::LAND_SAND, pal.sand),
            (layers::LAND_PAVED, pal.paved_land),
            (layers::LAND_FOREST, pal.forest),
            (layers::LAND_WATER, pal.water),
        ];
        for (class, col) in land_order {
            for cell in &cells {
                for t in cell.land[class].chunks_exact(3) {
                    p.world_poly(&[v(t[0]), v(t[1]), v(t[2])], col);
                }
            }
        }
        if look.trees {
            for cell in &cells {
                for &i in &cell.trees {
                    let t = self.layers.trees[i as usize];
                    let q = v(Vec2::new(t[0], t[1]));
                    p.world_disc(q, t[2] * 0.95 + 0.35, 0.0, pal.tree_edge);
                    p.world_disc(q, t[2] * 0.95, 0.0, pal.tree);
                }
            }
        }
    }

    /// The paved surfaces: the pavements' and yards' edges and surfaces, then the carriageways'
    /// (so a carriageway keeps its edge against the pavement beside it).
    pub fn build_paved(&self, p: &mut Painter, anchor: DVec2, centre: DVec2, radius: f64, pal: &Palette, look: GroundLook) {
        let k = self.shift(anchor);
        let c = (centre - self.layers.origin).as_vec2();
        let v = |q: Vec2| (q + k).extend(0.0);
        let cells: Vec<&Cell> = self.cells_near(c, radius as f32).collect();
        let classes: &[(usize, Color, Color)] = if look.surfaces {
            &[(layers::PAVE_FOOT, pal.foot, pal.foot_edge), (layers::PAVE_AREA, pal.area, pal.area_edge)]
        } else {
            &[(layers::PAVE_AREA, pal.area, pal.area_edge)]
        };
        for &(class, fill, edge) in classes {
            for cell in &cells {
                for t in cell.paved[class].chunks_exact(3) {
                    p.grown_tri([v(t[0]), v(t[1]), v(t[2])], 0.12, 0.8, edge);
                }
            }
            for cell in &cells {
                for t in cell.paved[class].chunks_exact(3) {
                    p.world_poly(&[v(t[0]), v(t[1]), v(t[2])], fill);
                }
            }
        }
        for cell in &cells {
            for t in cell.paved[layers::PAVE_ROAD].chunks_exact(3) {
                p.grown_tri([v(t[0]), v(t[1]), v(t[2])], 0.25, 1.0, pal.road_edge);
            }
        }
        for cell in &cells {
            for t in cell.paved[layers::PAVE_ROAD].chunks_exact(3) {
                p.world_poly(&[v(t[0]), v(t[1]), v(t[2])], pal.road);
            }
        }
    }

    /// Railways as a grey band with white dashes (Google's way), tram tracks as a fine line.
    pub fn build_lines(&self, p: &mut Painter, anchor: DVec2, centre: DVec2, radius: f64, pal: &Palette, look: GroundLook) {
        let k = self.shift(anchor);
        let c = (centre - self.layers.origin).as_vec2();
        let r = radius as f32;
        for &(lo, hi, i) in &self.lines {
            if lo.x > c.x + r || hi.x < c.x - r || lo.y > c.y + r || hi.y < c.y - r {
                continue;
            }
            let l = &self.layers.lines[i];
            let pts: Vec<Vec3> = l.points.iter().map(|q| (*q + k).extend(0.0)).collect();
            match l.class {
                layers::LINE_RAIL => {
                    p.ribbon(&pts, 3.6, 3.4, pal.rail, false);
                    if look.dashes {
                        dashes(p, &pts, 9.0, 9.0, 1.3, 1.4, pal.rail_dash);
                    }
                }
                layers::LINE_TRAM => p.ribbon(&pts, 0.9, 1.3, pal.tram, false),
                _ => p.ribbon(&pts, l.width.max(3.0), 2.0, pal.water, false),
            }
        }
    }

    /// The houses within `radius` of `centre`, relative to `anchor`, `height` times as tall as
    /// they are (0: flat roofs from above, the city map zoomed out): shadows first, then walls
    /// lit by which way they face, then roofs with a fine edge. Into `p` for a depth-tested layer.
    pub fn build_houses(&self, p: &mut Painter, anchor: DVec2, centre: DVec2, radius: f64, pal: &Palette, height: f32) {
        let k = self.shift(anchor);
        let c = (centre - self.layers.origin).as_vec2();
        let r = radius as f32;
        let ids: Vec<u32> = self.cells_near(c, r).flat_map(|cell| cell.buildings.iter().copied()).collect();
        let flat = height < 0.04;
        let light = -Vec2::from(SHADOW_DIR).normalize();
        let sd = Vec2::from(SHADOW_DIR);
        for &i in &ids {
            let b = &self.layers.buildings[i as usize];
            let o: Vec<Vec2> = b.outline.iter().map(|q| *q + k).collect();
            if flat {
                for t in &b.roof {
                    let tri = [o[t[0] as usize].extend(0.0), o[t[1] as usize].extend(0.0), o[t[2] as usize].extend(0.0)];
                    p.grown_tri(tri, 0.15, 0.7, pal.roof_edge);
                }
                continue;
            }
            let h = b.height * height;
            let d = sd * h;
            // the shadow: the footprint swept along the light
            for t in &b.roof {
                let (a, bb, cc) = (o[t[0] as usize], o[t[1] as usize], o[t[2] as usize]);
                p.shadow_tri((a + d).extend(0.0), (bb + d).extend(0.0), (cc + d).extend(0.0), pal.shadow);
                p.shadow_tri(a.extend(0.0), bb.extend(0.0), cc.extend(0.0), pal.shadow);
            }
            let n = o.len();
            for j in 0..n {
                let (a, bb) = (o[j], o[(j + 1) % n]);
                p.shadow_tri(a.extend(0.0), bb.extend(0.0), (bb + d).extend(0.0), pal.shadow);
                p.shadow_tri(a.extend(0.0), (bb + d).extend(0.0), (a + d).extend(0.0), pal.shadow);
            }
        }
        if flat {
            for &i in &ids {
                let b = &self.layers.buildings[i as usize];
                for t in &b.roof {
                    let q = |m: u16| (b.outline[m as usize] + k).extend(0.0);
                    p.world_poly(&[q(t[0]), q(t[1]), q(t[2])], pal.roof);
                }
            }
            return;
        }
        for &i in &ids {
            let b = &self.layers.buildings[i as usize];
            let o: Vec<Vec2> = b.outline.iter().map(|q| *q + k).collect();
            let h = b.height * height;
            let n = o.len();
            // walls: outward normal of an anticlockwise outline is to the right of each edge
            for j in 0..n {
                let (a, bb) = (o[j], o[(j + 1) % n]);
                let e = bb - a;
                if e.length_squared() < 1e-4 {
                    continue;
                }
                let nrm = Vec2::new(e.y, -e.x).normalize();
                let lit = (0.5 + 0.5 * nrm.dot(light)).clamp(0.0, 1.0);
                let col = pal.wall_dark.mix(pal.wall_lit, lit);
                let foot = col.darken(0.07);
                let (a0, b0, a1, b1) = (a.extend(0.0), bb.extend(0.0), a.extend(h), bb.extend(h));
                p.solid_tri(a0, b0, b1, foot, foot, col);
                p.solid_tri(a0, b1, a1, foot, col, col);
            }
            // the roof, a little edge round it just under it
            let start = p.verts.len();
            for t in &b.roof {
                let tri = [o[t[0] as usize].extend(h - 0.05), o[t[1] as usize].extend(h - 0.05), o[t[2] as usize].extend(h - 0.05)];
                p.grown_tri(tri, 0.12, 0.6, pal.roof_edge);
            }
            for v in &mut p.verts[start..] {
                v.mode[0] = 2.0;
            }
            for t in &b.roof {
                let q = |m: u16| o[m as usize].extend(h);
                p.solid_tri(q(t[0]), q(t[1]), q(t[2]), pal.roof, pal.roof, pal.roof);
            }
        }
    }

    /// The layers' extent (world).
    pub fn extent(&self) -> Option<(DVec2, DVec2)> {
        let (lo, hi) = self.layers.bounds()?;
        Some((self.layers.origin + lo.as_dvec2(), self.layers.origin + hi.as_dvec2()))
    }
}

/// Dashes along a polyline: `on` metres drawn, `off` left, `w_m` wide (at least `w_px`).
fn dashes(p: &mut Painter, pts: &[Vec3], on: f32, off: f32, w_m: f32, w_px: f32, c: Color) {
    let mut run: Vec<Vec3> = Vec::new();
    let mut phase = 0.0f32;
    let mut drawing = true;
    for ab in pts.windows(2) {
        let (a, b) = (ab[0], ab[1]);
        let len = (b - a).truncate().length();
        let mut s = 0.0;
        while s < len {
            let left = (if drawing { on } else { off }) - phase;
            let step = left.min(len - s);
            let q0 = a.lerp(b, s / len);
            let q1 = a.lerp(b, (s + step) / len);
            if drawing {
                if run.is_empty() {
                    run.push(q0);
                }
                run.push(q1);
            }
            s += step;
            phase += step;
            if phase >= (if drawing { on } else { off }) - 1e-4 {
                if drawing && run.len() >= 2 {
                    p.ribbon(&run, w_m, w_px, c, false);
                }
                run.clear();
                drawing = !drawing;
                phase = 0.0;
            }
        }
    }
    if drawing && run.len() >= 2 {
        p.ribbon(&run, w_m, w_px, c, false);
    }
}
