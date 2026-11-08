//! `config/horizonsync/config.toml`: HuskSync's `config.yml` and `server.yml` as TOML, same key
//! names, same defaults.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `server.yml`'s `name`: this server's name in snapshots and Redis checkouts. Must match
    /// the proxy's name for the server, as on Paper.
    pub server_name: String,
    pub language: String,
    pub cluster_id: String,
    pub debug_logging: bool,
    /// `check_for_updates`, `enable_plan_hook`, `cancel_packets`: HuskSync keys read for
    /// compatibility; Horizon has no update checker, Plan hook or packet library.
    pub check_for_updates: bool,
    pub enable_plan_hook: bool,
    pub cancel_packets: bool,
    /// What this server says it runs. Paper refuses snapshots from a newer Minecraft version and
    /// from another platform type, so keep `bukkit` to share data with Paper servers.
    pub minecraft_version: String,
    pub platform_type: String,
    pub disabled_commands: Vec<String>,
    pub database: Database,
    pub redis: Redis,
    pub synchronization: Synchronization,
    /// Bukkit world names and UUIDs for `husksync:location` (Paper saves the world by name).
    pub worlds: BTreeMap<String, WorldName>,
}

impl Default for Config {
    fn default() -> Self {
        let mut worlds = BTreeMap::new();
        worlds.insert(
            "minecraft:overworld".into(),
            WorldName { name: "world".into(), uuid: String::new(), environment: "NORMAL".into() },
        );
        worlds.insert(
            "minecraft:the_nether".into(),
            WorldName { name: "world_nether".into(), uuid: String::new(), environment: "NETHER".into() },
        );
        worlds.insert(
            "minecraft:the_end".into(),
            WorldName { name: "world_the_end".into(), uuid: String::new(), environment: "THE_END".into() },
        );
        Config {
            server_name: String::new(),
            language: "en-gb".into(),
            cluster_id: String::new(),
            debug_logging: false,
            check_for_updates: true,
            enable_plan_hook: true,
            cancel_packets: true,
            minecraft_version: "26.3".into(),
            platform_type: "bukkit".into(),
            disabled_commands: Vec::new(),
            database: Database::default(),
            redis: Redis::default(),
            synchronization: Synchronization::default(),
            worlds,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldName {
    pub name: String,
    /// The Bukkit world's `uid.dat` UUID; empty writes a name-derived placeholder.
    #[serde(default)]
    pub uuid: String,
    #[serde(default)]
    pub environment: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Database {
    /// MYSQL or MARIADB (both the same client here). MONGO and POSTGRES are not ported.
    #[serde(rename = "type")]
    pub kind: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub use_ssl: bool,
    pub maximum_pool_size: usize,
    pub minimum_idle: usize,
    pub connection_timeout: u64,
    /// HuskSync's `connection_pool.maximum_lifetime` / `keepalive_time` and `credentials.parameters`
    /// (JDBC URL parameters): read for compatibility, the Rust client has no use for them.
    pub maximum_lifetime: u64,
    pub keepalive_time: u64,
    pub parameters: String,
    pub table_names: TableNames,
    pub create_tables: bool,
    /// The time zone the `timestamp` column is written in. JDBC writes the JVM's default zone:
    /// `local` (the system zone, as a Paper JVM on the same machine) or a fixed `+01:00`.
    pub timestamp_zone: String,
}

impl Default for Database {
    fn default() -> Self {
        Database {
            kind: "MYSQL".into(),
            host: "localhost".into(),
            port: 3306,
            database: "HuskSync".into(),
            username: "root".into(),
            password: "pa55w0rd".into(),
            use_ssl: false,
            maximum_pool_size: 10,
            minimum_idle: 10,
            connection_timeout: 5000,
            maximum_lifetime: 1_800_000,
            keepalive_time: 0,
            parameters: "?autoReconnect=true&useSSL=false&useUnicode=true&characterEncoding=UTF-8".into(),
            table_names: TableNames::default(),
            create_tables: true,
            timestamp_zone: "local".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct TableNames {
    pub users: String,
    pub user_data: String,
    pub map_data: String,
    pub map_ids: String,
}

impl Default for TableNames {
    fn default() -> Self {
        TableNames {
            users: "husksync_users".into(),
            user_data: "husksync_user_data".into(),
            map_data: "husksync_map_data".into(),
            map_ids: "husksync_map_ids".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Redis {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: i64,
    pub use_ssl: bool,
    pub connection_timeout: u64,
    pub socket_timeout: u64,
    pub max_retries: u32,
    pub retry_backoff_millis: u64,
    /// HuskSync's Lettuce/Jedis pool and health-check keys: read for compatibility.
    pub max_total_connections: u32,
    pub max_idle_connections: u32,
    pub min_idle_connections: u32,
    pub test_on_borrow: bool,
    pub test_on_return: bool,
    pub test_while_idle: bool,
    pub min_evictable_idle_time_millis: u64,
    pub time_between_eviction_runs_millis: u64,
    /// `redis.sentinel`: not supported (a non-empty `master` is logged at start).
    pub sentinel: Sentinel,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sentinel {
    pub master: String,
    pub nodes: Vec<String>,
    pub password: String,
}

impl Default for Redis {
    fn default() -> Self {
        Redis {
            host: "localhost".into(),
            port: 6379,
            user: String::new(),
            password: String::new(),
            database: 0,
            use_ssl: false,
            connection_timeout: 2000,
            socket_timeout: 2000,
            max_retries: 3,
            retry_backoff_millis: 200,
            max_total_connections: 50,
            max_idle_connections: 8,
            min_idle_connections: 2,
            test_on_borrow: true,
            test_on_return: true,
            test_while_idle: true,
            min_evictable_idle_time_millis: 60_000,
            time_between_eviction_runs_millis: 30_000,
            sentinel: Sentinel::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Synchronization {
    /// LOCKSTEP or DELAY.
    pub mode: String,
    pub max_user_data_snapshots: i64,
    /// Hours; 0 keeps every snapshot until `max_user_data_snapshots`.
    pub snapshot_backup_frequency: i64,
    pub auto_pinned_save_causes: Vec<String>,
    pub save_on_world_save: bool,
    pub save_on_death: SaveOnDeath,
    pub compress_data: bool,
    /// ACTION_BAR, CHAT, TOAST (shown as the action bar here) or NONE.
    pub notification_display_slot: String,
    pub persist_locked_maps: bool,
    pub network_latency_milliseconds: u64,
    pub shutdown_save_timeout_milliseconds: u64,
    pub features: Features,
    pub attributes: AttributeSettings,
    pub blacklisted_commands_while_locked: Vec<String>,
    pub allow_pearl_spawning_while_locked: bool,
    pub checkin_petitions: bool,
}

impl Default for Synchronization {
    fn default() -> Self {
        Synchronization {
            mode: "LOCKSTEP".into(),
            max_user_data_snapshots: 16,
            snapshot_backup_frequency: 4,
            auto_pinned_save_causes: vec![
                "INVENTORY_COMMAND".into(),
                "ENDERCHEST_COMMAND".into(),
                "BACKUP_RESTORE".into(),
                "LEGACY_MIGRATION".into(),
                "MPDB_MIGRATION".into(),
            ],
            save_on_world_save: true,
            save_on_death: SaveOnDeath::default(),
            compress_data: true,
            notification_display_slot: "ACTION_BAR".into(),
            persist_locked_maps: true,
            network_latency_milliseconds: 500,
            shutdown_save_timeout_milliseconds: 5000,
            features: Features::default(),
            attributes: AttributeSettings::default(),
            blacklisted_commands_while_locked: vec!["*".into()],
            allow_pearl_spawning_while_locked: true,
            checkin_petitions: false,
        }
    }
}

impl Synchronization {
    pub fn lockstep(&self) -> bool {
        !self.mode.eq_ignore_ascii_case("DELAY")
    }

    pub fn auto_pin(&self, cause: &str) -> bool {
        self.auto_pinned_save_causes.iter().any(|c| c.eq_ignore_ascii_case(cause))
    }

    /// `LockedHandler.isCommandDisabled`.
    pub fn command_blocked_while_locked(&self, label: &str) -> bool {
        let label = label.trim_start_matches('/').split_whitespace().next().unwrap_or("").to_lowercase();
        self.blacklisted_commands_while_locked
            .iter()
            .any(|c| c == "*" || *c == label)
    }
}

/// `synchronization.attributes` (`Settings.AttributeSettings`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AttributeSettings {
    pub synced_attributes: Vec<String>,
    pub ignored_modifiers: Vec<String>,
}

impl Default for AttributeSettings {
    fn default() -> Self {
        let synced = [
            "minecraft:generic.max_health", "minecraft:max_health",
            "minecraft:generic.max_absorption", "minecraft:max_absorption",
            "minecraft:generic.luck", "minecraft:luck",
            "minecraft:generic.scale", "minecraft:scale",
            "minecraft:generic.step_height", "minecraft:step_height",
            "minecraft:generic.gravity", "minecraft:gravity",
        ];
        AttributeSettings {
            synced_attributes: synced.iter().map(|s| s.to_string()).collect(),
            ignored_modifiers: vec!["minecraft:effect.*".into(), "minecraft:creative_mode_*".into()],
        }
    }
}

/// `AttributeSettings.matchesWildcard`: `minecraft:` is assumed, `*` matches anything.
fn matches_wildcard(pattern: &str, value: &str) -> bool {
    let pat = if pattern.contains(':') { pattern.to_owned() } else { format!("minecraft:{pattern}") };
    let val = if value.contains(':') { value.to_owned() } else { format!("minecraft:{value}") };
    if !pat.contains('*') {
        return pat == val;
    }
    let parts: Vec<&str> = pat.split('*').collect();
    let mut rest = val.as_str();
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            match rest.strip_prefix(part) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(at) => rest = &rest[at + part.len()..],
                None => return false,
            }
        }
    }
    rest.is_empty()
}

impl AttributeSettings {
    pub fn is_ignored_attribute(&self, attribute: &str) -> bool {
        !self.synced_attributes.iter().any(|w| matches_wildcard(w, attribute))
    }

    pub fn is_ignored_modifier(&self, modifier: &str) -> bool {
        self.ignored_modifiers.iter().any(|w| matches_wildcard(w, modifier))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SaveOnDeath {
    pub enabled: bool,
    pub items_to_save: String,
    pub save_empty_items: bool,
    pub sync_dead_players_changing_server: bool,
}

impl Default for SaveOnDeath {
    fn default() -> Self {
        SaveOnDeath {
            enabled: false,
            items_to_save: "DROPS".into(),
            save_empty_items: true,
            sync_dead_players_changing_server: true,
        }
    }
}

/// `synchronization.features`: which data types are saved and applied.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Features {
    pub inventory: bool,
    pub ender_chest: bool,
    pub potion_effects: bool,
    pub advancements: bool,
    pub location: bool,
    pub statistics: bool,
    pub health: bool,
    pub hunger: bool,
    pub attributes: bool,
    pub experience: bool,
    pub game_mode: bool,
    pub flight_status: bool,
    pub persistent_data: bool,
}

impl Default for Features {
    fn default() -> Self {
        Features {
            inventory: true,
            ender_chest: true,
            potion_effects: true,
            advancements: true,
            location: false,
            statistics: true,
            health: true,
            hunger: true,
            attributes: true,
            experience: true,
            game_mode: true,
            flight_status: true,
            persistent_data: true,
        }
    }
}

impl Features {
    /// Whether a built-in key (`husksync:<name>`) is on; other namespaces are always kept.
    pub fn enabled(&self, key: &str) -> bool {
        let Some(name) = key.strip_prefix("husksync:") else {
            return true;
        };
        match name {
            "inventory" => self.inventory,
            "ender_chest" => self.ender_chest,
            "potion_effects" => self.potion_effects,
            "advancements" => self.advancements,
            "location" => self.location,
            "statistics" => self.statistics,
            "health" => self.health,
            "hunger" => self.hunger,
            "attributes" => self.attributes,
            "experience" => self.experience,
            "game_mode" => self.game_mode,
            "flight_status" => self.flight_status,
            "persistent_data" => self.persistent_data,
            _ => false,
        }
    }
}

impl Config {
    /// Reads `config.toml` in `dir`, writing the defaults there first when it is missing.
    pub fn load_or_create(dir: &Path) -> Result<Config, String> {
        let c = Self::read(dir)?;
        if !c.redis.sentinel.master.is_empty() {
            log::warn!("horizonsync: redis.sentinel is not supported; connecting to redis.host directly");
        }
        Ok(c)
    }

    fn read(dir: &Path) -> Result<Config, String> {
        let file = dir.join("config.toml");
        if !file.exists() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            std::fs::write(&file, crate::DEFAULT_CONFIG).map_err(|e| e.to_string())?;
        }
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let text = flatten_huskysync_layout(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))
    }

    /// The Bukkit world for a dimension id, and back.
    pub fn world_for(&self, dimension: &str) -> crate::data::World {
        match self.worlds.get(dimension) {
            Some(w) => crate::data::World {
                name: w.name.clone(),
                uuid: if w.uuid.is_empty() { placeholder_uuid(&w.name) } else { w.uuid.clone() },
                environment: if w.environment.is_empty() { "NORMAL".into() } else { w.environment.clone() },
            },
            None => crate::data::World {
                name: dimension.rsplit(':').next().unwrap_or(dimension).to_owned(),
                uuid: placeholder_uuid(dimension),
                environment: "CUSTOM".into(),
            },
        }
    }

    pub fn dimension_for(&self, world: &crate::data::World) -> Option<String> {
        self.worlds
            .iter()
            .find(|(_, w)| w.name == world.name || (!w.uuid.is_empty() && w.uuid == world.uuid))
            .map(|(d, _)| d.clone())
            .or_else(|| match world.environment.as_str() {
                "NORMAL" => Some("minecraft:overworld".into()),
                "NETHER" => Some("minecraft:the_nether".into()),
                "THE_END" => Some("minecraft:the_end".into()),
                _ => None,
            })
    }
}

/// HuskSync's config.yml nests the connection settings (`database.credentials`,
/// `database.connection_pool`, `redis.credentials`). Those tables are lifted into their parent so a
/// config copied from a Paper server loads unchanged; a key already set in the parent wins.
fn flatten_huskysync_layout(text: &str) -> Result<String, String> {
    let mut root: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    flatten_table(&mut root, "database", &["credentials", "connection_pool"]);
    flatten_table(&mut root, "redis", &["credentials"]);
    toml::to_string(&root).map_err(|e| e.to_string())
}

fn flatten_table(root: &mut toml::Table, section: &str, nested: &[&str]) {
    let Some(toml::Value::Table(table)) = root.get_mut(section) else { return };
    for name in nested {
        if let Some(toml::Value::Table(inner)) = table.remove(*name) {
            for (key, value) in inner {
                if !table.contains_key(&key) {
                    table.insert(key, value);
                }
            }
        }
    }
}

/// A stable stand-in for a Bukkit world UUID (Paper matches worlds by name first).
fn placeholder_uuid(name: &str) -> String {
    let mut h: u128 = 0xcbf2_9ce4_8422_2325_cbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h ^= b as u128;
        h = h.wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b);
    }
    crate::util::uuid_string(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_huskysync_layout_loads() {
        let text = "[database]\ntype = \"MYSQL\"\n[database.credentials]\nhost = \"db.internal\"\nport = 3307\n\
                    [database.connection_pool]\nmaximum_pool_size = 4\n[redis.credentials]\nhost = \"r.internal\"\n";
        let c: Config = toml::from_str(&flatten_huskysync_layout(text).unwrap()).unwrap();
        assert_eq!(c.database.kind, "MYSQL");
        assert_eq!(c.database.host, "db.internal");
        assert_eq!(c.database.port, 3307);
        assert_eq!(c.database.maximum_pool_size, 4);
        assert_eq!(c.redis.host, "r.internal");
    }

    #[test]
    fn default_file_parses_and_matches_defaults() {
        let c: Config = toml::from_str(crate::DEFAULT_CONFIG).unwrap();
        assert!(c.synchronization.lockstep());
        assert!(c.synchronization.auto_pin("INVENTORY_COMMAND"));
        assert!(!c.synchronization.features.location);
        assert!(c.synchronization.features.enabled("husksync:inventory"));
        assert!(c.synchronization.features.enabled("otherplugin:data"));
        assert!(c.synchronization.command_blocked_while_locked("/spawn"));
        let a = &c.synchronization.attributes;
        assert!(!a.is_ignored_attribute("minecraft:max_health"));
        assert!(!a.is_ignored_attribute("scale"));
        assert!(a.is_ignored_attribute("minecraft:attack_damage"));
        assert!(a.is_ignored_modifier("minecraft:effect.speed"));
        assert!(a.is_ignored_modifier("minecraft:creative_mode_block_range"));
        assert!(!a.is_ignored_modifier("minecraft:armor.helmet"));
        assert_eq!(c.world_for("minecraft:the_nether").name, "world_nether");
        assert_eq!(
            c.dimension_for(&c.world_for("minecraft:the_end")).as_deref(),
            Some("minecraft:the_end")
        );
    }
}
