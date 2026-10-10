//! Junction scenes run headless: lights (the amber memory, the lights of one crossing
//! object), the room on a junction's exit, priority signs against early claims,
//! roundabouts, the waits-for graph.

#[allow(unused_imports)]
use super::scenario_tests::{add_car, crossroads, street, Fixture};
use super::*;
use crate::ai_traffic::setup::RandomTypes;
use crate::traffic::{LaneBuilder, LaneKey, TrafficLightController};

/// Traffic on `net` with `lights` light programs of one light each (all green at first).
fn with_lights(f: &Fixture, mut net: Network, lights: usize) -> TrafficSim {
    net.build_grid();
    net.compute_reach();
    let random = RandomTypes { types: Vec::new(), groups: Vec::new(), group_curves: false, group_uvg: Vec::new(), uvg_defaults: Vec::new() };
    let ctls = (0..lights).map(|_| TrafficLightController::new(vec![vec![(6, 1000.0)]], 0.0)).collect();
    TrafficSim::assemble(&f.dir, net, random, ctls, HashMap::new(), (Vec::new(), Vec::new()), Vec::new(), (1.0, 0), 0)
}

/// Light `c` shows `state` from now on (6 green, 9 yellow, 0 red).
fn show(t: &mut TrafficSim, c: usize, state: i32) {
    t.lights[c].lights[0] = vec![(state, 1000.0)];
}

/// A car put on `lane` at `s` driving at `v` (not slowed down for being placed).
fn car(t: &mut TrafficSim, f: &Fixture, lane: usize, s: f32, seed: u64, v: f32) -> u64 {
    let id = add_car(t, f, lane, s, seed, Some(v));
    let c = t.cars.iter_mut().find(|c| c.id == id).unwrap();
    c.fresh = 0.0;
    c.state.speed = v;
    id
}

fn get(t: &TrafficSim, id: u64) -> &AiCar {
    t.cars.iter().find(|c| c.id == id).expect("the car is still there")
}

fn ticks(t: &mut TrafficSim, secs: f32) {
    for _ in 0..(secs / super::fixed_step::SIM_DT).round() as usize {
        t.tick(super::fixed_step::SIM_DT, None);
    }
}

/// Lanes 0 (150 m north to the line), 1 (the crossing object's path, 20 m, light 0), 2
/// (300 m on). With `second`: lane 1 leads into lane 3 of the same object (10 m, light 1,
/// the stop line after a footpath), and on into lane 2; with `other_approach` as well,
/// lane 4 from the east runs into lane 3 too (its light is that approach's).
fn signalled(second: bool, other_approach: bool) -> Network {
    let key = |path: u16| Some(LaneKey { tile: (0, 0), id: 5, path });
    let approach = street(DVec3::ZERO, 0.0, 150.0);
    let mut path = street(DVec3::new(0.0, 150.0, 0.0), 0.0, 20.0);
    path.source = 2;
    path.key = key(0);
    path.traffic_light = Some((0, 0));
    let mut lanes = vec![approach, path];
    if second {
        lanes.push(street(DVec3::new(0.0, 180.0, 0.0), 0.0, 300.0));
        let mut inner = street(DVec3::new(0.0, 170.0, 0.0), 0.0, 10.0);
        inner.source = 2;
        inner.key = key(1);
        inner.traffic_light = Some((1, 0));
        lanes.push(inner);
        if other_approach {
            // (a road of its own from the east bending north into lane 3)
            lanes.push(LaneBuilder::arc(DVec3::new(12.0, 158.0, 0.0), 270.0, 12.0 * std::f64::consts::FRAC_PI_2, 12.0, 0.0, LaneKind::Street, 3.0));
        }
    } else {
        lanes.push(street(DVec3::new(0.0, 170.0, 0.0), 0.0, 300.0));
    }
    let mut net = Network { lanes, ..Default::default() };
    net.link(1.5);
    net
}

/// The car's front, metres past the line of lane 1 (+ = over it).
fn over_line(t: &TrafficSim, id: u64) -> f32 {
    let c = get(t, id);
    let base = match c.state.lane {
        0 => 0.0,
        1 => 150.0,
        3 => 170.0,
        2 if t.net.lanes.len() > 3 => 180.0,
        _ => 170.0,
    };
    base + c.state.s + c.state.front - 150.0
}

#[test]
fn a_car_that_crept_up_to_the_line_stops_at_red() {
    let f = Fixture::new();
    let mut t = with_lights(&f, signalled(false, false), 1);
    // creeping up behind a queue that has gone: 1.5 m before the line at 1 m/s on green
    let id = car(&mut t, &f, 0, 0.0, 0x31, 1.0);
    let front = get(&t, id).state.front;
    t.cars.iter_mut().find(|c| c.id == id).unwrap().state.s = 150.0 - 1.5 - front;
    ticks(&mut t, 0.1);
    show(&mut t, 0, 9);
    ticks(&mut t, 1.0);
    show(&mut t, 0, 0);
    ticks(&mut t, 6.0);
    assert!(over_line(&t, id) < 0.3, "it crept over the line: {:.2} m", over_line(&t, id));
    assert!(get(&t, id).state.speed < 0.1);
}

#[test]
fn a_moving_car_clears_the_junction_on_amber() {
    let f = Fixture::new();
    let mut t = with_lights(&f, signalled(false, false), 1);
    // 12 m/s, 10 m before the line as the light goes yellow, red a second later
    let id = car(&mut t, &f, 0, 130.0, 0x32, 12.0);
    let front = get(&t, id).state.front;
    t.cars.iter_mut().find(|c| c.id == id).unwrap().state.s = 140.0 - front;
    show(&mut t, 0, 9);
    ticks(&mut t, 1.0);
    show(&mut t, 0, 0);
    ticks(&mut t, 4.0);
    assert!(get(&t, id).state.lane == 2, "it stopped in the junction: lane {}", get(&t, id).state.lane);
}

#[test]
fn a_car_standing_with_its_nose_over_the_line_does_not_set_off_at_red() {
    let f = Fixture::new();
    let mut t = with_lights(&f, signalled(false, false), 1);
    show(&mut t, 0, 0);
    let id = car(&mut t, &f, 0, 0.0, 0x33, 0.0);
    let front = get(&t, id).state.front;
    t.cars.iter_mut().find(|c| c.id == id).unwrap().state.s = 150.0 + 0.6 - front;
    ticks(&mut t, 8.0);
    assert!(over_line(&t, id) < 0.7, "it moved off at red: {:.2} m over", over_line(&t, id));
    // and it goes on green
    show(&mut t, 0, 6);
    ticks(&mut t, 6.0);
    assert!(over_line(&t, id) > 5.0);
}

#[test]
fn the_first_car_at_the_line_gets_through_a_short_green() {
    let f = Fixture::new();
    let mut t = with_lights(&f, signalled(false, false), 1);
    show(&mut t, 0, 0);
    let id = car(&mut t, &f, 0, 140.0, 0x34, 6.0);
    ticks(&mut t, 8.0);
    assert!(over_line(&t, id) < 0.0 && get(&t, id).state.speed < 0.1, "it waits at red");
    show(&mut t, 0, 6);
    ticks(&mut t, 2.0);
    show(&mut t, 0, 9);
    ticks(&mut t, 1.0);
    show(&mut t, 0, 0);
    ticks(&mut t, 8.0);
    assert!(over_line(&t, id) > 10.0, "the first car stopped again: {:.1} m", over_line(&t, id));
}

#[test]
fn another_light_of_the_same_junction_holds() {
    let f = Fixture::new();
    let mut t = with_lights(&f, signalled(true, false), 2);
    show(&mut t, 1, 0);
    let id = car(&mut t, &f, 0, 100.0, 0x35, 8.0);
    ticks(&mut t, 15.0);
    let c = get(&t, id);
    assert_eq!(c.state.lane, 1, "it went through the second red (lane {})", c.state.lane);
    assert!(c.state.speed < 0.1);
}

#[test]
fn a_light_passed_does_not_hold_again_further_in() {
    // lane 3's light belongs to the approach from the east: the car from the south went
    // through its own green and is not stopped by it in the middle of the junction
    let f = Fixture::new();
    let mut t = with_lights(&f, signalled(true, true), 2);
    assert!(t.net.prev[3].contains(&4), "the east approach runs into lane 3");
    show(&mut t, 1, 0);
    let id = car(&mut t, &f, 0, 100.0, 0x36, 8.0);
    ticks(&mut t, 15.0);
    assert_eq!(get(&t, id).state.lane, 2);
}
