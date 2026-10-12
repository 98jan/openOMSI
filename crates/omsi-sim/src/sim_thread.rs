//! The simulation's own thread, pipelined with the picture (`OMSI_SIM_THREAD`, off unless set).
//!
//! The window's frame runs its stages one after the other on the main thread (`WINDOW_FRAME`),
//! so a frame costs the simulation plus the picture. The main thread has to keep the window,
//! the input and the surface (macOS and iOS want them there); the simulation does not. With
//! the switch on, a step of the simulation runs on the thread here while the main thread draws
//! and presents the picture of the frame, and it is back before anything else of the frame
//! looks at it.
//!
//! What goes there so far is the AI traffic's fixed step (`TrafficSim::advance`): the traffic
//! stage does everything else of the traffic as before (its inputs, the population, the
//! timetable, the cars taken off, the signals, the sound, the view sync, all on the traffic as
//! the last step left it) and leaves the step itself, with the frame's time and the player's
//! box as the stage saw them, to `Pipeline::start_traffic` right before the picture; it comes
//! back at `Pipeline::join`, right after the picture is presented (before the frame is counted,
//! a session finished or saved). The cars are one step of the traffic later than the rest of
//! the frame, the picture shows them as they were synced, and the simulated time is the same
//! (the same frames' times summed, the same fixed steps). The player's own vehicle and its
//! input stay on the main thread in their stage: driving has no frame more of latency.
//!
//! The scenery objects' scripts (`SceneryState::step`) can go along in the same job, after the
//! traffic's step (they read its lights, as the scripted stage read them after the traffic
//! stage): the scripted stage then applies the commands of the step made during the last
//! picture and owes the next one, its inputs owned (`Pipeline::owe_scenery`). Their
//! commands are one frame later; the scripts' results are the same.
//!
//! The offscreen run never reads the switch: its step is the same code, inline. The window's
//! side (where the stages owe the step, where it starts and joins) is `omsi-app`'s
//! `sim_thread`.

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::time::Instant;

use crate::ai_traffic::model::PlayerBox;
use crate::ai_traffic::TrafficSim;
use crate::scenery::SceneryState;

type Job = Box<dyn FnOnce() + Send>;

/// One persistent thread ("sim") the simulation's steps run on, one at a time.
pub struct SimThread {
    jobs: Option<mpsc::SyncSender<Job>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// What a step's job sends back: the state, how long the step took, and its panic, if it
/// panicked (raised again on the main thread at `join`, as it would have been inline).
type Outcome<T> = (T, f64, Option<Box<dyn std::any::Any + Send>>);

/// A step under way (or made inline): `join` gives back its state.
pub enum InFlight<T> {
    Done(Outcome<T>),
    Thread(mpsc::Receiver<Outcome<T>>),
}

fn run_step<T>(mut state: T, step: impl FnOnce(&mut T)) -> Outcome<T> {
    let t = Instant::now();
    let panic = catch_unwind(AssertUnwindSafe(|| step(&mut state))).err();
    (state, t.elapsed().as_secs_f64(), panic)
}

impl SimThread {
    pub fn spawn() -> std::io::Result<SimThread> {
        // (one job at a time: the slot holds the next while one runs, never more)
        let (tx, rx) = mpsc::sync_channel::<Job>(1);
        let handle = std::thread::Builder::new().name("sim".into()).spawn(move || {
            for job in rx {
                job();
            }
        })?;
        Ok(SimThread { jobs: Some(tx), handle: Some(handle) })
    }

    /// `step` on `state`, on the sim thread; `InFlight::join` gives the state back.
    pub fn start<T: Send + 'static>(&self, state: T, step: impl FnOnce(&mut T) + Send + 'static) -> InFlight<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        let job: Job = Box::new(move || {
            let _ = tx.send(run_step(state, step));
        });
        match self.jobs.as_ref().map(|j| j.send(job)) {
            Some(Ok(())) => InFlight::Thread(rx),
            // (the thread is gone: the step runs here, as without the switch)
            Some(Err(mpsc::SendError(job))) => {
                job();
                InFlight::Thread(rx)
            }
            None => unreachable!("the sim thread's queue lives as long as it"),
        }
    }

    /// `step` on `state` right here, on the caller's thread: the same code without the thread.
    pub fn inline<T>(state: T, step: impl FnOnce(&mut T)) -> InFlight<T> {
        InFlight::Done(run_step(state, step))
    }
}

impl Drop for SimThread {
    fn drop(&mut self) {
        // the queue closed, the thread ends after its job (if any)
        self.jobs = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl<T> InFlight<T> {
    /// The state back, once its step is made, with the seconds the step took and the
    /// seconds this call waited for it. A step that panicked panics here.
    pub fn join(self) -> (T, f64, f64) {
        let t = Instant::now();
        let (state, secs, panic) = match self {
            InFlight::Done(o) => o,
            InFlight::Thread(rx) => rx.recv().expect("the sim thread sends every step's state back"),
        };
        if let Some(p) = panic {
            resume_unwind(p);
        }
        (state, secs, t.elapsed().as_secs_f64())
    }
}

/// The traffic's step the traffic stage left to be made with the picture.
#[derive(Clone, Copy, Debug)]
struct OwedStep {
    dt: f32,
    player: Option<PlayerBox>,
}

/// The light programs' state as the scenery scripts read it: (`TrafficLightPhase`,
/// `TrafficLightApproach`) of a controller's light.
pub type PhaseOf<'a> = &'a dyn Fn(usize, usize) -> (f32, f32);

/// The scenery's step the scripted stage left to be made with the picture: the step with its
/// inputs (owned, moved in), given the lights' state to read.
type ScenerySteps = Box<dyn FnOnce(&mut SceneryState, PhaseOf) + Send>;

/// What one job takes to the sim thread and brings back.
#[derive(Default)]
struct SimJob {
    traffic: Option<(Box<TrafficSim>, OwedStep)>,
    scenery: Option<(SceneryState, ScenerySteps)>,
    /// Seconds of the traffic's step and of the scenery's (each part's own).
    traffic_secs: f64,
    scenery_secs: f64,
}

impl SimJob {
    /// The parts in the frame's order: the traffic first, then the scenery scripts, which
    /// read the lights as the traffic's step left them (as the scripted stage did after the
    /// traffic stage). Without the traffic here there is none: its lights read unlinked.
    fn run(&mut self) {
        let t = Instant::now();
        if let Some((sim, owed)) = self.traffic.as_mut() {
            sim.advance(owed.dt, owed.player);
        }
        let mid = Instant::now();
        self.traffic_secs = (mid - t).as_secs_f64();
        if let Some((state, step)) = self.scenery.take() {
            let mut state = state;
            let traffic = self.traffic.as_ref().map(|(sim, _)| &**sim);
            let phase = |c: usize, li: usize| traffic.map(|t| t.light_vars(c, li)).unwrap_or((crate::traffic::UNLINKED_PHASE as f32, 0.0));
            step(&mut state, &phase);
            self.scenery = Some((state, Box::new(|_, _| {})));
        }
        self.scenery_secs = mid.elapsed().as_secs_f64();
    }
}

/// What came back at a join: the seconds of each part's step there and of the wait here.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Joined {
    pub traffic: f64,
    pub scenery: f64,
    pub waited: f64,
}

/// The window's pipeline: the sim thread (made the first time a step goes there) and what is
/// under way on it.
#[derive(Default)]
pub struct Pipeline {
    thread: Option<SimThread>,
    /// The traffic stage's step, while it waits for the picture.
    owed: Option<OwedStep>,
    /// The scripted stage's step, while it waits for the picture.
    owed_scenery: Option<ScenerySteps>,
    /// The job out on the sim thread.
    out: Option<InFlight<SimJob>>,
    /// What stands in `Traffic::sim` while the traffic is out: an empty traffic, made once.
    stand_in: Option<Box<TrafficSim>>,
    /// The scenery came back from a step whose commands the view has not applied yet.
    scenery_stepped: bool,
}

/// `OMSI_SIM_THREAD`: the window's simulation steps go to the sim thread.
pub fn enabled() -> bool {
    omsi_cfg::flags::OMSI_SIM_THREAD.is_set()
}

/// The steps go to the sim thread at all: the switch is on and the fixed step
/// (`OMSI_FIXED_STEP`) is off (its ticks run the stages inline, as they want them).
fn pipelined() -> bool {
    enabled() && !omsi_cfg::flags::OMSI_FIXED_STEP.is_set()
}

/// An empty traffic: nothing on the road, no lanes. It stands in `Traffic::sim` while the real
/// one is stepped on the sim thread (nothing of the frame looks at the traffic then).
fn empty_traffic() -> TrafficSim {
    TrafficSim::assemble(
        std::path::Path::new(""),
        Default::default(),
        crate::ai_traffic::setup::RandomTypes { types: Vec::new(), groups: Vec::new(), group_curves: false, group_uvg: Vec::new(), uvg_defaults: Vec::new() },
        Vec::new(),
        Default::default(),
        (Vec::new(), Vec::new()),
        Vec::new(),
        (1.0, 0),
        0,
    )
}

impl Pipeline {
    /// Whether the traffic stage leaves the step of `sim` to the sim thread this frame: the
    /// switch is on and the traffic drives itself (a LAN client's only follows the host's,
    /// frame by frame, inline). Under the fixed step (`OMSI_FIXED_STEP`) the traffic runs
    /// once per tick, inline, as the ticks want it.
    pub fn takes_traffic(&self, sim: &TrafficSim) -> bool {
        pipelined() && !sim.mirror
    }

    /// The traffic stage's step, left to be made with the picture (`dt` of this frame, the
    /// player's box as the stage saw it). Two in one frame (stages run more than once) add up.
    pub fn owe_traffic(&mut self, dt: f32, player: Option<PlayerBox>) {
        let dt = self.owed.map_or(0.0, |o| o.dt) + dt;
        self.owed = Some(OwedStep { dt, player });
    }

    /// Whether the scripted stage leaves the scenery's step to the sim thread this frame: the
    /// scripts read the lights after the traffic's step, so only with the traffic's step
    /// going there too, or without a traffic (whose lights read unlinked anyway).
    pub fn takes_scenery(&self, traffic: Option<&TrafficSim>) -> bool {
        pipelined() && traffic.is_none_or(|_| self.owed.is_some())
    }

    /// The scripted stage's step (its inputs moved into `step`), left to be made with the
    /// picture after the traffic's.
    pub fn owe_scenery(&mut self, step: impl FnOnce(&mut SceneryState, PhaseOf) + Send + 'static) {
        self.owed_scenery = Some(Box::new(step));
    }

    /// The scenery came back from the sim thread with commands not applied yet: the
    /// scripted stage applies them (`SceneryState::outputs`) before it owes the next step.
    pub fn take_scenery_stepped(&mut self) -> bool {
        std::mem::take(&mut self.scenery_stepped)
    }

    /// The steps owed, started on the sim thread with the traffic's simulation `sim` (an
    /// empty one stands in for it until `join`) and the scenery (an empty list meanwhile).
    /// Nothing owed, nothing starts.
    pub fn start(&mut self, sim: Option<&mut TrafficSim>, scenery: Option<&mut SceneryState>) {
        if self.out.is_some() {
            return;
        }
        let mut job = SimJob::default();
        if let (Some(owed), Some(sim)) = (self.owed.take(), sim) {
            let mut out = self.stand_in.take().unwrap_or_else(|| Box::new(empty_traffic()));
            std::mem::swap(sim, &mut *out);
            job.traffic = Some((out, owed));
        }
        if let (Some(step), Some(scenery)) = (self.owed_scenery.take(), scenery) {
            job.scenery = Some((std::mem::take(scenery), step));
        }
        if job.traffic.is_none() && job.scenery.is_none() {
            return;
        }
        if self.thread.is_none() {
            match SimThread::spawn() {
                Ok(th) => self.thread = Some(th),
                Err(e) => log::warn!("sim thread: not started ({e}); its steps run in the frame"),
            }
        }
        let run = |job: &mut SimJob| job.run();
        self.out = Some(match self.thread.as_ref() {
            Some(th) => th.start(job, run),
            None => SimThread::inline(job, run),
        });
    }

    /// Everything out on the sim thread back in place (waits for it). Nothing out: nothing.
    /// `sim`, `scenery`: where the traffic's simulation and the scenery stand in the frame.
    pub fn join(&mut self, sim: Option<&mut TrafficSim>, scenery: Option<&mut SceneryState>) -> Joined {
        let Some(f) = self.out.take() else { return Joined::default() };
        let (job, _, waited) = f.join();
        if let Some((mut back, _)) = job.traffic {
            match sim {
                Some(sim) => {
                    std::mem::swap(sim, &mut *back);
                    self.stand_in = Some(back);
                }
                // (the traffic was let go meanwhile - nothing of the frame does that while it
                // is out, but should it, the stepped one is dropped with it)
                None => log::warn!("sim thread: the traffic went away while it was stepped"),
            }
        }
        if let Some((back, _)) = job.scenery {
            match scenery {
                Some(s) => {
                    *s = back;
                    self.scenery_stepped = true;
                }
                None => log::warn!("sim thread: the scenery went away while it was stepped"),
            }
        }
        Joined { traffic: job.traffic_secs, scenery: job.scenery_secs, waited }
    }

    /// Something is out on the sim thread.
    pub fn busy(&self) -> bool {
        self.out.is_some()
    }
}

#[cfg(test)]
mod tests;
