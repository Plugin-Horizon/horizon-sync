//! HorizonSync core: HuskSync 4.1-compatible data format, database, Redis protocol and sync rules,
//! with no dependency on the game. `glue/horizonsync.rs` (inside Horizon) calls the functions
//! below on the server thread and carries out the [`Action`]s they return.
//!
//! Derived from HuskSync by William278 (Apache License 2.0); see NOTICE.

pub mod commands;
pub mod config;
pub mod data;
pub mod db;
pub mod engine;
pub mod messages;
pub mod nbt;
pub mod player;
pub mod records;
pub mod redis_mgr;
pub mod snapshot;
pub mod snappy;
pub mod util;
pub mod yaml;

pub use data::StoredItem;
pub use engine::{Action, Engine, MenuEdit, Sender};
pub use nbt::Nbt;
pub use player::ApplyPlan;

use std::path::Path;
use std::sync::{Arc, OnceLock};

/// The config written on first start.
pub const DEFAULT_CONFIG: &str = include_str!("../config.example.toml");

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();

fn engine() -> Option<&'static Arc<Engine>> {
    ENGINE.get()
}

/// The dedicated server starts: read HuskSync's `plugins/HuskSync/config.yml` of the server folder
/// (else `<config_dir>/horizonsync/config.toml`, written with the defaults when missing), connect to the database and Redis. `folder_name` names the server
/// when the config leaves `server_name` empty. Errors are logged and leave the mod off.
pub fn start(config_dir: &Path, folder_name: &str) {
    if ENGINE.get().is_some() {
        return;
    }
    // The server's folder is the config folder's parent (`plugins/HuskSync` is read from there).
    let server_dir = config_dir.parent().unwrap_or(config_dir);
    let dir = config_dir.join("horizonsync");
    let mut config = match config::Config::load(server_dir, &dir) {
        Ok(c) => c,
        Err(e) => {
            log::error!("horizonsync: not started: {e}");
            return;
        }
    };
    if config.server_name.trim().is_empty() {
        config.server_name = folder_name.to_owned();
    }
    match Engine::new(config, dir) {
        Ok(e) => {
            log::info!("horizonsync: synchronising as '{}'", e.platform.server_name);
            let _ = ENGINE.set(e);
        }
        Err(e) => log::error!("horizonsync: not started: {e}"),
    }
}

pub fn running() -> bool {
    engine().is_some_and(|e| !e.disabling())
}

/// A player was placed in the world (`PlayerJoinEvent`).
pub fn on_join(player: u128, name: &str) {
    if let Some(e) = engine() {
        e.on_join(player, name);
    }
}

/// A player is being saved because they leave (`PlayerQuitEvent`), with the compound the
/// server writes for them.
pub fn on_leave(player: u128, compound: &Nbt) {
    if let Some(e) = engine() {
        e.on_leave(player, compound);
    }
}

/// The world saved (`WorldSaveEvent`): every player here with their compound.
pub fn on_world_save(players: Vec<(u128, Nbt)>) {
    if let Some(e) = engine() {
        e.on_world_save(players);
    }
}

/// An orderly stop begins, players still on: saves them and waits (bounded) for every save.
pub fn on_server_stopping(players: Vec<(u128, Nbt)>) {
    if let Some(e) = engine() {
        e.on_server_stopping(players);
    }
}

/// Once a tick: the players on (UUID, entity id); returns what the game must do now.
pub fn tick(players: &[(u128, i32)]) -> Vec<Action> {
    engine().map(|e| e.tick(players)).unwrap_or_default()
}

/// The game carried out an `Action::Apply` (`ok` false: the player was gone or it failed).
pub fn applied(token: u64, ok: bool) {
    if let Some(e) = engine() {
        e.applied(token, ok);
    }
}

/// The game read a player for an `Action::Capture` (None: not on the server).
pub fn captured(token: u64, compound: Option<Nbt>) {
    if let Some(e) = engine() {
        e.captured(token, compound);
    }
}

/// Whether the player's data is still being loaded (or saved): they must not act.
pub fn is_locked(player: u128) -> bool {
    engine().is_some_and(|e| e.is_locked(player))
}

pub fn is_locked_entity(entity: i32) -> bool {
    engine().is_some_and(|e| e.is_locked_entity(entity))
}

/// Whether a locked player's throw or shot of `item` must be refused.
pub fn refuse_launch(player: u128, item: &str) -> bool {
    engine().is_some_and(|e| e.refuse_launch(player, item))
}

/// Whether a locked player's command line must be refused.
pub fn refuse_command(player: u128, line: &str) -> bool {
    engine().is_some_and(|e| e.refuse_command(player, line))
}

/// An editable menu closed with `items` in it (`Action::OpenMenu { edit: Some(..) }`).
pub fn on_menu_edited(edit: MenuEdit, items: Vec<Option<StoredItem>>) {
    if let Some(e) = engine() {
        e.on_menu_edited(edit, items);
    }
}

/// The command usages to register, with their operator levels.
pub fn command_usages() -> Vec<(String, u8)> {
    match engine() {
        Some(e) => commands::usages(&e.cfg().disabled_commands),
        None => Vec::new(),
    }
}

/// The permission node a command's registration names (`horizonsync.command.<command>`).
/// The node for editing the items of `/inventory` / `/enderchest`, if the line is one of those.
pub fn edit_permission_for(line: &str) -> Option<String> {
    commands::edit_permission_for(line)
}

pub fn command_node(label: &str) -> Option<String> {
    commands::command_node(label)
}

/// The node and default level a command line needs (its sub-command's node when it has one).
pub fn permission_for(line: &str) -> Option<(String, u8)> {
    commands::permission_for(line)
}

/// A command line (without its slash); true when it is ours.
pub fn perform_command(sender: Sender, line: &str) -> bool {
    match engine() {
        Some(e) => commands::perform(e, sender, line),
        None => false,
    }
}

#[cfg(test)]
mod engine_tests;
