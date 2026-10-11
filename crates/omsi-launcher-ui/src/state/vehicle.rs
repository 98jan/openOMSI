//! Vehicle selection defaults: depot and fleet number.
use super::*;

impl State {
    /// The depot file a bus uses on the chosen map: the map's own when the bus has it (or
    /// has none: the game borrows it), else the bus's first.
    /// The depot file for the chosen bus: the one the map's own buses use on the chosen date
    /// (the chrono scenarios change it: Berlin's 1994 depot has line 137 where 1986's had
    /// 92), which the bus has, else its first.
    pub fn default_hof(&self) -> String {
        let on_date = self.map().and_then(|m| {
            let dir = omsi_cfg::resolve_path(std::path::Path::new(&self.config.root), &m.file);
            omsi_map::ailists::depot_hof_on(dir.parent()?, omsi_map::ailists::date_code(&self.choice.date)?)
        });
        let want = on_date.or_else(|| self.map().map(|m| m.hof.clone())).unwrap_or_default();
        let Some(v) = self.bus() else { return want };
        // (the bus's own depot of the same place before the map's borrowed from another
        // bus, and one named like the map before its first, #896)
        let names: Vec<&str> = v.hofs.iter().map(|h| h.as_str()).collect();
        let like = |hints: &[&str]| omsi_vehicle::hof::closest_name(&names, hints).map(|i| v.hofs[i].clone());
        let map_hints: Vec<String> = self.map().map(|m| vec![m.name.clone(), m.friendly.clone(), m.file.trim_end_matches("/global.cfg").rsplit('/').next().unwrap_or("").to_string()]).unwrap_or_default();
        let map_hints: Vec<&str> = map_hints.iter().map(|h| h.as_str()).collect();
        // a depot file carrying the line driven: the day's, one named like the map, else the fullest
        let with_line = match (&self.choice.line, self.choice.free) {
            (Some(line), false) => core::depot_names_with_line(std::path::Path::new(&self.config.root), &v.file, line),
            _ => Vec::new(),
        };
        // (not over the map's own depot borrowed from elsewhere when it carries the line: a
        // bus without it took a depot of its folder that had a line of the same number - the
        // map's was no longer given to a bus that lacks it, #1837)
        let map_depot_has_line = match (&self.choice.line, want.is_empty()) {
            (Some(line), false) => core::depot_has_line(&want, line) == Some(true),
            _ => false,
        };
        if !with_line.is_empty() && !map_depot_has_line && !with_line.iter().any(|h| h.eq_ignore_ascii_case(&want)) {
            let names: Vec<&str> = with_line.iter().map(|h| h.as_str()).collect();
            let hints: Vec<&str> = std::iter::once(want.as_str()).chain(map_hints.iter().copied()).collect();
            let pick = omsi_vehicle::hof::closest_name(&names, &hints).unwrap_or(0);
            return with_line[pick].clone();
        }
        v.hofs
            .iter()
            .find(|h| h.eq_ignore_ascii_case(&want))
            .cloned()
            .or_else(|| like(&[want.as_str()]))
            .or(Some(want.clone()).filter(|w| !w.is_empty()))
            .or_else(|| like(&map_hints))
            .or_else(|| v.hofs.first().cloned())
            .unwrap_or_default()
    }

    /// Pick once when the bus changes; both launcher layouts use this stored choice.
    pub(super) fn random_bus_number(&self) -> String {
        self.bus().filter(|v| !v.numbers.is_empty()).map(|v| {
            let index = (omsi_net::random_session_id() % v.numbers.len() as u64) as usize;
            v.numbers[index].0.clone()
        }).unwrap_or_default()
    }

}
