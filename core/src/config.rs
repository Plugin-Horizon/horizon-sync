//! HuskSync's `plugins/HuskSync/config.yml` (with `server.yml`), read as it is; `config/horizonsync/
//! config.toml` is the fallback when that file is missing. Same key names, same defaults.

use crate::messages::Messages;
use crate::yaml;
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
    /// `messages-<language>.yml` (not part of the TOML fallback).
    #[serde(skip)]
    pub messages: Messages,
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
            messages: Messages::default(),
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
            // No placeholder password: a generated default must not connect with a guessable one.
            password: String::new(),
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
    /// The configuration of this server. `server_dir` is the server's folder: when HuskSync's
    /// `plugins/HuskSync/config.yml` is there it is read (with `server.yml` and the locale file),
    /// otherwise the port's `toml_dir/config.toml`, written with the defaults first when missing.
    pub fn load(server_dir: &Path, toml_dir: &Path) -> Result<Config, String> {
        let plugin_dir = server_dir.join("plugins").join("HuskSync");
        let c = if plugin_dir.join("config.yml").is_file() {
            Self::from_huskysync(&plugin_dir)?
        } else {
            Self::read(toml_dir)?
        };
        if !c.redis.sentinel.master.is_empty() {
            log::warn!("horizonsync: redis.sentinel is not supported; connecting to redis.host directly");
        }
        Ok(c)
    }

    /// HuskSync's own files: `config.yml`, `server.yml`'s `name` (`HUSKSYNC_SERVER_NAME` overrides
    /// it, as in HuskSync) and `messages-<language>.yml`.
    pub fn from_huskysync(plugin_dir: &Path) -> Result<Config, String> {
        let file = plugin_dir.join("config.yml");
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let node = yaml::parse(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut table = yaml_table(&node);
        let server = plugin_dir.join("server.yml");
        if server.is_file() {
            let text = std::fs::read_to_string(&server).map_err(|e| format!("{}: {e}", server.display()))?;
            let node = yaml::parse(&text).map_err(|e| format!("{}: {e}", server.display()))?;
            if let Some(name) = node.get("name").and_then(yaml::Node::as_str) {
                table.insert("server_name".into(), toml::Value::String(name.to_owned()));
            }
        }
        if let Ok(name) = std::env::var("HUSKSYNC_SERVER_NAME") {
            if !name.trim().is_empty() {
                table.insert("server_name".into(), toml::Value::String(name));
            }
        }
        flatten_table(&mut table, "database", &["credentials", "connection_pool"]);
        flatten_table(&mut table, "redis", &["credentials"]);
        let text = toml::to_string(&table).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut config: Config = toml::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        config.messages = Messages::load(plugin_dir, &config.language);
        Ok(config)
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

/// Keys whose value is text even when it reads as a number or a boolean (a numeric password).
const STRING_KEYS: &[&str] = &[
    "password", "username", "host", "user", "master", "parameters", "name", "language",
    "cluster_id", "server_name", "type", "mode", "items_to_save", "notification_display_slot", "uuid",
    "environment",
];

/// A YAML mapping as a TOML table, scalars typed as SnakeYAML types them (`int`, `float`, `bool`,
/// otherwise text). Null values are left out.
fn yaml_table(node: &yaml::Node) -> toml::Table {
    let mut table = toml::Table::new();
    for entry in node.entries() {
        if let Some(v) = yaml_value(&entry.key, &entry.value) {
            table.insert(entry.key.clone(), v);
        }
    }
    table
}

fn yaml_value(key: &str, node: &yaml::Node) -> Option<toml::Value> {
    if node.is_map() {
        return Some(toml::Value::Table(yaml_table(node)));
    }
    if node.is_seq() {
        return Some(toml::Value::Array(node.items().iter().filter_map(|n| yaml_value(key, n)).collect()));
    }
    let text = node.as_str()?;
    if STRING_KEYS.contains(&key) {
        return Some(toml::Value::String(text.to_owned()));
    }
    Some(match node.tag() {
        "int" => node.as_i64().map(toml::Value::Integer),
        "float" => node.as_f64().map(toml::Value::Float),
        "bool" => node.as_bool().map(toml::Value::Boolean),
        _ => None,
    }
    .unwrap_or_else(|| toml::Value::String(text.to_owned())))
}

/// The key paths of `config.yml` and `server.yml` that the loader reads (parsed into `Config`).
/// Keys HuskSync has and this port does not use (`database.mongo_settings`,
/// `synchronization.event_priorities`) are not in the list.
pub const CONFIG_READ_KEYS: &[&str] = &[
    "language", "check_for_updates", "cluster_id", "debug_logging", "enable_plan_hook", "cancel_packets",
    "disabled_commands",
    "database.type",
    "database.credentials.host", "database.credentials.port", "database.credentials.database",
    "database.credentials.username", "database.credentials.password", "database.credentials.parameters",
    "database.connection_pool.maximum_pool_size", "database.connection_pool.minimum_idle",
    "database.connection_pool.maximum_lifetime", "database.connection_pool.keepalive_time",
    "database.connection_pool.connection_timeout",
    "database.table_names.map_ids", "database.table_names.user_data", "database.table_names.map_data",
    "database.table_names.users", "database.create_tables",
    "redis.credentials.host", "redis.credentials.port", "redis.credentials.database",
    "redis.credentials.user", "redis.credentials.password", "redis.credentials.use_ssl",
    "redis.credentials.connection_timeout", "redis.credentials.socket_timeout",
    "redis.credentials.max_total_connections", "redis.credentials.max_idle_connections",
    "redis.credentials.min_idle_connections", "redis.credentials.test_on_borrow",
    "redis.credentials.test_on_return", "redis.credentials.test_while_idle",
    "redis.credentials.min_evictable_idle_time_millis", "redis.credentials.time_between_eviction_runs_millis",
    "redis.credentials.max_retries", "redis.credentials.retry_backoff_millis",
    "redis.sentinel.master", "redis.sentinel.nodes",
    "synchronization.mode", "synchronization.max_user_data_snapshots",
    "synchronization.snapshot_backup_frequency", "synchronization.auto_pinned_save_causes",
    "synchronization.save_on_world_save", "synchronization.save_on_death.enabled",
    "synchronization.save_on_death.items_to_save", "synchronization.save_on_death.save_empty_items",
    "synchronization.save_on_death.sync_dead_players_changing_server", "synchronization.compress_data",
    "synchronization.notification_display_slot", "synchronization.persist_locked_maps",
    "synchronization.network_latency_milliseconds", "synchronization.shutdown_save_timeout_milliseconds",
    "synchronization.features.advancements", "synchronization.features.attributes",
    "synchronization.features.location", "synchronization.features.ender_chest",
    "synchronization.features.flight_status", "synchronization.features.potion_effects",
    "synchronization.features.experience", "synchronization.features.health",
    "synchronization.features.statistics", "synchronization.features.hunger",
    "synchronization.features.inventory", "synchronization.features.persistent_data",
    "synchronization.features.game_mode",
    "synchronization.blacklisted_commands_while_locked", "synchronization.allow_pearl_spawning_while_locked",
    "synchronization.checkin_petitions",
    "synchronization.attributes.synced_attributes", "synchronization.attributes.ignored_modifiers",
];

/// The key paths of `server.yml` that the loader reads.
pub const SERVER_READ_KEYS: &[&str] = &["name"];

/// Which key paths of a file were read (`read`) and which were not (`not_read`): key paths only,
/// never values. `read` holds the listed keys the file has; a key the loader does not know is in
/// `not_read`, and so is every key under a list.
#[derive(Debug, Default, PartialEq)]
pub struct KeyReport {
    pub read: Vec<String>,
    pub not_read: Vec<String>,
}

pub fn key_report(root: &yaml::Node, read: &[&str]) -> KeyReport {
    let mut report = KeyReport::default();
    walk_keys(root, "", read, &mut report);
    report
}

fn walk_keys(node: &yaml::Node, prefix: &str, read: &[&str], report: &mut KeyReport) {
    for entry in node.entries() {
        let path = if prefix.is_empty() { entry.key.clone() } else { format!("{prefix}.{}", entry.key) };
        if read.contains(&path.as_str()) {
            report.read.push(path);
        } else if entry.value.is_map() {
            walk_keys(&entry.value, &path, read, report);
        } else {
            report.not_read.push(path);
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

#[cfg(test)]
mod yaml_tests {
    use super::*;

    /// A HuskSync-layout `config.yml` with invented values (not a copy of any server's file).
    const CONFIG_YML: &str = "# test fixture\nlanguage: en-gb\ncheck_for_updates: true\ncluster_id: ''\n\
        debug_logging: false\ndisabled_commands: []\ndatabase:\n  type: MARIADB\n  credentials:\n    host: db.test.invalid\n\
        \x20   port: 3307\n    database: HuskSync\n    username: tester\n    password: 12345\n\
        \x20   parameters: ?autoReconnect=true&useSSL=false\n  connection_pool:\n    maximum_pool_size: 4\n\
        \x20   minimum_idle: 2\n    maximum_lifetime: 1800000\n    keepalive_time: 0\n    connection_timeout: 5000\n\
        \x20 mongo_settings:\n    using_atlas: false\n  table_names:\n    map_ids: test_map_ids\n    user_data: test_user_data\n\
        \x20   map_data: test_map_data\n    users: test_users\n  create_tables: true\nredis:\n  credentials:\n    host: redis.test.invalid\n\
        \x20   port: 6380\n    database: 1\n    user: ''\n    use_ssl: false\n    max_retries: 2\n  sentinel:\n    master: ''\n\
        \x20   nodes: []\nsynchronization:\n  mode: LOCKSTEP\n  max_user_data_snapshots: 8\n  snapshot_backup_frequency: 0\n\
        \x20 auto_pinned_save_causes:\n  - INVENTORY_COMMAND\n  - BACKUP_RESTORE\n  save_on_world_save: false\n\
        \x20 save_on_death:\n    enabled: true\n    items_to_save: ITEMS_TO_KEEP\n  compress_data: false\n\
        \x20 notification_display_slot: CHAT\n  persist_locked_maps: false\n  features:\n    inventory: true\n\
        \x20   location: true\n    flight_status: false\n  blacklisted_commands_while_locked:\n  - '*'\n\
        \x20 allow_pearl_spawning_while_locked: false\n  attributes:\n    synced_attributes:\n    - minecraft:generic.max_health\n\
        \x20   ignored_modifiers:\n    - minecraft:effect.*\n  event_priorities:\n    death_listener: NORMAL\n";

    const SERVER_YML: &str = "# test fixture\nname: test_server\n";

    const MESSAGES_YML: &str = "locales:\n  synchronization_complete: <color:#123456>Synced %1%</color>\n  \
        data_update_failed: <red>Update failed</red>\n";

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("horizonsync-config-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn huskysync_files_are_read_as_paper_writes_them() {
        let root = scratch("yaml");
        let plugin = root.join("plugins").join("HuskSync");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("config.yml"), CONFIG_YML).unwrap();
        std::fs::write(plugin.join("server.yml"), SERVER_YML).unwrap();
        std::fs::write(plugin.join("messages-en-gb.yml"), MESSAGES_YML).unwrap();

        let c = Config::from_huskysync(&plugin).unwrap();
        assert_eq!(c.server_name, "test_server");
        assert_eq!(c.database.host, "db.test.invalid");
        assert_eq!(c.database.port, 3307);
        assert_eq!(c.database.username, "tester");
        assert_eq!(c.database.password, "12345");
        assert_eq!(c.database.parameters, "?autoReconnect=true&useSSL=false");
        assert_eq!(c.database.maximum_pool_size, 4);
        assert_eq!(c.database.table_names.user_data, "test_user_data");
        assert_eq!(c.redis.host, "redis.test.invalid");
        assert_eq!(c.redis.database, 1);
        assert_eq!(c.redis.max_retries, 2);
        assert_eq!(c.redis.connection_timeout, 2000, "not in the file: the default");
        assert!(c.synchronization.lockstep());
        assert_eq!(c.synchronization.snapshot_backup_frequency, 0);
        assert_eq!(c.synchronization.auto_pinned_save_causes, ["INVENTORY_COMMAND", "BACKUP_RESTORE"]);
        assert!(c.synchronization.save_on_death.enabled);
        assert_eq!(c.synchronization.save_on_death.items_to_save, "ITEMS_TO_KEEP");
        assert!(!c.synchronization.compress_data);
        assert_eq!(c.synchronization.notification_display_slot, "CHAT");
        assert!(c.synchronization.features.location);
        assert!(!c.synchronization.features.flight_status);
        assert!(c.synchronization.features.inventory);
        assert_eq!(c.synchronization.blacklisted_commands_while_locked, ["*"]);
        assert!(!c.synchronization.allow_pearl_spawning_while_locked);
        assert_eq!(c.synchronization.attributes.synced_attributes, ["minecraft:generic.max_health"]);
        assert_eq!(c.synchronization.attributes.ignored_modifiers, ["minecraft:effect.*"]);
        assert_eq!(c.messages.lines("synchronization_complete", &["x"])[0].plain, "Synced x");
        assert_eq!(c.messages.lines("data_update_failed", &[])[0].plain, "Update failed");
        // The file has no `user`/`sentinel` surprises: a missing locale key falls back to the built-in text.
        assert_eq!(c.messages.lines("user_registration_complete", &[])[0].plain, "⭐ User registration complete!");

        // `Config::load` prefers the HuskSync files over the TOML fallback.
        let toml_dir = root.join("config").join("horizonsync");
        let loaded = Config::load(&root, &toml_dir).unwrap();
        assert_eq!(loaded.database.host, "db.test.invalid");
        assert!(!toml_dir.exists(), "the fallback is not written when the HuskSync file is there");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn toml_fallback_is_used_and_written_without_a_huskysync_file() {
        let root = scratch("toml");
        let toml_dir = root.join("config").join("horizonsync");
        let c = Config::load(&root, &toml_dir).unwrap();
        assert!(toml_dir.join("config.toml").is_file(), "defaults written");
        assert!(c.database.password.is_empty(), "no placeholder password in the defaults");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn key_report_names_what_is_read_and_what_is_not() {
        let node = yaml::parse(CONFIG_YML).unwrap();
        let report = key_report(&node, CONFIG_READ_KEYS);
        assert!(report.read.iter().any(|k| k == "database.credentials.password"));
        assert!(report.read.iter().any(|k| k == "synchronization.features.location"));
        assert!(report.not_read.iter().any(|k| k == "database.mongo_settings.using_atlas"));
        assert!(report.not_read.iter().any(|k| k == "synchronization.event_priorities.death_listener"));
        assert!(!report.not_read.iter().any(|k| k.starts_with("database.credentials.")));
    }

    #[test]
    fn numeric_text_stays_text_for_string_keys() {
        let node = yaml::parse("database:\n  credentials:\n    password: 0x1F\n    port: 0x1F\n").unwrap();
        let t = yaml_table(&node);
        let cred = t["database"]["credentials"].as_table().unwrap();
        assert_eq!(cred["password"].as_str(), Some("0x1F"));
        assert_eq!(cred["port"].as_integer(), Some(31));
    }
}
