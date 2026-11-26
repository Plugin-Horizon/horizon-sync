//! `PostgresDatabase`: HuskSync's PostgreSQL storage on the blocking `postgres` client (no TLS), running the statements
//! of `pg_sql.rs`. One connection behind a mutex; a failed call drops it and the next call connects again.

use crate::horizonsync_core::config::{Config, TableNames};
use crate::horizonsync_core::db::{parse_zone, stored_from, Stored, Zone};
use crate::horizonsync_core::engine::Store;
use crate::horizonsync_core::pg_sql;
use crate::horizonsync_core::snapshot::Packed;
use crate::horizonsync_core::util;
use postgres::types::ToSql;
use postgres::{Client, NoTls, Row};
use std::sync::Mutex;
use std::time::Duration;

pub struct PgDb {
    conn: Mutex<Option<Client>>,
    cfg: postgres::Config,
    t: TableNames,
    zone: Zone,
    compress: bool,
    max_snapshots: i64,
    backup_frequency_hours: i64,
}

fn err(e: impl std::fmt::Display) -> String {
    // The driver's errors never carry the password.
    format!("database: {e}")
}

type P<'a> = &'a [&'a (dyn ToSql + Sync)];

/// The client settings of a `database` config section.
fn client_config(c: &crate::horizonsync_core::config::Database) -> Result<postgres::Config, String> {
    if c.use_ssl {
        return Err("database: use_ssl is not available for POSTGRES in this build (the client has no TLS)".into());
    }
    let mut p = postgres::Config::new();
    p.host(&c.host)
        .port(c.port)
        .dbname(&c.database)
        .user(&c.username)
        .connect_timeout(Duration::from_millis(c.connection_timeout.max(1000)));
    if !c.password.is_empty() {
        p.password(&c.password);
    }
    Ok(p)
}

fn row_stored(row: &Row, compress: bool) -> Stored {
    let version: String = row.try_get("version_uuid").unwrap_or_default();
    let data: Option<Vec<u8>> = row.try_get("data").ok();
    stored_from(version, data, compress)
}

impl PgDb {
    pub fn connect(config: &Config) -> Result<PgDb, String> {
        let c = &config.database;
        let cfg = client_config(c)?;
        let client = cfg.connect(NoTls).map_err(err)?;
        let db = PgDb {
            conn: Mutex::new(Some(client)),
            cfg,
            t: c.table_names.clone(),
            zone: parse_zone(&c.timestamp_zone),
            compress: config.synchronization.compress_data,
            max_snapshots: config.synchronization.max_user_data_snapshots,
            backup_frequency_hours: config.synchronization.snapshot_backup_frequency,
        };
        if c.create_tables {
            db.with(|cl| {
                for s in pg_sql::schema(&db.t) {
                    cl.batch_execute(&s)?;
                }
                Ok(())
            })?;
        }
        Ok(db)
    }

    /// Runs `f` on the connection, connecting again first when the last call failed.
    fn with<T>(&self, f: impl FnOnce(&mut Client) -> Result<T, postgres::Error>) -> Result<T, String> {
        let mut guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().is_none_or(|c| c.is_closed()) {
            *guard = Some(self.cfg.connect(NoTls).map_err(err)?);
        }
        let client = guard.as_mut().ok_or("database: no connection")?;
        match f(client) {
            Ok(v) => Ok(v),
            Err(e) => {
                if client.is_closed() {
                    *guard = None;
                }
                Err(err(e))
            }
        }
    }

    fn query(&self, sql: &str, params: P) -> Result<Vec<Row>, String> {
        self.with(|c| c.query(sql, params))
    }

    fn exec(&self, sql: &str, params: P) -> Result<u64, String> {
        self.with(|c| c.execute(sql, params))
    }

    fn create_snapshot(&self, player: u128, s: &Packed) -> Result<(), String> {
        let time = s.time().unwrap_or_else(util::now);
        let wall = self.zone.wall(&time);
        let (player, bytes) = (util::uuid_string(player), s.to_bytes(self.compress));
        self.exec(&pg_sql::insert_snapshot(&self.t), &[&player, &s.id, &wall, &s.save_cause, &s.pinned, &bytes]).map(|_| ())
    }

    fn rotate_snapshots(&self, player: u128) -> Result<(), String> {
        let id = util::uuid_string(player);
        let n: i64 = self.query(&pg_sql::unpinned_count(&self.t), &[&id])?.first().and_then(|r| r.try_get(0).ok()).unwrap_or(0);
        if n > self.max_snapshots {
            let extra = n - self.max_snapshots;
            self.exec(&pg_sql::rotate_snapshots(&self.t), &[&id, &extra])?;
        }
        Ok(())
    }
}

impl Store for PgDb {
    fn ensure_user(&self, uuid: u128, name: &str) -> Result<(), String> {
        self.exec(&pg_sql::insert_user(&self.t), &[&util::uuid_string(uuid), &name]).map(|_| ())
    }

    fn user_by_name(&self, name: &str) -> Result<Option<(u128, String)>, String> {
        let rows = self.query(&pg_sql::user_by_name(&self.t), &[&name])?;
        Ok(rows.first().and_then(|r| {
            let (u, n): (String, String) = (r.try_get(0).ok()?, r.try_get(1).ok()?);
            util::parse_uuid(&u).map(|u| (u, n))
        }))
    }

    fn user_by_uuid(&self, uuid: u128) -> Result<Option<String>, String> {
        let rows = self.query(&pg_sql::user_by_uuid(&self.t), &[&util::uuid_string(uuid)])?;
        Ok(rows.first().and_then(|r| r.try_get(0).ok()))
    }

    fn latest_snapshot(&self, player: u128) -> Result<Option<Stored>, String> {
        let rows = self.query(&pg_sql::latest_snapshot(&self.t), &[&util::uuid_string(player)])?;
        Ok(rows.first().map(|r| row_stored(r, self.compress)))
    }

    fn latest_snapshot_with_causes(&self, player: u128, causes: &[&str]) -> Result<Option<Stored>, String> {
        if causes.is_empty() {
            return Ok(None);
        }
        let list: Vec<String> = causes.iter().map(|c| (*c).to_owned()).collect();
        let rows = self.query(&pg_sql::latest_snapshot_with_causes(&self.t), &[&util::uuid_string(player), &list])?;
        Ok(rows.first().map(|r| row_stored(r, self.compress)))
    }

    fn all_snapshots(&self, player: u128) -> Result<Vec<Stored>, String> {
        let rows = self.query(&pg_sql::all_snapshots(&self.t), &[&util::uuid_string(player)])?;
        Ok(rows.iter().map(|r| row_stored(r, self.compress)).collect())
    }

    fn snapshot(&self, player: u128, version: u128) -> Result<Option<Stored>, String> {
        let rows = self.query(&pg_sql::snapshot(&self.t), &[&util::uuid_string(player), &util::uuid_string(version)])?;
        Ok(rows.first().map(|r| row_stored(r, self.compress)))
    }

    fn add_snapshot(&self, player: u128, s: &Packed) -> Result<(), String> {
        if !s.pinned && self.backup_frequency_hours > 0 {
            if let Some(t) = s.time() {
                let cut = self.zone.wall(&(t - chrono::Duration::hours(self.backup_frequency_hours)));
                self.exec(&pg_sql::rotate_latest(&self.t), &[&util::uuid_string(player), &cut])?;
            }
        }
        self.create_snapshot(player, s)?;
        self.rotate_snapshots(player)
    }

    fn add_snapshot_without_rotation(&self, player: u128, s: &Packed) -> Result<(), String> {
        self.create_snapshot(player, s)
    }

    fn update_snapshot(&self, player: u128, s: &Packed) -> Result<(), String> {
        let bytes = s.to_bytes(self.compress);
        self.exec(&pg_sql::update_snapshot(&self.t), &[&s.save_cause, &s.pinned, &bytes, &util::uuid_string(player), &s.id]).map(|_| ())
    }

    fn delete_snapshot(&self, player: u128, version: u128) -> Result<bool, String> {
        Ok(self.exec(&pg_sql::delete_snapshot(&self.t), &[&util::uuid_string(player), &util::uuid_string(version)])? > 0)
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
        self.with(|c| c.simple_query("SELECT 1").map(|_| ()))?;
        Ok(start.elapsed())
    }

    fn map_file(&self, server: &str, id: i32) -> Result<Option<Vec<u8>>, String> {
        let rows = self.query(&pg_sql::map_file(&self.t), &[&server, &id])?;
        let column: Option<Vec<u8>> = rows.first().and_then(|r| r.try_get(0).ok());
        Ok(column.and_then(|c| crate::horizonsync_core::maps::from_column(&c)))
    }

    fn save_map_file(&self, server: &str, id: i32, file: &[u8]) -> Result<(), String> {
        let column = crate::horizonsync_core::maps::to_column(file, self.compress);
        self.exec(&pg_sql::save_map(&self.t), &[&server, &id, &column]).map(|_| ())
    }

    fn bound_map_id(&self, from_server: &str, from_id: i32, to_server: &str) -> Result<Option<i32>, String> {
        let rows = self.query(&pg_sql::bound_map_id(&self.t), &[&from_server, &from_id, &to_server])?;
        Ok(rows.first().and_then(|r| r.try_get(0).ok()))
    }

    fn set_map_binding(&self, from_server: &str, from_id: i32, to_server: &str, to_id: i32) -> Result<(), String> {
        self.exec(&pg_sql::set_map_binding(&self.t), &[&from_server, &from_id, &to_server, &to_id]).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horizonsync_core::config::Database;

    #[test]
    fn client_settings_come_from_the_config() {
        let mut d = Database::default();
        d.kind = "POSTGRES".into();
        d.host = "db.local".into();
        d.port = 5433;
        d.database = "hs".into();
        d.username = "u".into();
        d.password = "p".into();
        let c = client_config(&d).unwrap();
        assert_eq!(c.get_dbname(), Some("hs"));
        assert_eq!(c.get_user(), Some("u"));
        assert_eq!(c.get_ports(), &[5433]);
        assert_eq!(c.get_password(), Some(&b"p"[..]));
        d.use_ssl = true;
        assert!(client_config(&d).unwrap_err().contains("TLS"));
    }

    #[test]
    fn a_failed_connection_says_so_without_the_password() {
        let mut config = Config::default();
        config.database.kind = "POSTGRES".into();
        config.database.host = "127.0.0.1".into();
        config.database.port = 1;
        config.database.password = "hunter2".into();
        let e = PgDb::connect(&config).err().unwrap();
        assert!(e.starts_with("database:") && !e.contains("hunter2"), "{e}");
    }

    #[test]
    fn rows_without_data_are_unusable() {
        let s = stored_from("abc".into(), None, false);
        assert_eq!(s.version_uuid, "abc");
        assert!(s.snapshot.is_err());
    }
}
