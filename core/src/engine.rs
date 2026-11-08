//! The synchronisation itself: `EventListener`, `DataSyncer` (LOCKSTEP and DELAY),
//! `RedisManager.onMessage` and the lock that keeps a player still until their data arrived.
//!
//! The engine never touches the game. The game calls in on its thread (`on_join`, `on_leave`,
//! `tick`...), the engine does the database and Redis work on worker threads, and hands back
//! [`Action`]s for the game's thread: apply this plan, capture that player, show this line.

use crate::config::Config;
use crate::data::{self, EnderChest, Inventory, StoredItem};
use crate::snapshot::keys;
use crate::db::{usable, Db, Stored};
use crate::nbt::Nbt;
use crate::player::{self, ApplyPlan};
use crate::redis_mgr::{Message, MessageType, Redis};
use crate::snapshot::{Packed, Platform};
use crate::util;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

/// The database calls the engine and the commands make (`Database`); [`Db`] on MariaDB/MySQL.
pub trait Store: Send + Sync {
    fn ensure_user(&self, uuid: u128, name: &str) -> Result<(), String>;
    fn user_by_name(&self, name: &str) -> Result<Option<(u128, String)>, String>;
    fn user_by_uuid(&self, uuid: u128) -> Result<Option<String>, String>;
    fn latest_snapshot(&self, player: u128) -> Result<Option<Stored>, String>;
    fn latest_snapshot_with_causes(&self, player: u128, causes: &[&str]) -> Result<Option<Stored>, String>;
    fn all_snapshots(&self, player: u128) -> Result<Vec<Stored>, String>;
    fn snapshot(&self, player: u128, version: u128) -> Result<Option<Stored>, String>;
    fn add_snapshot(&self, player: u128, s: &Packed) -> Result<(), String>;
    fn add_snapshot_without_rotation(&self, player: u128, s: &Packed) -> Result<(), String>;
    fn delete_snapshot(&self, player: u128, version: u128) -> Result<bool, String>;
    fn set_pinned(&self, player: u128, version: u128, pinned: bool) -> Result<bool, String>;
    fn ping(&self) -> Result<Duration, String>;
}

/// The Redis calls (`RedisManager`); [`Redis`] on a real server.
pub trait Cache: Send + Sync {
    fn ping(&self) -> Result<Duration, String>;
    fn version(&self) -> String;
    fn take_user_data(&self, uuid: u128) -> Result<Option<Result<Packed, String>>, String>;
    fn set_user_data(&self, uuid: u128, s: &Packed) -> bool;
    fn has_user_data(&self, uuid: u128) -> bool;
    fn clear_user_data(&self, uuid: u128);
    fn set_checked_out(&self, uuid: u128, checked_out: bool);
    fn checked_out(&self, uuid: u128) -> Option<String>;
    fn clear_checkouts_here(&self);
    fn set_server_switch(&self, uuid: u128);
    fn take_server_switch(&self, uuid: u128) -> bool;
    fn publish(&self, kind: MessageType, m: &Message);
    fn listen(&self, stop: &dyn Fn() -> bool, on_message: &dyn Fn(MessageType, Message));
}

macro_rules! forward {
    ($t:ty => $tr:ident { $(fn $name:ident(&self $(, $arg:ident: $aty:ty)*) -> $ret:ty;)* }) => {
        impl $tr for $t {
            $(fn $name(&self $(, $arg: $aty)*) -> $ret { <$t>::$name(self $(, $arg)*) })*
        }
    };
}

forward!(Db => Store {
    fn ensure_user(&self, uuid: u128, name: &str) -> Result<(), String>;
    fn user_by_name(&self, name: &str) -> Result<Option<(u128, String)>, String>;
    fn user_by_uuid(&self, uuid: u128) -> Result<Option<String>, String>;
    fn latest_snapshot(&self, player: u128) -> Result<Option<Stored>, String>;
    fn latest_snapshot_with_causes(&self, player: u128, causes: &[&str]) -> Result<Option<Stored>, String>;
    fn all_snapshots(&self, player: u128) -> Result<Vec<Stored>, String>;
    fn snapshot(&self, player: u128, version: u128) -> Result<Option<Stored>, String>;
    fn add_snapshot(&self, player: u128, s: &Packed) -> Result<(), String>;
    fn add_snapshot_without_rotation(&self, player: u128, s: &Packed) -> Result<(), String>;
    fn delete_snapshot(&self, player: u128, version: u128) -> Result<bool, String>;
    fn set_pinned(&self, player: u128, version: u128, pinned: bool) -> Result<bool, String>;
    fn ping(&self) -> Result<Duration, String>;
});

forward!(Redis => Cache {
    fn ping(&self) -> Result<Duration, String>;
    fn version(&self) -> String;
    fn take_user_data(&self, uuid: u128) -> Result<Option<Result<Packed, String>>, String>;
    fn set_user_data(&self, uuid: u128, s: &Packed) -> bool;
    fn has_user_data(&self, uuid: u128) -> bool;
    fn clear_user_data(&self, uuid: u128) -> ();
    fn set_checked_out(&self, uuid: u128, checked_out: bool) -> ();
    fn checked_out(&self, uuid: u128) -> Option<String>;
    fn clear_checkouts_here(&self) -> ();
    fn set_server_switch(&self, uuid: u128) -> ();
    fn take_server_switch(&self, uuid: u128) -> bool;
    fn publish(&self, kind: MessageType, m: &Message) -> ();
    fn listen(&self, stop: &dyn Fn() -> bool, on_message: &dyn Fn(MessageType, Message)) -> ();
});

/// `DataSyncer`'s constants.
const REDIS_SNAPSHOT_STALE_THRESHOLD_SECONDS: i64 = 5;
const REDIS_SNAPSHOT_MAX_POLL_TICK_DELAY: u64 = 10;
const REDIS_SNAPSHOT_MAX_POLL_TICKS: u64 = 20 * REDIS_SNAPSHOT_STALE_THRESHOLD_SECONDS as u64;
const SHUTDOWN_CRITICAL_CAUSES: [&str; 2] = ["SERVER_SHUTDOWN", "DISCONNECT"];
const SHUTDOWN_CRITICAL_DB_ATTEMPTS: u32 = 3;
const SHUTDOWN_CRITICAL_DB_RETRY_BACKOFF: Duration = Duration::from_millis(250);
const TICK: Duration = Duration::from_millis(50);

/// `DataSnapshot.UpdateCause`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateCause {
    Synchronized,
    NewUser,
    Updated,
}

impl UpdateCause {
    fn completed(self) -> &'static str {
        match self {
            UpdateCause::Synchronized => "⏵ Data synchronized!",
            UpdateCause::NewUser => "⭐ User registration complete!",
            UpdateCause::Updated => "🔔 Your data has been updated!",
        }
    }

    fn failed(self) -> Option<&'static str> {
        match self {
            UpdateCause::Synchronized => {
                Some("⏵ Failed to synchronize your data! Please contact an administrator.")
            }
            UpdateCause::NewUser => None,
            UpdateCause::Updated => Some("🔔 Failed to update your data! Please contact an administrator."),
        }
    }
}

/// What the game's thread must do.
#[derive(Clone, Debug)]
pub enum Action {
    /// Give the player this data, then call [`Engine::applied`] with `token`.
    Apply { player: u128, plan: Box<ApplyPlan>, token: u64 },
    /// Read the player's current compound and call [`Engine::captured`] with `token`.
    Capture { player: u128, token: u64 },
    /// A chat line (or action bar line) for a player, or the console when `player` is None.
    Message { player: Option<u128>, text: String, overlay: bool },
    /// A chat line with click and hover events: `json` is a text component (1.21.5+ field names,
    /// `click_event` / `hover_event`), `text` the same line as plain words.
    Component { player: Option<u128>, text: String, json: serde_json::Value },
    /// Show items in a chest menu (`/inventory`, `/enderchest`); with `edit` the viewer may take
    /// and put items, and the game calls [`Engine::on_menu_edited`] when the menu closes.
    OpenMenu { viewer: u128, title: String, rows: u8, items: Vec<Option<StoredItem>>, edit: Option<MenuEdit> },
}

/// What an editable `/inventory` or `/enderchest` menu shows.
#[derive(Clone, Debug)]
pub struct MenuEdit {
    pub viewer: u128,
    pub target: u128,
    pub target_name: String,
    pub ender_chest: bool,
}

struct Session {
    name: String,
    /// The data of the snapshot last applied: the types this port can't capture are carried
    /// from it into the player's next snapshot.
    carried: Option<BTreeMap<String, String>>,
    /// Bumped on each join, so work for an earlier visit is dropped.
    visit: u64,
}

enum Pending {
    Apply { player: u128, visit: u64, cause: UpdateCause, data: BTreeMap<String, String> },
    Capture(mpsc::Sender<Option<Nbt>>),
}

#[derive(Default)]
struct State {
    sessions: HashMap<u128, Session>,
    locked: HashSet<u128>,
    disconnecting: HashSet<u128>,
    locked_entities: HashSet<i32>,
    pending: HashMap<u64, Pending>,
    /// `RedisManager.pendingRequests`: cross-server snapshot requests by request id.
    requests: HashMap<u128, mpsc::Sender<Option<Packed>>>,
}

pub struct Engine {
    /// `config/horizonsync/`: the config file and `/userdata dump` files.
    pub(crate) dir: std::path::PathBuf,
    pub(crate) config: RwLock<Config>,
    pub(crate) platform: Platform,
    pub(crate) db: Box<dyn Store>,
    pub(crate) redis: Box<dyn Cache>,
    state: Mutex<State>,
    actions: Mutex<VecDeque<Action>>,
    tokens: AtomicU64,
    disabling: AtomicBool,
    saves: (Mutex<usize>, Condvar),
}

/// Who runs a command.
#[derive(Clone, Debug)]
pub struct Sender {
    /// May edit the items in `/inventory` and `/enderchest` (`horizonsync.command.*.edit`).
    pub edit: bool,
    /// None: the console.
    pub uuid: Option<u128>,
    pub name: String,
    pub level: u8,
}

impl Engine {
    pub fn new(config: Config, dir: std::path::PathBuf) -> Result<Arc<Engine>, String> {
        let db = Db::connect(&config)?;
        let redis = Redis::connect(&config, &config.server_name)?;
        Self::with(config, dir, Box::new(db), Box::new(redis))
    }

    /// An engine over any store and cache (the tests use in-memory ones).
    pub fn with(
        config: Config,
        dir: std::path::PathBuf,
        db: Box<dyn Store>,
        redis: Box<dyn Cache>,
    ) -> Result<Arc<Engine>, String> {
        let platform = Platform {
            minecraft_version: config.minecraft_version.clone(),
            platform_type: config.platform_type.clone(),
            compress: config.synchronization.compress_data,
            server_name: config.server_name.clone(),
        };
        let engine = Arc::new(Engine {
            dir,
            config: RwLock::new(config),
            platform,
            db,
            redis,
            state: Mutex::new(State::default()),
            actions: Mutex::new(VecDeque::new()),
            tokens: AtomicU64::new(1),
            disabling: AtomicBool::new(false),
            saves: (Mutex::new(0), Condvar::new()),
        });
        // LockstepDataSyncer.initialize.
        if engine.cfg().synchronization.lockstep() {
            engine.redis.clear_checkouts_here();
        }
        let listener = engine.clone();
        std::thread::Builder::new()
            .name("horizonsync-redis".into())
            .spawn(move || {
                let stop = || listener.disabling.load(Ordering::Relaxed);
                listener.redis.listen(&stop, &|kind, m| listener.on_redis_message(kind, m));
            })
            .map_err(|e| e.to_string())?;
        Ok(engine)
    }

    pub(crate) fn cfg(&self) -> std::sync::RwLockReadGuard<'_, Config> {
        self.config.read().unwrap_or_else(|e| e.into_inner())
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn token(&self) -> u64 {
        self.tokens.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn push(&self, a: Action) {
        self.actions.lock().unwrap_or_else(|e| e.into_inner()).push_back(a);
    }

    /// A line with click and hover events (`plain` is what a log or a console shows).
    pub(crate) fn tell_component(&self, player: Option<u128>, plain: impl Into<String>, json: serde_json::Value) {
        self.push(Action::Component { player, text: plain.into(), json });
    }

    pub(crate) fn tell(&self, player: Option<u128>, text: impl Into<String>) {
        self.push(Action::Message { player, text: text.into(), overlay: false });
    }

    fn debug(&self, msg: impl FnOnce() -> String) {
        if self.cfg().debug_logging {
            log::info!("horizonsync [debug]: {}", msg());
        }
    }

    /// A worker thread for one job; tracked when it is a save, so shutdown can wait for it.
    fn spawn(self: &Arc<Self>, save: bool, job: impl FnOnce(Arc<Engine>) + Send + 'static) {
        if save {
            *self.saves.0.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        }
        let me = self.clone();
        let r = std::thread::Builder::new().name("horizonsync-worker".into()).spawn(move || {
            let me2 = me.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(me2)));
            if result.is_err() {
                log::error!("horizonsync: a worker job panicked");
            }
            if save {
                let mut n = me.saves.0.lock().unwrap_or_else(|e| e.into_inner());
                *n = n.saturating_sub(1);
                me.saves.1.notify_all();
            }
        });
        if r.is_err() && save {
            let mut n = self.saves.0.lock().unwrap_or_else(|e| e.into_inner());
            *n = n.saturating_sub(1);
        }
    }

    // ----- what the game calls -----

    pub fn is_locked(&self, player: u128) -> bool {
        self.disabling.load(Ordering::Relaxed) || self.st().locked.contains(&player)
    }

    pub fn is_locked_entity(&self, entity: i32) -> bool {
        self.st().locked_entities.contains(&entity)
    }

    /// `LockedHandler.onCommandProcess`: the reply when a locked player's command is refused.
    pub fn refuse_command(&self, player: u128, line: &str) -> bool {
        self.is_locked(player) && self.cfg().synchronization.command_blocked_while_locked(line)
    }

    /// `LockedHandler.onProjectileLaunch`: a locked player may not throw or shoot, except an
    /// ender pearl when `allow_pearl_spawning_while_locked` is on.
    pub fn refuse_launch(&self, player: u128, item: &str) -> bool {
        launch_refused(self.is_locked(player), item, self.cfg().synchronization.allow_pearl_spawning_while_locked)
    }

    fn lock(&self, player: u128) {
        self.st().locked.insert(player);
    }

    fn unlock(&self, player: u128) {
        self.st().locked.remove(&player);
    }

    /// `EventListener.handlePlayerJoin`.
    pub fn on_join(self: &Arc<Self>, player: u128, name: &str) {
        let visit = self.token();
        {
            let mut st = self.st();
            st.disconnecting.remove(&player);
            st.locked.insert(player);
            st.sessions.insert(player, Session { name: name.to_owned(), carried: None, visit });
        }
        let name = name.to_owned();
        self.spawn(false, move |me| me.sync_apply(player, &name, visit));
    }

    fn still_here(&self, player: u128, visit: u64) -> bool {
        let st = self.st();
        !st.disconnecting.contains(&player) && st.sessions.get(&player).is_some_and(|s| s.visit == visit)
    }

    /// `DataSyncer.syncApplyUserData` for both modes, on a worker thread.
    fn sync_apply(self: &Arc<Self>, player: u128, name: &str, visit: u64) {
        let lockstep = self.cfg().synchronization.lockstep();
        if !lockstep {
            // DelayDataSyncer: wait the network latency, then the database unless switching.
            let latency = self.cfg().synchronization.network_latency_milliseconds;
            std::thread::sleep(Duration::from_millis(latency / 50 * 50));
            if !self.still_here(player, visit) {
                return;
            }
            if !self.redis.take_server_switch(player) {
                self.set_from_database(player, visit);
                return;
            }
        }
        // listenForRedisData / pollForRedisData: a growing Fibonacci tick delay up to 10,
        // falling back to the database after 100 ticks.
        let (mut elapsed, mut previous, mut current, mut attempt) = (0u64, 0u64, 0u64, 1u64);
        loop {
            std::thread::sleep(TICK * current as u32);
            if !self.still_here(player, visit) {
                return;
            }
            let so_far = elapsed + current;
            if self.disabling.load(Ordering::Relaxed) || so_far > REDIS_SNAPSHOT_MAX_POLL_TICKS {
                self.debug(|| format!("[{name}] Redis timed out after {attempt} attempts; setting from database"));
                self.set_from_database(player, visit);
                return;
            }
            let done = if lockstep {
                self.lockstep_attempt(player, name, visit)
            } else {
                match self.redis.take_user_data(player) {
                    Ok(Some(data)) => {
                        self.apply_latest(player, visit, data);
                        true
                    }
                    _ => false,
                }
            };
            if done {
                return;
            }
            let next = if current == 0 { 1 } else { REDIS_SNAPSHOT_MAX_POLL_TICK_DELAY.min(previous + current) };
            elapsed = so_far;
            previous = current;
            current = next;
            attempt += 1;
        }
    }

    /// `LockstepDataSyncer.syncApplyUserData`'s completion supplier.
    fn lockstep_attempt(self: &Arc<Self>, player: u128, name: &str, visit: u64) -> bool {
        if let Some(server) = self.redis.checked_out(player) {
            if server != self.platform.server_name {
                if self.cfg().synchronization.checkin_petitions {
                    let payload = format!("{}/{}", util::uuid_string(player), name).into_bytes();
                    self.redis.publish(MessageType::CheckInPetition, &Message::to_server(&server, payload));
                }
                return false;
            }
        }
        self.redis.set_checked_out(player, true);
        match self.redis.take_user_data(player) {
            Ok(Some(data)) => {
                self.debug(|| format!("[{name}] Applying data from Redis cache"));
                self.apply_latest(player, visit, data);
            }
            _ => {
                self.debug(|| format!("[{name}] no Redis data; loading from database"));
                self.set_from_database(player, visit);
            }
        }
        true
    }

    /// `DataSyncer.setUserFromDatabase`.
    fn set_from_database(self: &Arc<Self>, player: u128, visit: u64) {
        match self.db.latest_snapshot(player) {
            Ok(Some(stored)) => self.apply_snapshot(player, visit, stored.snapshot, UpdateCause::Synchronized),
            Ok(None) => self.complete_sync(player, true, UpdateCause::NewUser),
            Err(e) => {
                log::warn!("horizonsync: failed to set {}'s data from the database: {e}", util::uuid_string(player));
                self.complete_sync(player, false, UpdateCause::Synchronized);
            }
        }
    }

    /// `DataSyncer.applyLatestSnapshot`: a Redis snapshot older than 5 s loses to a newer
    /// shutdown-critical database snapshot.
    fn apply_latest(self: &Arc<Self>, player: u128, visit: u64, redis_data: Result<Packed, String>) {
        if let Ok(r) = &redis_data {
            let fresh = r
                .time()
                .is_some_and(|t| (util::now() - t).num_seconds() < REDIS_SNAPSHOT_STALE_THRESHOLD_SECONDS);
            if !fresh {
                if let Ok(Some(db)) = self.db.latest_snapshot_with_causes(player, &SHUTDOWN_CRITICAL_CAUSES) {
                    if let (Ok(d), Some(rt)) = (&db.snapshot, r.time()) {
                        if d.time().is_some_and(|dt| dt > rt) {
                            self.apply_snapshot(player, visit, db.snapshot, UpdateCause::Synchronized);
                            return;
                        }
                    }
                }
            }
        }
        self.apply_snapshot(player, visit, redis_data, UpdateCause::Synchronized);
    }

    /// `OnlineUser.applySnapshot` up to the game's thread: unpack and check, then queue.
    fn apply_snapshot(self: &Arc<Self>, player: u128, visit: u64, s: Result<Packed, String>, cause: UpdateCause) {
        let s = match usable(s, &self.platform) {
            Ok(s) => s,
            Err(e) => {
                log::error!("horizonsync: failed to unpack a data snapshot for {}: {e}", util::uuid_string(player));
                self.complete_sync(player, false, cause);
                return;
            }
        };
        let plan = player::plan(&s.data, &s.minecraft_version, &self.cfg());
        for (key, e) in &plan.skipped {
            log::warn!(
                "horizonsync: failed to deserialize {key} data for snapshot {}; skipping it: {e}",
                s.id
            );
        }
        let token = self.token();
        self.st().pending.insert(token, Pending::Apply { player, visit, cause, data: s.data });
        self.push(Action::Apply { player, plan: Box::new(plan), token });
    }

    /// The game applied (or could not apply) a plan.
    pub fn applied(self: &Arc<Self>, token: u64, ok: bool) {
        let Some(Pending::Apply { player, visit, cause, data }) = self.st().pending.remove(&token) else {
            return;
        };
        if let Some(s) = self.st().sessions.get_mut(&player).filter(|s| s.visit == visit) {
            if ok {
                s.carried = Some(data);
            }
        }
        self.complete_sync(player, ok, cause);
    }

    /// `OnlineUser.completeSync`: the notice, the unlock on success, and `ensureUser`.
    fn complete_sync(self: &Arc<Self>, player: u128, ok: bool, cause: UpdateCause) {
        let slot = self.cfg().synchronization.notification_display_slot.to_ascii_uppercase();
        if ok {
            self.unlock(player);
            match slot.as_str() {
                "CHAT" => self.tell(Some(player), cause.completed()),
                "ACTION_BAR" | "TOAST" => {
                    self.push(Action::Message { player: Some(player), text: cause.completed().into(), overlay: true })
                }
                _ => {}
            }
        } else if let Some(text) = cause.failed() {
            self.tell(Some(player), text);
        }
        let name = self.st().sessions.get(&player).map(|s| s.name.clone());
        if let Some(name) = name {
            self.spawn(false, move |me| {
                if let Err(e) = me.db.ensure_user(player, &name) {
                    log::warn!("horizonsync: {e}");
                }
            });
        }
    }

    /// The game read a player for a capture (None: they were not there).
    pub fn captured(&self, token: u64, compound: Option<Nbt>) {
        if let Some(Pending::Capture(tx)) = self.st().pending.remove(&token) {
            let _ = tx.send(compound);
        }
    }

    /// `UserDataHolder.createSnapshot` from a compound the game gave.
    fn snapshot_of(&self, player: u128, compound: &Nbt, cause: &str) -> Packed {
        let cfg = self.cfg();
        let captured = player::capture(compound, &cfg, None);
        let carried = self.st().sessions.get(&player).and_then(|s| s.carried.clone());
        let data = player::merge_with_carried(captured, carried.as_ref(), &cfg);
        Packed::new(&self.platform, cause, cfg.synchronization.auto_pin(cause), data)
    }

    /// Asks the game's thread for the player's compound and waits for it (worker threads only).
    fn capture_blocking(&self, player: u128, cause: &str) -> Option<Packed> {
        let (tx, rx) = mpsc::channel();
        let token = self.token();
        self.st().pending.insert(token, Pending::Capture(tx));
        self.push(Action::Capture { player, token });
        let compound = rx.recv_timeout(Duration::from_secs(5)).ok().flatten();
        if compound.is_none() {
            self.st().pending.remove(&token);
        }
        compound.map(|c| self.snapshot_of(player, &c, cause))
    }

    /// `EventListener.handlePlayerQuit` + `saveOnPlayerQuit`, with the player's compound as the
    /// server saved it on leaving.
    pub fn on_leave(self: &Arc<Self>, player: u128, compound: &Nbt) {
        if self.disabling.load(Ordering::Relaxed) {
            return;
        }
        let (was_locked, name) = {
            let mut st = self.st();
            st.disconnecting.insert(player);
            let was = !st.locked.insert(player);
            (was, st.sessions.get(&player).map(|s| s.name.clone()).unwrap_or_default())
        };
        if was_locked {
            self.debug(|| format!("[{name}] disconnected while locked - data will NOT be saved!"));
            self.forget(player);
            return;
        }
        let snapshot = self.snapshot_of(player, compound, "DISCONNECT");
        self.forget(player);
        let lockstep = self.cfg().synchronization.lockstep();
        if !lockstep {
            self.spawn(true, move |me| me.redis.set_server_switch(player));
        }
        self.spawn(true, move |me| {
            me.persist(player, &snapshot);
            if !me.redis.set_user_data(player, &snapshot) {
                me.redis.clear_user_data(player);
            }
            if lockstep {
                me.redis.set_checked_out(player, false);
            }
            me.unlock(player);
        });
    }

    fn forget(&self, player: u128) {
        let mut st = self.st();
        st.sessions.remove(&player);
        st.pending.retain(|_, p| !matches!(p, Pending::Apply { player: q, .. } if *q == player));
    }

    /// `DataSyncer.persistSnapshot`: during shutdown a DISCONNECT write is verified and retried.
    fn persist(&self, player: u128, s: &Packed) {
        let verify = self.disabling.load(Ordering::Relaxed) && s.save_cause == "DISCONNECT";
        if !verify {
            if let Err(e) = self.db.add_snapshot(player, s) {
                log::error!("horizonsync: failed to set user data in the database: {e}");
            }
            return;
        }
        let version = s.id_u128().unwrap_or(0);
        for attempt in 1..=SHUTDOWN_CRITICAL_DB_ATTEMPTS {
            let already = attempt > 1 && matches!(self.db.snapshot(player, version), Ok(Some(_)));
            if !already {
                let r = if attempt == 1 {
                    self.db.add_snapshot(player, s)
                } else {
                    self.db.add_snapshot_without_rotation(player, s)
                };
                if let Err(e) = r {
                    log::warn!("horizonsync: {e}");
                }
            }
            if already || matches!(self.db.snapshot(player, version), Ok(Some(_))) {
                return;
            }
            if attempt < SHUTDOWN_CRITICAL_DB_ATTEMPTS {
                std::thread::sleep(SHUTDOWN_CRITICAL_DB_RETRY_BACKOFF);
            }
        }
        log::error!("horizonsync: could not confirm {}'s data reached the database", util::uuid_string(player));
    }

    /// `DataSyncer.saveCurrentUserData`'s save part, for a snapshot already taken.
    fn save_current(self: &Arc<Self>, player: u128, snapshot: Packed, reply: Option<(Option<u128>, String)>) {
        self.spawn(true, move |me| {
            me.persist(player, &snapshot);
            let critical = SHUTDOWN_CRITICAL_CAUSES.contains(&snapshot.save_cause.as_str());
            if !me.redis.set_user_data(player, &snapshot) && critical {
                me.redis.clear_user_data(player);
            }
            if let Some((to, text)) = reply {
                me.tell(to, text);
            }
        });
    }

    /// `EventListener.saveOnWorldSave`: every player here, unlocked, with their compound.
    pub fn on_world_save(self: &Arc<Self>, players: Vec<(u128, Nbt)>) {
        if self.disabling.load(Ordering::Relaxed) || !self.cfg().synchronization.save_on_world_save {
            return;
        }
        for (player, compound) in players {
            let skip = {
                let st = self.st();
                st.locked.contains(&player) || st.disconnecting.contains(&player) || !st.sessions.contains_key(&player)
            };
            if skip {
                continue;
            }
            let s = self.snapshot_of(player, &compound, "WORLD_SAVE");
            self.save_current(player, s, None);
        }
    }

    /// `EventListener.handlePluginDisable`: SERVER_SHUTDOWN saves for everyone still unlocked,
    /// then wait (bounded) for every save in flight; then the checkouts of this server go.
    pub fn on_server_stopping(self: &Arc<Self>, players: Vec<(u128, Nbt)>) {
        let mut todo = Vec::new();
        {
            let mut st = self.st();
            for (player, compound) in players {
                if st.locked.insert(player) {
                    todo.push((player, compound));
                }
            }
        }
        for (player, compound) in todo {
            let s = self.snapshot_of(player, &compound, "SERVER_SHUTDOWN");
            self.save_current(player, s, None);
        }
        self.disabling.store(true, Ordering::Relaxed);
        let timeout = Duration::from_millis(
            self.cfg().synchronization.shutdown_save_timeout_milliseconds.clamp(5000, 50000),
        );
        let deadline = Instant::now() + timeout;
        let mut n = self.saves.0.lock().unwrap_or_else(|e| e.into_inner());
        while *n > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                log::warn!("horizonsync: timed out waiting for {} player saves; some data may not be saved", *n);
                break;
            }
            n = self.saves.1.wait_timeout(n, left).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0);
        }
        drop(n);
        if self.cfg().synchronization.lockstep() {
            self.redis.clear_checkouts_here();
        }
    }

    /// Called every tick on the game's thread: who is on (for the damage lock) and the actions.
    pub fn tick(&self, players: &[(u128, i32)]) -> Vec<Action> {
        {
            let mut st = self.st();
            let locked: HashSet<i32> = players
                .iter()
                .filter(|(u, _)| st.locked.contains(u))
                .map(|(_, e)| *e)
                .collect();
            st.locked_entities = locked;
        }
        self.actions.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect()
    }

    pub fn online_name(&self, player: u128) -> Option<String> {
        self.st().sessions.get(&player).map(|s| s.name.clone())
    }

    pub fn online_by_name(&self, name: &str) -> Option<(u128, String)> {
        self.st()
            .sessions
            .iter()
            .find(|(_, s)| s.name.eq_ignore_ascii_case(name))
            .map(|(u, s)| (*u, s.name.clone()))
    }

    // ----- Redis messages (on the listener thread) -----

    fn on_redis_message(self: &Arc<Self>, kind: MessageType, m: Message) {
        match kind {
            MessageType::UpdateUserData => {
                let Some(player) = m.target() else { return };
                let visit = match self.st().sessions.get(&player) {
                    Some(s) => s.visit,
                    None => return,
                };
                self.lock(player);
                let s = Packed::from_bytes(&m.payload, self.platform.compress);
                self.apply_snapshot(player, visit, s, UpdateCause::Updated);
            }
            MessageType::RequestUserData => {
                let Some(player) = m.target() else { return };
                if self.online_name(player).is_none() {
                    return;
                }
                let Some(request) = util::parse_uuid(&String::from_utf8_lossy(&m.payload)) else { return };
                self.spawn(false, move |me| {
                    if let Some(s) = me.capture_blocking(player, "INVENTORY_COMMAND") {
                        me.redis.publish(
                            MessageType::ReturnUserData,
                            &Message::to_player(request, s.to_bytes(me.platform.compress)),
                        );
                    }
                });
            }
            MessageType::ReturnUserData => {
                let Some(request) = m.target() else { return };
                if let Some(tx) = self.st().requests.remove(&request) {
                    let _ = tx.send(Packed::from_bytes(&m.payload, self.platform.compress).ok());
                }
            }
            MessageType::CheckInPetition => {
                if m.target_server.as_deref() != Some(self.platform.server_name.as_str())
                    || !self.cfg().synchronization.checkin_petitions
                {
                    return;
                }
                let payload = String::from_utf8_lossy(&m.payload).into_owned();
                let Some(player) = payload.split('/').next().and_then(util::parse_uuid) else { return };
                let busy = {
                    let st = self.st();
                    st.sessions.contains_key(&player) || st.locked.contains(&player)
                };
                if !busy {
                    self.redis.set_checked_out(player, false);
                }
            }
        }
    }

    /// `RedisManager.getOnlineUserData`: this server's player is read here, anyone else is
    /// asked for over Redis (answered within `network_latency_milliseconds`, else nothing).
    pub(crate) fn online_user_data(&self, player: u128, cause: &str) -> Option<Packed> {
        if self.online_name(player).is_some() {
            return self.capture_blocking(player, cause);
        }
        let request = util::random_uuid();
        let (tx, rx) = mpsc::channel();
        self.st().requests.insert(request, tx);
        self.redis.publish(
            MessageType::RequestUserData,
            &Message::to_player(player, util::uuid_string(request).into_bytes()),
        );
        let wait = Duration::from_millis(self.cfg().synchronization.network_latency_milliseconds.max(50));
        let got = rx.recv_timeout(wait).ok().flatten();
        self.st().requests.remove(&request);
        got
    }

    /// An editable `/inventory` or `/enderchest` menu closed with `items` in it
    /// (`InventoryCommand.updateItems`): a new snapshot, a copy of the player's latest with these
    /// items, saved and pushed to the player wherever they are. Nothing is saved when the items
    /// are the ones already stored.
    pub fn on_menu_edited(self: &Arc<Self>, edit: MenuEdit, items: Vec<Option<StoredItem>>) {
        self.run_job(move |me| {
            let to = Some(edit.viewer);
            let latest = match me.db.latest_snapshot(edit.target) {
                Ok(Some(s)) => s.snapshot,
                _ => return me.tell(to, "Error: Could not find any user data to display."),
            };
            let mut s = match crate::db::usable(latest, &me.platform) {
                Ok(s) => s.copy(),
                Err(e) => return me.tell(to, format!("Error: {e}")),
            };
            let key = if edit.ender_chest { keys::ENDER_CHEST } else { keys::INVENTORY };
            let fallback = util::data_version_of(&s.minecraft_version);
            let new = if edit.ender_chest {
                let mut slots = items;
                slots.resize(data::ENDER_CHEST_SLOTS, None);
                EnderChest { slots }.serialize()
            } else {
                let held = s
                    .data
                    .get(key)
                    .and_then(|v| Inventory::deserialize(v, fallback).ok())
                    .map_or(0, |i| i.held_item_slot);
                let mut slots = items;
                slots.resize(data::INVENTORY_SLOTS, None);
                Inventory { slots, held_item_slot: held }.serialize()
            };
            if s.data.get(key) == Some(&new) {
                return;
            }
            let cause = if edit.ender_chest { "ENDERCHEST_COMMAND" } else { "INVENTORY_COMMAND" };
            s.data.insert(key.to_owned(), new);
            s.save_cause = cause.into();
            s.pinned = me.cfg().synchronization.auto_pin(cause);
            me.save_and_update(edit.target, &s);
        });
    }

    /// `/userdata save`: a SAVE_COMMAND snapshot of a player on this server.
    pub(crate) fn save_command(self: &Arc<Self>, player: u128, name: &str, reply_to: Option<u128>) {
        match self.capture_blocking(player, "SAVE_COMMAND") {
            Some(s) => self.save_current(
                player,
                s,
                Some((reply_to, format!("Successfully saved a snapshot of {name}'s current user data."))),
            ),
            None => self.tell(reply_to, "Error: Could not find a player by that name."),
        }
    }

    /// Restore / edit flows: save a stored snapshot again and push it to the player wherever
    /// they are (`saveData` + `sendUserDataUpdate`).
    pub(crate) fn save_and_update(&self, player: u128, s: &Packed) {
        self.persist(player, s);
        if self.redis.has_user_data(player) {
            self.redis.set_user_data(player, s);
        }
        self.redis.publish(
            MessageType::UpdateUserData,
            &Message::to_player(player, s.to_bytes(self.platform.compress)),
        );
    }

    pub(crate) fn run_job(self: &Arc<Self>, job: impl FnOnce(Arc<Engine>) + Send + 'static) {
        self.spawn(false, job);
    }

    pub fn disabling(&self) -> bool {
        self.disabling.load(Ordering::Relaxed)
    }
}

/// The rule of `refuse_launch`, on its own for tests.
pub fn launch_refused(locked: bool, item: &str, allow_pearl: bool) -> bool {
    locked && !(allow_pearl && item == "minecraft:ender_pearl")
}
