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
    p.start_traffic(None);
    assert!(!p.busy());
    let mut sim = empty_traffic();
    sim.time = 5.0;
    p.owe_traffic(0.25, None);
    p.owe_traffic(0.25, None);
    p.start_traffic(Some(&mut sim));
    assert!(p.busy());
    // (the stand-in meanwhile)
    assert_eq!(sim.time, 0.0);
    p.join(Some(&mut sim));
    assert!(!p.busy());
    // half a second owed: at most 4 fixed steps of 0.02 s in one frame
    assert!((sim.time - 5.08).abs() < 1e-4, "{}", sim.time);
    // the stand-in is kept for the next frame
    assert!(p.stand_in.is_some());
}
