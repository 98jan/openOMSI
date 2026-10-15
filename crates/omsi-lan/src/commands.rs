//! What another game's command (`LanSession::command`) tells this one, read once into what it
//! means (`Command::parse`); the game carries out what is its own (the app's `lan_command`).
//! The words are the session's: most come from the host (player 1) alone, and one of those
//! from anybody else is not taken. See the app's `admin` for the administration's part.

use crate::voice::VoiceServer;
use glam::DVec3;

/// A command from another game.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// A passenger of ours works this switch of our bus (`passenger::may_work` says whether
    /// they may).
    Trigger(String),
    /// A game asks the host which voice server the session talks on.
    VoiceAsked,
    /// (host) the voice server the session talks on (None: none).
    Voice(Option<VoiceServer>),
    /// (host) our bus put there, stopped.
    Teleport { at: DVec3, heading: f64 },
    /// (host) the host's object editor: object `id` moved, turned or deleted...
    ObjectEdit { id: i64, moved: DVec3, turned: f64, deleted: bool },
    /// (host) ... or a copy of an object (the file `file`) where it stands now, or taken
    /// away again (`removed`).
    ObjectAdd { id: i64, at: DVec3, heading: f64, removed: bool, file: String },
    /// (host) a service for our bus: `repair`, `refuel` or `wash`.
    Service(String),
    /// (host) our bus back on its wheels.
    Unstick,
    /// (server) the admin password proven without sending it: the challenge's words
    /// (`admin::answer`).
    AdminChallenge(String),
    /// (host) a duty from the dispatch: (line, tour, trip, stop), see `admin::duty_arg` -
    /// None: one this game cannot read (answered `duty-no malformed`).
    Duty(Option<(String, String, usize, usize)>),
    /// (host) a notice over the navigator for a few seconds: `<id> <seconds> <info|warn|alert>
    /// <text>` (answered `notify-seen <id>`).
    Notify(String),
    /// (host) the duty taken back by the server's dispatch.
    DutyOff,
    /// (server) too many wrong admin passwords lately.
    AdminLocked,
    /// (server) the admin password was right: the administration menu is ours.
    AdminOk,
    /// (server) a wrong admin password.
    AdminNo,
    /// A game we host by code showed our notice (its id).
    NotifySeen(String),
}

/// The host's number in every session.
const HOST: u32 = 1;

impl Command {
    /// The command `text` from player `from`, or None: one this game does not take.
    pub fn parse(from: u32, text: &str) -> Option<Command> {
        if let Some(ev) = text.strip_prefix("trigger ") {
            return Some(Command::Trigger(ev.trim().to_string()));
        }
        if text == "voice?" {
            return Some(Command::VoiceAsked);
        }
        let host = from == HOST;
        if text.starts_with("voice ") {
            return VoiceServer::parse_command(text).filter(|_| host).map(Command::Voice);
        }
        let (verb, arg) = text.split_once(' ').unwrap_or((text, ""));
        let numbers = |s: &str| s.split_whitespace().filter_map(|x| x.parse().ok()).collect::<Vec<f64>>();
        Some(match verb {
            "notify-seen" => Command::NotifySeen(arg.trim().to_string()),
            _ if !host => return None,
            "teleport" => match numbers(arg)[..] {
                [x, y, z, heading] => Command::Teleport { at: DVec3::new(x, y, z), heading },
                _ => return None,
            },
            "objedit" => match numbers(arg)[..] {
                [id, x, y, z, turned, deleted] => Command::ObjectEdit { id: id as i64, moved: DVec3::new(x, y, z), turned, deleted: deleted > 0.5 },
                _ => return None,
            },
            "objadd" => {
                let mut it = arg.splitn(7, ' ');
                let n: Vec<f64> = (0..6).filter_map(|_| it.next().and_then(|x| x.parse().ok())).collect();
                let file = it.next().unwrap_or("").trim().to_string();
                if n.len() != 6 || file.is_empty() {
                    return None;
                }
                Command::ObjectAdd { id: n[0] as i64, at: DVec3::new(n[1], n[2], n[3]), heading: n[4], removed: n[5] >= 0.5, file }
            }
            "service" if matches!(arg.trim(), "repair" | "refuel" | "wash") => Command::Service(arg.trim().to_string()),
            "unstick" => Command::Unstick,
            "admin-challenge" => Command::AdminChallenge(arg.to_string()),
            "duty" => Command::Duty(crate::admin::parse_duty(arg)),
            "notify" => Command::Notify(arg.to_string()),
            "duty-off" => Command::DutyOff,
            "admin-locked" => Command::AdminLocked,
            "admin-ok" => Command::AdminOk,
            "admin-no" => Command::AdminNo,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The host's commands read as what they mean, and are not taken from anybody else.
    #[test]
    fn the_hosts_commands_and_nobody_elses() {
        assert_eq!(Command::parse(1, "teleport 1.5 2 3 90"), Some(Command::Teleport { at: DVec3::new(1.5, 2.0, 3.0), heading: 90.0 }));
        assert_eq!(Command::parse(2, "teleport 1.5 2 3 90"), None, "only the host moves our bus");
        assert_eq!(Command::parse(1, "teleport 1 2 3"), None);
        assert_eq!(Command::parse(1, "objedit 7 1 2 3 45 1"), Some(Command::ObjectEdit { id: 7, moved: DVec3::new(1.0, 2.0, 3.0), turned: 45.0, deleted: true }));
        assert_eq!(
            Command::parse(1, "objadd 8 1 2 3 90 0 Sceneryobjects/A/a b.sco"),
            Some(Command::ObjectAdd { id: 8, at: DVec3::new(1.0, 2.0, 3.0), heading: 90.0, removed: false, file: "Sceneryobjects/A/a b.sco".into() })
        );
        assert_eq!(Command::parse(1, "service wash"), Some(Command::Service("wash".into())));
        assert_eq!(Command::parse(1, "service paint"), None);
        assert_eq!(Command::parse(1, "duty 12 A 1 0"), Some(Command::Duty(Some(("12".into(), "A".into(), 1, 0)))));
        assert_eq!(Command::parse(1, "duty x"), Some(Command::Duty(None)), "answered as malformed");
        assert_eq!(Command::parse(1, "admin-ok"), Some(Command::AdminOk));
        assert_eq!(Command::parse(3, "admin-ok"), None);
        assert_eq!(Command::parse(3, "notify-seen 9"), Some(Command::NotifySeen("9".into())), "anybody's game says it saw a notice");
        assert_eq!(Command::parse(3, "trigger int_haltewunsch"), Some(Command::Trigger("int_haltewunsch".into())));
        assert_eq!(Command::parse(3, "voice?"), Some(Command::VoiceAsked));
        assert_eq!(Command::parse(1, "dance"), None);
    }
}
