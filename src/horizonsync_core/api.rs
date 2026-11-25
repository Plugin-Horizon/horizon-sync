//! HuskSync's event API (`net.william278.husksync.event`): `PreSyncEvent`, `DataSaveEvent` and
//! `SyncCompleteEvent`. Other code in the same process registers listeners here; the engine fires
//! them where HuskSync fires its Bukkit events. `PreSyncEvent` and `DataSaveEvent` are cancellable.
//! (A bus between separate Horizon mods does not exist; see ENGINE_NEEDS `mod_event_bus`.)

use crate::horizonsync_core::snapshot::Packed;
use std::sync::{Arc, Mutex, OnceLock};

/// Fired before a snapshot is applied to a joining player. Cancelling fails the sync.
pub struct PreSync<'a> {
    pub player: u128,
    pub snapshot: &'a Packed,
    /// `DataSnapshot.UpdateCause` name: SYNCHRONIZED, NEW_USER or UPDATED.
    pub cause: &'static str,
    pub cancelled: bool,
}

/// Fired before a snapshot is saved to the database. Cancelling skips the save.
pub struct DataSave<'a> {
    pub player: u128,
    pub snapshot: &'a Packed,
    pub save_cause: String,
    pub cancelled: bool,
}

/// Fired when a player's sync finished (the data was applied).
pub struct SyncComplete {
    pub player: u128,
    pub cause: &'static str,
}

type Pre = Arc<dyn Fn(&mut PreSync) + Send + Sync>;
type Save = Arc<dyn Fn(&mut DataSave) + Send + Sync>;
type Done = Arc<dyn Fn(&SyncComplete) + Send + Sync>;

#[derive(Default)]
struct Listeners {
    pre: Vec<Pre>,
    save: Vec<Save>,
    done: Vec<Done>,
}

fn listeners() -> &'static Mutex<Listeners> {
    static L: OnceLock<Mutex<Listeners>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(Listeners::default()))
}

pub fn on_pre_sync(f: impl Fn(&mut PreSync) + Send + Sync + 'static) {
    listeners().lock().unwrap_or_else(|e| e.into_inner()).pre.push(Arc::new(f));
}

pub fn on_data_save(f: impl Fn(&mut DataSave) + Send + Sync + 'static) {
    listeners().lock().unwrap_or_else(|e| e.into_inner()).save.push(Arc::new(f));
}

pub fn on_sync_complete(f: impl Fn(&SyncComplete) + Send + Sync + 'static) {
    listeners().lock().unwrap_or_else(|e| e.into_inner()).done.push(Arc::new(f));
}

/// The mods that called `subscribe` and the event lines waiting for the server thread to deliver.
#[derive(Default)]
struct Outbox {
    subscribers: Vec<String>,
    lines: Vec<String>,
}

fn outbox() -> &'static Mutex<Outbox> {
    static O: OnceLock<Mutex<Outbox>> = OnceLock::new();
    O.get_or_init(|| Mutex::new(Outbox::default()))
}

fn queue(line: String) {
    let mut o = outbox().lock().unwrap_or_else(|e| e.into_inner());
    if !o.subscribers.is_empty() && o.lines.len() < 10_000 {
        o.lines.push(line);
    }
}

/// The subscribed mod ids and the event lines since the last call (the server thread delivers them to the service
/// `<id>:husksync_event`). Mod listeners cannot cancel: the event is delivered after it ran.
pub fn take_events() -> (Vec<String>, Vec<String>) {
    let mut o = outbox().lock().unwrap_or_else(|e| e.into_inner());
    if o.lines.is_empty() {
        return (Vec::new(), Vec::new());
    }
    (o.subscribers.clone(), std::mem::take(&mut o.lines))
}

pub fn unsubscribe(id: &str) {
    outbox().lock().unwrap_or_else(|e| e.into_inner()).subscribers.retain(|s| s != id);
}

/// True when a listener cancelled the sync.
pub(crate) fn fire_pre_sync(player: u128, snapshot: &Packed, cause: &'static str) -> bool {
    queue(format!("pre_sync\t{}\t{cause}", util::uuid_string(player)));
    let list: Vec<Pre> = listeners().lock().unwrap_or_else(|e| e.into_inner()).pre.clone();
    let mut e = PreSync { player, snapshot, cause, cancelled: false };
    for l in list {
        l(&mut e);
    }
    e.cancelled
}

/// True when a listener cancelled the save.
pub(crate) fn fire_data_save(player: u128, snapshot: &Packed) -> bool {
    queue(format!("data_save\t{}\t{}", util::uuid_string(player), snapshot.save_cause));
    let list: Vec<Save> = listeners().lock().unwrap_or_else(|e| e.into_inner()).save.clone();
    let mut e = DataSave { player, snapshot, save_cause: snapshot.save_cause.clone(), cancelled: false };
    for l in list {
        l(&mut e);
    }
    e.cancelled
}

pub(crate) fn fire_sync_complete(player: u128, cause: &'static str) {
    queue(format!("sync_complete\t{}\t{cause}", util::uuid_string(player)));
    let list: Vec<Done> = listeners().lock().unwrap_or_else(|e| e.into_inner()).done.clone();
    let e = SyncComplete { player, cause };
    for l in list {
        l(&e);
    }
}


// ---- HuskSyncAPI as the mod service `husksync:api` ----------------------------------------------------------------
//
// One tab-separated line in (`<op>` then its arguments), one text line out. Snapshots travel as their packed JSON
// (`DataSnapshot.Packed`, one line, no tabs); an unknown user, version or operation answers `none`/None. Operations:
// `version`, `get_current_data <uuid>` (the newest saved snapshot: a live capture needs the game thread),
// `set_current_data <uuid> <packed json>` (stored as a new snapshot with the cause API), `edit_current_data
// <uuid> <key> <value>`, `get_snapshots <uuid>` (JSON array, newest first), `get_latest_snapshot <uuid>`,
// `get_snapshot <uuid> <version>`, `edit_snapshot <uuid> <version> <key> <value>`, `edit_latest_snapshot <uuid> <key>
// <value>`, `edit_packed_snapshot <uuid> <packed json>` (replaces the stored document of that id),
// `remove_snapshot_data <uuid> <version> <key>`, `get_snapshot_file_size <uuid> <version>`, `snapshot_builder <cause>
// <pinned> <packed data map json>`, `pack_snapshot <packed json>` (hex of the stored bytes), `unpack_snapshot <hex>`,
// `subscribe <mod id>` / `unsubscribe <mod id>` (events `pre_sync|data_save|sync_complete <uuid> <cause>` go to the
// service `<id>:husksync_event`), `register_data_serializer <id>`, `get_data_serializer <id>`, `serialize_data <id> <value>`, `deserialize_data <id>
// <value>` and `set_data_syncer <LOCKSTEP|DELAY>`.

use crate::horizonsync_core::engine::Engine;
use crate::horizonsync_core::snapshot::keys;
use crate::horizonsync_core::util;

fn registered() -> &'static Mutex<Vec<String>> {
    static R: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Vec::new()))
}

const BUILTIN_KEYS: [&str; 13] = [
    keys::INVENTORY,
    keys::ENDER_CHEST,
    keys::POTION_EFFECTS,
    keys::ADVANCEMENTS,
    keys::LOCATION,
    keys::STATISTICS,
    keys::HEALTH,
    keys::HUNGER,
    keys::ATTRIBUTES,
    keys::EXPERIENCE,
    keys::GAME_MODE,
    keys::FLIGHT_STATUS,
    keys::PERSISTENT_DATA,
];

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// A stored snapshot's document, if it can be read.
fn read_one(s: Option<crate::horizonsync_core::db::Stored>) -> Option<Packed> {
    s.and_then(|s| s.snapshot.ok())
}

/// Stores `p` as a new snapshot of the player and answers its json.
fn add(e: &Engine, player: u128, p: &Packed) -> Option<String> {
    e.db.add_snapshot(player, p).ok()?;
    Some(p.to_json())
}

/// Edits one data entry of a stored document (an empty value removes it) and writes it back.
fn edit_stored(e: &Engine, player: u128, mut p: Packed, key: &str, value: &str) -> Option<String> {
    if value.is_empty() {
        p.data.shift_remove(key);
    } else {
        p.data.insert(key.to_owned(), value.to_owned());
    }
    e.db.update_snapshot(player, &p).ok()?;
    Some(p.to_json())
}

/// Answers one call of the service `husksync:api`; None for an operation or arguments it does not know.
pub fn handle(e: &Engine, payload: &str) -> Option<String> {
    let mut parts = payload.split('\t');
    let op = parts.next()?;
    let a: Vec<&str> = parts.collect();
    let word = |i: usize| a.get(i).copied();
    let uuid = |i: usize| a.get(i).and_then(|s| util::parse_uuid(s));
    Some(match op {
        "version" => "4.1.0".into(),
        "subscribe" => {
            let id = word(0)?;
            let mut o = outbox().lock().unwrap_or_else(|x| x.into_inner());
            if !o.subscribers.iter().any(|s| s == id) {
                o.subscribers.push(id.to_owned());
            }
            "true".into()
        }
        "unsubscribe" => {
            unsubscribe(word(0)?);
            "true".into()
        }
        "get_current_data" | "get_latest_snapshot" => {
            read_one(e.db.latest_snapshot(uuid(0)?).ok()?).map_or_else(|| "none".into(), |p| p.to_json())
        }
        "set_current_data" => {
            let mut p: Packed = serde_json::from_str(word(1)?).ok()?;
            // A set document is a new version saved now, by the API.
            p.id = util::uuid_string(util::random_uuid());
            p.timestamp = util::format_timestamp(&util::now());
            p.save_cause = "API".into();
            p.pinned = false;
            add(e, uuid(0)?, &p)?
        }
        "edit_current_data" | "edit_latest_snapshot" => {
            let player = uuid(0)?;
            match read_one(e.db.latest_snapshot(player).ok()?) {
                Some(mut p) if op == "edit_latest_snapshot" => {
                    p = edit_stored_in_place(p, word(1)?, word(2).unwrap_or(""));
                    e.db.update_snapshot(player, &p).ok()?;
                    p.to_json()
                }
                Some(p) => {
                    // The current data edited is a new snapshot, as `editCurrentData` saves.
                    let mut c = p.copy();
                    c.save_cause = "API".into();
                    c = edit_stored_in_place(c, word(1)?, word(2).unwrap_or(""));
                    add(e, player, &c)?
                }
                None => "none".into(),
            }
        }
        "get_snapshots" => {
            let all: Vec<Packed> = e.db.all_snapshots(uuid(0)?).ok()?.into_iter().filter_map(|s| s.snapshot.ok()).collect();
            let docs: Vec<String> = all.iter().map(Packed::to_json).collect();
            format!("[{}]", docs.join(","))
        }
        "get_snapshot" => {
            read_one(e.db.snapshot(uuid(0)?, uuid(1)?).ok()?).map_or_else(|| "none".into(), |p| p.to_json())
        }
        "edit_snapshot" => {
            let player = uuid(0)?;
            match read_one(e.db.snapshot(player, uuid(1)?).ok()?) {
                Some(p) => edit_stored(e, player, p, word(2)?, word(3).unwrap_or(""))?,
                None => "none".into(),
            }
        }
        "remove_snapshot_data" => {
            let player = uuid(0)?;
            match read_one(e.db.snapshot(player, uuid(1)?).ok()?) {
                Some(p) => edit_stored(e, player, p, word(2)?, "")?,
                None => "none".into(),
            }
        }
        "edit_packed_snapshot" => {
            let player = uuid(0)?;
            let p: Packed = serde_json::from_str(word(1)?).ok()?;
            e.db.update_snapshot(player, &p).ok()?;
            p.to_json()
        }
        "get_snapshot_file_size" => {
            let compress = e.cfg().synchronization.compress_data;
            read_one(e.db.snapshot(uuid(0)?, uuid(1)?).ok()?)
                .map_or_else(|| "none".into(), |p| p.file_size(compress).to_string())
        }
        "snapshot_builder" => {
            let data: indexmap::IndexMap<String, String> = serde_json::from_str(word(2)?).ok()?;
            Packed::new(&e.platform, word(0)?, word(1)?.parse().ok()?, data).to_json()
        }
        "pack_snapshot" => {
            let p: Packed = serde_json::from_str(word(0)?).ok()?;
            to_hex(&p.to_bytes(e.cfg().synchronization.compress_data))
        }
        "unpack_snapshot" => {
            let bytes = from_hex(word(0)?)?;
            Packed::from_bytes(&bytes, e.cfg().synchronization.compress_data).ok()?.to_json()
        }
        "register_data_serializer" => {
            let id = word(0)?;
            if !id.contains(':') || BUILTIN_KEYS.contains(&id) {
                return Some("false".into());
            }
            let mut r = registered().lock().unwrap_or_else(|x| x.into_inner());
            if r.iter().any(|k| k == id) {
                return Some("false".into());
            }
            r.push(id.to_owned());
            "true".into()
        }
        "get_data_serializer" => {
            let id = word(0)?;
            let known = BUILTIN_KEYS.contains(&id) || registered().lock().unwrap_or_else(|x| x.into_inner()).iter().any(|k| k == id);
            known.to_string()
        }
        // Data is kept as the text the serializer produced, so both directions pass it through for a known id.
        "serialize_data" | "deserialize_data" => {
            let id = word(0)?;
            let known = BUILTIN_KEYS.contains(&id) || registered().lock().unwrap_or_else(|x| x.into_inner()).iter().any(|k| k == id);
            if known { word(1).unwrap_or("").to_owned() } else { "none".into() }
        }
        "set_data_syncer" => {
            let mode = word(0)?.to_ascii_uppercase();
            if mode != "LOCKSTEP" && mode != "DELAY" {
                return Some("false".into());
            }
            e.config.write().unwrap_or_else(|x| x.into_inner()).synchronization.mode = mode;
            "true".into()
        }
        _ => return None,
    })
}

fn edit_stored_in_place(mut p: Packed, key: &str, value: &str) -> Packed {
    if value.is_empty() {
        p.data.shift_remove(key);
    } else {
        p.data.insert(key.to_owned(), value.to_owned());
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listeners_fire_and_cancel() {
        let p: Packed = serde_json::from_str("{}").unwrap();
        assert!(!fire_pre_sync(1, &p, "SYNCHRONIZED"));
        // The registry is process-wide: listeners here only touch their own player ids.
        on_pre_sync(|e| e.cancelled = e.player == 7);
        assert!(fire_pre_sync(7, &p, "NEW_USER"));
        assert!(!fire_pre_sync(8, &p, "NEW_USER"));
        on_data_save(|e| e.cancelled = e.player == 0xDEAD_BEEF);
        assert!(!fire_data_save(1, &p));
        assert!(fire_data_save(0xDEAD_BEEF, &p));
        on_sync_complete(|_| {});
        fire_sync_complete(1, "UPDATED");
    }

}
