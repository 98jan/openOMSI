//! The administration over a real session (UDP on this machine): a server as the dedicated
//! one runs it (`server_command` on every command that comes), players who join it, an admin
//! and a moderator who prove their passwords the way the game does, and what reaches whom.

use omsi_admin::{Audit, Auth, AuthCfg, Hub, Passwords, Status};
use omsi_lan::admin::{answer, server_command, ServerAdmin, ServerRules};
use omsi_net::{LanSession, Pose, WorldInfo};
use std::time::{Duration, Instant};

fn world() -> WorldInfo {
    WorldInfo { map: "maps/Grundorf/global.cfg".into(), date: "1989-05-30".into(), time: 9.0 * 3600.0, ..Default::default() }
}

struct Server {
    lan: LanSession,
    adm: ServerAdmin,
    hub: Hub,
}

impl Server {
    fn new(port: u16) -> Server {
        std::env::set_var("OMSI_NO_BRIDGE", "1");
        let lan = LanSession::host(port, "server", world(), false).unwrap();
        let mut adm = ServerAdmin::default();
        adm.password = "admin secret".into();
        adm.moderator_password = "mod secret".into();
        adm.passengers_on = true;
        let hub = Hub::new(Auth::new(Passwords { admin: "admin secret".into(), moderator: "mod secret".into() }, AuthCfg::default()), Audit::new(None, 0));
        Server { lan, adm, hub }
    }

    /// One step of the server's loop: the session, the commands that came, the world's part.
    fn step(&mut self) {
        self.lan.tick(0.05, &Pose::default());
        let rules = ServerRules { real_time: false, traffic_max: 120, notice_ok: &|_| true, metar: false };
        self.adm.prune(&self.lan);
        for (from, text) in self.lan.take_commands() {
            server_command(&mut self.lan, from, &text, &mut self.adm, &|_| None, &rules, Some(&self.hub));
        }
        for job in self.adm.take_world() {
            omsi_lan::admin::report(&mut self.lan, Some(&self.hub), &job.actor, job.record, &omsi_admin::Outcome::applied("done"));
        }
    }
}

/// A player's game and the commands the server sent it.
struct Player {
    lan: LanSession,
    heard: Vec<String>,
}

impl Player {
    fn join(server: &mut Server, port: u16, name: &str) -> Player {
        let mut lan = LanSession::join(&format!("127.0.0.1:{port}"), name, world(), Duration::from_millis(10)).unwrap();
        let t0 = Instant::now();
        while !lan.connected && t0.elapsed() < Duration::from_secs(5) {
            lan.tick(1.0, &Pose::default());
            server.step();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(lan.connected, "{name} joined");
        Player { lan, heard: Vec::new() }
    }

    fn step(&mut self) {
        self.lan.tick(0.05, &Pose::default());
        self.heard.extend(self.lan.take_commands().into_iter().filter(|(from, _)| *from == 1).map(|(_, t)| t));
    }
}

/// Steps everybody until `done` says so (or 5 s).
fn until(server: &mut Server, players: &mut [&mut Player], mut done: impl FnMut(&Server, &[&mut Player]) -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(5) {
        for p in players.iter_mut() {
            p.step();
        }
        server.step();
        if done(server, players) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// The game's `/admin <password>`: `auth?`, the challenge answered, the server's word.
fn login(server: &mut Server, p: &mut Player, others: &mut [&mut Player], password: &str) -> String {
    p.heard.clear();
    p.lan.command(1, "auth?");
    let mut all: Vec<&mut Player> = vec![p];
    all.extend(others.iter_mut().map(|x| &mut **x));
    assert!(until(server, &mut all, |_, ps| ps[0].heard.iter().any(|h| h.starts_with("admin-challenge") || h == "admin-no")));
    let challenge = all[0].heard.iter().find_map(|h| h.strip_prefix("admin-challenge ")).map(str::to_string);
    let Some(challenge) = challenge else { return "admin-no".into() };
    let a = answer(&challenge, password).unwrap();
    all[0].heard.clear();
    all[0].lan.command(1, &format!("auth {a}"));
    assert!(until(server, &mut all, |_, ps| ps[0].heard.iter().any(|h| h.starts_with("admin-ok") || h == "admin-no" || h == "admin-locked")));
    all[0].heard.iter().find(|h| h.starts_with("admin-")).cloned().unwrap()
}

fn listed(p: &Player) -> Vec<omsi_admin::Player> {
    p.heard.iter().filter_map(|h| h.strip_prefix("admin-p ")).filter_map(omsi_lan::admin::parse_player).collect()
}

fn results(p: &Player) -> Vec<(Status, String)> {
    p.heard
        .iter()
        .filter_map(|h| h.strip_prefix("admin-result "))
        .filter_map(|r| {
            let mut w = r.splitn(3, ' ');
            let _id = w.next()?;
            Some((Status::parse(w.next()?)?, w.next().unwrap_or("").to_string()))
        })
        .collect()
}

/// The order's case: search `pichau`, pick the right connection, send it away - nobody else,
/// and the admin hears what became of it.
#[test]
fn an_admin_finds_and_sends_away_the_player_picked_and_nobody_else() {
    let port = 28760;
    let mut s = Server::new(port);
    let mut admin = Player::join(&mut s, port, "Chefe");
    let mut pichau = Player::join(&mut s, port, "Pichau");
    let mut fan = Player::join(&mut s, port, "pichau_fan");
    let mut other = Player::join(&mut s, port, "Joao");
    assert_eq!(login(&mut s, &mut admin, &mut [&mut pichau, &mut fan, &mut other], "admin secret"), "admin-ok admin");
    // the search: both names that hold it, the exact one first, nobody else
    admin.heard.clear();
    admin.lan.command(1, "admin-list PICHAU");
    assert!(until(&mut s, &mut [&mut admin, &mut pichau, &mut fan, &mut other], |_, ps| ps[0].heard.iter().any(|h| h.starts_with("admin-end"))));
    let found = listed(&admin);
    assert_eq!(found.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["Pichau", "pichau_fan"]);
    let target = found[0].clone();
    assert_eq!(target.id, pichau.lan.my_id);
    // sent away by number and connection, with a reason
    admin.heard.clear();
    admin.lan.command(1, &format!("admin kick {}#{} spam no chat", target.id, target.conn));
    assert!(until(&mut s, &mut [&mut admin, &mut pichau, &mut fan, &mut other], |_, ps| !results(ps[0]).is_empty() && ps[1].lan.turned_away.is_some()));
    assert_eq!(pichau.lan.turned_away.as_deref(), Some("spam no chat"));
    let r = results(&admin);
    assert_eq!(r[0].0, Status::Applied, "{r:?}");
    assert!(r[0].1.contains("Pichau (#"), "{r:?}");
    // nobody else
    for _ in 0..20 {
        fan.step();
        other.step();
        s.step();
    }
    assert!(fan.lan.connected && fan.lan.turned_away.is_none());
    assert!(other.lan.connected && other.lan.turned_away.is_none());
    assert!(s.lan.peers().any(|p| p.pose.name == "pichau_fan"));
    assert!(!s.lan.peers().any(|p| p.pose.name == "Pichau"));
    // the audit log says who did what to whom
    let audit = s.hub.audit_since(0, 100);
    assert!(audit.iter().any(|e| e.action == "kick" && e.result == "applied" && e.actor.contains("Chefe") && e.target.contains("Pichau")), "{audit:?}");
}

/// A choice made before a player left and came back is not them any more; a player who left
/// is not found; a number that is somebody else's connection now is refused.
#[test]
fn an_old_choice_reaches_nobody() {
    let port = 28770;
    let mut s = Server::new(port);
    let mut admin = Player::join(&mut s, port, "Chefe");
    let mut p = Player::join(&mut s, port, "Pichau");
    assert_eq!(login(&mut s, &mut admin, &mut [&mut p], "admin secret"), "admin-ok admin");
    let (id, conn) = s.lan.peers().find(|x| x.pose.name == "Pichau").map(|x| (x.pose.id, x.conn)).unwrap();
    // gone and back: the same number on a new connection
    p.lan.leave();
    assert!(until(&mut s, &mut [&mut admin], |s, _| !s.lan.peers().any(|x| x.pose.id == id)));
    assert!(p.lan.reconnect());
    assert!(until(&mut s, &mut [&mut admin, &mut p], |_, ps| ps[1].lan.connected));
    let now = s.lan.peers().find(|x| x.pose.id == id).map(|x| x.conn).unwrap();
    assert_ne!(now, conn);
    admin.heard.clear();
    admin.lan.command(1, &format!("admin kick {id}#{conn} old choice"));
    assert!(until(&mut s, &mut [&mut admin, &mut p], |_, ps| !results(ps[0]).is_empty()));
    let r = results(&admin);
    assert_eq!(r[0].0, Status::Rejected, "{r:?}");
    assert!(r[0].1.contains("another connection"), "{r:?}");
    assert!(p.lan.connected && p.lan.turned_away.is_none());
    // a number nobody has
    admin.heard.clear();
    admin.lan.command(1, "admin kick 999#1");
    assert!(until(&mut s, &mut [&mut admin, &mut p], |_, ps| !results(ps[0]).is_empty()));
    assert_eq!(results(&admin)[0].0, Status::Rejected);
    // the older menu's line (no connection) still works
    admin.heard.clear();
    admin.lan.command(1, &format!("admin kick {id}"));
    assert!(until(&mut s, &mut [&mut admin, &mut p], |_, ps| !results(ps[0]).is_empty()));
    assert_eq!(results(&admin)[0].0, Status::Applied);
}

/// `line` sent by `who`; the server's answer.
fn ask(s: &mut Server, who: &mut Player, others: &mut [&mut Player], line: &str) -> (Status, String) {
    who.heard.clear();
    who.lan.command(1, line);
    let mut all: Vec<&mut Player> = vec![who];
    all.extend(others.iter_mut().map(|x| &mut **x));
    assert!(until(s, &mut all, |_, ps| !results(ps[0]).is_empty()), "{line}");
    results(all[0]).remove(0)
}

/// The role decides on the server, whatever the menu showed: a moderator moderates and leaves
/// the world (and the admins) alone; a player who gave no password is told so; a wrong
/// password gives nothing.
#[test]
fn each_role_does_what_it_may() {
    let port = 28780;
    let mut s = Server::new(port);
    let mut admin = Player::join(&mut s, port, "Chefe");
    let mut moderator = Player::join(&mut s, port, "Mod");
    let mut nobody = Player::join(&mut s, port, "Curioso");
    assert_eq!(login(&mut s, &mut admin, &mut [&mut moderator, &mut nobody], "admin secret"), "admin-ok admin");
    assert_eq!(login(&mut s, &mut moderator, &mut [&mut admin, &mut nobody], "mod secret"), "admin-ok moderator");
    assert_eq!(login(&mut s, &mut nobody, &mut [&mut admin, &mut moderator], "guess"), "admin-no");
    // the world: the admin's alone
    let (st, why) = ask(&mut s, &mut moderator, &mut [&mut admin, &mut nobody], "admin clock 30600");
    assert_eq!(st, Status::Rejected);
    assert!(why.contains("moderator"), "{why}");
    assert_eq!(ask(&mut s, &mut moderator, &mut [&mut admin, &mut nobody], "admin passengers 2").0, Status::Rejected);
    assert_eq!(ask(&mut s, &mut admin, &mut [&mut moderator, &mut nobody], "admin passengers 2").0, Status::Applied);
    // moderation: the moderator's too, but not against an admin
    assert_eq!(ask(&mut s, &mut moderator, &mut [&mut admin, &mut nobody], "admin say hello").0, Status::Applied);
    let admin_id = admin.lan.my_id;
    let (st, why) = ask(&mut s, &mut moderator, &mut [&mut admin, &mut nobody], &format!("admin kick {admin_id}"));
    assert_eq!(st, Status::Rejected, "{why}");
    assert!(admin.lan.connected);
    // no password given: nothing, and told so
    let (st, why) = ask(&mut s, &mut nobody, &mut [&mut admin, &mut moderator], &format!("admin kick {admin_id}"));
    assert_eq!(st, Status::Rejected);
    assert!(why.contains("/admin"), "{why}");
    nobody.heard.clear();
    nobody.lan.command(1, "admin-list");
    let mut all = [&mut nobody, &mut admin, &mut moderator];
    until(&mut s, &mut all, |_, ps| ps[0].heard.iter().any(|h| h.starts_with("admin-")));
    assert!(listed(&nobody).is_empty());
    // a command that is not one, and a text with markup and separators: one value, no command
    let (st, _) = ask(&mut s, &mut admin, &mut [&mut moderator, &mut nobody], "admin dance");
    assert_eq!(st, Status::Rejected);
    let (st, _) = ask(&mut s, &mut admin, &mut [&mut moderator, &mut nobody], "admin say <b>hi</b>; kick 2 | kick 3");
    assert_eq!(st, Status::Applied);
    assert!(moderator.lan.connected && nobody.lan.connected);
    // logging out ends the rights at once
    let (st, _) = ask(&mut s, &mut moderator, &mut [&mut admin, &mut nobody], "admin-logout");
    assert_eq!(st, Status::Applied);
    let (st, why) = ask(&mut s, &mut moderator, &mut [&mut admin, &mut nobody], "admin say still here?");
    assert_eq!(st, Status::Rejected, "{why}");
}
