//! A player's advancements and statistics between the game's own files and HuskSync's data.
//!
//! The game side (`Host::player_advancements` / `player_stats` and their setters) speaks vanilla's
//! JSON: `players/advancements/<uuid>.json` (`{"<id>": {"criteria": {"<name>": "<date>"}, "done":
//! true}, "DataVersion": n}`) and `players/stats/<uuid>.json` (`{"stats": {"minecraft:mined":
//! {"minecraft:stone": 3}}, "DataVersion": n}`). HuskSync keeps
//!   advancements: `[{"key": "<id>", "completed_criteria": {"<name>": <epoch millis>}}]`
//!   statistics:   `{"generic": {..}, "blocks": {..}, "items": {..}, "entities": {..}}`
//! with Bukkit's statistic names (the key of `Statistic`, no namespace).

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{json, Map, Value};

/// Name of the string entry the game side adds to the player's compound: vanilla's advancements
/// JSON text. The core turns it into the snapshot's `husksync:advancements`.
pub const RECORD_ADVANCEMENTS: &str = "HorizonAdvancements";
/// Same for the statistics JSON text (`husksync:statistics`).
pub const RECORD_STATISTICS: &str = "HorizonStatistics";

/// Vanilla's criterion date format (`PlayerAdvancements.DATE_TIME_FORMATTER`).
const DATE_FORMAT: &str = "%Y-%m-%d %H:%M:%S %z";

fn parse_date(s: &str) -> Option<i64> {
    DateTime::parse_from_str(s, DATE_FORMAT).ok().map(|d| d.timestamp_millis())
}

fn format_date(ms: i64) -> String {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_default().format(DATE_FORMAT).to_string()
}

/// Vanilla advancements JSON to HuskSync's list (only advancements with a criterion done).
pub fn advancements_from_vanilla(v: &Value) -> String {
    let mut list = Vec::new();
    if let Some(o) = v.as_object() {
        for (key, entry) in o {
            if key == "DataVersion" {
                continue;
            }
            let Some(criteria) = entry.get("criteria").and_then(Value::as_object) else { continue };
            let done: Map<String, Value> = criteria
                .iter()
                .filter_map(|(name, date)| Some((name.clone(), json!(parse_date(date.as_str()?)?))))
                .collect();
            if !done.is_empty() {
                list.push(json!({ "key": key, "completed_criteria": done }));
            }
        }
    }
    Value::Array(list).to_string()
}

/// HuskSync's list to vanilla advancements JSON (`done` is left to the game, which works it out
/// from the advancement's requirements when it reads the file).
pub fn advancements_to_vanilla(s: &str, data_version: i32) -> Option<Value> {
    let list: Value = serde_json::from_str(s).ok()?;
    // An older build of this port documented the list under `completed`.
    let list = list.get("completed").unwrap_or(&list);
    let mut out = Map::new();
    for adv in list.as_array()? {
        let Some(key) = adv.get("key").and_then(Value::as_str) else { continue };
        let Some(criteria) = adv.get("completed_criteria").and_then(Value::as_object) else { continue };
        let dated: Map<String, Value> = criteria
            .iter()
            .filter_map(|(name, ms)| Some((name.clone(), json!(format_date(ms.as_i64()?)))))
            .collect();
        out.insert(key.to_owned(), json!({ "criteria": dated, "done": false }));
    }
    out.insert("DataVersion".into(), data_version.into());
    Some(Value::Object(out))
}

/// Vanilla stat category to Bukkit's typed statistic and the section of HuskSync's data.
const CATEGORIES: [(&str, &str, &str); 8] = [
    ("minecraft:mined", "mine_block", "blocks"),
    ("minecraft:broken", "break_item", "items"),
    ("minecraft:crafted", "craft_item", "items"),
    ("minecraft:used", "use_item", "items"),
    ("minecraft:picked_up", "pickup", "items"),
    ("minecraft:dropped", "drop", "items"),
    ("minecraft:killed", "kill_entity", "entities"),
    ("minecraft:killed_by", "entity_killed_by", "entities"),
];

fn bare(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}

/// Vanilla statistics JSON to HuskSync's four maps. Zero values are left out, as Bukkit's capture
/// does.
pub fn statistics_from_vanilla(v: &Value) -> String {
    let mut generic = Map::new();
    let mut sections: [Map<String, Value>; 3] = Default::default();
    if let Some(stats) = v.get("stats").and_then(Value::as_object) {
        for (category, values) in stats {
            let Some(values) = values.as_object() else { continue };
            if category == "minecraft:custom" {
                for (id, n) in values {
                    if n.as_i64().is_some_and(|n| n != 0) {
                        generic.insert(bare(id).to_owned(), n.clone());
                    }
                }
            } else if let Some((_, name, section)) = CATEGORIES.iter().find(|(c, _, _)| *c == category) {
                let at = ["blocks", "items", "entities"].iter().position(|s| s == section).unwrap_or(0);
                let inner: Map<String, Value> = values
                    .iter()
                    .filter(|(_, n)| n.as_i64().is_some_and(|n| n != 0))
                    .map(|(id, n)| (bare(id).to_owned(), n.clone()))
                    .collect();
                if !inner.is_empty() {
                    sections[at].insert((*name).to_owned(), Value::Object(inner));
                }
            }
        }
    }
    let [blocks, items, entities] = sections;
    json!({ "generic": generic, "blocks": blocks, "items": items, "entities": entities }).to_string()
}

/// HuskSync's statistics to vanilla JSON. Names this build does not know are dropped.
pub fn statistics_to_vanilla(s: &str, data_version: i32) -> Option<Value> {
    let v: Value = serde_json::from_str(s).ok()?;
    let mut stats = Map::new();
    let put = |stats: &mut Map<String, Value>, category: &str, id: &str, n: &Value| {
        let entry = stats.entry(category.to_owned()).or_insert_with(|| Value::Object(Map::new()));
        if let Some(o) = entry.as_object_mut() {
            o.insert(format!("minecraft:{id}"), n.clone());
        }
    };
    if let Some(generic) = v.get("generic").and_then(Value::as_object) {
        for (id, n) in generic {
            // Bukkit's name for the play time is the one from before vanilla renamed it.
            let id = if id == "play_one_minute" { "play_time" } else { id };
            put(&mut stats, "minecraft:custom", id, n);
        }
    }
    for section in ["blocks", "items", "entities"] {
        let Some(typed) = v.get(section).and_then(Value::as_object) else { continue };
        for (name, inner) in typed {
            let Some((category, _, _)) = CATEGORIES.iter().find(|(_, n, s)| n == name && *s == section) else {
                continue;
            };
            for (id, n) in inner.as_object().into_iter().flatten() {
                put(&mut stats, category, id, n);
            }
        }
    }
    Some(json!({ "stats": stats, "DataVersion": data_version }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advancements_round_trip() {
        let vanilla = json!({
            "minecraft:story/root": {
                "criteria": { "crafting_table": "2026-11-08 05:20:33 +0000" }, "done": true },
            "minecraft:story/open": { "criteria": {}, "done": false },
            "DataVersion": 5023
        });
        let ours = advancements_from_vanilla(&vanilla);
        let list: Value = serde_json::from_str(&ours).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["key"], "minecraft:story/root");
        assert_eq!(list[0]["completed_criteria"]["crafting_table"], 1_794_115_233_000i64);
        let back = advancements_to_vanilla(&ours, 5023).unwrap();
        assert_eq!(
            back["minecraft:story/root"]["criteria"]["crafting_table"],
            "2026-11-08 05:20:33 +0000"
        );
        assert_eq!(back["DataVersion"], 5023);
        // The older layout is read too.
        let old = format!(r#"{{"completed":{ours}}}"#);
        assert!(advancements_to_vanilla(&old, 1).unwrap().get("minecraft:story/root").is_some());
    }

    #[test]
    fn statistics_round_trip() {
        let vanilla = json!({ "stats": {
            "minecraft:custom": { "minecraft:jump": 3, "minecraft:walk_one_cm": 0 },
            "minecraft:mined": { "minecraft:stone": 5 },
            "minecraft:used": { "minecraft:bread": 2 },
            "minecraft:killed": { "minecraft:zombie": 4 },
            "minecraft:unknown_category": { "minecraft:x": 1 }
        }, "DataVersion": 5023 });
        let ours = statistics_from_vanilla(&vanilla);
        let v: Value = serde_json::from_str(&ours).unwrap();
        assert_eq!(v["generic"]["jump"], 3);
        assert!(v["generic"].get("walk_one_cm").is_none());
        assert_eq!(v["blocks"]["mine_block"]["stone"], 5);
        assert_eq!(v["items"]["use_item"]["bread"], 2);
        assert_eq!(v["entities"]["kill_entity"]["zombie"], 4);
        let back = statistics_to_vanilla(&ours, 5023).unwrap();
        assert_eq!(back["stats"]["minecraft:custom"]["minecraft:jump"], 3);
        assert_eq!(back["stats"]["minecraft:mined"]["minecraft:stone"], 5);
        assert_eq!(back["stats"]["minecraft:killed"]["minecraft:zombie"], 4);
        let play = r#"{"generic":{"play_one_minute":100}}"#;
        assert_eq!(statistics_to_vanilla(play, 1).unwrap()["stats"]["minecraft:custom"]["minecraft:play_time"], 100);
    }
}
