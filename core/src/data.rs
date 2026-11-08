//! The data types inside a snapshot's `data` map, in the exact strings HuskSync's Bukkit
//! serializers write (`BukkitSerializer`, `Serializer.Json` over `BukkitData.*`).
//!
//! Items are SNBT (NBT-API's `itemStackArrayToNBT`); everything else is Gson JSON.

use crate::nbt::{parse_snbt, Nbt};
use serde::{Deserialize, Serialize};

/// `Data.Items.Inventory.INVENTORY_SLOT_COUNT`: 0..36 main (0..9 hotbar), 36 feet, 37 legs,
/// 38 chest, 39 head, 40 off hand (Bukkit's `PlayerInventory.getContents()` order).
pub const INVENTORY_SLOTS: usize = 41;
/// `Data.Items.EnderChest.ENDER_CHEST_SLOT_COUNT`.
pub const ENDER_CHEST_SLOTS: usize = 27;

/// One stored item: the vanilla `ItemStack.CODEC` compound (`id`, `count`, `components`) without
/// HuskSync's `Slot`, plus the `DataVersion` it was saved at.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredItem {
    pub item: Nbt,
    pub data_version: i32,
}

/// NBT-API `NBTItem.convertItemArraytoNBT`: `{size:N, items:[{Slot:i, <item>, DataVersion:v}]}`.
/// Empty slots are left out; `Slot` is an int.
pub fn items_to_nbt(slots: &[Option<StoredItem>]) -> Nbt {
    let mut list = Vec::new();
    for (i, slot) in slots.iter().enumerate() {
        let Some(stored) = slot else { continue };
        let Nbt::Compound(map) = &stored.item else {
            continue;
        };
        if map.get("id").and_then(Nbt::as_str) == Some("minecraft:air") {
            continue;
        }
        let mut entry = Nbt::Compound(map.clone());
        // Vanilla's SNBT reader takes `NaNd`/`Infinityf` as strings, so a non-finite number
        // would come back on Paper as a string or cost the whole inventory type. Clamp them.
        clamp_non_finite(&mut entry);
        entry.put("Slot", Nbt::Int(i as i32));
        entry.put("DataVersion", Nbt::Int(stored.data_version));
        list.push(entry);
    }
    let mut root = Nbt::compound();
    root.put("size", Nbt::Int(slots.len() as i32));
    root.put("items", Nbt::List(list));
    root
}

/// Replaces NaN with 0 and infinities with the largest finite value of the same sign, in place,
/// through every nested compound and list.
pub fn clamp_non_finite(tag: &mut Nbt) {
    match tag {
        Nbt::Float(v) if !v.is_finite() => {
            *v = if v.is_nan() {
                0.0
            } else {
                f32::MAX.copysign(*v)
            };
        }
        Nbt::Double(v) if !v.is_finite() => {
            *v = if v.is_nan() {
                0.0
            } else {
                f64::MAX.copysign(*v)
            };
        }
        Nbt::List(items) => items.iter_mut().for_each(clamp_non_finite),
        Nbt::Compound(map) => map.values_mut().for_each(clamp_non_finite),
        _ => {}
    }
}

/// The reverse, as HuskSync's `upgradeItemStacks` places them: `Slot` (else the list index)
/// when it is free and in range, the rest into the first free slots. `fallback_version` is the
/// data version of the snapshot's Minecraft version, for items without their own.
pub fn items_from_nbt(tag: &Nbt, size: usize, fallback_version: i32) -> Vec<Option<StoredItem>> {
    let size = tag
        .i64_at("size")
        .map(|s| s.clamp(0, 256) as usize)
        .unwrap_or(size)
        .max(size);
    let mut slots: Vec<Option<StoredItem>> = vec![None; size];
    let mut overflow = Vec::new();
    if let Some(items) = tag.get("items").and_then(Nbt::as_list) {
        for (i, raw) in items.iter().enumerate() {
            let Nbt::Compound(_) = raw else { continue };
            let slot = raw.i64_at("Slot").unwrap_or(i as i64);
            if slot < 0 {
                continue;
            }
            let mut item = raw.clone();
            item.remove("Slot");
            let data_version = item
                .remove("DataVersion")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or(fallback_version);
            if item.str_at("id").is_none_or(|id| id == "minecraft:air") {
                continue;
            }
            let stored = StoredItem { item, data_version };
            let slot = slot as usize;
            if slot < slots.len() && slots[slot].is_none() {
                slots[slot] = Some(stored);
            } else {
                overflow.push(stored);
            }
        }
    }
    for stored in overflow {
        if let Some(free) = slots.iter_mut().find(|s| s.is_none()) {
            *free = Some(stored);
        }
    }
    slots
}

/// `husksync:inventory`: `{items:{size:41,items:[...]},held_item_slot:N}`.
#[derive(Clone, Debug, PartialEq)]
pub struct Inventory {
    pub slots: Vec<Option<StoredItem>>,
    pub held_item_slot: i32,
}

impl Inventory {
    pub fn serialize(&self) -> String {
        let mut root = Nbt::compound();
        root.put("items", items_to_nbt(&self.slots));
        root.put("held_item_slot", Nbt::Int(self.held_item_slot));
        root.to_snbt()
    }

    pub fn deserialize(s: &str, fallback_version: i32) -> Result<Self, String> {
        let root = parse_snbt(s)?;
        let slots = match root.get("items") {
            Some(items) => items_from_nbt(items, INVENTORY_SLOTS, fallback_version),
            None => vec![None; INVENTORY_SLOTS],
        };
        Ok(Inventory {
            slots,
            held_item_slot: root.i64_at("held_item_slot").unwrap_or(0) as i32,
        })
    }
}

/// `husksync:ender_chest`: the item array compound itself.
#[derive(Clone, Debug, PartialEq)]
pub struct EnderChest {
    pub slots: Vec<Option<StoredItem>>,
}

impl EnderChest {
    pub fn serialize(&self) -> String {
        items_to_nbt(&self.slots).to_snbt()
    }

    pub fn deserialize(s: &str, fallback_version: i32) -> Result<Self, String> {
        let root = parse_snbt(s)?;
        Ok(EnderChest {
            slots: items_from_nbt(&root, ENDER_CHEST_SLOTS, fallback_version),
        })
    }
}

/// `Data.PotionEffects.Effect` (`husksync:potion_effects` is a JSON list of these).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    #[serde(rename = "type")]
    pub kind: String,
    pub amplifier: i32,
    pub duration: i32,
    pub is_ambient: bool,
    pub show_particles: bool,
    pub has_icon: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub health: f64,
    #[serde(default)]
    pub health_scale: f64,
    #[serde(default)]
    pub is_health_scaled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hunger {
    pub food_level: i32,
    pub saturation: f32,
    pub exhaustion: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Experience {
    pub total_experience: i32,
    pub exp_level: i32,
    pub exp_progress: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameMode {
    /// Bukkit's `GameMode.name()`: SURVIVAL, CREATIVE, ADVENTURE, SPECTATOR.
    pub game_mode: String,
}

impl GameMode {
    pub fn from_id(id: i32) -> Self {
        GameMode {
            game_mode: match id {
                1 => "CREATIVE",
                2 => "ADVENTURE",
                3 => "SPECTATOR",
                _ => "SURVIVAL",
            }
            .into(),
        }
    }

    pub fn id(&self) -> Option<i32> {
        match self.game_mode.as_str() {
            "SURVIVAL" => Some(0),
            "CREATIVE" => Some(1),
            "ADVENTURE" => Some(2),
            "SPECTATOR" => Some(3),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlightStatus {
    pub allow_flight: bool,
    pub is_flying: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct World {
    pub name: String,
    pub uuid: String,
    /// Bukkit's `World.Environment.name()`: NORMAL, NETHER, THE_END, CUSTOM.
    pub environment: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub world: World,
}

/// `Data.Attributes.Modifier` (Gson: `uuid` left out when null; `equipment_slot` is the
/// deprecated int field, always written).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Modifier {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    pub name: String,
    pub amount: f64,
    pub operation: i32,
    #[serde(default)]
    pub equipment_slot: i32,
    #[serde(default = "any_slot")]
    pub equipment_slot_group: String,
}

fn any_slot() -> String {
    "any".into()
}

/// `Data.Attributes.Attribute` (a record: Gson uses the component names).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attribute {
    pub name: String,
    #[serde(rename = "baseValue")]
    pub base_value: f64,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
}

/// `husksync:attributes`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attributes {
    pub attributes: Vec<Attribute>,
}

pub fn to_json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

pub fn from_json<'a, T: Deserialize<'a>>(s: &'a str) -> Result<T, String> {
    serde_json::from_str(s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone(count: i32) -> StoredItem {
        let mut i = Nbt::compound();
        i.put("id", Nbt::String("minecraft:stone".into()));
        i.put("count", Nbt::Int(count));
        StoredItem {
            item: i,
            data_version: 5023,
        }
    }

    #[test]
    fn inventory_string_matches_nbt_api_shape() {
        let mut slots = vec![None; INVENTORY_SLOTS];
        slots[0] = Some(stone(5));
        slots[40] = Some(stone(1));
        let inv = Inventory {
            slots,
            held_item_slot: 2,
        };
        let s = inv.serialize();
        assert_eq!(
            s,
            "{held_item_slot:2,items:{items:[{DataVersion:5023,Slot:0,count:5,id:\"minecraft:stone\"},{DataVersion:5023,Slot:40,count:1,id:\"minecraft:stone\"}],size:41}}"
        );
        assert_eq!(Inventory::deserialize(&s, 5023).unwrap(), inv);
    }

    #[test]
    fn reads_paper_written_inventory() {
        // NBT-API output on Paper 26.3 (abridged): a sword with components, an older item
        // without a DataVersion, and a duplicate slot that overflows.
        let s = r#"{held_item_slot:0,items:{items:[{DataVersion:5023,Slot:0,components:{"minecraft:damage":12,"minecraft:enchantments":{"minecraft:sharpness":5}},count:1,id:"minecraft:diamond_sword"},{Slot:0,count:3,id:"minecraft:torch"},{DataVersion:4671,Slot:39,count:1,id:"minecraft:iron_helmet"}],size:41}}"#;
        let inv = Inventory::deserialize(s, 4786).unwrap();
        assert_eq!(inv.slots.len(), 41);
        let sword = inv.slots[0].as_ref().unwrap();
        assert_eq!(sword.item.str_at("id"), Some("minecraft:diamond_sword"));
        assert!(sword.item.get("Slot").is_none());
        let torch = inv.slots[1].as_ref().unwrap();
        assert_eq!(torch.item.str_at("id"), Some("minecraft:torch"));
        assert_eq!(torch.data_version, 4786);
        assert_eq!(inv.slots[39].as_ref().unwrap().data_version, 4671);
    }

    #[test]
    fn empty_ender_chest() {
        let e = EnderChest {
            slots: vec![None; 27],
        };
        assert_eq!(e.serialize(), "{items:[],size:27}");
        assert_eq!(
            EnderChest::deserialize("{items:[],size:27}", 5023).unwrap(),
            e
        );
    }

    #[test]
    fn json_types_match_gson_names() {
        let h = Health {
            health: 20.0,
            health_scale: 20.0,
            is_health_scaled: false,
        };
        assert_eq!(
            to_json(&h),
            r#"{"health":20.0,"health_scale":20.0,"is_health_scaled":false}"#
        );
        let e = Effect {
            kind: "minecraft:speed".into(),
            amplifier: 1,
            duration: 200,
            is_ambient: false,
            show_particles: true,
            has_icon: true,
        };
        assert_eq!(
            to_json(&vec![e]),
            r#"[{"type":"minecraft:speed","amplifier":1,"duration":200,"is_ambient":false,"show_particles":true,"has_icon":true}]"#
        );
        let x: Experience =
            from_json(r#"{"total_experience":7,"exp_level":1,"exp_progress":0.25}"#).unwrap();
        assert_eq!(x.exp_level, 1);
        assert_eq!(GameMode::from_id(3).game_mode, "SPECTATOR");
        let l: Location = from_json(r#"{"x":1.5,"y":64.0,"z":-3.25,"yaw":90.0,"pitch":0.0,"world":{"name":"world","uuid":"00000000-0000-0000-0000-000000000001","environment":"NORMAL"}}"#).unwrap();
        assert_eq!(l.world.environment, "NORMAL");
        let a: Attributes = from_json(r#"{"attributes":[{"name":"minecraft:max_health","baseValue":20.0,"modifiers":[{"name":"minecraft:armor.chestplate","amount":4.0,"operation":0,"equipment_slot":0,"equipment_slot_group":"chest"}]}]}"#).unwrap();
        assert_eq!(a.attributes[0].modifiers[0].equipment_slot_group, "chest");
        assert!(to_json(&a).contains(r#""baseValue":20.0"#));
    }

    #[test]
    fn non_finite_numbers_are_clamped_before_writing() {
        let mut item = Nbt::compound();
        item.put("id", Nbt::String("minecraft:stone".into()));
        item.put("count", Nbt::Int(1));
        let mut comps = Nbt::compound();
        comps.put("a", Nbt::Double(f64::NAN));
        comps.put("b", Nbt::Float(f32::INFINITY));
        comps.put(
            "c",
            Nbt::List(vec![Nbt::Double(f64::NEG_INFINITY), Nbt::Double(1.5)]),
        );
        item.put("components", comps);
        let slots = vec![Some(StoredItem {
            item,
            data_version: 5023,
        })];
        let tag = items_to_nbt(&slots);
        let text = tag.to_snbt();
        assert!(
            !text.contains("NaN") && !text.contains("Infinity"),
            "{text}"
        );
        let entry = &tag.get("items").and_then(Nbt::as_list).unwrap()[0];
        let comps = entry.get("components").unwrap();
        assert_eq!(comps.get("a"), Some(&Nbt::Double(0.0)));
        assert_eq!(comps.get("b"), Some(&Nbt::Float(f32::MAX)));
        assert_eq!(
            comps.get("c").and_then(Nbt::as_list).unwrap()[0],
            Nbt::Double(-f64::MAX)
        );
    }
}
