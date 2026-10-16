use super::{find_device_cfg, latch_release, names_match, DeviceCfg};

struct Held {
    device: String,
    button: usize,
    actions: Vec<String>,
    latching: bool,
    continuous: bool,
    fresh: bool,
}

/// Button-down actions remember their original mapping until release. Editing a binding
/// must release that action, rather than sending an unrelated new action's key-up.
#[derive(Default)]
pub(super) struct HeldButtons(Vec<Held>);

impl HeldButtons {
    pub(super) fn event(&mut self, cfg: &[DeviceCfg], name: &str, button: usize, down: bool, actions: &mut Vec<(String, bool)>) {
        if down {
            if self.0.iter().any(|h| names_match(&h.device, name) && h.button == button) {
                return;
            }
            let Some(d) = find_device_cfg(cfg, name) else { return };
            let bound = d.actions_of(button);
            if bound.is_empty() {
                return;
            }
            actions.extend(bound.iter().map(|a| (a.clone(), true)));
            self.0.push(Held { device: name.to_string(), button, actions: bound, latching: d.latching.contains(&button), continuous: d.continuous(button), fresh: true });
        } else if let Some(i) = self.0.iter().position(|h| names_match(&h.device, name) && h.button == button) {
            let h = self.0.remove(i);
            actions.extend(h.actions.iter().map(|a| (a.clone(), false)));
            if h.latching {
                for a in &h.actions {
                    let back = latch_release(a);
                    actions.push((back.clone(), true));
                    actions.push((back, false));
                }
            }
        }
    }

    pub(super) fn release(&mut self, actions: &mut Vec<(String, bool)>) {
        for h in self.0.drain(..) {
            // Do not toggle a latching switch just because its configuration changed.
            actions.extend(h.actions.into_iter().map(|a| (a, false)));
        }
    }

    /// Let go of the buttons held down, but not of the latching switches: a switch stays
    /// where it is while the pause menu is open or the window is in the background, and
    /// when it comes out later its release still reaches the bus. Let go of with the rest,
    /// the switch went out in the game at ESC or alt-tab and only came back once it was
    /// switched off and on again by hand (#1876).
    pub(super) fn release_momentary(&mut self, actions: &mut Vec<(String, bool)>) {
        self.0.retain(|h| {
            if !h.latching {
                actions.extend(h.actions.iter().map(|a| (a.clone(), false)));
            }
            h.latching
        });
    }

    pub(super) fn repeating(&mut self) -> Vec<String> {
        let mut due = Vec::new();
        for h in self.0.iter_mut().filter(|h| h.continuous) {
            if !std::mem::take(&mut h.fresh) {
                due.extend(h.actions.iter().cloned());
            }
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel() -> Vec<DeviceCfg> {
        let mut d = DeviceCfg { name: "Wheel".into(), ..Default::default() };
        d.buttons = vec![("bus_doorfront0".into(), "0".into()), ("retarder_up".into(), "1".into())];
        d.more = vec![(0, "bus_doorfront1".into()), (0, "bus_doorrear0".into())];
        vec![d]
    }

    #[test]
    fn one_button_works_every_action_it_is_given() {
        let (cfg, mut held, mut actions) = (wheel(), HeldButtons::default(), Vec::new());
        held.event(&cfg, "Wheel", 0, true, &mut actions);
        held.event(&cfg, "Wheel", 0, false, &mut actions);
        let names: Vec<(&str, bool)> = actions.iter().map(|(a, d)| (a.as_str(), *d)).collect();
        assert_eq!(names, [("bus_doorfront0", true), ("bus_doorfront1", true), ("bus_doorrear0", true), ("bus_doorfront0", false), ("bus_doorfront1", false), ("bus_doorrear0", false)]);
    }

    #[test]
    fn a_continuous_button_fires_again_every_frame_it_is_held() {
        let (cfg, mut held, mut actions) = (wheel(), HeldButtons::default(), Vec::new());
        held.event(&cfg, "Wheel", 1, true, &mut actions);
        held.event(&cfg, "Wheel", 0, true, &mut actions);
        assert!(held.repeating().is_empty());
        assert_eq!(held.repeating(), ["retarder_up"]);
        assert_eq!(held.repeating(), ["retarder_up"]);
        held.event(&cfg, "Wheel", 1, false, &mut actions);
        assert!(held.repeating().is_empty());
    }
}
