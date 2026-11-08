//! Between a player's vanilla NBT (what the server writes to `playerdata/<uuid>.dat`) and a
//! HuskSync snapshot's data.
//!
//! Capturing reads the same fields Bukkit's getters read on Paper (`BukkitData.*.adapt`), so a
//! Horizon snapshot says what a Paper one would for the same player. Applying gives the engine a
//! plan, in HuskSync's dependency order; the engine side (`glue/horizonsync.rs`) carries it out.

use crate::config::Config;
use crate::data::{self, *};
use crate::nbt::{parse_snbt, Nbt};
use crate::records;
use crate::snapshot::keys;
use std::collections::BTreeMap;

/// Horizon's own data version (`horizon_core::version::DATA_VERSION`, Minecraft 26.3).
pub const DATA_VERSION: i32 = 5023;

/// `equipment` compound names in Bukkit's 36..41 order.
const EQUIPMENT: [(&str, usize); 5] =
    [("feet", 36), ("legs", 37), ("chest", 38), ("head", 39), ("offhand", 40)];

fn strip_slot(item: &Nbt) -> Option<StoredItem> {
    let mut item = item.clone();
    item.remove("Slot");
    let data_version = item
        .remove("DataVersion")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32)
        .unwrap_or(DATA_VERSION);
    let id = item.str_at("id")?;
    if id == "minecraft:air" || item.i64_at("count").is_some_and(|c| c <= 0) {
        return None;
    }
    Some(StoredItem { item, data_version })
}

/// The 41 slots of a vanilla player compound: `Inventory` (0..36; also the pre-1.21.5 armour
/// slots 100..103 and off hand -106) and `equipment`.
pub fn inventory_slots(player: &Nbt) -> Vec<Option<StoredItem>> {
    let mut slots = vec![None; INVENTORY_SLOTS];
    if let Some(list) = player.get("Inventory").and_then(Nbt::as_list) {
        for it in list {
            let slot = it.i64_at("Slot").unwrap_or(-1) as i8;
            let index = match slot {
                0..=35 => slot as usize,
                100 => 36,
                101 => 37,
                102 => 38,
                103 => 39,
                -106 => 40,
                _ => continue,
            };
            slots[index] = strip_slot(it);
        }
    }
    if let Some(eq) = player.get("equipment") {
        for (name, index) in EQUIPMENT {
            if let Some(it) = eq.get(name) {
                slots[index] = strip_slot(it);
            }
        }
    }
    slots
}

pub fn ender_slots(player: &Nbt) -> Vec<Option<StoredItem>> {
    let mut slots = vec![None; ENDER_CHEST_SLOTS];
    if let Some(list) = player.get("EnderItems").and_then(Nbt::as_list) {
        for it in list {
            let slot = it.i64_at("Slot").unwrap_or(-1);
            if (0..ENDER_CHEST_SLOTS as i64).contains(&slot) {
                slots[slot as usize] = strip_slot(it);
            }
        }
    }
    slots
}

pub fn effects(player: &Nbt) -> Vec<Effect> {
    let mut out = Vec::new();
    if let Some(list) = player.get("active_effects").and_then(Nbt::as_list) {
        for e in list {
            let Some(id) = e.str_at("id") else { continue };
            let ambient = e.i64_at("ambient").unwrap_or(0) != 0;
            // BukkitData.PotionEffects.from leaves out ambient (beacon) effects.
            if ambient {
                continue;
            }
            // MobEffectInstance.Details: amplifier is an unsigned byte.
            let amplifier = match e.get("amplifier") {
                Some(Nbt::Byte(b)) => *b as u8 as i32,
                other => other.and_then(Nbt::as_i64).unwrap_or(0) as i32,
            };
            let particles = e.i64_at("show_particles").is_none_or(|v| v != 0);
            out.push(Effect {
                kind: if id.contains(':') { id.to_owned() } else { format!("minecraft:{id}") },
                amplifier,
                duration: e.i64_at("duration").unwrap_or(0) as i32,
                is_ambient: false,
                show_particles: particles,
                has_icon: e.i64_at("show_icon").map_or(particles, |v| v != 0),
            });
        }
    }
    out
}

/// What `UserDataHolder.getData` returns on Paper for this player, as snapshot strings: the
/// enabled built-in types this port can read from the player compound.
pub fn capture(player: &Nbt, config: &Config, dimension_hint: Option<&str>) -> BTreeMap<String, String> {
    let f = &config.synchronization.features;
    let mut out = BTreeMap::new();
    if f.inventory {
        let inv = Inventory {
            slots: inventory_slots(player),
            held_item_slot: player.i64_at("SelectedItemSlot").unwrap_or(0).clamp(0, 8) as i32,
        };
        out.insert(keys::INVENTORY.into(), inv.serialize());
    }
    if f.ender_chest {
        out.insert(keys::ENDER_CHEST.into(), EnderChest { slots: ender_slots(player) }.serialize());
    }
    if f.potion_effects {
        out.insert(keys::POTION_EFFECTS.into(), data::to_json(&effects(player)));
    }
    if f.health {
        let h = Health {
            health: player.f64_at("Health").unwrap_or(20.0) as f32 as f64,
            // CraftPlayer's defaults: a scale of 20, not scaled.
            health_scale: 20.0,
            is_health_scaled: false,
        };
        out.insert(keys::HEALTH.into(), data::to_json(&h));
    }
    if f.hunger {
        let h = Hunger {
            food_level: player.i64_at("foodLevel").unwrap_or(20) as i32,
            saturation: player.f64_at("foodSaturationLevel").unwrap_or(5.0) as f32,
            exhaustion: player.f64_at("foodExhaustionLevel").unwrap_or(0.0) as f32,
        };
        out.insert(keys::HUNGER.into(), data::to_json(&h));
    }
    if f.experience {
        let x = Experience {
            total_experience: player.i64_at("XpTotal").unwrap_or(0) as i32,
            exp_level: player.i64_at("XpLevel").unwrap_or(0) as i32,
            exp_progress: player.f64_at("XpP").unwrap_or(0.0) as f32,
        };
        out.insert(keys::EXPERIENCE.into(), data::to_json(&x));
    }
    if f.game_mode {
        let id = player.i64_at("playerGameType").unwrap_or(0) as i32;
        out.insert(keys::GAME_MODE.into(), data::to_json(&GameMode::from_id(id)));
    }
    if f.flight_status {
        let abilities = player.get("abilities");
        let allow = abilities.and_then(|a| a.i64_at("mayfly")).unwrap_or(0) != 0;
        let flying = abilities.and_then(|a| a.i64_at("flying")).unwrap_or(0) != 0;
        let fs = FlightStatus { allow_flight: allow, is_flying: allow && flying };
        out.insert(keys::FLIGHT_STATUS.into(), data::to_json(&fs));
    }
    // Added to the compound by the game side from `Host::player_advancements` / `player_stats`.
    if f.advancements {
        if let Some(v) = player.str_at(records::RECORD_ADVANCEMENTS).and_then(|s| serde_json::from_str(s).ok()) {
            out.insert(keys::ADVANCEMENTS.into(), records::advancements_from_vanilla(&v));
        }
    }
    if f.statistics {
        if let Some(v) = player.str_at(records::RECORD_STATISTICS).and_then(|s| serde_json::from_str(s).ok()) {
            out.insert(keys::STATISTICS.into(), records::statistics_from_vanilla(&v));
        }
    }
    if f.location {
        if let Some(pos) = player.get("Pos").and_then(Nbt::as_list).filter(|p| p.len() == 3) {
            let rot = player.get("Rotation").and_then(Nbt::as_list);
            let r = |i: usize| rot.and_then(|r| r.get(i)).and_then(Nbt::as_f64).unwrap_or(0.0) as f32;
            let dim = player
                .str_at("Dimension")
                .or(dimension_hint)
                .unwrap_or("minecraft:overworld");
            let loc = Location {
                x: pos[0].as_f64().unwrap_or(0.0),
                y: pos[1].as_f64().unwrap_or(0.0),
                z: pos[2].as_f64().unwrap_or(0.0),
                yaw: r(0),
                pitch: r(1),
                world: config.world_for(dim),
            };
            out.insert(keys::LOCATION.into(), data::to_json(&loc));
        }
    }
    if f.attributes {
        if let Some(list) = player.get("attributes").and_then(Nbt::as_list) {
            out.insert(keys::ATTRIBUTES.into(), data::to_json(&attributes_of(list, config)));
        }
    }
    if f.persistent_data {
        // Bukkit keeps a player's PersistentDataContainer in the entity's `BukkitValues`.
        if let Some(pdc) = player.get("BukkitValues") {
            out.insert(keys::PERSISTENT_DATA.into(), pdc.to_snbt());
        }
    }
    out
}

const OPERATIONS: [&str; 3] = ["add_value", "add_multiplied_base", "add_multiplied_total"];

/// 1.21.2 dropped the `generic.`/`player.`/`zombie.` prefixes; Paper may still name either.
fn modern_attribute(name: &str) -> String {
    let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let path = ["generic.", "player.", "zombie."]
        .iter()
        .find_map(|p| path.strip_prefix(p))
        .unwrap_or(path);
    format!("{ns}:{path}")
}

/// `BukkitData.Attributes.collectAttributes` from the vanilla `attributes` list: the synced
/// attributes with their non-ignored modifiers (operation as its ordinal, slot group `any`).
pub fn attributes_of(list: &[Nbt], config: &Config) -> Attributes {
    let settings = &config.synchronization.attributes;
    let mut out = Vec::new();
    for entry in list {
        let Some(id) = entry.str_at("id") else { continue };
        if settings.is_ignored_attribute(id) {
            continue;
        }
        let modifiers = entry
            .get("modifiers")
            .and_then(Nbt::as_list)
            .unwrap_or(&[])
            .iter()
            .filter_map(|m| {
                let name = m.str_at("id")?.to_owned();
                if settings.is_ignored_modifier(&name) {
                    return None;
                }
                let operation = match m.get("operation") {
                    Some(Nbt::String(op)) => OPERATIONS.iter().position(|o| o == op)? as i32,
                    other => other.and_then(Nbt::as_i64).unwrap_or(0) as i32,
                };
                Some(Modifier {
                    uuid: None,
                    name,
                    amount: m.f64_at("amount").unwrap_or(0.0),
                    operation,
                    equipment_slot: 0,
                    equipment_slot_group: "any".into(),
                })
            })
            .collect();
        out.push(Attribute {
            name: id.to_owned(),
            base_value: entry.f64_at("base").unwrap_or(0.0),
            modifiers,
        });
    }
    Attributes { attributes: out }
}

/// The reverse: a vanilla `attributes` list (`AttributeMap.load`'s input) for the synced ones.
pub fn attributes_nbt(a: &Attributes, config: &Config) -> Nbt {
    let settings = &config.synchronization.attributes;
    let mut list = Vec::new();
    for attr in &a.attributes {
        if settings.is_ignored_attribute(&attr.name) {
            continue;
        }
        let mut e = Nbt::compound();
        e.put("id", Nbt::String(modern_attribute(&attr.name)));
        e.put("base", Nbt::Double(attr.base_value));
        let mods: Vec<Nbt> = attr
            .modifiers
            .iter()
            .filter(|m| !settings.is_ignored_modifier(&m.name))
            .filter_map(|m| {
                let mut n = Nbt::compound();
                n.put("id", Nbt::String(m.name.clone()));
                n.put("amount", Nbt::Double(m.amount));
                n.put("operation", Nbt::String((*OPERATIONS.get(m.operation as usize)?).into()));
                Some(n)
            })
            .collect();
        if !mods.is_empty() {
            e.put("modifiers", Nbt::List(mods));
        }
        list.push(e);
    }
    Nbt::List(list)
}

/// Builds a snapshot's data from a fresh capture plus what the port cannot capture itself,
/// carried over from the snapshot the player last received (advancements, statistics,
/// attributes, other plugins' data...). Disabled built-in types are left out, as on Paper.
pub fn merge_with_carried(
    captured: BTreeMap<String, String>,
    carried: Option<&BTreeMap<String, String>>,
    config: &Config,
) -> BTreeMap<String, String> {
    let mut out = captured;
    if let Some(carried) = carried {
        for (k, v) in carried {
            if !out.contains_key(k) && config.synchronization.features.enabled(k) {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    out
}

/// Horizon keeps the mods' persistent compound (`native_set_player_persistent_data`) in the
/// player's file as `NeoForgeData`; Paper keeps Bukkit's PersistentDataContainer as
/// `BukkitValues`, which is what the capture reads and the snapshot carries. A non-empty
/// `NeoForgeData` becomes `BukkitValues`; without one a `BukkitValues` already there (written by
/// an older build of this port) stays.
pub fn mod_data_as_bukkit_values(player: &mut Nbt) {
    let Some(data) = player.get("NeoForgeData").cloned() else { return };
    if matches!(&data, Nbt::Compound(m) if !m.is_empty()) {
        player.put("BukkitValues", data);
    }
}

/// What to do to a player for a snapshot. Every part is optional: a type the snapshot lacks or
/// the config disables keeps what the player has.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ApplyPlan {
    pub persistent_data: Option<Nbt>,
    pub inventory: Option<(Vec<Option<StoredItem>>, i32)>,
    pub ender_chest: Option<Vec<Option<StoredItem>>>,
    pub effects: Option<Vec<Effect>>,
    /// A vanilla `attributes` list for the synced attributes.
    pub attributes: Option<Nbt>,
    pub game_mode: Option<i32>,
    pub flight: Option<FlightStatus>,
    pub health: Option<Health>,
    pub hunger: Option<Hunger>,
    pub experience: Option<Experience>,
    /// Vanilla advancements JSON (`Host::set_player_advancements`).
    pub advancements: Option<serde_json::Value>,
    /// Vanilla statistics JSON (`Host::set_player_stats`).
    pub statistics: Option<serde_json::Value>,
    /// Dimension id and position.
    pub location: Option<(String, Location)>,
    /// Types that failed to read, as `DataSnapshot.Unpacked` skips them with a warning.
    pub skipped: Vec<(String, String)>,
}

/// `DataSnapshot.Unpacked` + `Data.apply` for the types this port applies.
pub fn plan(data: &BTreeMap<String, String>, snapshot_mc_version: &str, config: &Config) -> ApplyPlan {
    let f = &config.synchronization.features;
    let fallback = crate::util::data_version_of(snapshot_mc_version);
    let mut p = ApplyPlan::default();
    let fail = |k: &str, e: String| p_skip(k, e);
    let mut skipped = Vec::new();
    let get = |key: &str, on: bool| -> Option<&String> { if on { data.get(key) } else { None } };
    if let Some(s) = get(keys::PERSISTENT_DATA, f.persistent_data) {
        match parse_snbt(s) {
            Ok(n) => p.persistent_data = Some(n),
            Err(e) => skipped.push(fail(keys::PERSISTENT_DATA, e)),
        }
    }
    if let Some(s) = get(keys::INVENTORY, f.inventory) {
        match Inventory::deserialize(s, fallback) {
            Ok(mut inv) => {
                inv.slots.resize(INVENTORY_SLOTS, None);
                p.inventory = Some((inv.slots, inv.held_item_slot.clamp(0, 8)));
            }
            Err(e) => skipped.push(fail(keys::INVENTORY, e)),
        }
    }
    if let Some(s) = get(keys::ENDER_CHEST, f.ender_chest) {
        match EnderChest::deserialize(s, fallback) {
            Ok(mut e) => {
                e.slots.resize(ENDER_CHEST_SLOTS, None);
                p.ender_chest = Some(e.slots);
            }
            Err(e) => skipped.push(fail(keys::ENDER_CHEST, e)),
        }
    }
    if let Some(s) = get(keys::POTION_EFFECTS, f.potion_effects) {
        match data::from_json::<Vec<Effect>>(s) {
            Ok(v) => p.effects = Some(v),
            Err(e) => skipped.push(fail(keys::POTION_EFFECTS, e)),
        }
    }
    if let Some(s) = get(keys::GAME_MODE, f.game_mode) {
        match data::from_json::<GameMode>(s) {
            Ok(g) => match g.id() {
                Some(id) => p.game_mode = Some(id),
                None => skipped.push(fail(keys::GAME_MODE, format!("unknown game mode {}", g.game_mode))),
            },
            Err(e) => skipped.push(fail(keys::GAME_MODE, e)),
        }
    }
    if let Some(s) = get(keys::FLIGHT_STATUS, f.flight_status) {
        match data::from_json::<FlightStatus>(s) {
            Ok(v) => p.flight = Some(v),
            Err(e) => skipped.push(fail(keys::FLIGHT_STATUS, e)),
        }
    }
    if let Some(s) = get(keys::ATTRIBUTES, f.attributes) {
        match data::from_json::<Attributes>(s) {
            Ok(a) => p.attributes = Some(attributes_nbt(&a, config)),
            Err(e) => skipped.push(fail(keys::ATTRIBUTES, e)),
        }
    }
    if let Some(s) = get(keys::HEALTH, f.health) {
        match data::from_json::<Health>(s) {
            Ok(v) => p.health = Some(v),
            Err(e) => skipped.push(fail(keys::HEALTH, e)),
        }
    }
    if let Some(s) = get(keys::HUNGER, f.hunger) {
        match data::from_json::<Hunger>(s) {
            Ok(v) => p.hunger = Some(v),
            Err(e) => skipped.push(fail(keys::HUNGER, e)),
        }
    }
    if let Some(s) = get(keys::EXPERIENCE, f.experience) {
        match data::from_json::<Experience>(s) {
            Ok(v) => p.experience = Some(v),
            Err(e) => skipped.push(fail(keys::EXPERIENCE, e)),
        }
    }
    if let Some(s) = get(keys::ADVANCEMENTS, f.advancements) {
        match records::advancements_to_vanilla(s, DATA_VERSION) {
            Some(v) => p.advancements = Some(v),
            None => skipped.push(fail(keys::ADVANCEMENTS, "not a list of advancements".into())),
        }
    }
    if let Some(s) = get(keys::STATISTICS, f.statistics) {
        match records::statistics_to_vanilla(s, DATA_VERSION) {
            Some(v) => p.statistics = Some(v),
            None => skipped.push(fail(keys::STATISTICS, "not statistics".into())),
        }
    }
    if let Some(s) = get(keys::LOCATION, f.location) {
        match data::from_json::<Location>(s) {
            Ok(loc) => match config.dimension_for(&loc.world) {
                Some(d) => p.location = Some((d, loc)),
                None => skipped.push(fail(keys::LOCATION, format!("no dimension for world {}", loc.world.name))),
            },
            Err(e) => skipped.push(fail(keys::LOCATION, e)),
        }
    }
    p.skipped = skipped;
    p
}

fn p_skip(key: &str, e: String) -> (String, String) {
    (key.to_owned(), e)
}

/// `/userdata restore` and `/inventory` edits rewrite one type of a stored snapshot.
pub fn set_health_at_least_one(data: &mut BTreeMap<String, String>) {
    if let Some(s) = data.get(keys::HEALTH) {
        if let Ok(mut h) = data::from_json::<Health>(s) {
            h.health = h.health.max(1.0);
            data.insert(keys::HEALTH.into(), data::to_json(&h));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player() -> Nbt {
        parse_snbt(
            r#"{
            Inventory:[{Slot:0b,id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":3}},
                       {Slot:8b,id:"minecraft:bread",count:12}],
            equipment:{head:{id:"minecraft:iron_helmet",count:1},offhand:{id:"minecraft:shield",count:1}},
            EnderItems:[{Slot:26b,id:"minecraft:emerald",count:64}],
            SelectedItemSlot:8,
            Health:17.5f, foodLevel:18, foodSaturationLevel:2.5f, foodExhaustionLevel:1.25f,
            XpLevel:5, XpP:0.5f, XpTotal:60,
            active_effects:[{id:"minecraft:speed",amplifier:1b,duration:600,ambient:0b,show_particles:1b,show_icon:1b},
                            {id:"minecraft:haste",amplifier:0b,duration:100,ambient:1b,show_particles:1b,show_icon:1b},
                            {id:"minecraft:strength",amplifier:-1b,duration:-1,ambient:0b,show_particles:0b,show_icon:1b}],
            playerGameType:2,
            abilities:{mayfly:1b,flying:1b},
            Pos:[1.5d,70.0d,-2.5d], Rotation:[90.0f,10.0f], Dimension:"minecraft:the_nether",
            BukkitValues:{"myplugin:coins":5},
            attributes:[{id:"minecraft:max_health",base:24.0d,modifiers:[{id:"minecraft:effect.health_boost",amount:4.0d,operation:"add_value"},{id:"myplugin:bonus",amount:0.5d,operation:"add_multiplied_total"}]},
                        {id:"minecraft:attack_damage",base:1.0d}]
        }"#,
        )
        .unwrap()
    }

    fn config(location: bool) -> Config {
        let mut c = Config::default();
        c.synchronization.features.location = location;
        c
    }

    #[test]
    fn capture_reads_what_bukkit_reads() {
        let d = capture(&player(), &config(true), None);
        let inv = Inventory::deserialize(&d[keys::INVENTORY], DATA_VERSION).unwrap();
        assert_eq!(inv.held_item_slot, 8);
        assert_eq!(inv.slots[0].as_ref().unwrap().item.str_at("id"), Some("minecraft:diamond_sword"));
        assert_eq!(inv.slots[39].as_ref().unwrap().item.str_at("id"), Some("minecraft:iron_helmet"));
        assert_eq!(inv.slots[40].as_ref().unwrap().item.str_at("id"), Some("minecraft:shield"));
        assert!(inv.slots[36].is_none());
        let ender = EnderChest::deserialize(&d[keys::ENDER_CHEST], DATA_VERSION).unwrap();
        assert!(ender.slots[26].is_some());
        let fx: Vec<Effect> = data::from_json(&d[keys::POTION_EFFECTS]).unwrap();
        assert_eq!(fx.len(), 2, "the ambient effect is left out");
        assert_eq!(fx[1].amplifier, 255);
        assert_eq!(fx[1].duration, -1);
        assert!(d[keys::GAME_MODE].contains("ADVENTURE"));
        assert!(d[keys::FLIGHT_STATUS].contains(r#""allow_flight":true"#));
        assert!(d[keys::HEALTH].contains("17.5"));
        let loc: Location = data::from_json(&d[keys::LOCATION]).unwrap();
        assert_eq!(loc.world.name, "world_nether");
        assert_eq!(d[keys::PERSISTENT_DATA], r#"{"myplugin:coins":5}"#);
        assert!(!capture(&player(), &config(false), None).contains_key(keys::LOCATION));
        let a: Attributes = data::from_json(&d[keys::ATTRIBUTES]).unwrap();
        assert_eq!(a.attributes.len(), 1, "attack_damage is not synced");
        assert_eq!(a.attributes[0].base_value, 24.0);
        assert_eq!(a.attributes[0].modifiers.len(), 1, "effect modifiers are ignored");
        assert_eq!(a.attributes[0].modifiers[0].operation, 2);
    }

    #[test]
    fn capture_then_plan_round_trips() {
        let c = config(true);
        let d = capture(&player(), &c, None);
        let p = plan(&d, "26.3", &c);
        assert!(p.skipped.is_empty());
        let (slots, held) = p.inventory.unwrap();
        assert_eq!(held, 8);
        assert_eq!(slots.len(), 41);
        assert_eq!(p.game_mode, Some(2));
        assert_eq!(p.health.unwrap().health, 17.5);
        assert_eq!(p.hunger.unwrap().food_level, 18);
        assert_eq!(p.experience.unwrap().total_experience, 60);
        let (dim, loc) = p.location.unwrap();
        assert_eq!(dim, "minecraft:the_nether");
        assert_eq!(loc.x, 1.5);
        let attrs = p.attributes.unwrap();
        let first = &attrs.as_list().unwrap()[0];
        assert_eq!(first.str_at("id"), Some("minecraft:max_health"));
        assert_eq!(first.get("modifiers").unwrap().as_list().unwrap()[0].str_at("operation"), Some("add_multiplied_total"));
        assert_eq!(modern_attribute("minecraft:generic.max_health"), "minecraft:max_health");
    }

    #[test]
    fn records_in_the_compound_are_captured_and_planned() {
        let mut pl = player();
        pl.put(records::RECORD_ADVANCEMENTS, Nbt::String(
            r#"{"minecraft:story/root":{"criteria":{"a":"2026-11-08 05:20:33 +0000"},"done":true},"DataVersion":1}"#.into()));
        pl.put(records::RECORD_STATISTICS, Nbt::String(r#"{"stats":{"minecraft:custom":{"minecraft:jump":3}}}"#.into()));
        let c = config(false);
        let d = capture(&pl, &c, None);
        assert!(d[keys::ADVANCEMENTS].contains("minecraft:story/root"));
        assert!(d[keys::STATISTICS].contains("\"jump\":3"));
        let p = plan(&d, "26.3", &c);
        assert!(p.skipped.is_empty());
        assert!(p.advancements.unwrap().get("minecraft:story/root").is_some());
        assert_eq!(p.statistics.unwrap()["stats"]["minecraft:custom"]["minecraft:jump"], 3);
    }

    #[test]
    fn carried_types_survive_and_disabled_ones_do_not() {
        let c = config(false);
        let mut carried = BTreeMap::new();
        carried.insert(keys::ADVANCEMENTS.to_owned(), "[]".to_owned());
        carried.insert(keys::LOCATION.to_owned(), "{}".to_owned());
        carried.insert("other:thing".to_owned(), "x".to_owned());
        carried.insert(keys::HEALTH.to_owned(), "old".to_owned());
        let d = merge_with_carried(capture(&player(), &c, None), Some(&carried), &c);
        assert_eq!(d[keys::ADVANCEMENTS], "[]");
        assert_eq!(d["other:thing"], "x");
        assert!(!d.contains_key(keys::LOCATION));
        assert_ne!(d[keys::HEALTH], "old");
    }

    #[test]
    fn broken_types_are_skipped_not_fatal() {
        let mut d = BTreeMap::new();
        d.insert(keys::HEALTH.to_owned(), "not json".to_owned());
        d.insert(keys::GAME_MODE.to_owned(), r#"{"game_mode":"CREATIVE"}"#.to_owned());
        let p = plan(&d, "26.3", &config(false));
        assert_eq!(p.game_mode, Some(1));
        assert_eq!(p.skipped.len(), 1);
    }

    #[test]
    fn mod_data_becomes_bukkit_values() {
        let mut pdc = Nbt::compound();
        pdc.put("myplugin:coins", Nbt::Int(5));
        let mut p = Nbt::compound();
        p.put("NeoForgeData", pdc.clone());
        mod_data_as_bukkit_values(&mut p);
        assert_eq!(p.get("BukkitValues"), Some(&pdc));

        // Empty mod data keeps an older BukkitValues.
        let mut old = Nbt::compound();
        old.put("old:key", Nbt::Byte(1));
        let mut p = Nbt::compound();
        p.put("NeoForgeData", Nbt::compound());
        p.put("BukkitValues", old.clone());
        mod_data_as_bukkit_values(&mut p);
        assert_eq!(p.get("BukkitValues"), Some(&old));
    }
}
