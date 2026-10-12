//! Signals, switches, particles, HTML objects and scripted scenery objects.
use super::*;

/// An object whose night textures are lit by a `[NightMapMode]` timetable.
#[derive(Debug, Clone, Copy)]
pub struct NightMode {
    pub inst: usize,
    /// This object's in-use window and darkness threshold (see [`InUse`]).
    pub use_: InUse,
    pub slots: usize,
}

pub use omsi_sim::scenery::InUse;
pub use omsi_map::calendar::DayKind;

/// A scripted object's dynamic texture change: its type, the variants chosen, its mesh
/// instances and the slots' switches.
type TextureUpdate = (Arc<ObjectType>, Vec<usize>, Vec<usize>, Vec<((usize, usize), bool)>);

/// Where a frame's scenery scripts went (seconds), for OMSI_PROFILE: waiting for the script
/// list (the tile loader places objects under the same lock), gathering what the scripts are
/// given, the scripts themselves, applying what they did (text and HTML textures, sounds,
/// materials, transforms), the dynamic textures, the lamps' sounds.
#[derive(Default, Clone, Copy, Debug)]
pub struct ScriptedTimes {
    pub lock: f64,
    pub inputs: f64,
    pub scripts: f64,
    pub apply: f64,
    pub textures: f64,
    pub lamps: f64,
}

/// The scripted scenery objects of the loaded tiles: their scripts and variables as the
/// simulation's own state, and beside each (at the same index) what the view keeps of it.
#[derive(Default)]
pub struct ScriptedSet {
    pub sim: omsi_sim::scenery::SceneryState,
    pub views: Vec<ScriptedView>,
}

/// The render side of a scripted scenery object (see [`ScriptedObject`] for the fields).
pub struct ScriptedView {
    pub ty: Arc<ObjectType>,
    pub xf: Mat4,
    pub instances: Vec<usize>,
    pub variants: Vec<(usize, usize, MaterialId, MaterialId, String)>,
    pub sounds: Option<omsi_audio::SoundSet>,
    pub texts: Vec<(TextureId, omsi_sim::texttex::TextTextureState)>,
    pub htmls: Vec<(usize, TextureId)>,
    pub alpha_slots: Vec<LampSlots>,
    pub alpha_last: Vec<Vec<f32>>,
}

impl ScriptedSet {
    /// A placed object: its script goes to the simulation's list, the rest to the view's.
    pub fn push(&mut self, o: ScriptedObject) {
        let ScriptedObject {
            ty,
            pos,
            xf,
            instances,
            inst,
            controller,
            light_index,
            light_parent,
            map_id,
            variants,
            sounds,
            tile,
            var_parent,
            texts,
            arrivals,
            htmls,
            alpha_slots,
            alpha_last,
        } = o;
        let mut s = omsi_sim::scenery::SceneryObject::new(map_id, pos, tile, inst, ty.sco.night_map_mode);
        s.controller = controller;
        s.light_index = light_index;
        s.light_parent = light_parent;
        s.var_parent = var_parent;
        s.arrivals = arrivals;
        s.has_texts = !texts.is_empty();
        s.has_pages = !htmls.is_empty();
        self.sim.push(s);
        self.views.push(ScriptedView { ty, xf, instances, variants, sounds, texts, htmls, alpha_slots, alpha_last });
    }

    pub fn len(&self) -> usize {
        self.views.len()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// (object, view) in the list's order.
    pub fn iter(&self) -> impl Iterator<Item = (&omsi_sim::scenery::SceneryObject, &ScriptedView)> {
        self.sim.objects().iter().zip(&self.views)
    }

    /// The first object of id `map_id`, and its view.
    pub fn find_mut(&mut self, map_id: i64) -> Option<(&mut omsi_sim::scenery::SceneryObject, &mut ScriptedView)> {
        let i = self.sim.find(map_id)? as usize;
        Some((&mut self.sim.objects_mut()[i], &mut self.views[i]))
    }

    /// The objects of tile `key` go (their sounds stop).
    pub fn drop_tile(&mut self, key: (i32, i32), audio: Option<&omsi_audio::AudioEngine>) {
        if !self.sim.objects().iter().any(|o| o.tile == key) {
            return;
        }
        let mut old = std::mem::take(&mut self.views).into_iter();
        let mut kept = Vec::with_capacity(old.len());
        self.sim.drop_tile(key, |_, stays| {
            let Some(mut v) = old.next() else { return };
            if stays {
                kept.push(v);
            } else if let (Some(a), Some(mut ss)) = (audio, v.sounds.take()) {
                ss.stop_all(a);
            }
        });
        self.views = kept;
    }
}

impl std::ops::Index<usize> for ScriptedSet {
    type Output = omsi_sim::scenery::SceneryObject;

    fn index(&self, i: usize) -> &Self::Output {
        &self.sim.objects()[i]
    }
}

impl World {
    /// Light or darken the night textures of the objects with a `[NightMapMode]` timetable
    /// for this hour of the day (0..24).
    pub fn update_night_modes(&self, renderer: &Renderer, scene: &mut Scene, clock: &omsi_sim::SimClock, brightness: f32) {
        let day = self.day_kind(clock);
        for m in self.night_modes.lock().iter() {
            let v = if m.use_.lit(clock.time, day, brightness) { 1.0 } else { 0.0 };
            renderer.set_slot_night(scene, m.inst, &vec![v; m.slots]);
        }
    }

    /// A number plate from `registrations.txt` for a vehicle with `[registration_free]`
    /// (OMSI gives such a vehicle's `ident` a random line of it at the spawn).
    pub fn free_registration(&self, seed: u64) -> Option<String> {
        let list = self.registrations.get_or_init(|| {
            self.chrono_dirs
                .read()
                .iter()
                .rev()
                .chain(std::iter::once(&self.map_dir))
                .map(|d| omsi_cfg::resolve_path(d, "registrations.txt"))
                .find(|p| omsi_cfg::vfs::is_file(p))
                .map(|p| omsi_map::ailists::load_list(&p))
                .unwrap_or_default()
        });
        (!list.is_empty()).then(|| list[(seed % list.len() as u64) as usize].clone())
    }

    /// Working day, holiday and school holidays at `clock`'s date, from the map's
    /// `Holidays.txt`.
    pub fn day_kind(&self, clock: &omsi_sim::SimClock) -> DayKind {
        let cal = self.calendar.get_or_init(|| omsi_map::Calendar::load(&self.map_dir.join("Holidays.txt")).unwrap_or_default());
        let date = clock.date_code();
        DayKind { workday: clock.weekday() < 5, holiday: cal.is_holiday(date), school_holiday: cal.in_holiday_range(date) }
    }

    /// Switch the `NightlightA` material variants of static objects.
    pub fn set_lamps(&self, renderer: &Renderer, scene: &mut Scene, on: bool) {
        for (inst, slot, m_on, m_off) in self.night_slots.lock().iter() {
            renderer.set_material(scene, *inst, *slot, if on { *m_on } else { *m_off });
        }
    }

    /// Whether switch object `id` is set to its path `path` (None: no such switch, or the
    /// path has no `[switchdir]`).
    pub fn switch_set_to(&self, id: i64, path: u16) -> Option<bool> {
        let scripted = self.scripted.lock();
        let i = scripted.sim.find(id)? as usize;
        let (o, v) = (&scripted.sim.objects()[i], &scripted.views[i]);
        let d = (*v.ty.sco.path_switch_dir.get(path as usize)?)?;
        Some(o.inst.var("Switch").map(|v| (v - d as f32).abs() < 0.5).unwrap_or(false))
    }

    /// Set the railway signals: `aspects` gives each signal object's `Signal` (0 stop, 1 go,
    /// 2 go at the route's speed limit) as the traffic worked it out, and every signal
    /// learns what the next one shows (`NextSignal`) - a distant signal what its main
    /// signal shows.
    pub fn set_signals(&self, aspects: &HashMap<i64, f32>) {
        if self.signal_routes.is_empty() {
            return;
        }
        let mut next: HashMap<i64, f32> = HashMap::new();
        for r in &self.signal_routes {
            let own = aspects.get(&r.signal.0).copied().unwrap_or(0.0);
            if let Some(d) = r.dist_signal {
                let e = next.entry(d).or_insert(0.0);
                *e = e.max(own);
            }
            if let Some(n) = r.next_signal {
                let e = next.entry(r.signal.0).or_insert(0.0);
                *e = e.max(aspects.get(&n).copied().unwrap_or(0.0));
            }
        }
        // (the signals of the routes, gathered once when the map opened)
        let ids = &self.signal_ids;
        let mut scripted = self.scripted.lock();
        for o in scripted.sim.objects_mut() {
            if !ids.contains(&o.map_id) {
                continue;
            }
            let a = aspects.get(&o.map_id).copied().unwrap_or(0.0);
            if o.inst.var("Signal") != Some(a) {
                o.inst.set_var("Signal", a);
                if omsi_cfg::flags::OMSI_DEBUG_SIGNALS.is_set() {
                    log::info!("signal {} at ({:.0}, {:.0}) shows {a}", o.map_id, o.pos.x, o.pos.y);
                }
            }
            o.inst.set_var("NextSignal", next.get(&o.map_id).copied().unwrap_or(0.0));
        }
    }

    /// The echo at `p`: (reverberation time, how much of it is heard) - full inside an
    /// underpass's box, fading over its edge distance at the sides.
    pub fn reverb_at(&self, p: DVec3) -> (f32, f32) {
        let me = omsi_sim::collision::Obb::point(p, 0.01);
        let mut best = (0.0f32, 0.0f32);
        for (b, time, fade) in self.reverb_zones.lock().iter() {
            if p.z < b.z0 - 1.0 || p.z > b.z1 + 1.0 {
                continue;
            }
            let inside = (-me.separation(b)) as f32;
            let mix = (inside / fade.max(0.1)).clamp(0.0, 1.0);
            if mix > best.1 {
                best = (*time, mix);
            }
        }
        best
    }

    /// The `[htmltexture]` page of a scenery object a ray lands on (within `reach` metres).
    /// The nearest triangle of those objects decides, as for the bus's pages: a part of
    /// the object in front of its page takes the click away from it.
    pub fn html_object_hit(&self, origin: DVec3, dir: glam::Vec3, reach: f32) -> Option<PageHit> {
        let scripted = self.scripted.lock();
        let mut best: Option<(f32, Option<PageHit>)> = None;
        for (o, v) in scripted.iter().filter(|(_, v)| !v.htmls.is_empty()) {
            if (o.pos - origin).length() > reach as f64 + 60.0 {
                continue;
            }
            let local = (origin - o.pos).as_vec3();
            for mi in 0..v.instances.len() {
                let Some((data, o3d_mats, overrides)) = v.ty.meshes.get(mi) else { continue };
                if !o.inst.mesh_visible.get(mi).copied().unwrap_or(true) {
                    continue;
                }
                let xf = v.xf * o.inst.mesh_transforms.get(mi).copied().unwrap_or(Mat4::IDENTITY);
                let Some(hit) = omsi_geometry::ray_mesh_hit(local, dir, data, &xf) else { continue };
                if hit.t > reach || best.as_ref().is_some_and(|b| b.0 <= hit.t) {
                    continue;
                }
                // the page the hit material slot shows (a slot that shows none is in the way)
                let slot = data.slot_of(hit.index) as usize;
                let page = overrides
                    .iter()
                    .filter(|m| !m.item && omsi_sim::vehicle::override_slot(o3d_mats, m) == Some(slot))
                    .find_map(|m| m.use_script_texture)
                    .map(|n| n.max(0) as usize)
                    .filter(|n| v.htmls.iter().any(|(i, _)| i == n));
                let page = page.map(|page| PageHit {
                    t: hit.t,
                    map_id: o.map_id,
                    page,
                    u: hit.uv.x.clamp(0.0, 1.0),
                    v: hit.uv.y.clamp(0.0, 1.0),
                });
                best = Some((hit.t, page));
            }
        }
        best.and_then(|b| b.1)
    }

    /// A press, release or move on a page of a scenery object (see [`Self::html_object_hit`]).
    /// What the page does (`omsi.setVar`, `omsi.trigger`) reaches the object's script.
    pub fn html_object_pointer(&self, map_id: i64, page: usize, u: f32, v: f32, kind: omsi_sim::htmltex::PointerKind) -> bool {
        let mut scripted = self.scripted.lock();
        match scripted.find_mut(map_id) {
            Some((o, _)) => o.inst.html_pointer(page, u, v, kind),
            None => false,
        }
    }

    /// Finds the closest scenery object mesh carrying a `[mouseevent]` under a ray.
    pub fn scenery_object_hit(&self, origin: DVec3, dir: glam::Vec3, reach: f32, spread: f32) -> Option<SceneryHit> {
        let scripted = self.scripted.lock();
        let mut best: Option<SceneryHit> = None;
        let right = glam::Vec3::new(-dir.y, dir.x, 0.0).normalize_or_zero();
        let up = dir.cross(right).normalize_or_zero();
        let dirs = if spread > 0.0 {
            vec![
                dir,
                (dir + right * spread).normalize(),
                (dir - right * spread).normalize(),
                (dir + up * spread).normalize(),
                (dir - up * spread).normalize(),
            ]
        } else {
            vec![dir]
        };
        for (o, v) in scripted.iter().filter(|(_, v)| v.ty.has_mouse_events) {
            if (o.pos - origin).length() > reach as f64 + 60.0 {
                continue;
            }
            let local = (origin - o.pos).as_vec3();
            for mi in 0..v.ty.meshes.len() {
                let Some((data, _, _)) = v.ty.meshes.get(mi) else { continue };
                if !o.inst.mesh_visible.get(mi).copied().unwrap_or(true) {
                    continue;
                }
                let Some(&def_idx) = v.ty.mesh_def_index.get(mi) else { continue };
                let Some(event) = v.ty.model.meshes.get(def_idx).and_then(|m| m.mouse_event.as_ref()) else { continue };
                let xf = v.xf * o.inst.mesh_transforms.get(mi).copied().unwrap_or(Mat4::IDENTITY);
                for d in &dirs {
                    if let Some(t) = omsi_geometry::ray_mesh(local, *d, data, &xf) {
                        if t <= reach && best.as_ref().map_or(true, |b| t < b.t) {
                            best = Some(SceneryHit {
                                map_id: o.map_id,
                                mesh_index: mi,
                                event: event.clone(),
                                t,
                            });
                            break;
                        }
                    }
                }
            }
        }
        best
    }

    /// Click down on a scenery object's `[mouseevent]` switch or button.
    pub fn scenery_object_click(&self, map_id: i64, event: &str) -> bool {
        let mut scripted = self.scripted.lock();
        let Some((o, _)) = scripted.find_mut(map_id) else {
            return false;
        };
        log::info!("scenery mouse event {event} on object {map_id}");
        let ok = o.inst.trigger(event);
        let drag = format!("{event}_drag");
        let low = event.to_ascii_lowercase();
        if (low.contains("taste") || low.contains("button") || low.contains("click"))
            && o.inst.program.trigger(&drag).is_some()
        {
            o.inst.host.mouse = (0.0, 0.0);
            o.inst.trigger(&drag);
            o.inst.host.mouse = (0.0, 0.0);
        }
        ok
    }

    /// Mouse dragged while holding down a scenery object switch.
    pub fn scenery_object_drag(&self, map_id: i64, event: &str, dx: f32, dy: f32) -> bool {
        let mut scripted = self.scripted.lock();
        let Some((o, _)) = scripted.find_mut(map_id) else {
            return false;
        };
        let drag = format!("{event}_drag");
        o.inst.host.mouse = (dx, dy);
        let ok = o.inst.trigger(&drag);
        o.inst.host.mouse = (0.0, 0.0);
        ok
    }

    /// Mouse button released from a scenery object switch (`<event>_off`).
    pub fn scenery_object_release(&self, map_id: i64, event: &str) -> bool {
        let mut scripted = self.scripted.lock();
        let Some((o, _)) = scripted.find_mut(map_id) else {
            return false;
        };
        let off = format!("{event}_off");
        o.inst.trigger(&off)
    }

    /// Mouse wheel notch over a scenery object switch.
    pub fn scenery_object_wheel(&self, map_id: i64, event: &str, amount: f32) -> bool {
        let mut scripted = self.scripted.lock();
        let Some((o, _)) = scripted.find_mut(map_id) else {
            return false;
        };
        o.inst.host.mouse = (0.0, amount);
        let _ = o.inst.trigger(&format!("{event}_drag"));
        o.inst.host.mouse = (0.0, 0.0);
        o.inst.trigger(&format!("{event}_off"))
    }

    /// The colour the tile's night light map (its own part, see [`own_tile_of_light_map`])
    /// has at `pos` (0..1, bilinear), or `None` where no light map is loaded: the light it
    /// throws on a vehicle standing there (Omsi.exe samples it at the vehicle's place,
    /// 0x61378c, for its ambient light and `Envir_Brightness`).
    pub fn light_map_light_at(&self, pos: DVec3) -> Option<glam::Vec3> {
        let ts = tile_size();
        let key = ((pos.x / ts).floor() as i32, (pos.y / ts).floor() as i32);
        let img = self.light_maps.lock().get(&key).cloned()?;
        let (w, h) = (img.width as usize, img.height as usize);
        if w == 0 || h == 0 || img.rgba.len() < w * h * 4 {
            return None;
        }
        let u = ((pos.x / ts - key.0 as f64) * w as f64 - 0.5).clamp(0.0, (w - 1) as f64);
        let v = ((1.0 - (pos.y / ts - key.1 as f64)) * h as f64 - 0.5).clamp(0.0, (h - 1) as f64);
        let (x0, y0) = (u.floor() as usize, v.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
        let (fx, fy) = ((u - x0 as f64) as f32, (v - y0 as f64) as f32);
        let px = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            glam::Vec3::new(img.rgba[i] as f32, img.rgba[i + 1] as f32, img.rgba[i + 2] as f32) / 255.0
        };
        let top = px(x0, y0).lerp(px(x1, y0), fx);
        let bottom = px(x0, y1).lerp(px(x1, y1), fx);
        Some(top.lerp(bottom, fy))
    }

    /// Fill the light map atlas with the 5x5 tiles around `eye` (when it moved to another
    /// tile or tiles came or went): the splines and `[LightMapMapping]` objects are lit by it
    /// at night as the terrain is.
    pub fn update_light_map_atlas(&self, renderer: &Renderer, eye: DVec3) {
        // (`OMSI_NO_LIGHT_MAP=1`: the tiles' night light maps left out, for an A/B)
        if omsi_cfg::flags::OMSI_NO_LIGHT_MAP.is_set() {
            return;
        }
        let ts = tile_size();
        let centre = ((eye.x / ts).floor() as i32, (eye.y / ts).floor() as i32);
        let generation = self.light_maps_generation.load(std::sync::atomic::Ordering::Relaxed);
        let mut last = self.light_map_atlas.lock();
        if *last == Some((centre, generation)) {
            return;
        }
        *last = Some((centre, generation));
        let n = omsi_render::LM_ATLAS_TILES as i32;
        let maps = self.light_maps.lock();
        for row in 0..n {
            for col in 0..n {
                // column from the west, row from the north
                let key = (centre.0 - n / 2 + col, centre.1 + n / 2 - row);
                renderer.set_light_map_tile((col as u32, row as u32), maps.get(&key).map(|a| a.as_ref()));
            }
        }
        let sw = ((centre.0 - n / 2) as f64 * ts, (centre.1 - n / 2) as f64 * ts);
        renderer.set_light_map_place(sw.0, sw.1, n as f64 * ts);
    }

    /// Throw the points the trains need (`Traffic::switch_requests`): a switch object whose
    /// `[path]` carries a `[switchdir]` gets that value in its script's `Switch` variable,
    /// and its blades turn with it.
    pub fn set_switches(&self, requests: &[(i64, u16)]) {
        if requests.is_empty() {
            return;
        }
        let mut scripted = self.scripted.lock();
        // (the first object of each id, as a search would find it: every train asks for the
        // points of its next five lanes on every frame)
        for &(id, path) in requests {
            let Some((o, v)) = scripted.find_mut(id) else { continue };
            if let Some(Some(d)) = v.ty.sco.path_switch_dir.get(path as usize) {
                let was = o.inst.var("Switch");
                if o.inst.set_var("Switch", *d as f32) && was != Some(*d as f32) && omsi_cfg::flags::OMSI_DEBUG_SWITCHES.is_set() {
                    log::info!("switch {} ({}) at ({:.0}, {:.0}) thrown to {d} for a train", id, v.ty.sco.path.display(), o.pos.x, o.pos.y);
                }
            }
        }
    }

    /// Move the particles of the placed objects within 1.5 km of `center`, their variables
    /// read from the object's script (the fireworks' frequency).
    pub fn update_particles(&self, dt: f32, center: DVec3) {
        let mut objs = self.particle_objects.lock();
        if objs.is_empty() {
            return;
        }
        let scripted = self.scripted.lock();
        // (an environment switch read once per process)
        static DEBUG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *DEBUG.get_or_init(|| omsi_cfg::flags::OMSI_DEBUG_PARTICLES.is_set()) {
            let mut near: Vec<(f64, &ParticleObject)> = objs.values().flatten().map(|po| ((po.pos - center).length(), po)).collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            for (d, po) in near.iter().take(3) {
                log::info!("particle object {} at ({:.1}, {:.1}, {:.1}), {d:.0} m: {} particles", po.map_id, po.pos.x, po.pos.y, po.pos.z, po.set.particles().count());
            }
        }
        // (the scripted object whose variables an emitter reads: the first of its id, as a
        // search would find it)
        for list in objs.values_mut() {
            for po in list.iter_mut() {
                if (po.pos - center).length() > 1500.0 {
                    continue;
                }
                let inst = scripted.sim.find(po.map_id).map(|i| &scripted.sim.get(i).inst);
                let value = |n: &str| inst.and_then(|i| i.var(n)).unwrap_or(0.0);
                po.set.update(dt, po.pos, po.rot, &value);
            }
        }
    }

    /// The departures for the HTML pages of the player's vehicle: the stop names its pages asked
    /// for go to the boards, and the departures made for them come back into its host.
    pub fn sync_html_departures(&self, host: &mut omsi_sim::host::VehicleHost) {
        if host.html_departure_wants.is_empty() {
            return;
        }
        let mut boards = self.timetable_boards.lock();
        for k in &host.html_departure_wants {
            if !boards.wanted_names.contains(k) {
                boards.wanted_names.push(k.clone());
            }
        }
        if host.html_departures_gen != boards.departures_gen {
            host.html_departures = host
                .html_departure_wants
                .iter()
                .filter_map(|k| boards.departures.get(k).map(|l| (k.clone(), l.clone())))
                .collect();
            host.html_departures_gen = boards.departures_gen;
        }
    }

    /// Run the scripts and animations of the placed objects near `center` and push their
    /// mesh transforms / visibility to the renderer. `phase_of(controller, light)` gives the
    /// light's current state (the `TrafficLightPhase` value) and whether a vehicle is
    /// asking for it (`TrafficLightApproach`).
    pub fn update_scripted(
        &self,
        renderer: &Renderer,
        scene: &mut Scene,
        dt: f32,
        center: DVec3,
        brightness: f32,
        phase_of: &dyn Fn(usize, usize) -> (f32, f32),
        audio: Option<&omsi_audio::AudioEngine>,
        muffled: bool,
        times: &mut ScriptedTimes,
    ) -> usize {
        self.update_particles(dt, center);
        let mut updated = 0;
        let now = self.script_clock();
        let day = self.day_kind(&now);
        let t = std::time::Instant::now();
        let mut scripted = self.scripted.lock();
        let mut boards = self.timetable_boards.lock();
        times.lock += t.elapsed().as_secs_f64();
        let mut texture_updates: Vec<TextureUpdate> = Vec::new();
        // The scripts run in the simulation's list (what each is given gathered in order
        // first, then the scripts side by side on the worker threads), and what they did
        // comes back as commands, in object order, applied here.
        let ScriptedSet { sim, views } = &mut *scripted;
        let mut step = omsi_sim::scenery::StepTimes::default();
        {
            let controllers = self.controller_of_object.lock();
            let inputs = omsi_sim::scenery::SceneryInputs {
                clock: boards.clock.as_ref(),
                time: now.time,
                day,
                brightness,
                center,
                phase_of,
                controllers: &controllers,
                by_stop: &boards.by_stop,
                departures: &boards.departures,
                departures_gen: boards.departures_gen,
                pages_near: HTML_OBJECT_NEAR,
            };
            sim.step(dt, &inputs, &mut step);
        }
        times.inputs += step.inputs;
        times.scripts += step.scripts;
        let t = std::time::Instant::now();
        let (out, objects) = sim.outputs();
        for cmd in out.cmds.iter_mut() {
            let s = match cmd {
                omsi_sim::scenery::SceneryCmd::Silence(i) => {
                    if let (Some(a), Some(mut ss)) = (audio, views[*i as usize].sounds.take()) {
                        ss.stop_all(a);
                    }
                    continue;
                }
                omsi_sim::scenery::SceneryCmd::Show(s) => s,
            };
            let (o, v) = (&objects[s.index as usize], &mut views[s.index as usize]);
            let (dist, nightlight) = (s.dist, s.nightlight);
            // text textures from the script's strings whenever they change (`update` leaves
            // an unchanged one alone): read only on `Refresh_Strings`, a board whose string
            // was still empty at its first frame stayed blank for good (#367)
            if !v.texts.is_empty() {
                for (tex, st) in v.texts.iter_mut() {
                    // (read in place: a copy each frame was only compared with the last)
                    let text = s.text(o, st.def.variable.trim());
                    if st.update(text) {
                        let (w, h) = (st.def.width.max(1) as u32, st.def.height.max(1) as u32);
                        if let Some(rgba) = st.pending.take() {
                            // OMSI_DUMP_SCENERY_TEXT=<dir>: the pictures as drawn
                            if let Some(dir) = omsi_cfg::flags::OMSI_DUMP_SCENERY_TEXT.os() {
                                let path = std::path::Path::new(&dir)
                                    .join(format!("text_{}.png", o.map_id));
                                log::info!(
                                    "scenery text of object {}: {text:?} -> {}",
                                    o.map_id,
                                    path.display()
                                );
                                if let Some(img) = image::RgbaImage::from_raw(w, h, rgba.clone()) {
                                    let _ = img.save(&path);
                                }
                            }
                            renderer.update_texture_mips(
                                scene,
                                *tex,
                                &Image {
                                    width: w,
                                    height: h,
                                    rgba,
                                    has_alpha: true,
                                },
                            );
                        }
                    }
                }
            }
            // [htmltexture] pages: only near the listener (a page is a whole browser frame;
            // the simulation drove them, here come their pictures)
            for (index, w, h, rgba) in s.pages.drain(..) {
                if let Some((_, tex)) = v.htmls.iter().find(|(i, _)| *i == index) {
                    renderer.update_texture(scene, *tex, &Image { width: w, height: h, rgba, has_alpha: true });
                }
            }
            // [sound] of scenery objects: crossing bells, ambient loops
            let fired: Vec<String> = std::mem::take(&mut s.fired);
            // (out of earshot with nothing playing: nothing to do - finding the sound file
            // for each of a city's scripted objects every frame took 1.8 ms)
            let near = dist < 300.0 || v.sounds.is_some();
            if let (Some(a), true) = (audio, near) {
                let ty = v.ty.clone();
                let path = ty.sound_path.get_or_init(|| {
                    let rel = ty.sco.sound.as_ref()?;
                    let dir = ty.sco.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                    Some(omsi_cfg::resolve_path(&dir, rel))
                });
                if let Some(path) = path {
                    let inst = &o.inst;
                    self.object_sounds(a, &mut v.sounds, path, dist, muffled, o.pos, v.xf, &|n| inst.var(n), &fired);
                }
            }
            for (inst, slot, base, item, var) in &v.variants {
                let x = if var.trim().eq_ignore_ascii_case("NightlightA") {
                    nightlight
                } else {
                    var.trim()
                        .parse()
                        .ok()
                        .or_else(|| o.inst.var(var))
                        .unwrap_or(0.0)
                };
                renderer.set_material(scene, *inst, *slot, if change_picks_item(x) { *item } else { *base });
            }
            if !v.ty.dynamic_textures.is_empty() {
                let selection = scenery_texture_selection(&v.ty, &o.inst);
                let switches = v
                    .variants
                    .iter()
                    .map(|(inst, slot, _, _, var)| {
                        let value = if var.trim().eq_ignore_ascii_case("NightlightA") {
                            nightlight
                        } else {
                            var.trim()
                                .parse::<f32>()
                                .ok()
                                .or_else(|| o.inst.var(var))
                                .unwrap_or(0.0)
                        };
                        ((*inst, *slot), change_picks_item(value))
                    })
                    .collect();
                texture_updates.push((v.ty.clone(), selection, v.instances.clone(), switches));
            }
            for (k, ((inst, xf), &visible)) in v.instances.iter().zip(&o.inst.mesh_transforms).zip(&o.inst.mesh_visible).enumerate() {
                renderer.set_transform(scene, *inst, o.pos, v.xf * *xf);
                // (the slots its `[alphascale]` variables fade, see `ScriptedObject::alpha_slots`)
                let alpha = v.alpha_slots.get(k).filter(|l| !l.alpha.is_empty()).map(|l| l.values(&|n| n.trim().parse::<f32>().ok().or_else(|| o.inst.var(n))).0);
                let p = &mut scene.instances[*inst];
                match alpha {
                    Some(a) => {
                        if p.visible != visible || v.alpha_last.get(k) != Some(&a) {
                            renderer.set_params(scene, *inst, &a, visible, &[]);
                            if let Some(last) = v.alpha_last.get_mut(k) {
                                *last = a;
                            }
                        }
                    }
                    None => {
                        if p.visible != visible {
                            renderer.set_params(scene, *inst, &[], visible, &[]);
                        }
                    }
                }
            }
            updated += 1;
        }
        // (the boards keep this frame's lists; the old ones are the next step's buffers)
        std::mem::swap(&mut boards.wanted, &mut out.wanted);
        std::mem::swap(&mut boards.wanted_names, &mut out.wanted_names);
        drop(scripted);
        drop(boards);
        times.apply += step.post + t.elapsed().as_secs_f64();
        let t = std::time::Instant::now();
        self.apply_texture_updates(renderer, scene, texture_updates);
        times.textures += t.elapsed().as_secs_f64();
        let t = std::time::Instant::now();
        // the lamps' own sounds (a level crossing's bell): their scripts run with the light
        // programs (`Traffic::sync`), what they fired is heard here
        for lamp in self.light_objects.lock().iter() {
            // (only the lamps that have a sound, and near enough to hear: a city map has
            // nearly a thousand lamps, and going through all of them took 1.8 ms a frame)
            let Some(path) = lamp.sound.as_ref() else { continue };
            let Some(script) = lamp.script.as_ref() else { continue };
            let dist = (lamp.pos - center).length();
            if dist >= 300.0 {
                if let (Some(a), Some(mut ss)) = (audio, lamp.sounds.lock().take()) {
                    ss.stop_all(a);
                }
                // (what it fired out of earshot is not heard later)
                script.lock().host.fired_triggers.clear();
                continue;
            }
            let mut inst = script.lock();
            let fired = std::mem::take(&mut inst.host.fired_triggers);
            if let Some(a) = audio {
                let mut sounds = lamp.sounds.lock();
                self.object_sounds(a, &mut sounds, path, dist, muffled, lamp.pos, lamp.xf, &|n| inst.var(n), &fired);
            }
        }
        times.lamps += t.elapsed().as_secs_f64();
        updated
    }

    /// The dynamic texture changes the scripts asked for this frame. Scenery placement takes
    /// the GPU-cache lock before the script list, so they are applied after the script-list
    /// lock is released, to keep that lock order consistent.
    fn apply_texture_updates(&self, renderer: &Renderer, scene: &mut Scene, texture_updates: Vec<TextureUpdate>) {
    for (ty, selection, instances, switches) in texture_updates {
        let variant = {
            let mut gpu = self.gpu.lock();
            gpu.dynamic_texture_variant(
                renderer,
                scene,
                Arc::as_ptr(&ty) as usize,
                &selection,
                &self.root,
                &HashMap::new(),
            )
        };
        if let Some(rows) = variant {
            for (mi, row) in rows.iter().enumerate() {
                let Some(&mesh_inst) = instances.get(mi) else {
                    continue;
                };
                for (slot, pair) in row.iter().enumerate() {
                    let Some((base, item)) = pair else { continue };
                    // (the last switch of a slot, as a map collected from them kept it)
                    let item_on = switches
                        .iter()
                        .rev()
                        .find(|(k, _)| *k == (mesh_inst, slot))
                        .is_some_and(|&(_, on)| on);
                    renderer.set_material(
                        scene,
                        mesh_inst,
                        slot,
                        if item_on { *item } else { *base },
                    );
                }
            }
        }
    }
    }

    /// Play a scenery object's `[sound]` (the config at `path`) while the listener is within
    /// 300 m: loaded when it comes near, stopped when it goes.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn object_sounds(
        &self,
        a: &omsi_audio::AudioEngine,
        sounds: &mut Option<omsi_audio::SoundSet>,
        path: &Path,
        dist: f64,
        muffled: bool,
        pos: DVec3,
        xf: Mat4,
        var: &dyn Fn(&str) -> Option<f32>,
        fired: &[String],
    ) {
        if dist >= 300.0 {
            if let Some(mut ss) = sounds.take() {
                ss.stop_all(a);
            }
            return;
        }
        if sounds.is_none() {
            // read once per file, the clips in the background (see AudioEngine::clips_ready)
            let cfg = self
                .sound_cfgs
                .lock()
                .entry(path.to_path_buf())
                .or_insert_with(|| {
                    omsi_vehicle::SoundCfg::load(path)
                        .map_err(|e| log::warn!("{e}"))
                        .ok()
                        .map(Arc::new)
                })
                .clone();
            if let Some(cfg) = cfg {
                let sdir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
                if a.clips_ready(&omsi_audio::SoundSet::clip_paths(&cfg, &sdir)) {
                    log::info!("scenery sound {}: {} sounds ({:.0} m away)", path.display(), cfg.sounds.len(), dist);
                    let mut ss = omsi_audio::SoundSet::new(a, &cfg, &sdir);
                    ss.master = super::host::sound_gain(&super::host::SOUND_SCENERY);
                    *sounds = Some(ss);
                }
            }
        }
        if let Some(ss) = sounds.as_mut() {
            // scenery (a fountain, machinery, a level crossing bell): heard through the
            // player's own bodywork and glass just like any other sound from outside the cabin
            ss.set_muffled(muffled);
            let xf = Mat4::from_translation(pos.as_vec3()) * xf;
            ss.update(a, var, &xf, fired);
        }
    }
}
