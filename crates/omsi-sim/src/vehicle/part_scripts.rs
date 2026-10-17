//! A coupled part's own scripts. In OMSI every car of a train runs its scripts itself, and
//! train scripts light their ends from where the car stands: `train_frontcoupling` and
//! `train_backcoupling` (something is coupled at that end of the car) and
//! `train_me_reverse` (the car runs turned round). A part whose scripts (or model) read
//! one of these gets a copy of the variables of its own: the leading vehicle's values are
//! taken over every frame (the engine's, the driver's switches), except those its own
//! {init} and {frame} write and the three above, and its meshes, lamps and particles show
//! that copy. Read from the leading vehicle, the second unit of a train turned round at its
//! back lit white head lamps at the tail. Parts that read none of them stay as they were.

use super::*;
use omsi_script::{Op, VarId};

/// The variables that say where a car stands in its train.
const TRAIN_VARS: [&str; 3] = ["train_frontcoupling", "train_backcoupling", "train_me_reverse"];

/// Whether a part's scripts or model read where it stands in its train.
pub(super) fn reads_train_ends(ty: &VehicleType) -> bool {
    let p = &ty.program;
    let named = |v: &str| TRAIN_VARS.iter().any(|n| v.trim().eq_ignore_ascii_case(n));
    TRAIN_VARS.iter().any(|n| p.var(n).is_some_and(|id| p.reads(id)))
        || ty.model.meshes.iter().any(|m| {
            m.animations.iter().any(|a| named(&a.variable))
                || m.visible.as_ref().is_some_and(|(v, _)| named(v))
                || m.light_enh.iter().any(|l| named(&l.variable))
                || m.light_enh_2.iter().any(|l| named(&l.variable))
        })
}

/// What is coupled at each end of the parts of a train, in the train's order (ahead,
/// behind): ahead of a part there is always the part before it, behind it the next one.
pub(super) fn mark_train_ends(parts: &mut [TrailerPart]) {
    let n = parts.len();
    for (i, t) in parts.iter_mut().enumerate() {
        t.coupled = [true, i + 1 < n];
    }
}

/// The variables a program's {init} and {frame} (and the macros they call) store into.
fn written_by_scripts(p: &Program) -> Vec<bool> {
    let mut out = vec![false; p.var_names.len()];
    let mut seen = hashbrown::HashSet::new();
    let mut todo: Vec<omsi_script::BlockId> = p.init.iter().chain(&p.frame).chain(&p.frame_ai).copied().collect();
    while let Some(b) = todo.pop() {
        if !seen.insert(b) {
            continue;
        }
        for op in p.blocks.get(b as usize).map(|b| b.ops.as_slice()).unwrap_or_default() {
            match op {
                Op::Store(v) => {
                    if let Some(w) = out.get_mut(*v as usize) {
                        *w = true;
                    }
                }
                Op::Macro(m) => todo.push(*m),
                _ => {}
            }
        }
    }
    out
}

/// The host a part's own scripts run with: the leading vehicle's for what they ask (the
/// clock, the weather, the depot), but no sound, message or system variable of theirs
/// reaches it - the leading vehicle's scripts did those already.
struct PartHost<'a>(&'a mut crate::host::VehicleHost);

impl omsi_script::Host for PartHost<'_> {
    fn sys_var(&mut self, v: omsi_script::SysVar) -> f32 {
        omsi_script::Host::sys_var(self.0, v)
    }
    fn callback(&mut self, name: &str, id: omsi_script::NameId, stacks: &mut omsi_script::Stacks, state: &mut State) {
        omsi_script::Host::callback(self.0, name, id, stacks, state);
    }
    fn sound_trigger(&mut self, _name: &str, _id: omsi_script::NameId) {}
    fn sound_trigger_vars(&mut self, _name: &str, _id: omsi_script::NameId, _vars: &[f32]) {}
    fn message(&mut self, _text: &str) {}
}

/// A coupled part's own copy of the variables and the scripts that run on it (see the
/// module).
pub struct PartScripts {
    program: Arc<Program>,
    pub(super) state: State,
    vm: Vm,
    /// The copy's names (lower case) and slots: the part's program's, then the leading
    /// vehicle's others.
    pub(super) var_index: Arc<HashMap<String, VarId>>,
    /// Per slot of the part's program: its own scripts write it.
    own: Vec<bool>,
    /// (slot here, slot in the leading vehicle) of what is taken over every frame.
    from_lead: Vec<(VarId, VarId)>,
    /// How many variables the leading vehicle had when `from_lead` was made.
    lead_seen: usize,
    /// `train_frontcoupling`, `train_backcoupling`, `train_me_reverse` here.
    train: [Option<VarId>; 3],
}

impl PartScripts {
    /// The part's scripts on a copy of the leading vehicle's values, its {init} run once
    /// with its own place in the train (`ends`, see [`Self::step`]).
    pub(super) fn new(ty: &VehicleType, main: &mut VehicleInstance, ends: [f32; 3]) -> PartScripts {
        let program = ty.program.clone();
        let mut state = State::new(&program);
        let mut index: HashMap<String, VarId> = HashMap::new();
        for (i, n) in program.var_names.iter().enumerate() {
            index.insert(n.to_ascii_lowercase(), i as VarId);
        }
        // (the strings as the leading vehicle has them: its number, its destination)
        let lead_program = main.ty.program.clone();
        for (i, n) in program.str_var_names.iter().enumerate() {
            if let Some(s) = lead_program.str_var(n).and_then(|k| main.state.str_vars.get(k as usize)) {
                state.str_vars[i] = s.clone();
            }
        }
        let own = written_by_scripts(&program);
        let mut p = PartScripts {
            program,
            state,
            vm: Vm::new(),
            var_index: Arc::new(index),
            own,
            from_lead: Vec::new(),
            lead_seen: usize::MAX,
            train: [None; 3],
        };
        p.take_over(main);
        // (every value the leading vehicle has to start from, its own ones too)
        for (name, &id) in p.var_index.iter() {
            if let Some(&lid) = main.var_index.get(name) {
                p.state.vars[id as usize] = main.state.vars[lid as usize];
            }
        }
        p.set_ends(ends);
        let was = std::mem::replace(&mut main.host.remote_script_textures, true);
        let coins = main.host.change_coins.len();
        let prog = p.program.clone();
        p.vm.run_init(&prog, &mut p.state, &mut PartHost(&mut main.host));
        main.host.change_coins.truncate(coins);
        main.host.remote_script_textures = was;
        p
    }

    /// Make `from_lead` (again) when the leading vehicle gained variables: its names the
    /// copy lacks are added, and each of its values the part's scripts do not write is
    /// taken over.
    fn take_over(&mut self, main: &VehicleInstance) {
        if self.lead_seen == main.var_index.len() {
            return;
        }
        self.lead_seen = main.var_index.len();
        let index = Arc::make_mut(&mut self.var_index);
        for name in main.var_index.keys() {
            if !index.contains_key(name) {
                index.insert(name.clone(), self.state.vars.len() as VarId);
                self.state.vars.push(0.0);
            }
        }
        self.from_lead = index
            .iter()
            .filter(|(_, &id)| !self.own.get(id as usize).copied().unwrap_or(false))
            .filter_map(|(name, &id)| main.var_index.get(name).map(|&lid| (id, lid)))
            .collect();
        self.from_lead.sort_unstable();
        self.train = TRAIN_VARS.map(|n| index.get(n).copied());
    }

    fn set_ends(&mut self, ends: [f32; 3]) {
        for (id, v) in self.train.iter().zip(ends) {
            if let Some(i) = id {
                self.state.vars[*i as usize] = v;
            }
        }
    }

    /// One frame: the leading vehicle's values taken over, the part's place in the train
    /// (`ends`: something coupled at its own front, at its own back, turned round) set, and
    /// its {frame} run.
    pub(super) fn step(&mut self, main: &mut VehicleInstance, ends: [f32; 3]) {
        self.take_over(main);
        for &(id, lid) in &self.from_lead {
            self.state.vars[id as usize] = main.state.vars[lid as usize];
        }
        self.set_ends(ends);
        // (an AI train's cars run what its leading car runs)
        let ai = main.ev(Ev::Ai).is_some_and(|v| v > 0.5) && !self.program.frame_ai.is_empty();
        let was = std::mem::replace(&mut main.host.remote_script_textures, true);
        let coins = main.host.change_coins.len();
        let prog = self.program.clone();
        let mut host = PartHost(&mut main.host);
        if ai {
            self.vm.run_frame_ai(&prog, &mut self.state, &mut host);
        } else {
            self.vm.run_frame(&prog, &mut self.state, &mut host);
        }
        main.host.change_coins.truncate(coins);
        main.host.remote_script_textures = was;
    }

    /// Variable `name` of the copy.
    pub fn var(&self, name: &str) -> Option<f32> {
        omsi_script::compile::with_lower(name, |k| self.var_index.get(k).copied()).map(|i| self.state.vars[i as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A railcar type whose script lights its ends from where it stands: white head lamps
    /// at the end that leads the train, red tail lamps at the end that trails it. Its lamps,
    /// in order: white at its front, red at its front, red at its back, white at its back.
    fn railcar() -> Arc<VehicleType> {
        let dir = std::env::temp_dir().join(format!("openomsi-part-scripts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let osc = dir.join("ends.osc");
        std::fs::write(
            &osc,
            "{frame}\n\
             (L.L.train_frontcoupling) ! (L.L.train_me_reverse) ! && (S.L.head_front)\n\
             (L.L.train_frontcoupling) ! (L.L.train_me_reverse) && (S.L.tail_front)\n\
             (L.L.train_backcoupling) ! (L.L.train_me_reverse) ! && (S.L.tail_back)\n\
             (L.L.train_backcoupling) ! (L.L.train_me_reverse) && (S.L.head_back)\n\
             {end}\n",
        )
        .unwrap();
        let vars = dir.join("vars.txt");
        std::fs::write(&vars, "train_frontcoupling\ntrain_backcoupling\ntrain_me_reverse\nhead_front\ntail_front\ntail_back\nhead_back\n").unwrap();
        let program = omsi_script::compile(&CompileInput { varlists: vec![vars], scripts: vec![osc], ..Default::default() });
        let _ = std::fs::remove_dir_all(&dir);
        assert!(program.errors.is_empty(), "{:?}", program.errors);
        let lamp = |variable: &str, y: f32, white: bool| omsi_model::LightEnh2 {
            pos: [0.0, y, 1.0],
            omni: true,
            color: if white { [255.0; 3] } else { [255.0, 0.0, 0.0] },
            size: 0.2,
            factor: 1.0,
            variable: variable.to_string(),
            ..Default::default()
        };
        let mut ty = super::super::tests::coupling_test_type(Some(-5.0));
        let t = Arc::get_mut(&mut ty).unwrap();
        t.program = Arc::new(program);
        t.model.meshes = vec![omsi_model::MeshDef {
            light_enh_2: vec![lamp("head_front", 6.0, true), lamp("tail_front", 6.0, false), lamp("tail_back", -6.0, false), lamp("head_back", -6.0, true)],
            ..Default::default()
        }];
        ty
    }

    fn frame(v: &mut VehicleInstance) {
        v.update_dirt(0.0);
        v.update_scripts_only(0.0);
        v.update_visuals(1.0 / 30.0);
    }

    fn lamps(fades: &[f32]) -> [f32; 4] {
        [0, 1, 2, 3].map(|k| fades.get(k).copied().unwrap_or(0.0))
    }

    /// Three cars of one type, the last turned round: white at the head of the train, red
    /// at its tail and no white there; uncoupled, the middle car becomes the tail.
    #[test]
    fn each_car_lights_its_own_ends() {
        let ty = railcar();
        let mut v = VehicleInstance::new(ty.clone(), crate::host::VehicleHost::new(Default::default()));
        v.attach_trailer_ex(ty.clone(), false);
        v.attach_trailer_ex(ty.clone(), true);
        for _ in 0..3 {
            frame(&mut v);
        }
        assert_eq!(lamps(&v.light_fade), [1.0, 0.0, 0.0, 0.0], "the leading car: white ahead");
        assert_eq!(lamps(&v.trailers[0].light_fade), [0.0; 4], "the middle car: dark");
        assert_eq!(lamps(&v.trailers[1].light_fade), [0.0, 1.0, 0.0, 0.0], "the last car, turned round: red at its front, the train's tail");
        assert!(v.trailers.iter().all(|t| t.own.is_some()));
        assert_eq!(v.trailers[1].own_var("train_me_reverse"), Some(1.0));
        // the leading car's own values are not touched by its cars' scripts
        assert_eq!(v.var("head_front"), Some(1.0));
        assert_eq!(v.var("tail_front"), Some(0.0));
        v.detach_last_trailer();
        for _ in 0..3 {
            frame(&mut v);
        }
        assert_eq!(lamps(&v.trailers[0].light_fade), [0.0, 0.0, 1.0, 0.0], "the middle car is the tail now: red at its back");
    }
}
