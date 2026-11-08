//! The sync rules against an in-memory database and Redis (no server, no network).

use crate::config::Config;
use crate::db::Stored;
use crate::engine::{Action, Cache, Engine, Store};
use crate::nbt::parse_snbt;
use crate::redis_mgr::{Message, MessageType};
use crate::snapshot::{keys, Packed, Platform};
use crate::util;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct FakeDb {
    rows: Mutex<HashMap<u128, Vec<Packed>>>,
    users: Mutex<HashMap<u128, String>>,
}

impl FakeDb {
    fn stored(p: &Packed) -> Stored {
        Stored { version_uuid: p.id.clone(), snapshot: Ok(p.clone()) }
    }
    fn sorted(&self, player: u128) -> Vec<Packed> {
        let mut v = self.rows.lock().unwrap().get(&player).cloned().unwrap_or_default();
        v.sort_by_key(|p| std::cmp::Reverse(p.time()));
        v
    }
}

impl Store for Arc<FakeDb> {
    fn ensure_user(&self, uuid: u128, name: &str) -> Result<(), String> {
        self.users.lock().unwrap().insert(uuid, name.into());
        Ok(())
    }
    fn user_by_name(&self, name: &str) -> Result<Option<(u128, String)>, String> {
        Ok(self.users.lock().unwrap().iter().find(|(_, n)| *n == name).map(|(u, n)| (*u, n.clone())))
    }
    fn user_by_uuid(&self, uuid: u128) -> Result<Option<String>, String> {
        Ok(self.users.lock().unwrap().get(&uuid).cloned())
    }
    fn latest_snapshot(&self, player: u128) -> Result<Option<Stored>, String> {
        Ok(self.sorted(player).first().map(FakeDb::stored))
    }
    fn latest_snapshot_with_causes(&self, player: u128, causes: &[&str]) -> Result<Option<Stored>, String> {
        Ok(self.sorted(player).iter().find(|p| causes.contains(&p.save_cause.as_str())).map(FakeDb::stored))
    }
    fn all_snapshots(&self, player: u128) -> Result<Vec<Stored>, String> {
        Ok(self.sorted(player).iter().map(FakeDb::stored).collect())
    }
    fn snapshot(&self, player: u128, version: u128) -> Result<Option<Stored>, String> {
        Ok(self.sorted(player).iter().find(|p| p.id_u128() == Some(version)).map(FakeDb::stored))
    }
    fn add_snapshot(&self, player: u128, s: &Packed) -> Result<(), String> {
        self.rows.lock().unwrap().entry(player).or_default().push(s.clone());
        Ok(())
    }
    fn add_snapshot_without_rotation(&self, player: u128, s: &Packed) -> Result<(), String> {
        self.add_snapshot(player, s)
    }
    fn delete_snapshot(&self, player: u128, version: u128) -> Result<bool, String> {
        let mut rows = self.rows.lock().unwrap();
        let v = rows.entry(player).or_default();
        let before = v.len();
        v.retain(|p| p.id_u128() != Some(version));
        Ok(v.len() != before)
    }
    fn set_pinned(&self, _: u128, _: u128, _: bool) -> Result<bool, String> {
        Ok(true)
    }
    fn ping(&self) -> Result<Duration, String> {
        Ok(Duration::ZERO)
    }
}

#[derive(Default)]
struct FakeRedis {
    latest: Mutex<HashMap<u128, Packed>>,
    checkout: Mutex<HashMap<u128, String>>,
    switch: Mutex<HashMap<u128, ()>>,
    published: Mutex<Vec<(MessageType, Message)>>,
}

const HERE: &str = "horizon";

impl Cache for Arc<FakeRedis> {
    fn ping(&self) -> Result<Duration, String> {
        Ok(Duration::ZERO)
    }
    fn version(&self) -> String {
        "fake".into()
    }
    fn take_user_data(&self, uuid: u128) -> Result<Option<Result<Packed, String>>, String> {
        Ok(self.latest.lock().unwrap().remove(&uuid).map(Ok))
    }
    fn set_user_data(&self, uuid: u128, s: &Packed) -> bool {
        self.latest.lock().unwrap().insert(uuid, s.clone());
        true
    }
    fn has_user_data(&self, uuid: u128) -> bool {
        self.latest.lock().unwrap().contains_key(&uuid)
    }
    fn clear_user_data(&self, uuid: u128) {
        self.latest.lock().unwrap().remove(&uuid);
    }
    fn set_checked_out(&self, uuid: u128, out: bool) {
        let mut c = self.checkout.lock().unwrap();
        if out {
            c.insert(uuid, HERE.into());
        } else {
            c.remove(&uuid);
        }
    }
    fn checked_out(&self, uuid: u128) -> Option<String> {
        self.checkout.lock().unwrap().get(&uuid).cloned()
    }
    fn clear_checkouts_here(&self) {
        self.checkout.lock().unwrap().retain(|_, s| s != HERE);
    }
    fn set_server_switch(&self, uuid: u128) {
        self.switch.lock().unwrap().insert(uuid, ());
    }
    fn take_server_switch(&self, uuid: u128) -> bool {
        self.switch.lock().unwrap().remove(&uuid).is_some()
    }
    fn publish(&self, kind: MessageType, m: &Message) {
        self.published.lock().unwrap().push((kind, m.clone()));
    }
    fn listen(&self, stop: &dyn Fn() -> bool, _: &dyn Fn(MessageType, Message)) {
        while !stop() {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn setup(mode: &str) -> (Arc<Engine>, Arc<FakeDb>, Arc<FakeRedis>) {
    let mut c = Config::default();
    c.server_name = HERE.into();
    c.synchronization.mode = mode.into();
    c.synchronization.network_latency_milliseconds = 100;
    let db = Arc::new(FakeDb::default());
    let redis = Arc::new(FakeRedis::default());
    let dir = std::env::temp_dir().join("horizonsync-core-tests");
    let e = Engine::with(c, dir, Box::new(db.clone()), Box::new(redis.clone())).unwrap();
    (e, db, redis)
}

fn platform() -> Platform {
    Platform {
        minecraft_version: "26.3".into(),
        platform_type: "bukkit".into(),
        compress: true,
        server_name: "paper".into(),
    }
}

fn snapshot(cause: &str, game_mode: &str, age_secs: i64) -> Packed {
    let mut data = BTreeMap::new();
    data.insert(keys::GAME_MODE.into(), format!(r#"{{"game_mode":"{game_mode}"}}"#));
    data.insert(keys::ADVANCEMENTS.into(), "[]".into());
    let mut p = Packed::new(&platform(), cause, false, data);
    p.timestamp = util::format_timestamp(&(util::now() - chrono::Duration::seconds(age_secs)));
    p
}

/// Ticks the engine (as the game would) until `want` returns something, playing the game's part:
/// every Apply succeeds, every Capture gets `compound`.
fn run_until<T>(e: &Arc<Engine>, secs: u64, mut want: impl FnMut(&Action) -> Option<T>) -> Option<T> {
    let compound = parse_snbt(r#"{playerGameType:1,Health:12.0f,foodLevel:20,XpLevel:3,XpTotal:30,XpP:0.1f}"#).unwrap();
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        for a in e.tick(&[]) {
            let found = want(&a);
            match &a {
                Action::Apply { token, .. } => e.applied(*token, true),
                Action::Capture { token, .. } => e.captured(*token, Some(compound.clone())),
                _ => {}
            }
            if found.is_some() {
                return found;
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

fn applied_mode(a: &Action) -> Option<i32> {
    match a {
        Action::Apply { plan, .. } => plan.game_mode,
        _ => None,
    }
}

const P: u128 = 0x069a79f4_44e9_4726_a5be_fca90e38aaf5;

#[test]
fn new_user_is_registered_and_unlocked() {
    let (e, db, redis) = setup("LOCKSTEP");
    e.on_join(P, "Alex");
    assert!(e.is_locked(P));
    let msg = run_until(&e, 3, |a| match a {
        Action::Message { text, overlay: true, .. } => Some(text.clone()),
        _ => None,
    });
    assert_eq!(msg.as_deref(), Some("⭐ User registration complete!"));
    assert!(!e.is_locked(P));
    assert_eq!(redis.checked_out(P).as_deref(), Some(HERE));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(db.users.lock().unwrap().get(&P).map(String::as_str), Some("Alex"));
}

#[test]
fn lockstep_switch_takes_the_redis_snapshot_then_saves_on_leave() {
    let (e, db, redis) = setup("LOCKSTEP");
    redis.latest.lock().unwrap().insert(P, snapshot("DISCONNECT", "ADVENTURE", 0));
    e.on_join(P, "Alex");
    assert_eq!(run_until(&e, 3, applied_mode), Some(2));
    run_until(&e, 1, |_| None::<()>);
    assert!(!e.is_locked(P));
    assert!(redis.latest.lock().unwrap().get(&P).is_none(), "the Redis snapshot is consumed");

    let compound = parse_snbt("{playerGameType:3,Health:5.0f}").unwrap();
    e.on_leave(P, &compound);
    let deadline = Instant::now() + Duration::from_secs(3);
    while redis.checked_out(P).is_some() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(redis.checked_out(P).is_none(), "the checkout is released");
    let rows = db.sorted(P);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].save_cause, "DISCONNECT");
    assert!(rows[0].data[keys::GAME_MODE].contains("SPECTATOR"));
    assert_eq!(rows[0].data[keys::ADVANCEMENTS], "[]", "uncaptured types are carried over");
    assert!(redis.latest.lock().unwrap().contains_key(&P), "the next server gets it from Redis");
}

#[test]
fn checked_out_elsewhere_waits_then_falls_back_to_the_database() {
    let (e, db, redis) = setup("LOCKSTEP");
    redis.checkout.lock().unwrap().insert(P, "paper".into());
    db.add_snapshot(P, &snapshot("WORLD_SAVE", "CREATIVE", 30)).unwrap();
    let start = Instant::now();
    e.on_join(P, "Alex");
    assert_eq!(run_until(&e, 10, applied_mode), Some(1));
    let waited = start.elapsed();
    assert!(waited >= Duration::from_secs(4), "polled about 100 ticks first ({waited:?})");
}

#[test]
fn a_stale_redis_snapshot_loses_to_a_newer_shutdown_snapshot() {
    let (e, db, redis) = setup("LOCKSTEP");
    redis.latest.lock().unwrap().insert(P, snapshot("WORLD_SAVE", "ADVENTURE", 120));
    db.add_snapshot(P, &snapshot("SERVER_SHUTDOWN", "CREATIVE", 60)).unwrap();
    e.on_join(P, "Alex");
    assert_eq!(run_until(&e, 3, applied_mode), Some(1));
}

#[test]
fn leaving_while_locked_saves_nothing() {
    let (e, db, redis) = setup("LOCKSTEP");
    redis.checkout.lock().unwrap().insert(P, "paper".into());
    e.on_join(P, "Alex");
    e.on_leave(P, &parse_snbt("{}").unwrap());
    std::thread::sleep(Duration::from_millis(300));
    assert!(db.sorted(P).is_empty());
}

#[test]
fn delay_mode_reads_the_database_without_a_switch_key() {
    let (e, db, _) = setup("DELAY");
    db.add_snapshot(P, &snapshot("DISCONNECT", "ADVENTURE", 10)).unwrap();
    e.on_join(P, "Alex");
    assert_eq!(run_until(&e, 3, applied_mode), Some(2));
}

#[test]
fn delay_mode_waits_for_the_redis_snapshot_on_a_switch() {
    let (e, db, redis) = setup("DELAY");
    db.add_snapshot(P, &snapshot("DISCONNECT", "SURVIVAL", 10)).unwrap();
    redis.set_server_switch(P);
    let r2 = redis.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        r2.set_user_data(P, &snapshot("DISCONNECT", "SPECTATOR", 0));
    });
    e.on_join(P, "Alex");
    assert_eq!(run_until(&e, 5, applied_mode), Some(3));
}

#[test]
fn a_snapshot_from_another_platform_fails_and_stays_locked() {
    let (e, db, _) = setup("LOCKSTEP");
    let mut s = snapshot("DISCONNECT", "SURVIVAL", 10);
    s.platform_type = "fabric".into();
    db.add_snapshot(P, &s).unwrap();
    e.on_join(P, "Alex");
    let msg = run_until(&e, 3, |a| match a {
        Action::Message { text, overlay: false, .. } => Some(text.clone()),
        _ => None,
    });
    assert!(msg.unwrap().contains("Failed to synchronize"));
    assert!(e.is_locked(P));
}

#[test]
fn locked_players_cannot_run_commands_and_world_save_skips_them() {
    let (e, db, redis) = setup("LOCKSTEP");
    redis.checkout.lock().unwrap().insert(P, "paper".into());
    e.on_join(P, "Alex");
    assert!(e.refuse_command(P, "spawn"));
    e.on_world_save(vec![(P, parse_snbt("{}").unwrap())]);
    std::thread::sleep(Duration::from_millis(200));
    assert!(db.sorted(P).is_empty());
}

#[test]
fn server_stop_saves_everyone_unlocked_and_releases_checkouts() {
    let (e, db, redis) = setup("LOCKSTEP");
    e.on_join(P, "Alex");
    run_until(&e, 3, |a| matches!(a, Action::Message { .. }).then_some(()));
    e.on_server_stopping(vec![(P, parse_snbt("{playerGameType:0}").unwrap())]);
    let rows = db.sorted(P);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].save_cause, "SERVER_SHUTDOWN");
    assert!(redis.checked_out(P).is_none());
    // Players saved by the server afterwards are not saved again.
    e.on_leave(P, &parse_snbt("{}").unwrap());
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(db.sorted(P).len(), 1);
}

#[test]
fn locked_launch_rule_follows_husksync() {
    use crate::engine::launch_refused;
    assert!(!launch_refused(false, "minecraft:snowball", false));
    assert!(launch_refused(true, "minecraft:snowball", true));
    assert!(!launch_refused(true, "minecraft:ender_pearl", true));
    assert!(launch_refused(true, "minecraft:ender_pearl", false));
}
