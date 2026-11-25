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

/// True when a listener cancelled the sync.
pub(crate) fn fire_pre_sync(player: u128, snapshot: &Packed, cause: &'static str) -> bool {
    let list: Vec<Pre> = listeners().lock().unwrap_or_else(|e| e.into_inner()).pre.clone();
    let mut e = PreSync { player, snapshot, cause, cancelled: false };
    for l in list {
        l(&mut e);
    }
    e.cancelled
}

/// True when a listener cancelled the save.
pub(crate) fn fire_data_save(player: u128, snapshot: &Packed) -> bool {
    let list: Vec<Save> = listeners().lock().unwrap_or_else(|e| e.into_inner()).save.clone();
    let mut e = DataSave { player, snapshot, save_cause: snapshot.save_cause.clone(), cancelled: false };
    for l in list {
        l(&mut e);
    }
    e.cancelled
}

pub(crate) fn fire_sync_complete(player: u128, cause: &'static str) {
    let list: Vec<Done> = listeners().lock().unwrap_or_else(|e| e.into_inner()).done.clone();
    let e = SyncComplete { player, cause };
    for l in list {
        l(&e);
    }
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
