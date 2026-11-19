//! HorizonSync as an arismoko native mod (trait mod on `mod_api`). The sync logic is the unchanged
//! `horizonsync-core` crate (`core/`, HuskSync 4.1 format, database and Redis); this file wires it
//! to hooks and host methods. `legacy_glue.rs` is the horizon-rust glue, compiled out, kept as the
//! reference for the APIs that are still missing (PORT_NOTES.md).
//!
//! Off by default. It starts only when `config/husksync-mod.toml` says `enabled = true`, and the
//! `config/horizonsync/config.toml` the game writes points at `disabled.invalid` (no database, no
//! Redis) until the operator sets the hosts. A plan with anything the engine cannot take (an item at
//! another data version, an unreadable item or attribute) is refused whole, never applied in part.

pub mod horizonsync_core;
#[cfg(any())]
mod legacy_glue;

use horizonsync_core as core;
use mod_api::{TriState,
    ArmorStandManipulateEvent, AttackEntityEvent, AttributeModifier, AttributeOperation, Identifier, ItemStack,
    ModifierLifetime, BreakBlockEvent, Command, CommandArgumentValue,
    CommandInvocation, CommandSource, Compound, Config, ConfigKind, ContainerOpenedEvent, EntityInteractEvent,
    EntityPlaceEvent, Flow, GameMode, HostSide, Hooks, InteractionResult, ItemPickedUpEvent, ItemTossEvent,
    GameRuleValue, HostTransaction, ItemPlace, ItemResource, LeftClickBlockEvent, LevelRandomHost,
    LivingDropsEvent, LivingIncomingDamageEvent, LivingRef,
    MenuClickedEvent, MenuClosedEvent, MenuRef, MenuReplace, PlayerDiedEvent, NodeDefault, PermissionGatherNodesEvent, PermissionLevel,
    PermissionNode, PermissionValueType, PlayerCommandEvent, PlayerLoggedInEvent, PlayerLoggedOutEvent, PlayerRef,
    PlayerSaveToFileEvent,
    RegisterCommandsEvent, RegisterConfigsEvent, RightClickBlockEvent, RunModCommandEvent,
    ServerAboutToStartEvent, ServerHost, ServerStoppingEvent, ServerTickEvent, ServerTickPhase,
    StatType, Tag, Text, UseItemOnBlockEvent, WorldSavedEvent,
};
use std::sync::Mutex;

const ID: &str = "husksync";
/// Relative to the server folder. Only `enabled = true` on its own line turns the mod on.
const GATE_FILE: &str = "config/husksync-mod.toml";
const GATE_TEXT: &str = "# HorizonSync (husksync mod). Off unless a line below says enabled = true.\n\
# Before enabling: set the database and Redis hosts in config/horizonsync/config.toml.\n\
enabled = false\n";
/// Ticks a player who joins dead may wait for their own Respawn click before the mod respawns
/// them (HuskSync: 200).
const RESPAWN_WAIT_TICKS: u32 = 200;

fn log(line: &str) {
    eprintln!("[husksync] {line}");
}

/// True when the gate file has `enabled = true` on a line of its own (comments ignored).
fn enabled() -> bool {
    std::fs::read_to_string(GATE_FILE).is_ok_and(|text| {
        text.lines().any(|line| line.split('#').next().unwrap_or("").trim() == "enabled = true")
    })
}

/// Core's example config with every host replaced by `disabled.invalid` (RFC 2606: never resolves),
/// so a server that is not set up for the sync reaches neither MariaDB nor Redis.
fn disabled_core_config() -> &'static str {
    Box::leak(core::DEFAULT_CONFIG.replace("host = \"localhost\"", "host = \"disabled.invalid\"").into_boxed_str())
}

/// The command labels this mod registers (`husksync`, `userdata`, ...).
fn labels() -> Vec<String> {
    let mut labels: Vec<String> = core::command_usages()
        .into_iter()
        .filter_map(|(usage, _)| usage.split_whitespace().next().map(str::to_owned))
        .collect();
    labels.sort();
    labels.dedup();
    labels
}

/// A plan for a player who was dead when it came.
struct Deferred {
    player: u128,
    plan: Box<core::ApplyPlan>,
    token: u64,
    waited: u32,
}

/// An open `/inventory` or `/enderchest` menu its viewer may edit: the clicks are carried out here (the
/// menu itself moves nothing) and the contents are saved when it closes.
struct EditMenu {
    viewer: u128,
    window: i32,
    edit: core::MenuEdit,
    rows: u8,
    /// The menu's own slots as the core stores them, as of the last click that changed them.
    items: Vec<Option<core::StoredItem>>,
    /// Whether a click changed anything: a menu only looked at saves nothing.
    changed: bool,
}

#[derive(Default)]
struct Mod {
    edit_menus: Mutex<Vec<EditMenu>>,
    deferred: Mutex<Vec<Deferred>>,
    /// Players read at `player_save_to_file` this tick, for the leave snapshot in `player_logged_out`
    /// (the leave's save comes first, with the player still readable). Emptied every tick start, so
    /// an autosave's read never stands in for a leave.
    saved_now: Mutex<Vec<(u128, core::Nbt)>>,
}

// ----- NBT between the game (horizon_core `Tag`) and the core (`Nbt`) -----

/// The game's compound tag as the core's NBT type. Same tags, same values; floats and doubles come
/// from their bit patterns. An end tag (only an empty list's element) becomes an empty list.
fn to_core(tag: &Tag) -> core::Nbt {
    match tag {
        Tag::End => core::Nbt::List(Vec::new()),
        Tag::Byte(v) => core::Nbt::Byte(*v),
        Tag::Short(v) => core::Nbt::Short(*v),
        Tag::Int(v) => core::Nbt::Int(*v),
        Tag::Long(v) => core::Nbt::Long(*v),
        Tag::Float(bits) => core::Nbt::Float(f32::from_bits(*bits)),
        Tag::Double(bits) => core::Nbt::Double(f64::from_bits(*bits)),
        Tag::ByteArray(v) => core::Nbt::ByteArray(v.iter().map(|b| *b as i8).collect()),
        Tag::String(s) => core::Nbt::String(s.to_string_lossy()),
        Tag::List(items) => core::Nbt::List(items.iter().map(to_core).collect()),
        Tag::Compound(map) => {
            let mut out = core::Nbt::compound();
            for (key, value) in map {
                out.put(&key.to_string_lossy(), to_core(value));
            }
            out
        }
        Tag::IntArray(v) => core::Nbt::IntArray(v.clone()),
        Tag::LongArray(v) => core::Nbt::LongArray(v.clone()),
    }
}

/// The core's NBT as the game's tag; None for a value the game cannot hold, so the caller refuses
/// the whole write.
fn to_tag(nbt: &core::Nbt) -> Option<Tag> {
    Some(match nbt {
        core::Nbt::Byte(v) => Tag::Byte(*v),
        core::Nbt::Short(v) => Tag::Short(*v),
        core::Nbt::Int(v) => Tag::Int(*v),
        core::Nbt::Long(v) => Tag::Long(*v),
        core::Nbt::Float(v) => Tag::Float(v.to_bits()),
        core::Nbt::Double(v) => Tag::Double(v.to_bits()),
        core::Nbt::ByteArray(v) => Tag::ByteArray(v.iter().map(|b| *b as u8).collect()),
        core::Nbt::String(s) => Tag::String(s.as_str().into()),
        core::Nbt::List(items) => Tag::List(items.iter().map(to_tag).collect::<Option<Vec<_>>>()?),
        core::Nbt::Compound(map) => {
            let mut out = Compound::new();
            for (key, value) in map {
                out.insert(key.as_str().into(), to_tag(value)?);
            }
            Tag::Compound(out)
        }
        core::Nbt::IntArray(v) => Tag::IntArray(v.clone()),
        core::Nbt::LongArray(v) => Tag::LongArray(v.clone()),
    })
}

/// The compound a plan's persistent data is, for `set_persistent_data`; None when it is not one.
fn persistent_compound(nbt: &core::Nbt) -> Option<Compound> {
    match to_tag(nbt)? {
        Tag::Compound(map) => Some(map),
        _ => None,
    }
}

/// The player's data as the core reads it: the game's entity compound, the advancements text and
/// the mods' persistent compound. The last is put where Paper keeps Bukkit's container
/// (`BukkitValues`), which the core's capture reads. None when the player is not on the server.
fn read_player(host: &mut dyn ServerHost, uuid: u128) -> Option<core::Nbt> {
    let player = host.player(uuid)?;
    let entity = player.entity();
    let mut nbt = to_core(&Tag::Compound(entity.nbt(host)?));
    // HMOD-GAP(player-advancements): Horizon main has no per-player advancement read (only `advancement(id)`
    // lookups), so the advancements are not read here and not synced; a snapshot carries them over.
    if core::feature_enabled("husksync:statistics") {
        if let Some(json) = read_statistics(&*host, player) {
            nbt.put(core::records::RECORD_STATISTICS, core::Nbt::String(json));
        }
    }
    if let Some(data) = player.persistent_data(&*host) {
        nbt.put("NeoForgeData", to_core(&Tag::Compound(data)));
    }
    core::player::mod_data_as_bukkit_values(&mut nbt);
    Some(nbt)
}

/// The statistic types HuskSync syncs: vanilla's category name, the type and the registry that lists the keys.
const STAT_CATEGORIES: [(&str, StatType, &str); 9] = [
    ("minecraft:mined", StatType::Mined, "minecraft:block"),
    ("minecraft:crafted", StatType::Crafted, "minecraft:item"),
    ("minecraft:used", StatType::Used, "minecraft:item"),
    ("minecraft:broken", StatType::Broken, "minecraft:item"),
    ("minecraft:picked_up", StatType::PickedUp, "minecraft:item"),
    ("minecraft:dropped", StatType::Dropped, "minecraft:item"),
    ("minecraft:killed", StatType::Killed, "minecraft:entity_type"),
    ("minecraft:killed_by", StatType::KilledBy, "minecraft:entity_type"),
    ("minecraft:custom", StatType::Custom, "minecraft:custom_stat"),
];

/// The player's statistics as vanilla's `stats/<uuid>.json` says them (`{"stats":{"minecraft:mined":
/// {"minecraft:stone":5}}}`, counts above zero only): every key of every registry is asked, as the game
/// has no list of what a player has counted.
fn read_statistics(host: &dyn ServerHost, player: PlayerRef) -> Option<String> {
    let mut found: Vec<(&str, String, i32)> = Vec::new();
    for (category, kind, registry) in STAT_CATEGORIES {
        let Some(keys) = host.registry_entries(registry) else {
            continue;
        };
        for key in keys {
            let id = key.to_string();
            let count = host.player_stat(player, kind, &id).unwrap_or(0);
            if count > 0 {
                found.push((category, id, count));
            }
        }
    }
    Some(statistics_document(&found))
}

/// The `stats/<uuid>.json` text of counted statistics (category, key, count).
fn statistics_document(found: &[(&str, String, i32)]) -> String {
    let mut stats = serde_json::Map::new();
    for (category, key, count) in found {
        let entry = stats.entry((*category).to_owned()).or_insert_with(|| serde_json::json!({}));
        entry[key.as_str()] = serde_json::json!(count);
    }
    serde_json::json!({ "stats": stats }).to_string()
}

/// A plan's statistics (`{"stats":{category:{key:count}}}`) as (type, key, count) to set. A category the
/// game does not have, a key that is not a name and a count that is negative or too big are left out.
fn statistic_updates(plan: &serde_json::Value) -> Vec<(StatType, String, i32)> {
    let mut out = Vec::new();
    let Some(categories) = plan.get("stats").and_then(|s| s.as_object()) else {
        return out;
    };
    for (category, counts) in categories {
        let Some((_, kind, _)) = STAT_CATEGORIES.iter().find(|(name, _, _)| name == category) else {
            continue;
        };
        for (key, count) in counts.as_object().into_iter().flatten() {
            if let Some(count) = count.as_i64().and_then(|c| i32::try_from(c).ok()).filter(|c| *c >= 0) {
                out.push((*kind, key.clone(), count));
            }
        }
    }
    out
}

/// Binary NBT (big-endian, Java's file form) as the core's NBT, one tag of type `id`.
fn nbt_tag(id: u8, b: &[u8], i: &mut usize, depth: u32) -> Option<core::Nbt> {
    fn take<'a>(b: &'a [u8], i: &mut usize, n: usize) -> Option<&'a [u8]> {
        let s = b.get(*i..i.checked_add(n)?)?;
        *i += n;
        Some(s)
    }
    fn int(b: &[u8], i: &mut usize) -> Option<i32> {
        Some(i32::from_be_bytes(take(b, i, 4)?.try_into().ok()?))
    }
    fn string(b: &[u8], i: &mut usize) -> Option<String> {
        let n = u16::from_be_bytes(take(b, i, 2)?.try_into().ok()?) as usize;
        // Java's modified UTF-8: the same as UTF-8 for every text a player file holds but NUL and
        // surrogate pairs, which are kept as replacement characters.
        Some(String::from_utf8_lossy(take(b, i, n)?).into_owned())
    }
    if depth > 512 {
        return None;
    }
    let len = |b: &[u8], i: &mut usize| int(b, i).and_then(|n| usize::try_from(n).ok());
    Some(match id {
        1 => core::Nbt::Byte(take(b, i, 1)?[0] as i8),
        2 => core::Nbt::Short(i16::from_be_bytes(take(b, i, 2)?.try_into().ok()?)),
        3 => core::Nbt::Int(int(b, i)?),
        4 => core::Nbt::Long(i64::from_be_bytes(take(b, i, 8)?.try_into().ok()?)),
        5 => core::Nbt::Float(f32::from_be_bytes(take(b, i, 4)?.try_into().ok()?)),
        6 => core::Nbt::Double(f64::from_be_bytes(take(b, i, 8)?.try_into().ok()?)),
        7 => {
            let n = len(b, i)?;
            core::Nbt::ByteArray(take(b, i, n)?.iter().map(|v| *v as i8).collect())
        }
        8 => core::Nbt::String(string(b, i)?),
        9 => {
            let kind = take(b, i, 1)?[0];
            let n = len(b, i)?;
            let mut items = Vec::new();
            for _ in 0..n {
                items.push(nbt_tag(kind, b, i, depth + 1)?);
            }
            core::Nbt::List(items)
        }
        10 => {
            let mut out = core::Nbt::compound();
            loop {
                let kind = take(b, i, 1)?[0];
                if kind == 0 {
                    break;
                }
                let key = string(b, i)?;
                out.put(&key, nbt_tag(kind, b, i, depth + 1)?);
            }
            out
        }
        11 => {
            let n = len(b, i)?;
            core::Nbt::IntArray((0..n).map(|_| int(b, i)).collect::<Option<_>>()?)
        }
        12 => {
            let n = len(b, i)?;
            core::Nbt::LongArray(
                (0..n)
                    .map(|_| Some(i64::from_be_bytes(take(b, i, 8)?.try_into().ok()?)))
                    .collect::<Option<_>>()?,
            )
        }
        _ => return None,
    })
}

/// The player as their leave save wrote them, for the leave snapshot when the game no longer lets
/// the player be read: `<directory>/<uuid>.dat` (gzip NBT), and beside `directory` the
/// `stats/<uuid>.json` and `advancements/<uuid>.json` the same save writes. The same compound
/// `read_player` builds. None when the player file is missing or unreadable.
fn read_saved(directory: &std::path::Path, uuid: u128) -> Option<core::Nbt> {
    use std::io::Read;
    let id = core::util::uuid_string(uuid);
    let file = std::fs::File::open(directory.join(format!("{id}.dat"))).ok()?;
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(file).take(64 << 20).read_to_end(&mut bytes).ok()?;
    // The root: type 10, a name (empty), then the compound.
    let mut i = 1usize;
    if bytes.first() != Some(&10) {
        return None;
    }
    let name = u16::from_be_bytes(bytes.get(1..3)?.try_into().ok()?) as usize;
    i += 2 + name;
    let mut nbt = nbt_tag(10, &bytes, &mut i, 0)?;
    let parent = directory.parent()?;
    if let Ok(json) = std::fs::read_to_string(parent.join("advancements").join(format!("{id}.json"))) {
        nbt.put(core::records::RECORD_ADVANCEMENTS, core::Nbt::String(json));
    }
    if let Ok(json) = std::fs::read_to_string(parent.join("stats").join(format!("{id}.json"))) {
        nbt.put(core::records::RECORD_STATISTICS, core::Nbt::String(json));
    }
    core::player::mod_data_as_bukkit_values(&mut nbt);
    Some(nbt)
}

/// Every player on the server, read as `read_player` reads one (for stop and world save).
fn all_players(host: &mut dyn ServerHost) -> Vec<(u128, core::Nbt)> {
    let uuids: Vec<u128> = players_online(&*host).into_iter().map(|(uuid, _)| uuid).collect();
    uuids.into_iter().filter_map(|uuid| read_player(host, uuid).map(|nbt| (uuid, nbt))).collect()
}

/// Items of a plan as the game's stacks, slot by slot (`item_from_snbt`). Err names why the plan
/// cannot be applied: an item the game does not read, or an item saved with another data version
/// (the engine has no item data fixer for mods, so an older or newer item is refused, never
/// guessed at).
fn stacks(host: &dyn ServerHost, slots: &[Option<core::StoredItem>]) -> Result<Vec<Option<ItemStack>>, String> {
    let version = host.server_data_version();
    slots
        .iter()
        .enumerate()
        .map(|(slot, stored)| {
            let Some(stored) = stored else { return Ok(None) };
            if stored.data_version != version {
                return Err(format!(
                    "slot {slot}: item data version {} is not the server's {version} (no item data fixer)",
                    stored.data_version
                ));
            }
            let snbt = stored.item.to_snbt();
            host.item_from_snbt(&snbt).map(Some).ok_or_else(|| format!("slot {slot}: item not readable: {snbt}"))
        })
        .collect()
}

/// The vanilla `attributes` list of a plan as (id, base, modifiers), for `living_set_attribute_base`
/// and `living_replace_attribute_modifier`. Err for an entry or modifier that cannot be read.
fn attribute_list(list: &core::Nbt) -> Result<Vec<(String, f64, Vec<AttributeModifier>)>, String> {
    let Some(entries) = list.as_list() else {
        return Err("attributes are not a list".into());
    };
    entries
        .iter()
        .map(|e| {
            let id = e.str_at("id").ok_or("attribute without id")?.to_owned();
            let base = e.f64_at("base").ok_or_else(|| format!("{id}: no base"))?;
            let mut mods = Vec::new();
            for m in e.get("modifiers").and_then(core::Nbt::as_list).unwrap_or(&[]) {
                let mid = m.str_at("id").and_then(|i| Identifier::parse(i).ok());
                let amount = m.f64_at("amount");
                let op = match m.str_at("operation") {
                    Some("add_value") => Some(AttributeOperation::AddValue),
                    Some("add_multiplied_base") => Some(AttributeOperation::AddMultipliedBase),
                    Some("add_multiplied_total") => Some(AttributeOperation::AddMultipliedTotal),
                    _ => None,
                };
                let (Some(mid), Some(amount), Some(op)) = (mid, amount, op) else {
                    return Err(format!("{id}: unreadable modifier {}", m.to_snbt()));
                };
                mods.push(AttributeModifier::new(mid, amount, op));
            }
            Ok((id, base, mods))
        })
        .collect()
}

fn finite_or(v: f32, or: f32) -> f32 {
    if v.is_finite() { v } else { or }
}

/// Whether the player is in the world but dead (`LivingEntity.isAlive` is false) and the health data
/// type is on: HuskSync only waits for the respawn then (`user.isDead() && Identifier.HEALTH.isEnabled()`).
fn is_dead(host: &dyn ServerHost, uuid: u128) -> bool {
    core::feature_enabled("husksync:health")
        && host
            .player(uuid)
            .is_some_and(|p| LivingRef::new(p.entity()).alive(host) == Some(false))
}

/// Respawns a dead player as their own Respawn click would (`player_respawn`). False when the
/// player is alive, gone, or the engine refuses.
fn respawn(host: &mut dyn ServerHost, uuid: u128) -> bool {
    let Some(player) = host.player(uuid) else {
        return false;
    };
    player.respawn(host).is_ok()
}

/// A typed Boolean permission node, as the game's handler answers it.
fn boolean_node(name: &str) -> PermissionNode {
    PermissionNode::new(name, PermissionValueType::Boolean)
}

/// Every node `core::permission_for` can name, with its default operator level: the group nodes (the
/// command, the inventory and ender chest views), the sub-command nodes and the edit nodes. The bare
/// `/husksync` and `about` are for everyone (HuskSync's `Permission.Default.TRUE`).
fn permission_nodes() -> Vec<(String, u8)> {
    let op = core::commands::OP;
    let mut nodes: Vec<(String, u8)> = vec![("husksync.command.husksync".into(), 0)];
    for sub in ["status", "reload", "update", "dump"] {
        nodes.push((format!("husksync.command.husksync.{sub}"), op));
    }
    nodes.push(("husksync.command.userdata".into(), op));
    for sub in ["view", "list", "delete", "save", "restore", "pin", "dump"] {
        nodes.push((format!("husksync.command.userdata.{sub}"), op));
    }
    for group in ["inventory", "enderchest"] {
        nodes.push((format!("husksync.command.{group}"), op));
        // HuskSync never declares the edit nodes: Bukkit's default for an unknown node is operators.
        nodes.push((format!("husksync.command.{group}.edit"), op));
    }
    nodes
}

/// The menu's own slots (the first `rows * 9`) as the core stores them.
fn menu_items(host: &mut dyn ServerHost, menu: MenuRef, rows: u8) -> Vec<Option<core::StoredItem>> {
    let version = host.server_data_version();
    let mut out = Vec::new();
    for slot in 0..usize::from(rows) * 9 {
        let stack = ItemPlace::MenuSlot { menu, slot }.stack(host);
        out.push(stack.and_then(|stack| {
            let compound = stack.to_nbt(&*host)?;
            Some(core::StoredItem { item: to_core(&Tag::Compound(compound)), data_version: version })
        }));
    }
    out
}

/// A click in an editable menu, as vanilla's `doClick` does it for the two kinds that move items: a left or
/// right click that picks a stack up, puts the cursor's down or swaps them (a right click takes half or
/// puts one), and a shift-click that sends a stack from the menu's own slots (the first `own`) to the
/// viewer's inventory part and the other way. Everything in one transaction, so a click that cannot be
/// carried out whole changes nothing. Drags, number keys, throws and double clicks do nothing. True when
/// something moved.
fn edit_click(host: &mut dyn ServerHost, menu: MenuRef, own: usize, slot: usize, button: i32, mode: i32) -> bool {
    let place = ItemPlace::MenuSlot { menu, slot };
    let cursor = menu.cursor();
    let moved: Result<(), ()> = host.transaction(|host, tx| {
        let here = place.stack(host);
        if mode == 1 {
            let Some(stack) = here else { return Err(()) };
            let resource = ItemResource::of(&stack);
            let targets = if slot < own { own..own + 36 } else { 0..own };
            let mut left = stack.count();
            for target in targets {
                if left == 0 {
                    break;
                }
                left -= ItemPlace::MenuSlot { menu, slot: target }.insert(host, tx, &resource, left).min(left);
            }
            let sent = stack.count() - left;
            return if sent > 0 && place.extract(host, tx, &resource, sent) == sent { Ok(()) } else { Err(()) };
        }
        let held = cursor.stack(host);
        match (held, here) {
            (None, None) => Err(()),
            // Pick up: all of the slot's stack, or half of it (rounded up) on a right click.
            (None, Some(stack)) => {
                let take = if button == 1 { stack.count().div_ceil(2) } else { stack.count() };
                let resource = ItemResource::of(&stack);
                let ok = place.extract(host, tx, &resource, take) == take && cursor.insert(host, tx, &resource, take) == take;
                if ok { Ok(()) } else { Err(()) }
            }
            // Put down onto an empty slot or the same stack: all of the cursor's, or one on a right click.
            (Some(carried), None) => put_down(host, tx, &place, &cursor, &carried, button),
            (Some(carried), Some(stack)) if carried.same_item(&stack) && carried.same_components(&stack) => {
                put_down(host, tx, &place, &cursor, &carried, button)
            }
            // Another stack: swap.
            (Some(carried), Some(stack)) => {
                let (carried_resource, resource) = (ItemResource::of(&carried), ItemResource::of(&stack));
                let (carried_count, count) = (carried.count(), stack.count());
                let ok = place.extract(host, tx, &resource, count) == count
                    && cursor.extract(host, tx, &carried_resource, carried_count) == carried_count
                    && place.insert(host, tx, &carried_resource, carried_count) == carried_count
                    && cursor.insert(host, tx, &resource, count) == count;
                if ok { Ok(()) } else { Err(()) }
            }
        }
    });
    moved.is_ok()
}

/// Moves the cursor's stack (or one of it on a right click) into the slot as far as the slot takes it.
fn put_down(
    host: &mut dyn ServerHost,
    tx: &mod_api::Transaction,
    place: &ItemPlace,
    cursor: &ItemPlace,
    carried: &ItemStack,
    button: i32,
) -> Result<(), ()> {
    let resource = ItemResource::of(carried);
    let put = if button == 1 { 1 } else { carried.count() };
    let placed = place.insert(host, tx, &resource, put);
    if placed > 0 && cursor.extract(host, tx, &resource, placed) == placed { Ok(()) } else { Err(()) }
}

/// Item stacks as the core stores them: the vanilla item compound at the server's data version. A stack
/// the game cannot write as a compound is left out.
fn stored_items<'a>(host: &dyn ServerHost, stacks: impl Iterator<Item = &'a ItemStack>) -> Vec<core::StoredItem> {
    let version = host.server_data_version();
    stacks
        .filter_map(|stack| {
            let compound = stack.to_nbt(host)?;
            Some(core::StoredItem { item: to_core(&Tag::Compound(compound)), data_version: version })
        })
        .collect()
}

/// Carries out an apply plan on a player who is on, in HuskSync's dependency order. False when the
/// plan cannot be applied or the player is not here. Everything that can be refused is checked
/// before the first write (items readable at the server's data version, attributes readable); the
/// persistent data and the advancements are written first, and if the engine refuses one, nothing
/// else of the plan is written.
fn apply(host: &mut dyn ServerHost, uuid: u128, plan: &core::ApplyPlan) -> bool {
    let refuse = |why: String| {
        log(&format!("snapshot not applied for {uuid:032x}: {why}"));
        false
    };
    let inventory = match &plan.inventory {
        Some((slots, held)) => match stacks(&*host, slots) {
            Ok(stacks) => Some((stacks, *held)),
            Err(why) => return refuse(format!("inventory {why}")),
        },
        None => None,
    };
    let ender_chest = match &plan.ender_chest {
        Some(slots) => match stacks(&*host, slots) {
            Ok(stacks) => Some(stacks),
            Err(why) => return refuse(format!("ender chest {why}")),
        },
        None => None,
    };
    let attributes = match &plan.attributes {
        Some(list) => match attribute_list(list) {
            Ok(list) => Some(list),
            Err(why) => return refuse(format!("attributes: {why}")),
        },
        None => None,
    };
    let Some(player) = host.player(uuid) else {
        return false;
    };
    if let Some(data) = &plan.persistent_data {
        let Some(compound) = persistent_compound(data) else {
            log(&format!("persistent data not applied for {uuid:032x}: not a compound"));
            return false;
        };
        if player.set_persistent_data(host, compound).is_err() {
            log(&format!("persistent data not applied for {uuid:032x}: refused by the engine"));
            return false;
        }
    }
    // HMOD-GAP(player-advancements): Horizon main has no per-player advancement write. The plan's
    // advancements are not applied; the mod loses that part of the sync.
    // Statistics: each saved count replaces the player's (`Player.resetStat`, then `awardStat`); a statistic the
    // snapshot does not list keeps its count, as Bukkit's `setStatistic` loop does.
    if let Some(stats) = &plan.statistics {
        for (kind, key, count) in statistic_updates(stats) {
            if !host.player_reset_stat(player, kind, &key) {
                log(&format!("statistic {key} not written for {uuid:032x}"));
                continue;
            }
            if count > 0 && !host.player_award_stat(player, kind, &key, count) {
                log(&format!("statistic {key} not awarded for {uuid:032x}"));
            }
        }
    }
    // Inventory (all 43 vanilla slots, empty ones cleared) and the held slot, then the ender chest.
    if let Some((stacks, held)) = inventory {
        for (slot, stack) in stacks.into_iter().enumerate() {
            if player.set_inventory_slot(host, slot as u32, stack).is_err() {
                log(&format!("inventory slot {slot} not written for {uuid:032x}"));
            }
        }
        player.set_selected_slot(host, held.clamp(0, 8) as usize);
    }
    if let Some(stacks) = ender_chest {
        for (slot, stack) in stacks.into_iter().enumerate() {
            if player.set_ender_chest_slot(host, slot as u32, stack).is_err() {
                log(&format!("ender chest slot {slot} not written for {uuid:032x}"));
            }
        }
    }
    let living = LivingRef::new(player.entity());
    if let Some(effects) = &plan.effects {
        // Replace all effects, as HuskSync's `setActivePotionEffects` does.
        living.remove_all_effects(host);
        for e in effects {
            // TODO(verify): the registry accessor for effect keys; `effect_key` is in arismoko's keys host.
            let Some(id) = mod_api::__private::effect_key(&e.kind) else {
                log(&format!("unknown effect {} not applied", e.kind));
                continue;
            };
            let effect = mod_api::ActiveEffect::new(
                id,
                e.duration,
                e.amplifier.clamp(0, 255),
                e.is_ambient,
                e.show_particles,
                // Horizon has no switch for the icon; HuskSync's default is shown.
                true,
            );
            living.add_effect(host, &effect);
        }
    }
    // Attributes as vanilla's AttributeMap.load reads the list: the base, then each saved modifier
    // (permanent, replacing one with the same id). Before health: max_health caps it.
    if let Some(attributes) = attributes {
        for (id, base, modifiers) in attributes {
            if !living.set_attribute_base(host, &id, base) {
                log(&format!("attribute {id} not applied for {uuid:032x}"));
                continue;
            }
            for m in modifiers {
                living.replace_attribute_modifier(host, &id, m, ModifierLifetime::Permanent);
            }
        }
    }
    if let Some(mode) = plan.game_mode {
        let mode = match mode.clamp(0, 3) {
            0 => GameMode::Survival,
            1 => GameMode::Creative,
            2 => GameMode::Adventure,
            _ => GameMode::Spectator,
        };
        player.set_game_mode(host, mode);
    }
    // Flight after the game mode: FlightStatus depends on it.
    if let Some(f) = &plan.flight {
        if let Some(mut abilities) = player.abilities(&*host) {
            abilities.may_fly = f.allow_flight;
            abilities.flying = f.allow_flight && f.is_flying;
            player.set_abilities(host, abilities);
        }
    }
    // Food and experience before health (a max-health cap may lower it).
    if let Some(f) = &plan.hunger {
        player.set_food_level(host, f.food_level.clamp(0, 20));
        player.set_saturation(host, finite_or(f.saturation, 0.0));
        player.set_exhaustion(host, finite_or(f.exhaustion, 0.0));
    }
    if let Some(x) = &plan.experience {
        // arismoko sets the level and the progress as points of the level's bar (not the total):
        // progress * the points this level needs. The total follows from the level.
        player.set_experience_levels(host, x.exp_level.max(0));
        let needed = player.xp_needed_for_next_level(&*host).unwrap_or(0).max(0) as f32;
        let points = (finite_or(x.exp_progress, 0.0).clamp(0.0, 1.0) * needed).floor() as i32;
        player.set_experience_points(host, points);
    }
    if let Some(h) = &plan.health {
        if (h.health as f32).is_finite() {
            living.set_health(host, (h.health as f32).clamp(0.0, 1024.0));
        }
    }
    if let Some((dim, l)) = &plan.location {
        player.entity().teleport_to_level(host, dim.as_str(), [l.x, l.y, l.z], Some([l.yaw, l.pitch]));
    }
    true
}

/// Shows a line to a player: the action bar or chat.
fn show(host: &mut dyn ServerHost, uuid: u128, line: Text, overlay: bool) {
    let Some(player) = host.player(uuid) else {
        return;
    };
    if overlay {
        let _ = player.action_bar(host, line);
    } else {
        let _ = player.tell(host, line);
    }
}

/// The command line as the player typed it, rebuilt from the parsed invocation: the literal path,
/// then the arguments in parse order. arismoko has no raw-line accessor (TODO(arismoko-api:
/// command_raw_line)); this matches what the core's parser needs for the usages it registers.
fn command_line(host: &dyn ServerHost, command: &CommandInvocation) -> String {
    let mut words = command.literals.clone();
    for argument in &command.arguments {
        let text = match &argument.value {
            CommandArgumentValue::String(s) => s.clone(),
            CommandArgumentValue::Integer(i) => i.to_string(),
            CommandArgumentValue::Number(n) => n.to_string(),
            CommandArgumentValue::Boolean(b) => b.to_string(),
            CommandArgumentValue::Player(p) => p.name(host).unwrap_or_default(),
            _ => continue,
        };
        words.push(text);
    }
    words.join(" ")
}

/// Holds a command or action of a player whose data is still loading (BukkitLockedEventListener).
fn acting_flow(host: &dyn ServerHost) -> Flow {
    let locked = host.players().acting_player().is_some_and(|p| core::is_locked(p.uuid()));
    if locked { Flow::Cancel } else { Flow::Continue }
}

fn players_online(host: &dyn ServerHost) -> Vec<(u128, i32)> {
    host.players()
        .server_players()
        .into_iter()
        .filter(|p: &PlayerRef| p.is_fake(host) != Some(true))
        .map(|p| (p.uuid(), p.entity().id() as i32))
        .collect()
}

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register_configs(&self, _: HostSide, event: &mut RegisterConfigsEvent<'_>) {
        // The gate: off unless enabled, written with the default on the first start.
        let mut gate = Config::new(ID, ConfigKind::Common, GATE_TEXT);
        // arismoko refuses a config path with a folder in it, so the gate is `config/husksync-mod.toml`.
        gate.file = "husksync-mod.toml".to_string();
        event.configs.push(gate);
        // The core's own `config/horizonsync/config.toml` is written by this mod at start when missing
        // (with every host set to disabled.invalid), never by the core's defaults (localhost).
    }

    fn server_about_to_start(&self, _: &mut dyn ServerHost, event: &mut ServerAboutToStartEvent) {
        if !event.dedicated {
            return;
        }
        if !enabled() {
            log("off: config/husksync-mod.toml does not say enabled = true");
            return;
        }
        let Ok(cwd) = std::env::current_dir() else {
            log("no working folder; not started");
            return;
        };
        let config_dir = cwd.join("config");
        let core_file = config_dir.join("horizonsync").join("config.toml");
        if !core_file.exists() {
            let written = std::fs::create_dir_all(config_dir.join("horizonsync"))
                .and_then(|()| std::fs::write(&core_file, disabled_core_config()));
            if let Err(e) = written {
                log(&format!("cannot write {}: {e}; not started", core_file.display()));
                return;
            }
        }
        let folder = cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "server".into());
        core::start(&config_dir, &folder);
    }

    // The leave snapshot (HuskSync's DISCONNECT): `player_logged_out` comes after the player left the
    // list, but the game saves a leaving player first (`player_save_to_file`): the player is read there,
    // from the file that save wrote (the game no longer lets a leaving player be read). Only a read of
    // the same tick counts (see `saved_now`).
    fn player_save_to_file(&self, host: &mut dyn ServerHost, event: &mut PlayerSaveToFileEvent<'_>) {
        if !core::running() {
            return;
        }
        // A leaving player is saved after the game stopped letting them be read: then the file the
        // save just wrote is read instead.
        let read = read_player(host, event.player).or_else(|| read_saved(event.directory, event.player));
        if let Some(nbt) = read {
            let mut saved = self.saved_now.lock().unwrap_or_else(|e| e.into_inner());
            saved.retain(|(uuid, _)| *uuid != event.player);
            saved.push((event.player, nbt));
        }
    }

    fn player_logged_out(&self, _: &mut dyn ServerHost, event: &mut PlayerLoggedOutEvent<'_>) {
        if !core::running() {
            return;
        }
        let mut saved = self.saved_now.lock().unwrap_or_else(|e| e.into_inner());
        match saved.iter().position(|(uuid, _)| *uuid == event.player) {
            Some(i) => {
                let (uuid, nbt) = saved.remove(i);
                drop(saved);
                core::on_leave(uuid, &nbt);
            }
            None => log(&format!("no leave snapshot for {}: the player was not read at their last save", event.name)),
        }
    }

    fn server_stopping(&self, host: &mut dyn ServerHost, _: &mut ServerStoppingEvent) {
        // The stop is told with the players still listed (player_logged_out hook doc), so their data
        // is read now and saved with SERVER_SHUTDOWN.
        if core::running() {
            core::on_server_stopping(all_players(host));
        }
    }

    fn world_saved(&self, host: &mut dyn ServerHost, _: &mut WorldSavedEvent) {
        if core::running() {
            core::on_world_save(all_players(host));
        }
    }

    fn player_logged_in(&self, _: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>) {
        // Fields `player` and `name` follow the hook's documented arguments; not compiled yet.
        core::on_join(event.player, &event.name);
    }

    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if !matches!(event.phase, ServerTickPhase::ServerTickStart { world_running: true, .. }) {
            return;
        }
        if !core::running() {
            return;
        }
        self.saved_now.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let players = players_online(&*host);
        let deferred = std::mem::take(&mut *self.deferred.lock().unwrap_or_else(|e| e.into_inner()));
        for mut d in deferred {
            if is_dead(&*host, d.player) && d.waited < RESPAWN_WAIT_TICKS {
                d.waited += 1;
                self.deferred.lock().unwrap_or_else(|e| e.into_inner()).push(d);
            } else {
                if is_dead(&*host, d.player) {
                    // The player did not click Respawn in time: respawn them, then apply.
                    respawn(host, d.player);
                }
                let ok = apply(host, d.player, &d.plan);
                core::applied(d.token, ok);
            }
        }
        for action in core::tick(&players) {
            match action {
                core::Action::Apply { player, plan, token } => {
                    if is_dead(&*host, player) {
                        self.deferred
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(Deferred { player, plan, token, waited: 0 });
                    } else {
                        let ok = apply(host, player, &plan);
                        core::applied(token, ok);
                    }
                }
                core::Action::Capture { player, token } => {
                    let data = read_player(host, player);
                    core::captured(token, data);
                }
                core::Action::Message { player: Some(p), text, overlay } => show(host, p, Text::literal(text), overlay),
                core::Action::Message { player: None, text, .. } => log(&text),
                // The JSON component as the game reads it (`text_from_json`); plain text if it does not parse.
                core::Action::Component { player: Some(p), text, json, overlay } => {
                    let line = match host.text_from_json(&json.to_string()) {
                        Ok(Some(line)) => line,
                        _ => Text::literal(text),
                    };
                    show(host, p, line, overlay);
                }
                core::Action::Component { player: None, text, .. } => log(&text),
                // The /inventory and /enderchest views as a native chest menu (`item_from_snbt`). A menu its viewer
                // may edit is remembered: `menu_clicked` carries the clicks out, `menu_closed` saves the contents.
                core::Action::OpenMenu { viewer, title, rows, items, edit } => {
                    let rows = rows.clamp(1, 6);
                    let stacks: Vec<Option<ItemStack>> = items
                        .iter()
                        .map(|i| i.as_ref().and_then(|i| host.item_from_snbt(&i.item.to_snbt())))
                        .collect();
                    match host.open_menu(viewer, rows, Text::literal(title), stacks, MenuReplace::CloseFirst) {
                        Ok(Some(window)) => {
                            if let Some(edit) = edit {
                                self.edit_menus.lock().unwrap_or_else(|e| e.into_inner()).push(EditMenu {
                                    viewer,
                                    window,
                                    edit,
                                    rows,
                                    items,
                                    changed: false,
                                });
                            }
                        }
                        Ok(None) | Err(_) => log(&format!("menu for {viewer:032x} not opened")),
                    }
                }
            }
        }
    }

    // The clicks of an editable `/inventory` or `/enderchest` menu (`InventoryCommand` lets its viewer
    // take and put items); the menu moves nothing itself, so they are carried out here.
    fn menu_clicked(&self, host: &mut dyn ServerHost, event: &mut MenuClickedEvent) {
        if !matches!(event.mode, 0 | 1) || !matches!(event.button, 0 | 1) || event.slot < 0 {
            return;
        }
        let rows = {
            let menus = self.edit_menus.lock().unwrap_or_else(|e| e.into_inner());
            match menus.iter().find(|m| m.viewer == event.uuid && m.window == event.window) {
                Some(m) => m.rows,
                None => return,
            }
        };
        let own = usize::from(rows) * 9;
        let slot = event.slot as usize;
        if slot >= own + 36 {
            return;
        }
        let Some(menu) = host.menu(event.uuid, event.window) else {
            return;
        };
        if !edit_click(host, menu, own, slot, event.button, event.mode) {
            return;
        }
        let items = menu_items(host, menu, rows);
        let mut menus = self.edit_menus.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = menus.iter_mut().find(|m| m.viewer == event.uuid && m.window == event.window) {
            m.items = items;
            m.changed = true;
        }
    }

    // The viewer closed an editable menu (or it was replaced, or they left): the new contents become a snapshot.
    fn menu_closed(&self, _: &mut dyn ServerHost, event: &mut MenuClosedEvent) {
        let closed = {
            let mut menus = self.edit_menus.lock().unwrap_or_else(|e| e.into_inner());
            menus.iter().position(|m| m.viewer == event.uuid && m.window == event.window).map(|i| menus.remove(i))
        };
        if let Some(menu) = closed {
            if menu.changed {
                core::on_menu_edited(menu.edit, menu.items);
            }
        }
    }

    fn permission_gather_nodes(&self, event: &mut PermissionGatherNodesEvent<'_>) {
        for (node, level) in permission_nodes() {
            let _ = event.nodes.add(
                boolean_node(&node),
                NodeDefault::Level(PermissionLevel::from_level(i32::from(level))),
            );
        }
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        // The public usages (`horizonsync`, `horizonsync about`) and the console-only ones (level 4) are open.
        // Every operator usage carries HuskSync's node (`horizonsync.command.*`, `permission_for`): the engine
        // leaves it out of the tab list of a player without the node (as Bukkit hides a command whose node the
        // sender lacks), and `player_command` answers the typed line with HuskSync's own denial.
        for (usage, level) in core::command_usages() {
            // arismoko has no `player_name` argument kind; the core reads the name from the text anyway.
            let usage = usage.replace(":player_name>", ":word>");
            if level == 0 || level >= 4 {
                event.commands.push(Command::new(&usage));
                continue;
            }
            let literals: Vec<&str> = usage.split_whitespace().take_while(|w| !w.starts_with('<') && !w.starts_with('[')).collect();
            match core::permission_for(&literals.join(" ")) {
                Some((node, _)) => event.commands.push(Command::op(&usage, level).with_node(&boolean_node(&node))),
                None => event.commands.push(Command::new(&usage)),
            }
        }
    }

    fn run_mod_command(&self, host: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
        let Some(label) = event.command.literals.first() else {
            return Flow::Continue;
        };
        if !labels().iter().any(|l| l == label) {
            return Flow::Continue;
        }
        let line = command_line(&*host, &event.command);
        let Some((node, _)) = core::permission_for(&line) else {
            return Flow::Continue;
        };
        let edit_node = core::edit_permission_for(&line);
        let (uuid, name, granted, edit, edit_all, dump) = match &event.source {
            CommandSource::Player { player, .. } => {
                // The node's default (`permission_gather_nodes`) decides unless the handler answers; the
                // bare command defaults to everyone. Any error is a denial.
                let mut has = |n: &str| player.permission_bool(&mut *host, &boolean_node(n)).unwrap_or(false);
                let granted = has(&node);
                let edit = match &edit_node {
                    Some(n) => has(n),
                    None => false,
                };
                // `/userdata view` shows the item buttons to who may edit both, the dump buttons to who may dump.
                let edit_all = has("husksync.command.inventory.edit") && has("husksync.command.enderchest.edit");
                let dump = has("husksync.command.userdata.dump");
                (Some(player.uuid()), player.name(&*host).unwrap_or_default(), granted, edit, edit_all, dump)
            }
            CommandSource::Console { .. } => (None, "CONSOLE".to_string(), true, edit_node.is_some(), true, true),
            // Command blocks and functions are not HuskSync's senders.
            _ => return Flow::Cancel,
        };
        // The core refuses a player below its operator level (2): 4 when the node is granted, 0 when
        // not, so the core answers the denial itself.
        let level = if granted { 4 } else { 0 };
        let sender = core::Sender { uuid, name, level, edit, edit_all, dump };
        if core::perform_command(sender, &line) {
            return Flow::Cancel;
        }
        // Off (`enabled` not `true`): answer instead of "unknown command".
        if !core::is_running() {
            let _ = host.commands().feedback(Text::literal(
                "HuskSync is switched off on this server (enabled = false). Nothing was run.",
            ));
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn player_command(&self, host: &mut dyn ServerHost, event: &mut PlayerCommandEvent<'_>) -> Flow {
        // Fields `player` and `command` follow the hook's documented arguments; not compiled yet.
        // blacklisted_commands_while_locked: a player whose data is loading may not run commands.
        if core::refuse_command(event.player, event.command) {
            return Flow::Cancel;
        }
        // An operator usage the player lacks the node for is hidden from their tree: answer HuskSync's denial
        // here (also while the mod is off), not the engine's "unknown command". The console is never asked.
        let line = event.command.trim_start_matches('/').to_owned();
        let root = line.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        let sub = line.split_whitespace().nth(1).unwrap_or("").to_ascii_lowercase();
        if !labels().iter().any(|l| *l == root) || matches!(sub.as_str(), "forceupgrade" | "migrate") {
            return Flow::Continue;
        }
        let Some((node, required)) = core::permission_for(&line) else {
            return Flow::Continue;
        };
        if required == 0 {
            return Flow::Continue;
        }
        let allowed = match host.player(event.player) {
            Some(p) => p.permission_bool(&mut *host, &boolean_node(&node)).unwrap_or(false),
            None => false,
        };
        if allowed {
            return Flow::Continue;
        }
        event.cancelled = true;
        for line in core::locale_lines("error_no_permission", &[]) {
            let text = match host.text_from_json(&line.component.to_string()) {
                Ok(Some(text)) => text,
                _ => Text::literal(line.plain),
            };
            event.replies.push(text);
        }
        Flow::Cancel
    }

    // Save on death (HuskSync's `saveOnPlayerDeath`) with `items_to_save: DROPS`: the stacks the player
    // drops. A locked player drops nothing (`cancelPlayerEvent` clears the drops).
    fn living_drops(&self, host: &mut dyn ServerHost, event: &mut LivingDropsEvent<'_>) -> Flow {
        let Some(player) = event.player else {
            return Flow::Continue;
        };
        if !core::running() {
            return Flow::Continue;
        }
        if core::is_locked(player) {
            event.drops.clear();
            return Flow::Continue;
        }
        if core::death_saves_kept_items() {
            return Flow::Continue;
        }
        let items = stored_items(&*host, event.drops.iter().map(|drop| &drop.stack));
        if let Some(nbt) = read_player(host, player) {
            core::on_death(player, &nbt, items);
        }
        Flow::Continue
    }

    // `items_to_save: ITEMS_TO_KEEP`: what the player keeps, which is the whole inventory when
    // `keep_inventory` is on and nothing otherwise (Paper's `getItemsToKeep`). The inventory is still
    // intact where `ServerPlayer.die` starts.
    fn player_died(&self, host: &mut dyn ServerHost, event: &mut PlayerDiedEvent<'_>) {
        if !core::running() || !core::death_saves_kept_items() {
            return;
        }
        let Some(nbt) = read_player(host, event.player) else {
            return;
        };
        let keep = matches!(host.game_rule("keep_inventory"), Ok(GameRuleValue::Bool(true)));
        let items: Vec<core::StoredItem> =
            if keep { core::player::inventory_slots(&nbt).into_iter().flatten().collect() } else { Vec::new() };
        core::on_death(event.player, &nbt, items);
    }

    // ----- the lock while a player's data loads (BukkitLockedEventListener) -----

    fn break_block(&self, host: &mut dyn ServerHost, _: &mut BreakBlockEvent<'_>) -> Flow {
        acting_flow(&*host)
    }

    fn left_click_block(&self, host: &mut dyn ServerHost, _: &mut LeftClickBlockEvent) -> Flow {
        acting_flow(&*host)
    }

    fn right_click_block(&self, host: HostSide<'_>, _: &mut RightClickBlockEvent<'_>) -> Flow {
        match host {
            HostSide::Server(host) => acting_flow(&*host),
            _ => Flow::Continue,
        }
    }

    // Placing a block against air and clicking a block to place are both refused here; the
    // use-on hook covers the second case as before.
    fn use_item_on_block(&self, host: HostSide<'_>, _: &mut UseItemOnBlockEvent<'_>) -> Flow {
        match host {
            HostSide::Server(host) => acting_flow(&*host),
            _ => Flow::Continue,
        }
    }

    fn entity_place(&self, host: &mut dyn ServerHost, _: &mut EntityPlaceEvent) -> Flow {
        acting_flow(&*host)
    }

    fn attack_entity(&self, host: &mut dyn ServerHost, _: &mut AttackEntityEvent<'_>) -> Flow {
        acting_flow(&*host)
    }

    fn entity_interact(&self, host: HostSide<'_>, _: &mut EntityInteractEvent<'_>) -> Option<InteractionResult> {
        match host {
            HostSide::Server(host) => match acting_flow(&*host) {
                Flow::Cancel => Some(InteractionResult::Fail),
                _ => None,
            },
            _ => None,
        }
    }

    fn armor_stand_manipulate(&self, host: &mut dyn ServerHost, _: &mut ArmorStandManipulateEvent<'_>) -> Flow {
        acting_flow(&*host)
    }

    // Upstream made this a listener (`can_pickup`): False leaves the item for a locked player.
    fn item_picked_up(&self, host: &mut dyn ServerHost, event: &mut ItemPickedUpEvent<'_>) {
        if matches!(acting_flow(&*host), Flow::Cancel) {
            *event.can_pickup = TriState::False;
        }
    }

    fn container_opened(&self, host: &mut dyn ServerHost, _: &mut ContainerOpenedEvent<'_>) -> Flow {
        acting_flow(&*host)
    }

    fn item_toss(&self, host: &mut dyn ServerHost, _: &mut ItemTossEvent) -> Flow {
        acting_flow(&*host)
    }

    // A locked player takes no damage (`BukkitLockedEventListener.onPlayerTakeDamage`). The core keeps the
    // entity ids of the locked players (`tick`).
    fn living_incoming_damage(&self, _: &mut dyn LevelRandomHost, event: &mut LivingIncomingDamageEvent<'_>) -> Flow {
        if event.kind == "minecraft:player" && core::is_locked_entity(event.entity) { Flow::Cancel } else { Flow::Continue }
    }

    // HMOD-GAP(locked-projectile-cancel): `player_launch_projectile` and `projectile_launch` only listen, after the
    // projectile exists; removing it would also lose the thrown item, which HuskSync's cancel gives back. A locked
    // player's ender pearl or arrow is not refused (`core::refuse_launch` is ready for a cancelling hook).
    // HMOD-GAP(locked-inventory-click): no hook answers a click in the player's own inventory screen, which
    // `BukkitLockedEventListener.onInventoryClick` cancels; only `container_opened` is refused.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics_become_the_vanilla_document_and_back() {
        let doc = statistics_document(&[
            ("minecraft:mined", "minecraft:stone".into(), 5),
            ("minecraft:custom", "minecraft:jump".into(), 3),
            ("minecraft:mined", "minecraft:dirt".into(), 1),
        ]);
        let v: serde_json::Value = serde_json::from_str(&doc).unwrap();
        assert_eq!(v["stats"]["minecraft:mined"]["minecraft:stone"], 5);
        assert_eq!(v["stats"]["minecraft:mined"]["minecraft:dirt"], 1);
        assert_eq!(v["stats"]["minecraft:custom"]["minecraft:jump"], 3);
        assert_eq!(statistics_document(&[]), r#"{"stats":{}}"#);
        let updates = statistic_updates(&v);
        assert_eq!(updates.len(), 3);
        assert!(updates.contains(&(StatType::Custom, "minecraft:jump".to_owned(), 3)));
        assert!(updates.contains(&(StatType::Mined, "minecraft:stone".to_owned(), 5)));
    }

    #[test]
    fn updates_skip_what_the_game_cannot_take() {
        let plan = serde_json::json!({ "stats": {
            "minecraft:used": { "minecraft:bread": 2, "minecraft:neg": -1, "minecraft:huge": 99999999999i64 },
            "minecraft:unknown": { "minecraft:x": 1 }
        }, "DataVersion": 5023 });
        assert_eq!(statistic_updates(&plan), vec![(StatType::Used, "minecraft:bread".to_owned(), 2)]);
        assert!(statistic_updates(&serde_json::json!({})).is_empty());
    }
}
