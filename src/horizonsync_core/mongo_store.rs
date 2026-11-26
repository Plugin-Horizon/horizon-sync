//! `MongoDbDatabase`: HuskSync's MongoDB storage on the official driver's blocking API. The collections carry the
//! configured table names; a document holds the columns of the SQL tables as fields (`uuid`, `username` in the users
//! collection; `player_uuid`, `version_uuid`, `timestamp`, `save_cause`, `pinned`, `data` in the user data collection;
//! `server_name`, `map_id`, `data` and `from_server_name`, `from_id`, `to_server_name`, `to_id` for maps). Uuids are
//! their text form, the data is a generic binary.

use crate::horizonsync_core::config::{Config, Database as DbConfig, TableNames};
use crate::horizonsync_core::db::{stored_from, Stored};
use crate::horizonsync_core::engine::Store;
use crate::horizonsync_core::snapshot::Packed;
use crate::horizonsync_core::util;
use mongodb::bson::spec::BinarySubtype;
use mongodb::bson::{doc, Binary, Bson, DateTime, Document};
use mongodb::sync::{Client, Collection, Database};
use std::time::Duration;

pub struct MongoDb {
    _client: Client,
    db: Database,
    t: TableNames,
    compress: bool,
    max_snapshots: i64,
    backup_frequency_hours: i64,
}

fn err(e: impl std::fmt::Display) -> String {
    // The driver's messages name hosts, never the credentials.
    format!("database: {e}")
}

/// Percent-encodes a user name or password for a connection string.
fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The connection string of a `database` section: `mongodb://user:pass@host:port/?authSource=admin`.
pub fn uri(c: &DbConfig) -> String {
    let auth = if c.username.is_empty() { String::new() } else { format!("{}:{}@", enc(&c.username), enc(&c.password)) };
    let mut q = Vec::new();
    if !c.username.is_empty() {
        q.push("authSource=admin");
    }
    if c.use_ssl {
        q.push("tls=true");
    }
    q.push("serverSelectionTimeoutMS=");
    let timeout = c.connection_timeout.max(1000).to_string();
    let query = q.iter().map(|p| if p.ends_with('=') { format!("{p}{timeout}") } else { (*p).to_owned() }).collect::<Vec<_>>().join("&");
    format!("mongodb://{auth}{}:{}/?{query}", c.host, c.port)
}

fn binary(bytes: Vec<u8>) -> Bson {
    Bson::Binary(Binary { subtype: BinarySubtype::Generic, bytes })
}

fn bytes_of(d: &Document, key: &str) -> Option<Vec<u8>> {
    match d.get(key)? {
        Bson::Binary(b) => Some(b.bytes.clone()),
        _ => None,
    }
}

fn int_of(d: &Document, key: &str) -> Option<i32> {
    match d.get(key)? {
        Bson::Int32(i) => Some(*i),
        Bson::Int64(i) => i32::try_from(*i).ok(),
        _ => None,
    }
}

fn doc_stored(d: &Document, compress: bool) -> Stored {
    stored_from(d.get_str("version_uuid").unwrap_or_default().to_owned(), bytes_of(d, "data"), compress)
}

/// The document of a snapshot row.
pub fn snapshot_doc(player: u128, s: &Packed, time: &chrono::DateTime<chrono::FixedOffset>, compress: bool) -> Document {
    doc! {
        "player_uuid": util::uuid_string(player),
        "version_uuid": &s.id,
        "timestamp": DateTime::from_millis(time.timestamp_millis()),
        "save_cause": &s.save_cause,
        "pinned": s.pinned,
        "data": binary(s.to_bytes(compress)),
    }
}

impl MongoDb {
    pub fn connect(config: &Config) -> Result<MongoDb, String> {
        let c = &config.database;
        let client = Client::with_uri_str(uri(c)).map_err(err)?;
        let db = client.database(&c.database);
        let m = MongoDb {
            _client: client,
            db,
            t: c.table_names.clone(),
            compress: config.synchronization.compress_data,
            max_snapshots: config.synchronization.max_user_data_snapshots,
            backup_frequency_hours: config.synchronization.snapshot_backup_frequency,
        };
        // Reaches the server now, so a wrong host fails at start and not at the first join.
        m.ping()?;
        if c.create_tables {
            let _ = m.users().create_index(mongodb::IndexModel::builder().keys(doc! {"uuid": 1}).options(mongodb::options::IndexOptions::builder().unique(true).build()).build()).run();
            let _ = m.data().create_index(mongodb::IndexModel::builder().keys(doc! {"player_uuid": 1, "timestamp": -1}).build()).run();
            let _ = m.data().create_index(mongodb::IndexModel::builder().keys(doc! {"version_uuid": 1}).build()).run();
        }
        Ok(m)
    }

    fn col(&self, name: &str) -> Collection<Document> {
        self.db.collection(name)
    }
    fn users(&self) -> Collection<Document> {
        self.col(&self.t.users)
    }
    fn data(&self) -> Collection<Document> {
        self.col(&self.t.user_data)
    }

    fn snapshots(&self, filter: Document, limit: Option<i64>) -> Result<Vec<Stored>, String> {
        let mut find = self.data().find(filter).sort(doc! {"timestamp": -1});
        if let Some(n) = limit {
            find = find.limit(n);
        }
        let docs = find.run().map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
        Ok(docs.iter().map(|d| doc_stored(d, self.compress)).collect())
    }

    fn create_snapshot(&self, player: u128, s: &Packed) -> Result<(), String> {
        let time = s.time().unwrap_or_else(util::now);
        self.data().insert_one(snapshot_doc(player, s, &time, self.compress)).run().map(|_| ()).map_err(err)
    }

    /// `rotateSnapshots`: the oldest unpinned snapshots beyond the limit go.
    fn rotate_snapshots(&self, player: u128) -> Result<(), String> {
        let filter = doc! {"player_uuid": util::uuid_string(player), "pinned": false};
        let n = self.data().count_documents(filter.clone()).run().map_err(err)? as i64;
        if n > self.max_snapshots {
            let oldest = self
                .data()
                .find(filter)
                .sort(doc! {"timestamp": 1})
                .limit(n - self.max_snapshots)
                .run()
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)?;
            let ids: Vec<Bson> = oldest.iter().filter_map(|d| d.get("_id").cloned()).collect();
            if !ids.is_empty() {
                self.data().delete_many(doc! {"_id": {"$in": ids}}).run().map_err(err)?;
            }
        }
        Ok(())
    }
}

impl Store for MongoDb {
    fn ensure_user(&self, uuid: u128, name: &str) -> Result<(), String> {
        self.users()
            .update_one(doc! {"uuid": util::uuid_string(uuid)}, doc! {"$set": {"username": name}})
            .upsert(true)
            .run()
            .map(|_| ())
            .map_err(err)
    }

    fn user_by_name(&self, name: &str) -> Result<Option<(u128, String)>, String> {
        // `LOWER(username)=LOWER(?)` of the SQL storages, as a case-insensitive exact match.
        let pattern = format!("^{}$", name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c.to_string() } else { format!("\\{c}") }).collect::<String>());
        let d = self.users().find_one(doc! {"username": {"$regex": pattern, "$options": "i"}}).run().map_err(err)?;
        Ok(d.and_then(|d| Some((util::parse_uuid(d.get_str("uuid").ok()?)?, d.get_str("username").ok()?.to_owned()))))
    }

    fn user_by_uuid(&self, uuid: u128) -> Result<Option<String>, String> {
        let d = self.users().find_one(doc! {"uuid": util::uuid_string(uuid)}).run().map_err(err)?;
        Ok(d.and_then(|d| d.get_str("username").ok().map(str::to_owned)))
    }

    fn latest_snapshot(&self, player: u128) -> Result<Option<Stored>, String> {
        Ok(self.snapshots(doc! {"player_uuid": util::uuid_string(player)}, Some(1))?.into_iter().next())
    }

    fn latest_snapshot_with_causes(&self, player: u128, causes: &[&str]) -> Result<Option<Stored>, String> {
        if causes.is_empty() {
            return Ok(None);
        }
        let list: Vec<&str> = causes.to_vec();
        Ok(self.snapshots(doc! {"player_uuid": util::uuid_string(player), "save_cause": {"$in": list}}, Some(1))?.into_iter().next())
    }

    fn all_snapshots(&self, player: u128) -> Result<Vec<Stored>, String> {
        self.snapshots(doc! {"player_uuid": util::uuid_string(player)}, None)
    }

    fn snapshot(&self, player: u128, version: u128) -> Result<Option<Stored>, String> {
        Ok(self.snapshots(doc! {"player_uuid": util::uuid_string(player), "version_uuid": util::uuid_string(version)}, Some(1))?.into_iter().next())
    }

    fn add_snapshot(&self, player: u128, s: &Packed) -> Result<(), String> {
        if !s.pinned && self.backup_frequency_hours > 0 {
            if let Some(t) = s.time() {
                let cut = t - chrono::Duration::hours(self.backup_frequency_hours);
                let newer = self
                    .data()
                    .find(doc! {"player_uuid": util::uuid_string(player), "pinned": false, "timestamp": {"$gt": DateTime::from_millis(cut.timestamp_millis())}})
                    .sort(doc! {"timestamp": 1})
                    .limit(1)
                    .run()
                    .map_err(err)?
                    .next()
                    .transpose()
                    .map_err(err)?;
                if let Some(id) = newer.and_then(|d| d.get("_id").cloned()) {
                    self.data().delete_one(doc! {"_id": id}).run().map_err(err)?;
                }
            }
        }
        self.create_snapshot(player, s)?;
        self.rotate_snapshots(player)
    }

    fn add_snapshot_without_rotation(&self, player: u128, s: &Packed) -> Result<(), String> {
        self.create_snapshot(player, s)
    }

    fn update_snapshot(&self, player: u128, s: &Packed) -> Result<(), String> {
        let set = doc! {"save_cause": &s.save_cause, "pinned": s.pinned, "data": binary(s.to_bytes(self.compress))};
        self.data()
            .update_one(doc! {"player_uuid": util::uuid_string(player), "version_uuid": &s.id}, doc! {"$set": set})
            .run()
            .map(|_| ())
            .map_err(err)
    }

    fn delete_snapshot(&self, player: u128, version: u128) -> Result<bool, String> {
        let r = self.data().delete_one(doc! {"player_uuid": util::uuid_string(player), "version_uuid": util::uuid_string(version)}).run().map_err(err)?;
        Ok(r.deleted_count > 0)
    }

    fn set_pinned(&self, player: u128, version: u128, pinned: bool) -> Result<bool, String> {
        let Some(stored) = self.snapshot(player, version)? else {
            return Ok(false);
        };
        let mut s = stored.snapshot?;
        s.pinned = pinned;
        self.update_snapshot(player, &s)?;
        Ok(true)
    }

    fn ping(&self) -> Result<Duration, String> {
        let start = std::time::Instant::now();
        self.db.run_command(doc! {"ping": 1}).run().map_err(err)?;
        Ok(start.elapsed())
    }

    fn map_file(&self, server: &str, id: i32) -> Result<Option<Vec<u8>>, String> {
        let d = self.col(&self.t.map_data).find_one(doc! {"server_name": server, "map_id": id}).run().map_err(err)?;
        Ok(d.and_then(|d| bytes_of(&d, "data")).and_then(|c| crate::horizonsync_core::maps::from_column(&c)))
    }

    fn save_map_file(&self, server: &str, id: i32, file: &[u8]) -> Result<(), String> {
        let column = crate::horizonsync_core::maps::to_column(file, self.compress);
        self.col(&self.t.map_data)
            .update_one(doc! {"server_name": server, "map_id": id}, doc! {"$set": {"data": binary(column)}})
            .upsert(true)
            .run()
            .map(|_| ())
            .map_err(err)
    }

    fn bound_map_id(&self, from_server: &str, from_id: i32, to_server: &str) -> Result<Option<i32>, String> {
        let d = self
            .col(&self.t.map_ids)
            .find_one(doc! {"from_server_name": from_server, "from_id": from_id, "to_server_name": to_server})
            .run()
            .map_err(err)?;
        Ok(d.and_then(|d| int_of(&d, "to_id")))
    }

    fn set_map_binding(&self, from_server: &str, from_id: i32, to_server: &str, to_id: i32) -> Result<(), String> {
        self.col(&self.t.map_ids)
            .update_one(
                doc! {"from_server_name": from_server, "from_id": from_id, "to_server_name": to_server},
                doc! {"$set": {"to_id": to_id}},
            )
            .upsert(true)
            .run()
            .map(|_| ())
            .map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packed() -> Packed {
        Packed {
            id: "11111111-2222-3333-4444-555555555555".into(),
            pinned: true,
            timestamp: "2026-11-26T10:00:00Z".into(),
            save_cause: "DISCONNECT".into(),
            server_name: Some("rec".into()),
            minecraft_version: "26.3".into(),
            platform_type: "bukkit".into(),
            format_version: 6,
            data: Default::default(),
        }
    }

    #[test]
    fn the_connection_string_escapes_the_credentials() {
        let mut c = DbConfig::default();
        c.host = "mongo.local".into();
        c.port = 27018;
        c.username = "us er".into();
        c.password = "p@ss:/".into();
        c.connection_timeout = 7000;
        assert_eq!(uri(&c), "mongodb://us%20er:p%40ss%3A%2F@mongo.local:27018/?authSource=admin&serverSelectionTimeoutMS=7000");
        c.username.clear();
        c.use_ssl = true;
        assert_eq!(uri(&c), "mongodb://mongo.local:27018/?tls=true&serverSelectionTimeoutMS=7000");
    }

    #[test]
    fn a_snapshot_document_round_trips() {
        let s = packed();
        let time = s.time().unwrap();
        let d = snapshot_doc(0x1234, &s, &time, true);
        assert_eq!(d.get_str("player_uuid").unwrap(), util::uuid_string(0x1234));
        assert_eq!(d.get_str("version_uuid").unwrap(), s.id);
        assert_eq!(d.get_datetime("timestamp").unwrap().timestamp_millis(), time.timestamp_millis());
        assert!(d.get_bool("pinned").unwrap());
        let back = doc_stored(&d, true);
        assert_eq!(back.version_uuid, s.id);
        assert_eq!(back.snapshot.unwrap().save_cause, "DISCONNECT");
    }

    #[test]
    fn a_document_without_data_is_unusable() {
        let d = doc! {"version_uuid": "x"};
        assert!(doc_stored(&d, false).snapshot.is_err());
    }

    #[test]
    fn integers_come_back_from_either_width() {
        let d = doc! {"a": 5i32, "b": 6i64, "c": "x"};
        assert_eq!((int_of(&d, "a"), int_of(&d, "b"), int_of(&d, "c")), (Some(5), Some(6), None));
    }
}
