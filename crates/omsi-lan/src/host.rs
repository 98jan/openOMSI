//! What LAN play needs of the game around it that is not plain data: the game implements
//! [`LanHost`] and hands it to the session's calls.

use crate::config::LanConfig;
use std::sync::Arc;

pub trait LanHost {
    /// With a world loaded: the season's texture folder it would have with `cfg`'s date,
    /// season and weather, and the one loaded now (None in either: the default look).
    fn season_folders(&self, cfg: &LanConfig) -> Option<(Option<String>, Option<String>)>;
    /// `weather` names a weather the game makes without a file (the natural model, the
    /// cycle, a custom one).
    fn weather_without_file(&self, weather: &str) -> bool;
    /// `weather` is a weather report (METAR): Some(whether the game can read it).
    fn weather_report(&self, weather: &str) -> Option<bool>;
    /// The depot file of vehicle type `ty` for the map and `cfg`'s depot choice.
    fn depot_of(&self, cfg: &LanConfig, ty: &omsi_sim::VehicleType) -> Option<Arc<omsi_vehicle::Hof>>;
}
