//! Launcher-specific configuration paths for the shared controller worker.
pub(super) use omsi_input::launcher_pads::Pads;

pub(super) fn paths() -> omsi_input::launcher_pads::Paths {
    omsi_input::launcher_pads::Paths {
        read_root: || {
            let _ = omsi_launcher_lib::content_dir();
            omsi_launcher_lib::root_dir().unwrap_or_default()
        },
        write_root: || omsi_launcher_lib::content_dir()
            .filter(|c| c.join("Inputs").exists() || std::fs::create_dir_all(c.join("Inputs")).is_ok())
            .filter(|c| omsi_cfg::is_writable(&c.join("Inputs")))
            .unwrap_or_else(|| omsi_launcher_lib::data_dir().join("content")),
    }
}
