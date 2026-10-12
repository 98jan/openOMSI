//! Opt-in restoration of the earlier procedural human poses.
//! Keep the original OMSI animation as the default for compatibility and A/B testing.

use super::*;
use glam::{Affine3A, Vec3};
use omsi_sim::human::{Pose, PoseInput, SLOTS};
use omsi_sim::people::{BusId, PoseFacts};

mod floor;

fn is_procedural_mode(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "procedural" | "enhanced" | "1")
}

pub(super) fn enabled() -> bool {
    // Read once per game session (PeopleView::new), not once per process: on Android
    // another game is started in the launcher's existing process.
    let mode = omsi_cfg::flags::OMSI_PAX_ANIMATION.live_var()
        .unwrap_or_else(|_| crate::settings::Settings::load().passenger_animation);
    is_procedural_mode(&mode)
}

/// The previous foot-planted / IK pose system, driven by the current people's state (as
/// the frame has it: `PersonFrame::facts`). This changes only how bodies are skinned,
/// never boarding, movement or ticket logic. `st`: the person as drawn (the pose's own
/// state, the ankles for `OMSI_TRACE_PAX`).
pub(super) fn bones(p: &PersonFrame, st: &mut PersonView, dt: f32, world: &World, buses: &HashMap<BusId, &omsi_sim::people::cabin::BusNow>) -> [Affine3A; SLOTS] {
    let (origin, heading, frame) = match p.place {
        Place::Ground => (p.position, p.heading, 0),
        Place::Bus(bus, at) => {
            let frame = match bus {
                BusId::Player => 1,
                BusId::Ai(id) => id.wrapping_add(2),
            };
            (at.as_dvec3(), p.lheading, frame)
        }
    };
    let PoseFacts { activity, seat, reach, floor_lift, hold, seated_level } = p.facts;
    let bus = match p.place {
        Place::Bus(id, _) => buses.get(&id).copied(),
        Place::Ground => None,
    };
    // A seated renderer anchor is [passpos] - human [seatheight], not the floor.
    let level = match seated_level {
        Some(lift) => origin.z + lift,
        None => origin.z,
    };
    let sample = |at| floor::sample(world, bus, at, level);
    let input = PoseInput {
        activity,
        // The simulation's renderer still places a seated person at hip - model height.
        // The IK foot floor is hip - physical seat height, which can be higher.
        origin: origin + glam::DVec3::Z * floor_lift as f64,
        heading,
        frame,
        velocity: p.vel,
        seat,
        reach,
        hold,
        floor: Some(&sample),
        ..PoseInput::default()
    };
    let seed = p.id;
    let pose = st.procedural.get_or_insert_with(|| Pose::new(seed));
    pose.advance(&p.ty.rig, &input, dt);
    let mut posed = pose.bones(&p.ty.rig);
    // Shift the local pose back to the renderer's original anchor; no game logic moves.
    if floor_lift != 0.0 {
        let lift = Affine3A::from_translation(Vec3::Z * floor_lift);
        for bone in &mut posed.bones {
            *bone = lift * *bone;
        }
        posed.ankle = posed.ankle.map(|a| lift.transform_point3(a));
    }
    // OMSI_TRACE_PAX records the deformed ankles rather than the untouched spawn markers.
    st.ankles = posed.ankle;
    posed.bones
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhanced_pose_requires_explicit_opt_in() {
        assert!(!is_procedural_mode(""));
        assert!(!is_procedural_mode("original"));
        assert!(!is_procedural_mode("0"));
        assert!(is_procedural_mode("procedural"));
        assert!(is_procedural_mode("enhanced"));
        assert!(is_procedural_mode("ENHANCED"));
        assert!(is_procedural_mode("1"));
    }
}
