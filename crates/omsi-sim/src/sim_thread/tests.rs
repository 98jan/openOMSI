use super::*;

fn send<T: Send>() {}

/// What the sim thread is handed must be `Send` (the compiler says so here, not at a call
/// site that may not exist yet).
#[test]
fn sim_state_is_send() {
    send::<TrafficSim>();
    send::<Box<TrafficSim>>();
    send::<crate::people::PeopleSim>();
    send::<crate::VehicleInstance>();
}

#[test]
fn thread_and_inline_give_the_same() {
    let th = SimThread::spawn().unwrap();
    let step = |v: &mut Vec<u32>| {
        for k in 0..1000 {
            v.push(k * 3 + 1);
        }
    };
    let (a, _, _) = th.start(Vec::new(), step).join();
    let (b, _, _) = SimThread::inline(Vec::new(), step).join();
    assert_eq!(a, b);
    // one after the other, in order
    let mut v = vec![0u64];
    for k in 1..50u64 {
        v = th.start(v, move |v| v.push(k)).join().0;
    }
    assert_eq!(v, (0..50).collect::<Vec<_>>());
}

#[test]
fn a_panic_comes_back_at_join() {
    let th = SimThread::spawn().unwrap();
    let f = th.start(1u32, |_| panic!("step failed"));
    let r = std::panic::catch_unwind(AssertUnwindSafe(|| f.join()));
    assert!(r.is_err());
    // and the thread goes on
    assert_eq!(th.start(2u32, |v| *v += 1).join().0, 3);
}

#[test]
fn the_traffic_goes_out_and_comes_back() {
    let mut p = Pipeline::default();
    // nothing owed: nothing starts
    p.start(None, None);
    assert!(!p.busy());
    let mut sim = empty_traffic();
    sim.time = 5.0;
    p.owe_traffic(0.25, None);
    p.owe_traffic(0.25, None);
    p.start(Some(&mut sim), None);
    assert!(p.busy());
    // (the stand-in meanwhile)
    assert_eq!(sim.time, 0.0);
    p.join(Some(&mut sim), None);
    assert!(!p.busy());
    // half a second owed: at most 4 fixed steps of 0.02 s in one frame
    assert!((sim.time - 5.08).abs() < 1e-4, "{}", sim.time);
    // the stand-in is kept for the next frame
    assert!(p.stand_in.is_some());
}

/// Stepped on the sim thread frame after frame, the traffic comes out where the same frames
/// stepped inline leave it (the handover changes nothing of the computation).
#[test]
fn pipelined_frames_match_inline_frames() {
    let mut p = Pipeline::default();
    let mut piped = empty_traffic();
    let mut inline = empty_traffic();
    let mut x = 12345u32;
    for _ in 0..200 {
        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
        let dt = 0.004 + (x >> 8) as f32 / (1u32 << 24) as f32 * 0.05;
        inline.advance(dt, None);
        p.owe_traffic(dt, None);
        p.start(Some(&mut piped), None);
        p.join(Some(&mut piped), None);
    }
    assert_eq!(piped.time.to_bits(), inline.time.to_bits());
    assert_eq!(piped.day_time.to_bits(), inline.day_time.to_bits());
    assert_eq!(piped.sim_accum.to_bits(), inline.sim_accum.to_bits());
    assert_eq!(piped.frame_alpha.to_bits(), inline.frame_alpha.to_bits());
    assert_eq!(piped.rng, inline.rng);
}

fn scenery_of(n: i64) -> SceneryState {
    use crate::scenery::{SceneryInstance, SceneryObject};
    let mut s = SceneryState::default();
    for i in 0..n {
        let inst = SceneryInstance::new(std::sync::Arc::new(omsi_script::Program::default()), &[], crate::SimClock::default(), &[]);
        let p = glam::DVec3::new((i * 37 % 1700) as f64 - 850.0, (i * 91 % 1300) as f64 - 650.0, 0.0);
        s.push(SceneryObject::new(i, p, ((p.x / 300.0).floor() as i32, 0), inst, 0));
    }
    s
}

/// One frame's scenery step as the scripted stage makes it, its inputs owned.
fn scenery_step(dt: f32, center: glam::DVec3) -> impl FnOnce(&mut SceneryState, PhaseOf) + Send + 'static {
    move |s: &mut SceneryState, phase_of: PhaseOf| {
        let (controllers, by_stop, departures) = (hashbrown::HashMap::new(), hashbrown::HashMap::new(), Default::default());
        let inputs = crate::scenery::SceneryInputs {
            clock: None,
            time: 3600.0,
            day: Default::default(),
            brightness: 0.4,
            center,
            phase_of,
            controllers: &controllers,
            by_stop: &by_stop,
            departures: &departures,
            departures_gen: 0,
            pages_near: 60.0,
        };
        s.step(dt, &inputs, &mut crate::scenery::StepTimes::default());
    }
}

fn outputs_of(s: &mut SceneryState) -> String {
    let (o, _) = s.outputs();
    format!("{:?} {:?} {:?} {:?} {}", o.cmds, o.vars, o.transforms, o.visible, o.ran)
}

/// The scenery stepped on the sim thread (with the traffic, after it) gives the commands the
/// same frames' steps give inline, frame by frame.
#[test]
fn pipelined_scenery_matches_inline_scenery() {
    let mut p = Pipeline::default();
    let (mut piped, mut inline) = (scenery_of(400), scenery_of(400));
    let (mut piped_t, mut inline_t) = (empty_traffic(), empty_traffic());
    let unlinked = |_: usize, _: usize| (crate::traffic::UNLINKED_PHASE as f32, 0.0);
    for k in 0..120 {
        let dt = 1.0 / 60.0 + (k % 7) as f32 * 0.003;
        let center = glam::DVec3::new(-900.0 + k as f64 * 15.0, (k as f64 * 0.1).sin() * 300.0, 2.0);
        inline_t.advance(dt, None);
        scenery_step(dt, center)(&mut inline, &unlinked);
        p.owe_traffic(dt, None);
        p.owe_scenery(scenery_step(dt, center));
        p.start(Some(&mut piped_t), Some(&mut piped));
        assert!(piped.is_empty(), "an empty list stands in meanwhile");
        p.join(Some(&mut piped_t), Some(&mut piped));
        assert!(p.take_scenery_stepped());
        assert_eq!(outputs_of(&mut piped), outputs_of(&mut inline), "frame {k}");
    }
    assert_eq!(piped_t.time.to_bits(), inline_t.time.to_bits());
}
