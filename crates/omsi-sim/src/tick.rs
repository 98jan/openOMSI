//! The simulation's fixed step (`OMSI_FIXED_STEP`, off unless asked for): the traffic, the
//! player's vehicle with its scripts, the people, the duty and the clock move on in ticks of
//! one length (1/120 s unless the switch names another rate), however long the frames are.
//! The frame's time is collected and spent in whole ticks; what is left over says how far the
//! picture stands between the last two ticks (`Ticks::alpha`), and the player's bus and the
//! camera are drawn there (`BodyPose::between`).
//!
//! With the switch off nothing here runs and every stage takes the frame's own time, as it
//! always did. With it on, two runs of one session at different frame rates step the same
//! ticks with the same inputs, which is what makes them comparable at all.

use crate::ai_traffic::fixed_step::advance_fixed_clock;
use glam::DVec3;

/// The rate when the switch is only turned on (`OMSI_FIXED_STEP=1`).
pub const DEFAULT_RATE_HZ: f64 = 120.0;
/// The rates a switch may name; outside them it is off (a typo should not run the game at 2
/// ticks a second, nor at 100 000).
pub const RATES: std::ops::RangeInclusive<f64> = 30.0..=1000.0;
/// The window's frame is at most a tenth of a second (`timing.rs`): at most that much time
/// is made up in one frame; a longer stall is dropped, as the frame's own time was.
const MAX_FRAME_S: f64 = 0.1;

/// `OMSI_FIXED_STEP`: unset, empty, `0` or `off` is off; `1` or `on` the default rate; a
/// number of ticks a second within [`RATES`] that rate.
pub fn parse_rate(spec: &str) -> Option<f64> {
    match spec.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "off" | "false" | "no" => None,
        "1" | "on" | "true" | "yes" => Some(DEFAULT_RATE_HZ),
        s => s.parse::<f64>().ok().filter(|r| RATES.contains(r)),
    }
}

/// The rate the switch asks for, if it is on.
pub fn rate_from_flag() -> Option<f64> {
    omsi_cfg::flags::OMSI_FIXED_STEP.var().and_then(parse_rate)
}

/// What a frame runs: `n` ticks, and where the picture stands between the last two of them
/// (0: at the second last, towards 1: at the last).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ticks {
    pub n: u32,
    pub alpha: f32,
}

/// The accumulator of the fixed step.
#[derive(Debug, Clone)]
pub struct FixedStep {
    rate: f64,
    dt: f32,
    accum: f64,
    ticks: u64,
    max_per_frame: u32,
}

impl FixedStep {
    pub fn new(rate_hz: f64) -> Self {
        let rate = rate_hz.clamp(*RATES.start(), *RATES.end());
        FixedStep {
            rate,
            dt: (1.0 / rate) as f32,
            accum: 0.0,
            ticks: 0,
            max_per_frame: (MAX_FRAME_S * rate).ceil() as u32 + 1,
        }
    }

    /// The fixed step the switch asks for, or none (off).
    pub fn from_flag() -> Option<Self> {
        rate_from_flag().map(Self::new)
    }

    /// Ticks a second.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// One tick (s): the time every simulation stage is given.
    pub fn dt(&self) -> f32 {
        self.dt
    }

    /// Ticks run since the start.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// The simulation's time since the start (s): the ticks run, each `dt` long.
    pub fn sim_time(&self) -> f64 {
        self.ticks as f64 * self.dt as f64
    }

    /// A frame of `frame_dt` seconds went by: the whole ticks it completes (at most a tenth
    /// of a second's worth; the rest of a longer stall is dropped).
    pub fn advance(&mut self, frame_dt: f32) -> Ticks {
        let n = advance_fixed_clock(&mut self.accum, frame_dt, self.dt, self.max_per_frame, self.max_per_frame);
        self.ticks += n as u64;
        Ticks { n, alpha: self.alpha() }
    }

    /// Where the picture stands between the last two ticks.
    pub fn alpha(&self) -> f32 {
        ((self.accum / self.dt as f64) as f32).clamp(0.0, 1.0)
    }
}

/// A body's pose as the picture takes it: position, heading (degrees, clockwise from north),
/// pitch and bank.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyPose {
    pub position: DVec3,
    pub heading: f64,
    pub pitch: f32,
    pub bank: f32,
}

/// A body that moved further than this in one tick was put somewhere (placed, reset, taken
/// over): it is drawn where it is, not swept there.
const JUMP_M: f64 = 10.0;

impl BodyPose {
    /// The pose `alpha` of the way from `prev` to `self` (the heading the short way round).
    pub fn between(prev: &BodyPose, cur: &BodyPose, alpha: f32) -> BodyPose {
        if (cur.position - prev.position).length() > JUMP_M {
            return *cur;
        }
        let a = alpha.clamp(0.0, 1.0);
        let dh = (cur.heading - prev.heading + 540.0).rem_euclid(360.0) - 180.0;
        BodyPose {
            position: prev.position.lerp(cur.position, a as f64),
            heading: cur.heading - dh * (1.0 - a as f64),
            pitch: prev.pitch + (cur.pitch - prev.pitch) * a,
            bank: prev.bank + (cur.bank - prev.bank) * a,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ticks of `secs` seconds in frames of the given lengths, repeated.
    fn ticks_of(frames: &[f32], secs: f32) -> u64 {
        let mut f = FixedStep::new(120.0);
        let (mut t, mut k) = (0.0f32, 0);
        while t < secs - 1e-4 {
            let dt = frames[k % frames.len()];
            f.advance(dt);
            t += dt;
            k += 1;
        }
        f.ticks()
    }

    #[test]
    fn the_ticks_do_not_depend_on_the_frame_rate() {
        // ten seconds at 30, 45, 60, 144 and 240 frames a second, and frames of uneven length:
        // 1200 ticks each (one either way where the frames' sum ends a hair before or after)
        for frames in [vec![1.0 / 30.0], vec![1.0 / 45.0], vec![1.0 / 60.0], vec![1.0 / 144.0], vec![1.0 / 240.0], vec![0.011, 0.023, 0.016, 0.0067]] {
            let n = ticks_of(&frames, 10.0);
            assert!((1199..=1201).contains(&n), "{frames:?}: {n} ticks");
        }
        // whole multiples come out exact
        assert_eq!(ticks_of(&[1.0 / 30.0], 10.0), 1200);
        assert_eq!(ticks_of(&[1.0 / 60.0], 10.0), 1200);
    }

    #[test]
    fn a_frame_of_a_tick_s_multiple_gives_that_many_ticks() {
        let mut f = FixedStep::new(120.0);
        for _ in 0..1000 {
            assert_eq!(f.advance(1.0 / 30.0).n, 4);
        }
    }

    #[test]
    fn alpha_is_where_the_frame_ends_between_two_ticks() {
        let mut f = FixedStep::new(100.0);
        let t = f.advance(0.025);
        assert_eq!(t.n, 2);
        assert!((t.alpha - 0.5).abs() < 1e-3, "{t:?}");
        let t = f.advance(0.004);
        assert_eq!(t.n, 0);
        assert!((t.alpha - 0.9).abs() < 1e-3, "{t:?}");
    }

    #[test]
    fn a_stall_is_not_made_up_beyond_a_tenth_of_a_second() {
        let mut f = FixedStep::new(120.0);
        let t = f.advance(2.0);
        assert_eq!(t.n, 13);
        // (and nothing of it is owed afterwards)
        assert!(f.advance(1.0 / 120.0).n <= 2);
    }

    #[test]
    fn the_switch() {
        assert_eq!(parse_rate(""), None);
        assert_eq!(parse_rate("0"), None);
        assert_eq!(parse_rate("off"), None);
        assert_eq!(parse_rate("1"), Some(120.0));
        assert_eq!(parse_rate("on"), Some(120.0));
        assert_eq!(parse_rate("60"), Some(60.0));
        assert_eq!(parse_rate("2"), None);
        assert_eq!(parse_rate("100000"), None);
    }

    #[test]
    fn a_pose_between_two_ticks() {
        let a = BodyPose { position: DVec3::new(0.0, 0.0, 0.0), heading: 350.0, pitch: 0.0, bank: 1.0 };
        let b = BodyPose { position: DVec3::new(1.0, 2.0, 0.0), heading: 10.0, pitch: 2.0, bank: 1.0 };
        let m = BodyPose::between(&a, &b, 0.5);
        assert!((m.position - DVec3::new(0.5, 1.0, 0.0)).length() < 1e-9);
        assert!((m.heading.rem_euclid(360.0) - 0.0).abs() < 1e-9 || (m.heading.rem_euclid(360.0) - 360.0).abs() < 1e-9, "{m:?}");
        assert_eq!(m.pitch, 1.0);
        assert_eq!(BodyPose::between(&a, &b, 1.0), b);
        // a jump is not swept
        let far = BodyPose { position: DVec3::new(100.0, 0.0, 0.0), ..b };
        assert_eq!(BodyPose::between(&a, &far, 0.3), far);
    }
}
