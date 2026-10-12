//! The scripted scenery objects of the loaded tiles as plain simulation state: their
//! scripts and variables, which of them run (the active set), and one step of them that
//! ends in a list of commands for the view (what to draw and play), in object order.

use super::{InUse, SceneryInstance, SceneryVars};
use crate::host::Arrival;
use crate::vehicle_api::Departure;
use crate::SimClock;
use glam::DVec3;
use omsi_map::calendar::DayKind;
use hashbrown::HashMap;

/// Objects farther than this from the listener do not run (OMSI's own cut).
pub const SCRIPT_RANGE: f64 = 800.0;

/// The side of the cells the camera moves through: the active set is made anew when it
/// crosses into another one (or tiles came or went), from the objects within reach of
/// anywhere in its cell.
const CELL: f64 = 50.0;

/// A placed scenery object with a running script: what the simulation keeps of it (its
/// render side, materials and sounds stay with the view, at the same index).
pub struct SceneryObject {
    pub map_id: i64,
    pub pos: DVec3,
    /// The tile it belongs to (it goes when the tile is unloaded).
    pub tile: (i32, i32),
    pub inst: SceneryInstance,
    /// Traffic light program of this object (crossings) or of its parent (lamps).
    pub controller: Option<usize>,
    pub light_index: usize,
    /// A child of a crossing that names one of its lights without being one of our lamps
    /// (its own textures to choose): (the crossing, the light). Its script reads that
    /// light's `TrafficLightPhase` and `TrafficLightApproach`, as Omsi.exe's
    /// RefreshAmpelParenting (0x77d460) hands them to any such child. The crossing's
    /// program is looked up when the script runs: it may stand on a tile loaded later.
    pub light_parent: Option<(i64, usize)>,
    /// `[varparent]`: the object whose data this one shows (a departure display's stop).
    pub var_parent: Option<i64>,
    /// The script asks for the buses due at its stop (`GetArrBus*`).
    pub arrivals: bool,
    /// It has `[texttexture]`s (drawn from its string variables).
    pub has_texts: bool,
    /// It has `[htmltexture]` pages.
    pub has_pages: bool,
    /// Its `[NightMapMode]` hours (the same building keeps its hours: seeded by its id).
    pub use_: InUse,
}

impl SceneryObject {
    /// `night_map_mode`: the type's `[NightMapMode]`.
    pub fn new(map_id: i64, pos: DVec3, tile: (i32, i32), inst: SceneryInstance, night_map_mode: i32) -> SceneryObject {
        SceneryObject {
            map_id,
            pos,
            tile,
            inst,
            controller: None,
            light_index: 0,
            light_parent: None,
            var_parent: None,
            arrivals: false,
            has_texts: false,
            has_pages: false,
            use_: InUse::new(night_map_mode, map_id as u64),
        }
    }
}

/// What the scripts read of the world this frame besides their own variables, gathered
/// before they run: the time and day, the daylight, the traffic lights and the timetable's
/// boards.
pub struct SceneryInputs<'a> {
    /// The scripts' time of day, the timetable's clock (None: they keep their own).
    pub clock: Option<&'a SimClock>,
    /// The time of day the buildings' hours and lights go by.
    pub time: f64,
    pub day: DayKind,
    pub brightness: f32,
    /// The listener: the scripts within [`SCRIPT_RANGE`] of it run.
    pub center: DVec3,
    /// `(controller, light)` -> (`TrafficLightPhase`, `TrafficLightApproach`).
    pub phase_of: &'a dyn Fn(usize, usize) -> (f32, f32),
    /// The crossing (map id) -> its light program, for `light_parent`.
    pub controllers: &'a HashMap<i64, usize>,
    /// Per bus stop (map object id): the buses due, soonest first, as (line, terminus,
    /// expected arrival in seconds of the day).
    pub by_stop: &'a HashMap<i64, Vec<(String, String, f64)>>,
    /// The departures made for the stop names the pages asked for, and their generation.
    pub departures: &'a std::collections::HashMap<String, Vec<Departure>>,
    pub departures_gen: u64,
    /// The HTML pages of objects nearer than this are driven.
    pub pages_near: f64,
}

/// What the view does for one object after a step, in object order.
#[derive(Debug)]
pub enum SceneryCmd {
    /// The object is out of range: its sounds stop.
    Silence(u32),
    /// The object ran: show what its script made of it (meshes, materials, texts, pages,
    /// sounds; the view reads its variables through [`SceneryState::get`]).
    Show(SceneryShow),
}

#[derive(Debug)]
pub struct SceneryShow {
    pub index: u32,
    /// From the listener (m).
    pub dist: f64,
    /// Its `NightlightA` this frame.
    pub nightlight: f32,
    /// The sound triggers its script fired.
    pub fired: Vec<String>,
    /// The new pictures of its pages: (script texture index, width, height, RGBA).
    pub pages: Vec<(usize, u32, u32, Vec<u8>)>,
    /// Its string variables as they were before its pages ran (only for an object with
    /// texts whose pages ran: the texts show the strings of before, as they always did).
    pub strings_before_pages: Option<Vec<String>>,
}

impl SceneryShow {
    /// The string variable `name` of object `o` as its texts show it this frame.
    pub fn text<'a>(&'a self, o: &'a SceneryObject, name: &str) -> &'a str {
        match &self.strings_before_pages {
            Some(strs) => o.inst.program.str_var(name).and_then(|i| strs.get(i as usize)).map(|s| s.as_str()).unwrap_or(""),
            None => o.inst.str_var(name),
        }
    }
}

/// The commands of one step and what the timetable is asked for.
#[derive(Default, Debug)]
pub struct SceneryOutputs {
    pub cmds: Vec<SceneryCmd>,
    /// The stops whose displays asked (sorted, each once).
    pub wanted: Vec<i64>,
    /// The stop names the HTML pages asked departures for, in the order first asked.
    pub wanted_names: Vec<String>,
    /// How many objects ran.
    pub ran: usize,
}

/// (object, its engine variables, its distance from the listener) for the parallel run;
/// kept between frames as a buffer of the same layout (see [`reuse`]).
type Job<'a> = (&'a mut SceneryObject, SceneryVars, f64, u32);
type JobBuf = (usize, SceneryVars, f64, u32);

/// Times of one step (seconds): gathering what the scripts are given, the scripts, what
/// came of them (pages, triggers).
#[derive(Default, Clone, Copy, Debug)]
pub struct StepTimes {
    pub inputs: f64,
    pub scripts: f64,
    pub post: f64,
}

/// The scripted scenery objects of the loaded tiles.
#[derive(Default)]
pub struct SceneryState {
    objects: Vec<SceneryObject>,
    /// The first object of each map id (a search of the list would find that one).
    by_id: HashMap<i64, u32>,
    /// The objects within reach of the camera's cell, by index (ascending).
    candidates: Vec<u32>,
    /// The cell the candidates were gathered for (None: to be gathered).
    cell: Option<(i64, i64)>,
    jobs: Vec<JobBuf>,
    out: SceneryOutputs,
    /// What the scripts took last frame, to give the next frame's run as many helpers as
    /// that work is worth.
    scripts_last: f64,
    /// Counts up whenever objects come or go.
    generation: u64,
}

/// A vector's allocation for another element type of the same size (the parallel run's
/// list borrows the objects for one frame; its buffer stays).
fn reuse<T, U>(mut v: Vec<T>) -> Vec<U> {
    v.clear();
    v.into_iter().map(|_| unreachable!()).collect()
}

impl SceneryState {
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn objects(&self) -> &[SceneryObject] {
        &self.objects
    }

    /// For changes that keep the list as it is (variables, triggers).
    pub fn objects_mut(&mut self) -> &mut [SceneryObject] {
        &mut self.objects
    }

    pub fn get(&self, index: u32) -> &SceneryObject {
        &self.objects[index as usize]
    }

    /// Counts up whenever objects come or go (indices kept elsewhere are good while it
    /// stays).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The first object of id `map_id`.
    pub fn find(&self, map_id: i64) -> Option<u32> {
        self.by_id.get(&map_id).copied()
    }

    pub fn find_mut(&mut self, map_id: i64) -> Option<&mut SceneryObject> {
        let i = self.find(map_id)?;
        Some(&mut self.objects[i as usize])
    }

    /// A newly placed object goes to the end of the list (its index is returned); it runs
    /// from the next step when it is within reach of the camera's cell.
    pub fn push(&mut self, o: SceneryObject) -> u32 {
        self.generation += 1;
        let i = self.objects.len() as u32;
        self.by_id.entry(o.map_id).or_insert(i);
        if let Some(cell) = self.cell {
            if near_cell(o.pos, cell) {
                self.candidates.push(i);
            }
        }
        self.objects.push(o);
        i
    }

    /// The objects of tile `key` go; `keep` is told for each object whether it stays, in
    /// order (to take the view's halves away with them).
    pub fn drop_tile(&mut self, key: (i32, i32), mut keep: impl FnMut(usize, bool)) -> bool {
        if !self.objects.iter().any(|o| o.tile == key) {
            return false;
        }
        self.generation += 1;
        let mut i = 0;
        self.objects.retain(|o| {
            let stays = o.tile != key;
            keep(i, stays);
            i += 1;
            stays
        });
        self.by_id.clear();
        for (i, o) in self.objects.iter().enumerate() {
            self.by_id.entry(o.map_id).or_insert(i as u32);
        }
        self.cell = None;
        true
    }

    /// The scripts of the objects within [`SCRIPT_RANGE`] of `inputs.center` run one frame
    /// (side by side on the worker threads, each on its own variables); the commands for
    /// the view come back in object order, as they did when the view walked the list.
    pub fn step(&mut self, dt: f32, inputs: &SceneryInputs, times: &mut StepTimes) {
        let t = std::time::Instant::now();
        let out = &mut self.out;
        out.cmds.clear();
        out.wanted.clear();
        out.wanted_names.clear();
        out.ran = 0;
        let cell = cell_of(inputs.center);
        if self.cell != Some(cell) {
            // (the objects that drop out of reach were all farther than the range: they are
            // silenced below, in order with the rest)
            let was = std::mem::take(&mut self.candidates);
            self.candidates.extend(
                self.objects.iter().enumerate().filter(|(_, o)| near_cell(o.pos, cell)).map(|(i, _)| i as u32),
            );
            let fresh = self.cell.is_none();
            self.cell = Some(cell);
            if !fresh {
                // the old ones that are no candidates any more (the list did not change)
                let mut j = 0;
                for &i in &was {
                    while j < self.candidates.len() && self.candidates[j] < i {
                        j += 1;
                    }
                    if self.candidates.get(j) != Some(&i) {
                        out.cmds.push(SceneryCmd::Silence(i));
                    }
                }
            } else {
                // (tiles came or went: any object may have been heard)
                let mut j = 0;
                for i in 0..self.objects.len() as u32 {
                    if self.candidates.get(j) == Some(&i) {
                        j += 1;
                    } else {
                        out.cmds.push(SceneryCmd::Silence(i));
                    }
                }
            }
        }
        let mut jobs: Vec<Job> = reuse(std::mem::take(&mut self.jobs));
        {
            let mut rest = self.objects.iter_mut();
            let mut next = 0u32;
            for &i in &self.candidates {
                let o = rest.nth((i - next) as usize).expect("candidate in the list");
                next = i + 1;
                let dist = (o.pos - inputs.center).length();
                if dist > SCRIPT_RANGE {
                    out.cmds.push(SceneryCmd::Silence(i));
                    continue;
                }
                let vars = gather(o, dist, inputs, out);
                jobs.push((o, vars, dist, i));
            }
        }
        // (the silences of objects that left the candidates, and of the candidates out of
        // range, in object order: as the old walk over the whole list stopped them)
        out.cmds.sort_unstable_by_key(|c| match c {
            SceneryCmd::Silence(i) => *i,
            SceneryCmd::Show(s) => s.index,
        });
        out.wanted.sort_unstable();
        out.wanted.dedup();
        times.inputs += t.elapsed().as_secs_f64();
        let t = std::time::Instant::now();
        // (on this thread, workers joining in as they are free: handed whole to the pool,
        // the frame waited on a latch for busy or unscheduled workers - 20-70 ms frames
        // standing still on a city map - for 4 ms of scripts)
        let clock = inputs.clock;
        omsi_cfg::par::frame_each(&mut jobs, omsi_cfg::par::helpers_for(self.scripts_last, 0.0005), |(o, vars, _, _)| {
            o.inst.update_on(dt, vars, clock);
        });
        self.scripts_last = t.elapsed().as_secs_f64();
        times.scripts += self.scripts_last;
        let t = std::time::Instant::now();
        for (o, vars, dist, index) in jobs.iter_mut() {
            // (what the view did between the scripts and the texts: `Refresh_Strings` is
            // cleared, then the pages run - and may change the strings the texts show)
            if o.has_texts {
                let _ = o.inst.take_refresh_strings();
            }
            let pages_run = o.has_pages && *dist < inputs.pages_near;
            let strings_before_pages = (pages_run && o.has_texts && !o.inst.html_textures.is_empty()).then(|| o.inst.state.str_vars.clone());
            let pages = if pages_run { o.inst.update_html_textures() } else { Vec::new() };
            let fired = std::mem::take(&mut o.inst.host.fired_triggers);
            out.cmds.push(SceneryCmd::Show(SceneryShow {
                index: *index,
                dist: *dist,
                nightlight: vars.nightlight,
                fired,
                pages,
                strings_before_pages,
            }));
        }
        out.ran = jobs.len();
        self.jobs = reuse(jobs);
        times.post += t.elapsed().as_secs_f64();
    }

    /// The last step's commands (to be taken by the view), with the objects to read.
    pub fn outputs(&mut self) -> (&mut SceneryOutputs, &[SceneryObject]) {
        (&mut self.out, &self.objects)
    }
}

/// What object `o` is given this frame (in order: the light programs and the boards are
/// read here), and what it asks the timetable for.
fn gather(o: &mut SceneryObject, dist: f64, inputs: &SceneryInputs, out: &mut SceneryOutputs) -> SceneryVars {
    // the object's own hours ([NightMapMode]): in use, and lit while in use and the
    // daylight under its own threshold (0.6, or 0.3-0.75 with a [NightMapMode])
    let in_use = o.use_.in_use(inputs.time, inputs.day);
    let light = match (o.controller, o.light_parent) {
        (Some(c), _) => Some((c, o.light_index)),
        (None, Some((parent, li))) => inputs.controllers.get(&parent).map(|&c| (c, li)),
        _ => None,
    };
    let vars = SceneryVars {
        nightlight: o.use_.lit(inputs.time, inputs.day, inputs.brightness) as i32 as f32,
        in_use: in_use as i32 as f32,
        traffic_light_phase: light
            .map(|(c, li)| (inputs.phase_of)(c, li).0)
            .unwrap_or(crate::traffic::UNLINKED_PHASE as f32),
        traffic_light_approach: light.map(|(c, li)| (inputs.phase_of)(c, li).1).unwrap_or(0.0),
        switch: None,
    };
    // a departure display: the buses due at its stop
    if let (true, Some(stop)) = (o.arrivals, o.var_parent) {
        out.wanted.push(stop);
        let now = inputs.clock.map(|c| c.time).unwrap_or(0.0);
        // The list changes about once a second, the times to go every frame: the
        // same buses keep their names and only count down (the whole list was made
        // anew, its names copied, every frame for every board).
        let arrivals = &mut o.inst.host.arrivals;
        match inputs.by_stop.get(&stop) {
            Some(l)
                if arrivals.len() == l.len()
                    && arrivals.iter().zip(l).all(|(a, (line, terminus, _))| a.line == *line && a.terminus == *terminus) =>
            {
                for (a, (_, _, t)) in arrivals.iter_mut().zip(l) {
                    a.due = (t - now) as f32;
                }
            }
            Some(l) => {
                *arrivals = l
                    .iter()
                    .map(|(line, terminus, t)| Arrival { line: line.clone(), terminus: terminus.clone(), due: (t - now) as f32 })
                    .collect();
            }
            None => arrivals.clear(),
        }
    }
    // an HTML page that asks for departures by stop name
    if o.has_pages && dist < inputs.pages_near && !o.inst.host.html_departure_wants.is_empty() {
        for k in &o.inst.host.html_departure_wants {
            if !out.wanted_names.contains(k) {
                out.wanted_names.push(k.clone());
            }
        }
        if o.inst.host.html_departures_gen != inputs.departures_gen {
            o.inst.host.html_departures = o
                .inst
                .host
                .html_departure_wants
                .iter()
                .filter_map(|k| inputs.departures.get(k).map(|l| (k.clone(), l.clone())))
                .collect();
            o.inst.host.html_departures_gen = inputs.departures_gen;
        }
    }
    vars
}

fn cell_of(p: DVec3) -> (i64, i64) {
    ((p.x / CELL).floor() as i64, (p.y / CELL).floor() as i64)
}

/// Whether an object at `p` can come within range of a camera anywhere in `cell` (a metre
/// to spare for the rounding).
fn near_cell(p: DVec3, cell: (i64, i64)) -> bool {
    let c = DVec3::new((cell.0 as f64 + 0.5) * CELL, (cell.1 as f64 + 0.5) * CELL, 0.0);
    let reach = SCRIPT_RANGE + CELL * std::f64::consts::FRAC_1_SQRT_2 + 1.0;
    let d = p - c;
    // (height is part of the distance the range is measured in; leaving it out keeps more)
    d.x * d.x + d.y * d.y <= reach * reach
}

#[cfg(test)]
mod tests {
    use super::*;
    use omsi_script::Program;
    use std::sync::Arc;

    fn object(map_id: i64, pos: DVec3, tile: (i32, i32)) -> SceneryObject {
        let inst = SceneryInstance::new(Arc::new(Program::default()), &[], SimClock::default(), &[]);
        SceneryObject::new(map_id, pos, tile, inst, 0)
    }

    fn step(s: &mut SceneryState, center: DVec3) -> (Vec<u32>, Vec<u32>) {
        let phase = |_: usize, _: usize| (0.0, 0.0);
        let (controllers, by_stop, departures) = (HashMap::new(), HashMap::new(), Default::default());
        let inputs = SceneryInputs {
            clock: None,
            time: 0.0,
            day: DayKind::default(),
            brightness: 1.0,
            center,
            phase_of: &phase,
            controllers: &controllers,
            by_stop: &by_stop,
            departures: &departures,
            departures_gen: 0,
            pages_near: 60.0,
        };
        s.step(1.0 / 30.0, &inputs, &mut StepTimes::default());
        let (out, _) = s.outputs();
        let (mut shown, mut silenced) = (Vec::new(), Vec::new());
        for c in &out.cmds {
            match c {
                SceneryCmd::Silence(i) => {
                    assert!(shown.is_empty(), "the silences come first");
                    silenced.push(*i)
                }
                SceneryCmd::Show(s) => shown.push(s.index),
            }
        }
        assert!(silenced.windows(2).all(|w| w[0] < w[1]) && shown.windows(2).all(|w| w[0] < w[1]), "in object order");
        (shown, silenced)
    }

    /// The objects that run are exactly those within range, in list order, wherever the
    /// camera goes; one that was heard and is out of range now is silenced.
    #[test]
    fn the_active_set_is_the_range_cut() {
        let mut s = SceneryState::default();
        let mut seed = 7u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 11) as f64 / (1u64 << 53) as f64) * 4000.0 - 2000.0
        };
        for i in 0..3000 {
            let p = DVec3::new(rnd(), rnd(), rnd() / 100.0);
            s.push(object(i, p, ((p.x / 300.0).floor() as i32, 0)));
        }
        let mut last: Vec<u32> = Vec::new();
        for k in 0..400 {
            let t = k as f64;
            // (along a road, then a jump across the map, then back)
            let center = if k == 200 { DVec3::new(1500.0, -1500.0, 3.0) } else { DVec3::new(-1500.0 + t * 7.3, (t * 0.05).sin() * 600.0, 2.0) };
            if k == 300 {
                s.drop_tile((2, 0), |_, _| {});
                last.clear();
            }
            if k == 310 {
                s.push(object(99_999, center + DVec3::new(10.0, 0.0, 0.0), (9, 9)));
            }
            let (shown, silenced) = step(&mut s, center);
            let expected: Vec<u32> =
                s.objects().iter().enumerate().filter(|(_, o)| !((o.pos - center).length() > SCRIPT_RANGE)).map(|(i, _)| i as u32).collect();
            assert_eq!(shown, expected, "step {k}");
            for i in &last {
                assert!(shown.contains(i) || silenced.contains(i), "step {k}: object {i} went out of range unheard");
            }
            last = shown;
        }
        assert!(s.find(99_999).is_some());
    }

    #[test]
    fn find_gives_the_first_of_an_id_after_a_tile_went() {
        let mut s = SceneryState::default();
        s.push(object(5, DVec3::ZERO, (0, 0)));
        s.push(object(5, DVec3::ZERO, (1, 0)));
        s.push(object(6, DVec3::ZERO, (1, 0)));
        assert_eq!(s.find(5), Some(0));
        let mut kept = Vec::new();
        assert!(s.drop_tile((0, 0), |i, stays| kept.push((i, stays))));
        assert_eq!(kept, vec![(0, false), (1, true), (2, true)]);
        assert_eq!((s.find(5), s.find(6), s.len()), (Some(0), Some(1), 2));
        assert!(!s.drop_tile((7, 7), |_, _| unreachable!()));
    }
}
