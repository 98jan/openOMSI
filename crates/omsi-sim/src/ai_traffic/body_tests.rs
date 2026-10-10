//! Whole scenes for the body's side of the traffic, run headless: the planner keeping to
//! its body (`body_link`) and the bodies keeping out of the scenery (`scenery`).

use super::*;
use crate::ai_traffic::setup::RandomTypes;
use crate::traffic::{LaneBuilder, BODY_MAX_LEAD, BODY_SLACK};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A vehicle type with no model and no scripts, in a folder of its own.
pub(super) struct Fixture {
    dir: std::path::PathBuf,
    pub ty: Arc<VehicleType>,
}

impl Fixture {
    pub fn new() -> Fixture {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("omsi-traffic-body-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("car.bus"), "[model]\nmodel.cfg\n").unwrap();
        std::fs::write(dir.join("model.cfg"), "").unwrap();
        let ty = Arc::new(VehicleType::load_ai(&dir, &dir.join("car.bus")).unwrap());
        Fixture { dir, ty }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(super) fn traffic(f: &Fixture, mut net: Network) -> TrafficSim {
    net.link(1.5);
    net.build_grid();
    net.compute_reach();
    let random = RandomTypes { types: Vec::new(), groups: Vec::new(), group_curves: false, group_uvg: Vec::new(), uvg_defaults: Vec::new() };
    TrafficSim::assemble(&f.dir, net, random, Vec::new(), HashMap::new(), (Vec::new(), Vec::new()), Vec::new(), (1.0, 0), 0)
}

pub(super) fn add_car(t: &mut TrafficSim, f: &Fixture, lane: usize, s: f32, seed: u64, speed: Option<f32>) -> u64 {
    let vehicle = VehicleInstance::new(f.ty.clone(), crate::VehicleHost::new(crate::SimClock::default()));
    t.place_car(vehicle, LaneKind::Street, lane, s, f.ty.clone(), seed, None, None, speed, None)
}

pub(super) fn car(t: &TrafficSim, id: u64) -> &AiCar {
    t.cars.iter().find(|c| c.id == id).expect("the car is there")
}

/// 120 m north, a right turn of radius `r` over 90°, 200 m east.
pub(super) fn turn(r: f64) -> Network {
    let a = LaneBuilder::arc(DVec3::ZERO, 0.0, 120.0, 0.0, 0.0, LaneKind::Street, 3.0);
    let bend = LaneBuilder::arc(a.end(), 0.0, r * std::f64::consts::FRAC_PI_2, r, 0.0, LaneKind::Street, 3.0);
    let c = LaneBuilder::arc(bend.end(), 90.0, 200.0, 0.0, 0.0, LaneKind::Street, 3.0);
    Network { lanes: vec![a, bend, c], ..Default::default() }
}

/// Driving round a tight corner the body is where the plan is: found on its way all the
/// time, never more than half a metre behind it, and the plan never waits for it.
#[test]
fn a_body_keeps_to_its_plan_round_a_corner() {
    let f = Fixture::new();
    for r in [8.0, 15.0] {
        let mut t = traffic(&f, turn(r));
        let id = add_car(&mut t, &f, 0, 60.0, 0x42, Some(12.0));
        let (mut worst, mut off, mut steps) = (f32::MIN, 0, 0);
        for _ in 0..1000 {
            t.advance(0.02, None);
            let c = car(&t, id);
            let b = c.state.body;
            assert!(b.known, "radius {r}: the body is lost at lane {} s {:.1}", c.state.lane, c.state.s);
            worst = worst.max(b.lead);
            off += (!b.on_way) as u32;
            steps += 1;
        }
        let c = car(&t, id);
        assert!(c.state.lane == 2 && c.state.s > 30.0, "radius {r}: only at lane {} s {:.1}", c.state.lane, c.state.s);
        assert!(worst < BODY_SLACK, "radius {r}: the plan ran {worst:.2} m ahead of its body");
        assert_eq!(off, 0, "radius {r}: off its way {off} of {steps} steps");
    }
}

/// A body that cannot follow (held where it is, here by hand every step) is not run ahead
/// of: within a step or two the plan stands two metres ahead of it and waits, whether the
/// body stands on its way or three metres beside it.
#[test]
fn a_body_off_its_route_is_not_run_ahead_of() {
    let f = Fixture::new();
    for beside in [0.0, 3.0] {
        let mut t = traffic(&f, turn(15.0));
        let id = add_car(&mut t, &f, 0, 40.0, 0x77, Some(10.0));
        // drive for a moment, then the body stops where it is
        for _ in 0..50 {
            t.advance(0.02, None);
        }
        let i = t.cars.iter().position(|c| c.id == id).unwrap();
        let mut held = t.cars[i].body.clone();
        if beside > 0.0 {
            let st = t.cars[i].state.clone();
            let net = &t.net;
            held.place(&|d| st.way_point(net, d) + DVec3::new(beside, 0.0, 0.0), None, None, 0.0);
        }
        let s0 = t.cars[i].state.s;
        let mut worst = f32::MIN;
        for _ in 0..250 {
            t.cars[i].body = held.clone();
            // (the body stands: its own speed is nothing)
            t.cars[i].body.moved = 0.0;
            t.cars[i].state.body = body_link::link(&t.cars[i].state, &t.net, &t.cars[i].body);
            t.advance(0.02, None);
            let lead = body_link::project(&t.cars[i].state, &t.net, &held, 1.5).map(|p| -p.along).unwrap_or(f32::MAX);
            worst = worst.max(lead);
        }
        let st = &t.cars[i].state;
        assert!(worst <= BODY_MAX_LEAD + 0.25, "{beside} m beside: the plan ran {worst:.2} m ahead of its body");
        assert!(st.speed < 0.5, "{beside} m beside: the plan still goes {:.1} m/s", st.speed);
        assert!(st.s - s0 < BODY_MAX_LEAD + 0.5, "{beside} m beside: the plan ran {:.1} m on", st.s - s0);
    }
}
