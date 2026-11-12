// COMPILED OUT (see lib.rs: #[cfg(any())] mod legacy_glue). This is the horizon-rust glue of
// horizonsync, kept verbatim as the logic reference for the pass that adds the missing player-data APIs.
// HorizonSync's engine side, a module of `crate::game` (so it reaches `Game`'s fields and
// the hub's guests). It turns the core's actions into changes on the server's players and
// reads players back as their vanilla compound (`playerdata::player_tag`, what Horizon writes to
// `players/data/<uuid>.dat`).
//
// Derived from HuskSync by William278 (Apache License 2.0); see NOTICE.

use super::Game;
use horizon_client::items::ItemStack;
use horizon_client::text::Text;
use horizonsync_core as core;
use horizon_world::nbt::{JavaString, Tag};
use std::sync::Mutex;

// ----- NBT between Horizon's `Tag` and the core's `Nbt` -----

pub(crate) fn to_nbt(t: &Tag) -> Option<core::Nbt> {
    use core::Nbt as N;
    Some(match t {
        Tag::End => return None,
        Tag::Byte(v) => N::Byte(*v),
        Tag::Short(v) => N::Short(*v),
        Tag::Int(v) => N::Int(*v),
        Tag::Long(v) => N::Long(*v),
        Tag::Float(bits) => N::Float(f32::from_bits(*bits)),
        Tag::Double(bits) => N::Double(f64::from_bits(*bits)),
        Tag::ByteArray(v) => N::ByteArray(v.iter().map(|b| *b as i8).collect()),
        Tag::String(s) => N::String(s.to_string_lossy()),
        Tag::List(v) => N::List(v.iter().filter_map(to_nbt).collect()),
        Tag::Compound(m) => N::Compound(
            m.iter()
                .filter_map(|(k, v)| Some((k.to_string_lossy(), to_nbt(v)?)))
                .collect(),
        ),
        Tag::IntArray(v) => N::IntArray(v.clone()),
        Tag::LongArray(v) => N::LongArray(v.clone()),
    })
}

pub(crate) fn to_tag(n: &core::Nbt) -> Tag {
    use core::Nbt as N;
    match n {
        N::Byte(v) => Tag::Byte(*v),
        N::Short(v) => Tag::Short(*v),
        N::Int(v) => Tag::Int(*v),
        N::Long(v) => Tag::Long(*v),
        N::Float(v) => Tag::Float(v.to_bits()),
        N::Double(v) => Tag::Double(v.to_bits()),
        N::ByteArray(v) => Tag::ByteArray(v.iter().map(|b| *b as u8).collect()),
        N::String(s) => Tag::String(JavaString::from(s.as_str())),
        N::List(v) => Tag::List(v.iter().map(to_tag).collect()),
        N::Compound(m) => Tag::Compound(
            m.iter().map(|(k, v)| (JavaString::from(k.as_str()), to_tag(v))).collect(),
        ),
        N::IntArray(v) => Tag::IntArray(v.clone()),
        N::LongArray(v) => Tag::LongArray(v.clone()),
    }
}

/// A stored HuskSync item as a Horizon stack: brought up to this version's data first
/// (`DataFixer`, as NBT-API's `fixUpRawItemData` does on Paper), then read as
/// `ItemStack.CODEC`.
fn stack_of(item: &core::StoredItem) -> Option<ItemStack> {
    let mut tag = to_tag(&item.item);
    if item.data_version < horizon_world::DATA_VERSION {
        horizon_world::datafix::update(
            horizon_world::datafix::schema::Ty::ItemStack,
            &mut tag,
            item.data_version,
            horizon_world::DATA_VERSION,
        );
    }
    horizon_items::item_nbt::item_from_nbt(&tag)
}

fn stacks_of(items: &[Option<core::StoredItem>]) -> Vec<Option<ItemStack>> {
    items.iter().map(|s| s.as_ref().and_then(stack_of)).collect()
}

/// The player's compound through the game's helper (`native_player_compound`), with the mods'
/// persistent compound (`native_player_persistent_data`, saved as `NeoForgeData`) put where the
/// core and Paper expect it, `BukkitValues`. A compound written by an older build of this port
/// still holds its own `BukkitValues`; that one is kept when the mods' compound is empty.
fn capture_compound(game: &Game<'_>, uuid: u128) -> Option<core::Nbt> {
    let mut tag = game.native_player_compound(uuid)?;
    let pdc = game.native_player_persistent_data(uuid);
    let filled = matches!(&pdc, Some(Tag::Compound(m)) if !m.is_empty());
    if let (true, Some(pdc), Tag::Compound(map)) = (filled, pdc, &mut tag) {
        map.insert(JavaString::from("BukkitValues"), pdc);
    }
    // The records the compound lacks: advancements and statistics in vanilla's JSON, as text
    // entries the core reads (`records`).
    if let Tag::Compound(map) = &mut tag {
        if let Some(v) = game.native_player_advancements(uuid) {
            map.insert(
                JavaString::from(core::records::RECORD_ADVANCEMENTS),
                Tag::String(JavaString::from(v.to_string().as_str())),
            );
        }
        if let Some(v) = game.native_player_stats(uuid) {
            map.insert(
                JavaString::from(core::records::RECORD_STATISTICS),
                Tag::String(JavaString::from(v.to_string().as_str())),
            );
        }
    }
    let mut c = to_nbt(&tag)?;
    core::player::mod_data_as_bukkit_values(&mut c);
    Some(c)
}

fn players_with_compounds(game: &Game<'_>) -> Vec<(u128, core::Nbt)> {
    let Some(host) = game.lan.as_ref() else {
        return Vec::new();
    };
    let uuids: Vec<u128> = host
        .hub
        .guests
        .iter()
        // A fake player (a mod's bot with no client) has no snapshot: it is neither fetched nor saved.
        .filter(|g| g.placed && !g.released && !g.fake)
        .map(|g| g.uuid)
        .collect();
    uuids
        .into_iter()
        .filter_map(|u| Some((u, capture_compound(game, u)?)))
        .collect()
}

// ----- lifecycle -----

/// `server_about_to_start`: start on a dedicated server only.
pub(crate) fn start(dedicated: bool) {
    if !dedicated {
        return;
    }
    let Ok(config_dir) = Game::native_config_dir() else {
        log::error!("horizonsync: no config folder; not started");
        return;
    };
    let folder = config_dir
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "server".into());
    core::start(&config_dir, &folder);
}

/// `server_stopping`: SERVER_SHUTDOWN saves for everyone still on, then wait for the saves.
pub(super) fn stopping(game: &mut Game<'_>) {
    if core::running() {
        core::on_server_stopping(players_with_compounds(game));
    }
}

/// `world_saved`: WORLD_SAVE snapshots.
pub(super) fn world_saved(game: &mut Game<'_>) {
    if core::running() {
        core::on_world_save(players_with_compounds(game));
    }
}

/// `player_logging_out`: a player leaves with their state still readable (Bukkit's
/// `PlayerQuitEvent`); the DISCONNECT snapshot. Not heard when the server stops, where
/// `stopping` saves everyone.
pub(crate) fn leaving(game: &mut Game<'_>, player: u128) {
    if !core::running() {
        return;
    }
    if let Some(c) = capture_compound(game, player) {
        core::on_leave(player, &c);
    }
}

// ----- every tick: the core's actions -----

pub(super) fn tick(game: &mut Game<'_>) {
    if !core::running() {
        return;
    }
    if game.lan_acting.is_some() {
        return;
    }
    let players: Vec<(u128, i32)> = game
        .lan
        .as_ref()
        .map(|h| h.hub.guests.iter().filter(|g| g.placed && !g.fake).map(|g| (g.uuid, g.entity_id)).collect())
        .unwrap_or_default();
    retry_dead(game);
    for action in core::tick(&players) {
        match action {
            core::Action::Apply { player, plan, token } => {
                if is_dead(game, player) {
                    // A player who joins dead is respawned first (Spigot's `respawn()`); the
                    // plan waits for them to be alive.
                    game.native_respawn_player(player);
                    DEFERRED.lock().unwrap().push(Deferred { player, plan, token, waited: 0 });
                } else {
                    let ok = apply(game, player, &plan);
                    core::applied(token, ok);
                }
            }
            core::Action::Capture { player, token } => {
                let c = capture_compound(game, player);
                core::captured(token, c);
            }
            core::Action::Message { player: Some(p), text, overlay } => {
                if overlay {
                    game.native_action_bar(p, Text::literal(text));
                } else {
                    game.native_tell(p, Text::literal(text));
                }
            }
            core::Action::Message { player: None, text, .. } => log::info!("[HorizonSync] {text}"),
            core::Action::Component { player: Some(p), text, json, overlay } => {
                // Clickable chat or the action bar: the plain words with the component (click and hover events).
                let line = Text::literal(text).with_component(json);
                if overlay {
                    game.native_action_bar(p, line);
                } else {
                    game.native_tell(p, line);
                }
            }
            core::Action::Component { player: None, text, .. } => log::info!("[HorizonSync] {text}"),
            core::Action::OpenMenu { viewer, title, rows, items, edit } => {
                let mut stacks = stacks_of(&items);
                let size = rows as usize * 9;
                stacks.resize(size, None);
                match edit {
                    // Every slot takes and gives items; `menu_input_closed` hears the result.
                    Some(edit) => {
                        if let Some(window) =
                            game.native_open_menu_with_input(viewer, rows, Text::literal(title), stacks, 0..size)
                        {
                            let mut edits = EDITS.lock().unwrap();
                            edits.retain(|(v, _, _)| *v != viewer);
                            edits.push((viewer, window, edit));
                        }
                    }
                    None => {
                        game.native_open_menu(viewer, rows, Text::literal(title), stacks);
                    }
                }
            }
        }
    }
}

/// A plan for a player who was dead when it came.
struct Deferred {
    player: u128,
    plan: Box<core::ApplyPlan>,
    token: u64,
    waited: u32,
}

static DEFERRED: Mutex<Vec<Deferred>> = Mutex::new(Vec::new());

/// Ticks a dead player gets to respawn before their plan is given up.
const RESPAWN_WAIT_TICKS: u32 = 200;

fn is_dead(game: &Game<'_>, player: u128) -> bool {
    game.lan.as_ref().is_some_and(|h| {
        h.hub
            .guests
            .iter()
            .any(|g| g.placed && !g.released && g.uuid == player && g.dead)
    })
}

/// Plans that waited for a respawn: applied once the player is alive, given up after
/// `RESPAWN_WAIT_TICKS` or when they left.
fn retry_dead(game: &mut Game<'_>) {
    let waiting: Vec<Deferred> = std::mem::take(&mut *DEFERRED.lock().unwrap());
    for mut d in waiting {
        if is_dead(game, d.player) && d.waited < RESPAWN_WAIT_TICKS {
            d.waited += 1;
            if d.waited % 20 == 0 {
                game.native_respawn_player(d.player);
            }
            DEFERRED.lock().unwrap().push(d);
        } else {
            let ok = apply(game, d.player, &d.plan);
            core::applied(d.token, ok);
        }
    }
}

const GAME_MODES: [&str; 4] = ["survival", "creative", "adventure", "spectator"];

/// Carries out an apply plan on a player who is on, in HuskSync's dependency order, through the
/// game's player-state helpers (`native_set_inventory`, `native_set_effects`,
/// `native_set_attributes`, `native_set_player_status`, `native_set_flight`,
/// `native_teleport_player`, `native_set_player_persistent_data`). False when they are not
/// here (left, dead, or their state is swapped in).
fn apply(game: &mut Game<'_>, player: u128, plan: &core::ApplyPlan) -> bool {
    // Who they are now: placed, alive, and their name and mode for the one command left.
    let (name, mode_now) = {
        let Some(host) = game.lan.as_ref() else { return false };
        let Some(g) = host
            .hub
            .guests
            .iter()
            .find(|g| g.placed && !g.released && g.uuid == player)
        else {
            return false;
        };
        if g.dead {
            return false;
        }
        (g.name.clone(), g.game_mode)
    };
    if let Some(a) = &plan.advancements {
        if !game.native_set_player_advancements(player, a) {
            log::warn!("horizonsync: {name}'s advancements were not applied");
        }
    }
    if let Some(s) = &plan.statistics {
        if !game.native_set_player_stats(player, s) {
            log::warn!("horizonsync: {name}'s statistics were not applied");
        }
    }
    if let Some(pdc) = &plan.persistent_data {
        // Bukkit's PersistentDataContainer (`BukkitValues` in Paper's file); Horizon keeps the
        // mods' compound as `NeoForgeData`. `capture_compound` maps it back.
        if !game.native_set_player_persistent_data(player, to_tag(pdc)) {
            log::warn!("horizonsync: {name}'s persistent data was not applied");
        }
    }
    if let Some((slots, held)) = &plan.inventory {
        // BukkitData.Items.Inventory.apply: cursor and 2x2 grid are cleared by the helper.
        let mut s = stacks_of(slots);
        // HuskSync's snapshot holds Paper's 43 slots; a Horizon player inventory has 41 (36 + 4
        // armour + off hand). Slots 41 and 42 stay in the stored snapshot as read; the engine
        // gets its 41.
        s.resize(41, None);
        let selected = Some((*held).clamp(0, 8) as usize);
        if let Err(e) = game.native_set_inventory(player, super::NativeInventoryKind::Player, s, selected) {
            log::warn!("horizonsync: {name}'s inventory was not applied: {e}");
        }
    }
    if let Some(ender) = &plan.ender_chest {
        let mut s = stacks_of(ender);
        s.resize(core::data::ENDER_CHEST_SLOTS, None);
        if let Err(e) = game.native_set_inventory(player, super::NativeInventoryKind::Ender, s, None) {
            log::warn!("horizonsync: {name}'s ender chest was not applied: {e}");
        }
    }
    if let Some(effects) = &plan.effects {
        let list: Vec<super::NativeEffect> = effects
            .iter()
            .map(|e| super::NativeEffect {
                id: e.kind.clone(),
                amplifier: e.amplifier.clamp(0, 255) as u8,
                duration: e.duration,
                ambient: e.is_ambient,
                particles: e.show_particles,
            })
            .collect();
        if !game.native_set_effects(player, &list) {
            log::warn!("horizonsync: {name}'s effects were not applied");
        }
    }
    if let Some(attrs) = &plan.attributes {
        // AttributeMap.load of the synced attributes, before health (max_health caps it).
        if !game.native_set_attributes(player, &to_tag(attrs)) {
            log::warn!("horizonsync: {name}'s attributes were not applied");
        }
    }
    if let Some(mode) = plan.game_mode {
        if mode != mode_now {
            // No game-mode helper yet (GAPS.md): the vanilla command.
            let c = format!("gamemode {} {}", GAME_MODES[mode.clamp(0, 3) as usize], name);
            if !game.native_run_command(&c) {
                log::warn!("horizonsync: '{c}' failed");
            }
        }
    }
    // Flight after the game mode (FlightStatus depends on GameMode), now also allow-flight in
    // survival (`Player.setAllowFlight`).
    if let Some(f) = &plan.flight {
        if !game.native_set_flight(player, f.allow_flight, f.allow_flight && f.is_flying) {
            log::warn!("horizonsync: {name}'s flight was not applied");
        }
    }
    // Food and experience before health; the helper refuses a whole call on one bad value, so
    // each part goes on its own and is clamped to what it accepts.
    if let Some(f) = &plan.hunger {
        let food = (
            f.food_level.clamp(0, 20),
            finite_or(f.saturation, 0.0),
            finite_or(f.exhaustion, 0.0),
        );
        if !game.native_set_player_status(player, None, Some(food), None) {
            log::warn!("horizonsync: {name}'s hunger was not applied");
        }
    }
    if let Some(x) = &plan.experience {
        let xp = (
            x.exp_level.max(0),
            finite_or(x.exp_progress, 0.0).clamp(0.0, 1.0),
            x.total_experience.max(0),
        );
        if !game.native_set_player_status(player, None, None, Some(xp)) {
            log::warn!("horizonsync: {name}'s experience was not applied");
        }
    }
    if let Some(h) = &plan.health {
        // Health 0 kills the player as `/kill` does, as Bukkit's `setHealth(0)` (a dead snapshot).
        if (h.health as f32).is_finite() {
            if !game.native_set_player_status(player, Some((h.health as f32).clamp(0.0, 1024.0)), None, None) {
                log::warn!("horizonsync: {name}'s health was not applied");
            }
        }
    }
    if let Some((dim, l)) = &plan.location {
        if !game.native_teleport_player(player, dim, [l.x, l.y, l.z], l.yaw, l.pitch) {
            log::warn!("horizonsync: {name} was not teleported to {dim}");
        }
    }
    true
}

fn finite_or(v: f32, or: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        or
    }
}

// ----- locks: a player whose data is loading may not act -----

pub(crate) fn locked(player: Option<u128>) -> bool {
    player.is_some_and(core::is_locked)
}

pub(crate) fn refuse_launch(player: u128, stack: &ItemStack) -> bool {
    let id: &str = &stack.item;
    if id.contains(':') {
        core::refuse_launch(player, id)
    } else {
        core::refuse_launch(player, &format!("minecraft:{id}"))
    }
}

pub(crate) fn refuse_command(player: u128, line: &str) -> bool {
    core::refuse_command(player, line)
}

// ----- editable /inventory and /enderchest -----

/// The editable menus open: viewer, window, what they edit.
static EDITS: Mutex<Vec<(u128, i32, core::MenuEdit)>> = Mutex::new(Vec::new());

/// `menu_input_closed`: an editable menu closed with `slots` in it. What the menu holds is the
/// player's data now: the slots are taken, and the core saves a new snapshot if they changed.
/// Slots past the data's size (the menu's last row) go back to the player.
pub(crate) fn menu_input_closed(player: u128, window: i32, slots: &mut Vec<Option<ItemStack>>) {
    let edit = {
        let mut edits = EDITS.lock().unwrap();
        let Some(at) = edits.iter().position(|(v, w, _)| *v == player && *w == window) else {
            return;
        };
        edits.remove(at).2
    };
    if !core::running() {
        return;
    }
    let size = if edit.ender_chest { core::data::ENDER_CHEST_SLOTS } else { core::data::INVENTORY_SLOTS };
    let mut items = Vec::with_capacity(size);
    for slot in slots.iter_mut().take(size) {
        items.push(slot.take().and_then(|stack| {
            let tag = horizon_items::item_nbt::item_to_nbt(&stack);
            Some(core::StoredItem { item: to_nbt(&tag)?, data_version: horizon_world::DATA_VERSION })
        }));
    }
    core::on_menu_edited(edit, items);
}

// ----- commands -----

pub(crate) fn register_commands(commands: &mut Vec<horizon_client::mod_api::NativeCommand>) {
    for (usage, level) in core::command_usages() {
        let command = if level == 0 {
            horizon_client::mod_api::NativeCommand::new(&usage)
        } else {
            horizon_client::mod_api::NativeCommand::op(&usage, level)
        };
        commands.push(match core::command_node(usage.split_whitespace().next().unwrap_or("")) {
            Some(node) => command.permission(&node),
            None => command,
        });
    }
}

pub(crate) fn perform_command(
    game: &mut Game<'_>,
    player: &horizon_client::mod_api::NativeServerPlayer,
    command: &str,
    ran: &mut bool,
) {
    if *ran {
        return;
    }
    // The sub-command's node with its default level (HuskSync's IF_OP, about is for everyone): a
    // permissions mod grants it to anyone; otherwise the operator level decides, as before.
    let level = match core::permission_for(command) {
        Some((node, default)) if game.native_has_permission(player.uuid, &node, default) => 4,
        _ => player.permission,
    };
    // `/inventory` and `/enderchest` open an editable menu for who holds the `.edit` node.
    let edit = core::edit_permission_for(command)
        .is_some_and(|node| game.native_has_permission(player.uuid, &node, 2));
    let sender = core::Sender { uuid: Some(player.uuid), name: player.name.clone(), level, edit };
    if core::perform_command(sender, command) {
        *ran = true;
    }
}

pub(crate) fn perform_console_command(command: &str, ran: &mut bool) {
    if *ran {
        return;
    }
    let sender = core::Sender { uuid: None, name: "CONSOLE".into(), level: 4, edit: false };
    if core::perform_command(sender, command) {
        *ran = true;
    }
}
