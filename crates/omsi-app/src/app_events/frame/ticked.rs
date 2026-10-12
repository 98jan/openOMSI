//! The window's frame with the fixed simulation step (`OMSI_FIXED_STEP`, see
//! `omsi_sim::tick`): the same stages as `WINDOW_FRAME`, in three lists. What the player does
//! and the frame's housekeeping run once a frame (`TICKED_PRE`); the simulation - the traffic,
//! the player's bus with its scripts, the people, the duty, the clock and the weather - runs
//! once per tick, each tick as long as the next (`TICKED_SIM`); then the view of it, the
//! sounds, the LAN, the interface and the picture once a frame again (`TICKED_POST`), with the
//! player's bus and the camera drawn between the last two ticks.
//!
//! Off (the default) none of this runs: `App::frame` takes `WINDOW_FRAME` as it always did.

use super::stages::{WindowFrame, WindowStage};
use super::*;
use crate::frame_stages::Flow;

/// Once a frame, before the ticks.
pub(crate) const TICKED_PRE: &[WindowStage] = {
    use WindowStage::*;
    &[Timing, ShellActions, Placement, Menus, Streaming, Controllers, MouseDrive, PadActions]
};

/// Once per tick, with the tick's time. (The bus's step before the people: they sit in it.)
pub(crate) const TICKED_SIM: &[WindowStage] = {
    use WindowStage::*;
    &[Traffic, PlayerTick, OnFoot, Humans, AfterHumans, Duty, Weather, WheelSurface]
};

/// Once a frame, after the ticks, with the frame's time.
pub(crate) const TICKED_POST: &[WindowStage] = {
    use WindowStage::*;
    &[PlayerView, Audio, Lan, Integrations, ViewKeys, Lights, Scripted, Photo, Ui, Lighting, TraceLook, Render]
};

impl App {
    /// One frame of the window with the fixed step (see the module's notes).
    pub(super) fn frame_ticked(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let mut frame = WindowFrame::new(self, event_loop);
        if crate::frame_stages::run(&mut frame, TICKED_PRE)? == Flow::Stop {
            return Ok(());
        }
        let frame_dt = frame.frame_dt();
        let Some(ft) = frame.app().fixed_tick.as_mut() else { return Ok(()) };
        let tick_dt = ft.step.dt();
        let ticks = ft.step.advance(frame_dt);
        for k in 0..ticks.n {
            let app = frame.app();
            if let Some(ft) = app.fixed_tick.as_mut() {
                ft.enter_tick(k, ticks.n);
                if k + 1 == ticks.n {
                    ft.remember(app.player.as_ref().map(|p| &p.vehicle));
                }
            }
            frame.set_tick_dt(Some(tick_dt));
            let flow = crate::frame_stages::run(&mut frame, TICKED_SIM);
            frame.set_tick_dt(None);
            flow?;
        }
        // the daylight of the last tick (of the frame before, when this one had none)
        let daylight = match frame.daylight() {
            Some(d) => d,
            None => {
                let app = frame.app();
                let kept = app.fixed_tick.as_ref().and_then(|ft| ft.daylight());
                kept.unwrap_or_else(|| omsi_sim::Daylight::compute(&app.clock, app.session.envir.as_ref()))
            }
        };
        frame.set_daylight(daylight);
        if let Some(ft) = frame.app().fixed_tick.as_mut() {
            ft.frame_done(ticks, daylight);
        }
        crate::frame_stages::run(&mut frame, TICKED_POST)?;
        Ok(())
    }

    /// The player's bus as the frame shows it (`TICKED_POST`): between its last two ticks,
    /// into the scene and under the camera, then back where the simulation has it.
    pub(super) fn frame_player_view(&mut self, dt: f32) {
        if !self.frame_player_input(dt) {
            return;
        }
        let between = match (self.fixed_tick.as_ref(), self.player.as_ref()) {
            (Some(ft), Some(p)) if !self.paused => ft.between(&p.vehicle),
            _ => None,
        };
        if let (Some((_, at)), Some(p)) = (between.as_ref(), self.player.as_mut()) {
            omsi_sim::tick::put(&mut p.vehicle, at);
        }
        super::driving::player_to_scene(super::driving::PlayerToSceneCx::of(&mut self.parts()), dt);
        self.frame_view_camera(dt);
        if let (Some((cur, _)), Some(p)) = (between.as_ref(), self.player.as_mut()) {
            omsi_sim::tick::put(&mut p.vehicle, cur);
        }
        self.frame_player_hover();
    }
}
