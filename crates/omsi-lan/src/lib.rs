//! openOMSI's multiplayer on top of the transport (`omsi-net`): what of a session needs no
//! game world - the host's mods for the players who join (`mods`) and the voice chat's link
//! to the TeamSpeak plugin (`voice`). The game's side (the remote buses drawn and heard, the
//! shared world, the chat on the HUD) stays in the app, which hands this crate plain values.

pub mod mods;
pub mod voice;

use std::path::PathBuf;

/// `~/.openomsi`: the session's status file, the LAN mods' store, the voice chat's files.
pub fn data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".openomsi"))
}

/// The line the launcher looks for in the game's log when the game is over: the server sent
/// the player away (kick, ban) or turned it away at the door, with the server's message.
pub const LEFT_SERVER: &str = "LAN: disconnected from the server: ";
