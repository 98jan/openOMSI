use super::{Settings, SETTINGS_VERSION};

impl Settings {
    pub(super) fn migrate_old_settings(&mut self, version: u32) {
        // Files older than version 2 say `boarding=pay` because that was the launcher's
        // default, not because anybody chose it: passengers then stood at the cash desk
        // waiting for a driver who did not know he had to sell them a ticket.
        // Files older than version 3 say `passenger_animation=original` because that was the
        // default, not because anybody chose it: the people's own poses are the default now.
        if version < SETTINGS_VERSION && self.passenger_animation == "original" {
            log::info!("settings: passenger_animation=original from an old settings file taken as the people's own poses (choose Original OMSI again to keep it)");
            self.passenger_animation = "enhanced".into();
        }
        if version < 2 && self.boarding == "pay" {
            log::info!("settings: boarding=pay from an old settings file taken as auto (choose pay again in the launcher to keep it)");
            self.boarding = "auto".into();
        }
    }

}
