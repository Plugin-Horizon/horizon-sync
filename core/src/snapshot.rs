//! `DataSnapshot.Packed`: the JSON document HuskSync stores (in MariaDB's `data` blob and in
//! Redis), optionally Snappy-compressed.
//!
//! ```json
//! {"id":"<uuid>","pinned":false,"timestamp":"2026-11-06T06:00:00.123+01:00",
//!  "save_cause":"DISCONNECT","server_name":"world","minecraft_version":"26.3",
//!  "platform_type":"bukkit","format_version":5,
//!  "data":{"husksync:inventory":"<SNBT>","husksync:health":"<JSON>", ...}}
//! ```

use crate::snappy;
use crate::util::{self, compare_versions};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// `DataSnapshot.CURRENT_FORMAT_VERSION`.
pub const FORMAT_VERSION: i32 = 5;

/// Built-in data keys (`Identifier.toString()`).
pub mod keys {
    pub const INVENTORY: &str = "husksync:inventory";
    pub const ENDER_CHEST: &str = "husksync:ender_chest";
    pub const POTION_EFFECTS: &str = "husksync:potion_effects";
    pub const ADVANCEMENTS: &str = "husksync:advancements";
    pub const LOCATION: &str = "husksync:location";
    pub const STATISTICS: &str = "husksync:statistics";
    pub const HEALTH: &str = "husksync:health";
    pub const HUNGER: &str = "husksync:hunger";
    pub const ATTRIBUTES: &str = "husksync:attributes";
    pub const EXPERIENCE: &str = "husksync:experience";
    pub const GAME_MODE: &str = "husksync:game_mode";
    pub const FLIGHT_STATUS: &str = "husksync:flight_status";
    pub const PERSISTENT_DATA: &str = "husksync:persistent_data";
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Packed {
    // Gson fills absent fields with defaults (null / 0), so a document without them still
    // reaches `validate` and is reported as legacy (`format_version` 0) instead of failing as
    // JSON. `#[serde(default)]` on every field gives the same outcome.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub save_cause: String,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub minecraft_version: String,
    #[serde(default)]
    pub platform_type: String,
    #[serde(default)]
    pub format_version: i32,
    #[serde(default)]
    pub data: BTreeMap<String, String>,
}

/// `DataException.Reason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    MinecraftVersion,
    FormatVersion,
    /// Format 1-3 (HuskSync 1.x/2.x); the legacy converter is not ported.
    Legacy,
    PlatformType,
}

impl Invalid {
    pub fn message(self) -> &'static str {
        match self {
            Invalid::MinecraftVersion => "the snapshot was saved on a newer Minecraft version",
            Invalid::FormatVersion => "the snapshot uses a newer HuskSync format",
            Invalid::Legacy => "the snapshot is in a legacy (v1/v2) format, which is not supported",
            Invalid::PlatformType => "the snapshot was saved on another platform",
        }
    }
}

/// Where the snapshot came from, for `deserialize`'s checks.
#[derive(Clone, Debug)]
pub struct Platform {
    /// What this server reports as its Minecraft version (`26.3`).
    pub minecraft_version: String,
    /// `bukkit` to share data with Paper servers.
    pub platform_type: String,
    pub compress: bool,
    pub server_name: String,
}

impl Packed {
    pub fn new(
        platform: &Platform,
        cause: &str,
        pinned: bool,
        data: BTreeMap<String, String>,
    ) -> Self {
        Packed {
            id: util::uuid_string(util::random_uuid()),
            pinned,
            timestamp: util::format_timestamp(&util::now()),
            save_cause: truncate_cause(cause),
            server_name: Some(platform.server_name.clone()),
            minecraft_version: util::strip_version_metadata(&platform.minecraft_version).to_owned(),
            platform_type: platform.platform_type.clone(),
            format_version: FORMAT_VERSION,
            data,
        }
    }

    pub fn id_u128(&self) -> Option<u128> {
        util::parse_uuid(&self.id)
    }

    pub fn short_id(&self) -> &str {
        self.id.get(..8).unwrap_or(&self.id)
    }

    pub fn time(&self) -> Option<chrono::DateTime<chrono::FixedOffset>> {
        util::parse_timestamp(&self.timestamp)
    }

    /// `Packed.copy()`: a new id and the current time, same data.
    pub fn copy(&self) -> Self {
        let mut c = self.clone();
        c.id = util::uuid_string(util::random_uuid());
        c.timestamp = util::format_timestamp(&util::now());
        c
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn to_bytes(&self, compress: bool) -> Vec<u8> {
        let json = self.to_json().into_bytes();
        if compress {
            snappy::compress(&json)
        } else {
            json
        }
    }

    /// Reads a stored snapshot. `compress` is the config's setting; the other encoding is tried
    /// when the configured one fails, so a mixed network still reads.
    pub fn from_bytes(bytes: &[u8], compress: bool) -> Result<Self, String> {
        let first = if compress {
            snappy::decompress(bytes).and_then(|j| parse_json(&j))
        } else {
            parse_json(bytes)
        };
        first.or_else(|e| {
            let second = if compress {
                parse_json(bytes)
            } else {
                snappy::decompress(bytes).and_then(|j| parse_json(&j))
            };
            second.map_err(|_| e)
        })
    }

    /// `DataSnapshot.deserialize`'s checks, in its order.
    pub fn validate(&self, platform: &Platform) -> Result<(), Invalid> {
        if compare_versions(&self.minecraft_version, &platform.minecraft_version)
            == Ordering::Greater
        {
            return Err(Invalid::MinecraftVersion);
        }
        if self.format_version > FORMAT_VERSION {
            return Err(Invalid::FormatVersion);
        }
        if self.format_version < 4 {
            return Err(Invalid::Legacy);
        }
        if !self
            .platform_type
            .eq_ignore_ascii_case(&platform.platform_type)
        {
            return Err(Invalid::PlatformType);
        }
        Ok(())
    }

    /// The approximate stored size, for `/userdata` (`getFileSize`).
    pub fn file_size(&self, compress: bool) -> usize {
        self.to_bytes(compress).len()
    }
}

fn parse_json(bytes: &[u8]) -> Result<Packed, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("snapshot JSON: {e}"))
}

/// `SaveCause.of`: names are cut to 31 characters when longer than 32.
pub fn truncate_cause(name: &str) -> String {
    if name.chars().count() > 32 {
        name.chars().take(31).collect()
    } else {
        name.to_owned()
    }
}

/// `SaveCause.getDisplayName`: `INVENTORY_COMMAND` -> `Inventory Command`.
pub fn cause_display(name: &str) -> String {
    name.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let lower = w.to_lowercase();
            let mut c = lower.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn platform() -> Platform {
        Platform {
            minecraft_version: "26.3".into(),
            platform_type: "bukkit".into(),
            compress: true,
            server_name: "test".into(),
        }
    }

    #[test]
    fn json_shape_and_round_trip() {
        let mut data = BTreeMap::new();
        data.insert(
            keys::HEALTH.into(),
            r#"{"health":20.0,"health_scale":20.0,"is_health_scaled":false}"#.into(),
        );
        let s = Packed::new(&platform(), "DISCONNECT", false, data);
        let json = s.to_json();
        for key in [
            "\"id\":",
            "\"pinned\":false",
            "\"timestamp\":",
            "\"save_cause\":\"DISCONNECT\"",
            "\"server_name\":\"test\"",
            "\"minecraft_version\":\"26.3\"",
            "\"platform_type\":\"bukkit\"",
            "\"format_version\":5",
            "\"husksync:health\":",
        ] {
            assert!(json.contains(key), "{key} in {json}");
        }
        for compress in [true, false] {
            let back = Packed::from_bytes(&s.to_bytes(compress), compress).unwrap();
            assert_eq!(back, s);
            // The other setting still reads it.
            assert_eq!(
                Packed::from_bytes(&s.to_bytes(compress), !compress).unwrap(),
                s
            );
        }
        assert_eq!(s.validate(&platform()), Ok(()));
    }

    #[test]
    fn reads_a_gson_document() {
        // As Gson writes it on Paper (field order of the Java class, no server_name null).
        let json = r#"{"id":"5a2f0c5e-3b8a-4c49-9a39-6d2c1f0a7b11","pinned":true,"timestamp":"2026-11-06T05:12:44.913477+01:00","save_cause":"INVENTORY_COMMAND","server_name":"rec","minecraft_version":"26.3","platform_type":"bukkit","format_version":5,"data":{"husksync:game_mode":"{\"game_mode\":\"SURVIVAL\"}"}}"#;
        let s = Packed::from_bytes(json.as_bytes(), false).unwrap();
        assert!(s.pinned);
        assert_eq!(s.short_id(), "5a2f0c5e");
        assert!(s.time().is_some());
        assert_eq!(cause_display(&s.save_cause), "Inventory Command");
    }

    #[test]
    fn validation_follows_husksync() {
        let mut s = Packed::new(&platform(), "API", false, BTreeMap::new());
        s.minecraft_version = "26.4".into();
        assert_eq!(s.validate(&platform()), Err(Invalid::MinecraftVersion));
        s.minecraft_version = "1.21.11".into();
        s.platform_type = "fabric".into();
        assert_eq!(s.validate(&platform()), Err(Invalid::PlatformType));
        s.platform_type = "bukkit".into();
        s.format_version = 3;
        assert_eq!(s.validate(&platform()), Err(Invalid::Legacy));
        assert_eq!(truncate_cause(&"X".repeat(40)).len(), 31);
    }

    #[test]
    fn missing_fields_are_legacy_not_a_json_error() {
        // A pre-v4 style document: only `data`, everything else absent (Gson defaults).
        let json = br#"{"data":{"husksync:health":"{}"}}"#;
        let s = Packed::from_bytes(json, false).unwrap();
        assert_eq!(s.format_version, 0);
        assert_eq!(s.validate(&platform()), Err(Invalid::Legacy));
        // Compressed the same way.
        let packed = crate::snappy::compress(json);
        let s = Packed::from_bytes(&packed, true).unwrap();
        assert_eq!(s.validate(&platform()), Err(Invalid::Legacy));
        // Not JSON at all is still an error.
        assert!(Packed::from_bytes(b"not json", false).is_err());
    }
}
