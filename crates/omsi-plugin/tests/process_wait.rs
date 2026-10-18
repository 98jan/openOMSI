//! Real child processes and Lua continuations: waiting must never block a game frame.
mod common;
use common::{dir, Game};
use omsi_plugin::{
    api::{json, Value},
    HostConfig, Plugins,
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[test]
#[ignore = "subprocess helper, exercised by the process wait tests"]
fn process_wait_child() {
    let gate = std::env::var_os("PLUGIN_WAIT_TEST_GATE").expect("helper gate");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !Path::new(&gate).exists() {
        if Instant::now() > deadline {
            std::process::exit(99);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    std::process::exit(7);
}

fn game() -> Game {
    let mut g = Game::new();
    for name in [
        "before_wait",
        "after_wait",
        "following",
        "other_frames",
        "child_pid",
        "stopped",
    ] {
        g.vars.insert(name.into(), 0.0);
    }
    g
}

fn child(gate: &Path) -> String {
    let quote = |s: &str| json::encode(&Value::Str(s.into()));
    format!(
        r#"assert(omsi.process.start({}, {{"--exact", "process_wait_child", "--ignored"}}, {{env = {{PLUGIN_WAIT_TEST_GATE = {}}}}}))"#,
        quote(std::env::current_exe().unwrap().to_str().unwrap()),
        quote(gate.to_str().unwrap())
    )
}

fn drive_until(p: &mut Plugins, g: &mut Game, done: impl Fn(&Game) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(g) && Instant::now() < deadline {
        p.frame(g);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(done(g), "did not finish: {:?}", g.messages);
}

#[test]
fn main_chunk_waits_without_blocking_game_or_other_plugins() {
    let d = dir("wait-main");
    let gate = d.join("gate");
    let script = format!(
        r#"
        local handle = {}
        local count, state = omsi.process.wait(handle)
        assert(count == 7 and state == "exited")
        assert(omsi.process.exit_code(handle) == 7)
        omsi.process.forget(handle)
        omsi.set_var("after_wait", count)
        function on_start() omsi.message("started after wait") end
    "#,
        child(&gate)
    );
    std::fs::write(d.join("a_wait.lua"), script).unwrap();
    std::fs::write(
        d.join("b_other.lua"),
        "function on_frame() omsi.set_var('other_frames', (omsi.var('other_frames') or 0) + 1) end",
    )
    .unwrap();
    let mut p = Plugins::load(&[d], &HostConfig::default());
    let mut g = game();
    for _ in 0..8 {
        let at = Instant::now();
        p.frame(&mut g);
        assert!(
            at.elapsed() < Duration::from_millis(250),
            "game frame blocked on child"
        );
        assert!(g.vars["after_wait"] == 0.0);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(g.vars["other_frames"], 8.0);
    std::fs::write(gate, "").unwrap();
    drive_until(&mut p, &mut g, |g| g.vars["after_wait"] != 0.0);
    assert_eq!(g.vars["after_wait"], 7.0);
    assert_eq!(g.messages, ["started after wait"]);
    p.finalize();
}

#[test]
fn wait_in_handler_preserves_locals_and_defers_following_callbacks() {
    let d = dir("wait-handler");
    let gate = d.join("gate");
    std::fs::write(
        d.join("wait.lua"),
        format!(
            r#"
        local once = false
        omsi.on("frame", function()
            if once then return end
            once = true
            local before = 35
            local handle = {}
            omsi.set_var("before_wait", 1)
            local count = omsi.process.wait(handle)
            omsi.set_var("after_wait", before + count)
        end)
        omsi.on("frame", function() omsi.set_var("following", (omsi.var("following") or 0) + 1) end)
    "#,
            child(&gate)
        ),
    )
    .unwrap();
    let mut p = Plugins::load(&[d], &HostConfig::default());
    let mut g = game();
    p.frame(&mut g);
    assert_eq!(g.vars["before_wait"], 1.0);
    for _ in 0..3 {
        p.frame(&mut g);
    }
    assert!(g.vars["after_wait"] == 0.0 && g.vars["following"] == 0.0);
    std::fs::write(gate, "").unwrap();
    drive_until(&mut p, &mut g, |g| g.vars["after_wait"] != 0.0);
    assert_eq!(g.vars["after_wait"], 42.0);
    assert_eq!(g.vars["following"], 1.0);
    p.finalize();
}

#[test]
fn resumed_lua_keeps_its_instruction_budget() {
    let d = dir("wait-budget");
    let gate = d.join("gate");
    std::fs::write(
        d.join("wait.lua"),
        format!(
            r#"
        function on_start()
            omsi.process.wait({})
            while true do end
        end
    "#,
            child(&gate)
        ),
    )
    .unwrap();
    let mut p = Plugins::load(&[d], &HostConfig::default());
    let mut g = game();
    p.frame(&mut g);
    assert!(!p.lua[0].disabled);
    std::fs::write(gate, "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !p.lua[0].disabled && Instant::now() < deadline {
        p.frame(&mut g);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(p.lua[0].disabled, "resumed runaway escaped the budget");
    assert!(
        g.messages.iter().any(|s| s.contains("longer than 50 ms")),
        "{:?}",
        g.messages
    );
    p.finalize();
}

#[test]
fn timer_can_wait_and_be_cancelled_by_unloading() {
    let d = dir("wait-unload");
    let gate = d.join("gate");
    std::fs::write(
        d.join("wait.lua"),
        format!(
            r#"
        omsi.after(0.01, function()
            local handle = {}
            omsi.set_var("child_pid", omsi.process.status(handle).pid)
            omsi.process.wait(handle)
            omsi.set_var("after_wait", 1)
        end)
        function on_stop() omsi.set_var("stopped", 1) end
    "#,
            child(&gate)
        ),
    )
    .unwrap();
    let mut p = Plugins::load(&[d], &HostConfig::default());
    let mut g = game();
    drive_until(&mut p, &mut g, |g| g.vars["child_pid"] > 0.0);
    let pid = g.vars["child_pid"] as u32;
    p.lua[0].stop(&mut g);
    assert_eq!(g.vars["stopped"], 1.0);
    p.finalize();
    assert!(g.vars["after_wait"] == 0.0);
    // The stop handler ran, but the suspended continuation must never resume.
    #[cfg(target_os = "linux")]
    {
        let proc = std::path::PathBuf::from(format!("/proc/{pid}"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while proc.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!proc.exists(), "waiting child survived plugin unload");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
}

#[test]
fn a_resumed_handler_can_wait_for_another_child() {
    let d = dir("wait-twice");
    let first = d.join("first");
    let second = d.join("second");
    std::fs::write(
        d.join("wait.lua"),
        format!(
            r#"
        local once = false
        function on_frame()
            if once then return end
            once = true
            local a = omsi.process.wait({})
            omsi.set_var("before_wait", a)
            local b = omsi.process.wait({})
            omsi.set_var("after_wait", a + b)
        end
    "#,
            child(&first),
            child(&second)
        ),
    )
    .unwrap();
    let mut p = Plugins::load(&[d], &HostConfig::default());
    let mut g = game();
    p.frame(&mut g);
    assert_eq!(g.vars["before_wait"], 0.0);
    std::fs::write(first, "").unwrap();
    drive_until(&mut p, &mut g, |g| g.vars["before_wait"] == 7.0);
    for _ in 0..3 {
        p.frame(&mut g);
    }
    assert_eq!(g.vars["after_wait"], 0.0);
    std::fs::write(second, "").unwrap();
    drive_until(&mut p, &mut g, |g| g.vars["after_wait"] == 14.0);
    p.finalize();
}
