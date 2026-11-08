//! `MySqlDatabase`: HuskSync's tables and statements on MariaDB/MySQL, unchanged.
//!
//! Tables (`mariadb_schema.sql`): `husksync_users(uuid char(36), username varchar(16))` and
//! `husksync_user_data(version_uuid, player_uuid, timestamp datetime, save_cause varchar(32),
//! pinned boolean, data longblob)`; maps tables are created but not used by this port.

use crate::config::{Config, Database as DbConfig};
use crate::snapshot::{Packed, Platform};
use crate::util;
use mysql::prelude::Queryable;
use mysql::{OptsBuilder, Pool, PoolConstraints, PoolOpts, Row, Value};
use std::time::Duration;

/// A stored snapshot as read back: the packed document, or why it can't be used.
#[derive(Clone, Debug)]
pub struct Stored {
    pub version_uuid: String,
    pub snapshot: Result<Packed, String>,
}

pub struct Db {
    pool: Pool,
    t: crate::config::TableNames,
    zone: Zone,
    compress: bool,
    max_snapshots: i64,
    backup_frequency_hours: i64,
}

#[derive(Clone, Copy, Debug)]
enum Zone {
    Local,
    Fixed(i32),
}

fn parse_zone(s: &str) -> Zone {
    if s.eq_ignore_ascii_case("local") || s.is_empty() {
        return Zone::Local;
    }
    if s.eq_ignore_ascii_case("utc") || s == "Z" {
        return Zone::Fixed(0);
    }
    let sign = if s.starts_with('-') { -1 } else { 1 };
    let body = s.trim_start_matches(['+', '-']);
    let mut it = body.split(':');
    let h: i32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let m: i32 = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    Zone::Fixed(sign * (h * 3600 + m * 60))
}

impl Zone {
    /// `Timestamp.from(instant)` as JDBC writes it into a DATETIME: the wall time in the zone.
    fn to_value(self, t: &chrono::DateTime<chrono::FixedOffset>) -> Value {
        let wall = match self {
            Zone::Local => t.with_timezone(&chrono::Local).naive_local(),
            Zone::Fixed(secs) => t
                .with_timezone(&chrono::FixedOffset::east_opt(secs).unwrap_or(*t.offset()))
                .naive_local(),
        };
        // `mysql`'s `chrono` feature binds a wall time as a DATETIME.
        Value::from(wall)
    }
}

/// The schema, as `mariadb_schema.sql` (MySQL gets the same statements).
fn schema(t: &crate::config::TableNames) -> Vec<String> {
    let s = [
        "SET DEFAULT_STORAGE_ENGINE = INNODB",
        "SET FOREIGN_KEY_CHECKS = 1",
        "CREATE TABLE IF NOT EXISTS `%users_table%`
(
    `uuid`     char(36)    NOT NULL UNIQUE,
    `username` varchar(16) NOT NULL,
    PRIMARY KEY (`uuid`)
) ENGINE = InnoDB
  DEFAULT CHARSET = utf8mb4
  COLLATE = utf8mb4_unicode_ci",
        "CREATE INDEX IF NOT EXISTS `%users_table%_username` ON `%users_table%` (`username`)",
        "CREATE TABLE IF NOT EXISTS `%user_data_table%`
(
    `version_uuid` char(36)    NOT NULL UNIQUE,
    `player_uuid`  char(36)    NOT NULL,
    `timestamp`    datetime    NOT NULL,
    `save_cause`   varchar(32) NOT NULL,
    `pinned`       boolean     NOT NULL DEFAULT FALSE,
    `data`         longblob    NOT NULL,
    PRIMARY KEY (`version_uuid`, `player_uuid`),
    FOREIGN KEY (`player_uuid`) REFERENCES `%users_table%` (`uuid`) ON DELETE CASCADE
) ENGINE = InnoDB
  DEFAULT CHARSET = utf8mb4
  COLLATE = utf8mb4_unicode_ci",
        "CREATE TABLE IF NOT EXISTS `%map_data_table%`
(
    `server_name` varchar(32) NOT NULL,
    `map_id`      int         NOT NULL,
    `data`        longblob    NOT NULL,
    PRIMARY KEY (`server_name`, `map_id`)
) ENGINE = InnoDB
  DEFAULT CHARSET = utf8mb4
  COLLATE = utf8mb4_unicode_ci",
        "CREATE TABLE IF NOT EXISTS `%map_ids_table%`
(
    `from_server_name` varchar(32) NOT NULL,
    `from_id`          int         NOT NULL,
    `to_server_name`   varchar(32) NOT NULL,
    `to_id`            int         NOT NULL,
    PRIMARY KEY (`from_server_name`, `from_id`, `to_server_name`),
    FOREIGN KEY (`from_server_name`, `from_id`) REFERENCES `%map_data_table%` (`server_name`, `map_id`) ON DELETE CASCADE
) ENGINE = InnoDB
  DEFAULT CHARSET = utf8mb4
  COLLATE = utf8mb4_unicode_ci",
    ];
    s.iter().map(|q| tables(q, t)).collect()
}

/// `formatStatementTables`.
fn tables(q: &str, t: &crate::config::TableNames) -> String {
    q.replace("%users_table%", &t.users)
        .replace("%user_data_table%", &t.user_data)
        .replace("%map_data_table%", &t.map_data)
        .replace("%map_ids_table%", &t.map_ids)
}

fn err(e: impl std::fmt::Display) -> String {
    format!("database: {e}")
}

impl Db {
    pub fn connect(config: &Config) -> Result<Db, String> {
        let c: &DbConfig = &config.database;
        let kind = c.kind.to_ascii_uppercase();
        if kind != "MYSQL" && kind != "MARIADB" {
            return Err(format!("database type {} is not supported (MYSQL or MARIADB)", c.kind));
        }
        let max = c.maximum_pool_size.max(1);
        let min = c.minimum_idle.min(max);
        let mut opts = OptsBuilder::new()
            .ip_or_hostname(Some(c.host.clone()))
            .tcp_port(c.port)
            .db_name(Some(c.database.clone()))
            .user(Some(c.username.clone()))
            .pass(Some(c.password.clone()))
            .tcp_connect_timeout(Some(Duration::from_millis(c.connection_timeout.max(1000))))
            .prefer_socket(false)
            .pool_opts(PoolOpts::new().with_constraints(
                PoolConstraints::new(min, max).unwrap_or_default(),
            ));
        if c.use_ssl {
            opts = opts.ssl_opts(mysql::SslOpts::default());
        }
        let pool = Pool::new(opts).map_err(err)?;
        let db = Db {
            pool,
            t: c.table_names.clone(),
            zone: parse_zone(&c.timestamp_zone),
            compress: config.synchronization.compress_data,
            max_snapshots: config.synchronization.max_user_data_snapshots,
            backup_frequency_hours: config.synchronization.snapshot_backup_frequency,
        };
        if c.create_tables {
            db.create_tables()?;
        }
        Ok(db)
    }

    fn q(&self, s: &str) -> String {
        tables(s, &self.t)
    }

    fn conn(&self) -> Result<mysql::PooledConn, String> {
        self.pool.get_conn().map_err(err)
    }

    pub fn create_tables(&self) -> Result<(), String> {
        let mut c = self.conn()?;
        for s in schema(&self.t) {
            c.query_drop(s).map_err(err)?;
        }
        Ok(())
    }

    /// `Database.ensureUser`: adds the user, or renames them when their name changed.
    pub fn ensure_user(&self, uuid: u128, name: &str) -> Result<(), String> {
        let id = util::uuid_string(uuid);
        let mut c = self.conn()?;
        let existing: Option<String> = c
            .exec_first(self.q("SELECT `username` FROM `%users_table%` WHERE `uuid`=?"), (id.clone(),))
            .map_err(err)?;
        match existing {
            Some(old) if old == name => Ok(()),
            Some(_) => c
                .exec_drop(self.q("UPDATE `%users_table%` SET `username`=? WHERE `uuid`=?"), (name, id))
                .map_err(err),
            None => c
                .exec_drop(self.q("INSERT INTO `%users_table%` (`uuid`,`username`) VALUES (?,?);"), (id, name))
                .map_err(err),
        }
    }

    pub fn user_by_name(&self, name: &str) -> Result<Option<(u128, String)>, String> {
        let mut c = self.conn()?;
        let row: Option<(String, String)> = c
            .exec_first(self.q("SELECT `uuid`, `username` FROM `%users_table%` WHERE `username`=?"), (name,))
            .map_err(err)?;
        Ok(row.and_then(|(u, n)| util::parse_uuid(&u).map(|u| (u, n))))
    }

    pub fn user_by_uuid(&self, uuid: u128) -> Result<Option<String>, String> {
        let mut c = self.conn()?;
        c.exec_first(self.q("SELECT `username` FROM `%users_table%` WHERE `uuid`=?"), (util::uuid_string(uuid),))
            .map_err(err)
    }

    fn stored(&self, row: Row) -> Stored {
        let version_uuid: String = row.get("version_uuid").unwrap_or_default();
        let data: Option<Vec<u8>> = row.get("data");
        let snapshot = match data {
            Some(bytes) => Packed::from_bytes(&bytes, self.compress).map(|mut p| {
                // The row's id is the one the database knows it by (`deserialize(..., id, ...)`
                // leaves the JSON's; they are the same unless edited by hand).
                if p.id.is_empty() {
                    p.id = version_uuid.clone();
                }
                p
            }),
            None => Err("no data".into()),
        };
        Stored { version_uuid, snapshot }
    }

    pub fn latest_snapshot(&self, player: u128) -> Result<Option<Stored>, String> {
        let mut c = self.conn()?;
        let row: Option<Row> = c
            .exec_first(
                self.q("SELECT `version_uuid`, `timestamp`, `data` FROM `%user_data_table%` WHERE `player_uuid`=? ORDER BY `timestamp` DESC LIMIT 1;"),
                (util::uuid_string(player),),
            )
            .map_err(err)?;
        Ok(row.map(|r| self.stored(r)))
    }

    pub fn latest_snapshot_with_causes(&self, player: u128, causes: &[&str]) -> Result<Option<Stored>, String> {
        if causes.is_empty() {
            return Ok(None);
        }
        let placeholders = vec!["?"; causes.len()].join(",");
        let mut params: Vec<Value> = vec![util::uuid_string(player).into()];
        params.extend(causes.iter().map(|c| Value::from(*c)));
        let mut c = self.conn()?;
        let row: Option<Row> = c
            .exec_first(
                self.q(&format!("SELECT `version_uuid`, `timestamp`, `data` FROM `%user_data_table%` WHERE `player_uuid`=? AND `save_cause` IN ({placeholders}) ORDER BY `timestamp` DESC LIMIT 1;")),
                params,
            )
            .map_err(err)?;
        Ok(row.map(|r| self.stored(r)))
    }

    pub fn all_snapshots(&self, player: u128) -> Result<Vec<Stored>, String> {
        let mut c = self.conn()?;
        let rows: Vec<Row> = c
            .exec(
                self.q("SELECT `version_uuid`, `timestamp`,  `data` FROM `%user_data_table%` WHERE `player_uuid`=? ORDER BY `timestamp` DESC;"),
                (util::uuid_string(player),),
            )
            .map_err(err)?;
        Ok(rows.into_iter().map(|r| self.stored(r)).collect())
    }

    pub fn snapshot(&self, player: u128, version: u128) -> Result<Option<Stored>, String> {
        let mut c = self.conn()?;
        let row: Option<Row> = c
            .exec_first(
                self.q("SELECT `version_uuid`, `timestamp`,  `data` FROM `%user_data_table%` WHERE `player_uuid`=? AND `version_uuid`=? ORDER BY `timestamp` DESC LIMIT 1;"),
                (util::uuid_string(player), util::uuid_string(version)),
            )
            .map_err(err)?;
        Ok(row.map(|r| self.stored(r)))
    }

    fn unpinned_count(&self, player: u128) -> Result<i64, String> {
        let mut c = self.conn()?;
        let n: Option<i64> = c
            .exec_first(
                self.q("SELECT COUNT(`version_uuid`) FROM `%user_data_table%` WHERE `player_uuid`=? AND `pinned`=false;"),
                (util::uuid_string(player),),
            )
            .map_err(err)?;
        Ok(n.unwrap_or(0))
    }

    /// `rotateSnapshots`: drops the oldest unpinned snapshots beyond `max_user_data_snapshots`.
    fn rotate_snapshots(&self, player: u128) -> Result<(), String> {
        let unpinned = self.unpinned_count(player)?;
        if unpinned > self.max_snapshots {
            let mut c = self.conn()?;
            c.exec_drop(
                self.q(&format!(
                    "DELETE FROM `%user_data_table%` WHERE `player_uuid`=? AND `pinned` IS FALSE ORDER BY `timestamp` ASC LIMIT {};",
                    unpinned - self.max_snapshots
                )),
                (util::uuid_string(player),),
            )
            .map_err(err)?;
        }
        Ok(())
    }

    /// `rotateLatestSnapshot`: the backup-frequency rule, one unpinned snapshot newer than
    /// `within` is replaced by the new one.
    fn rotate_latest(&self, player: u128, within: &chrono::DateTime<chrono::FixedOffset>) -> Result<(), String> {
        let mut c = self.conn()?;
        c.exec_drop(
            self.q("DELETE FROM `%user_data_table%` WHERE `player_uuid`=? AND `timestamp`>? AND `pinned` IS FALSE ORDER BY `timestamp` ASC LIMIT 1;"),
            (util::uuid_string(player), self.zone.to_value(within)),
        )
        .map_err(err)
    }

    fn create_snapshot(&self, player: u128, s: &Packed) -> Result<(), String> {
        let time = s.time().unwrap_or_else(util::now);
        let mut c = self.conn()?;
        c.exec_drop(
            self.q("INSERT INTO `%user_data_table%` (`player_uuid`,`version_uuid`,`timestamp`,`save_cause`,`pinned`,`data`) VALUES (?,?,?,?,?,?);"),
            (
                util::uuid_string(player),
                s.id.clone(),
                self.zone.to_value(&time),
                s.save_cause.clone(),
                s.pinned,
                s.to_bytes(self.compress),
            ),
        )
        .map_err(err)
    }

    /// `Database.addSnapshot`.
    pub fn add_snapshot(&self, player: u128, s: &Packed) -> Result<(), String> {
        if !s.pinned && self.backup_frequency_hours > 0 {
            if let Some(t) = s.time() {
                self.rotate_latest(player, &(t - chrono::Duration::hours(self.backup_frequency_hours)))?;
            }
        }
        self.create_snapshot(player, s)?;
        self.rotate_snapshots(player)
    }

    /// `addSnapshotWithoutRotation`: a retried shutdown write.
    pub fn add_snapshot_without_rotation(&self, player: u128, s: &Packed) -> Result<(), String> {
        self.create_snapshot(player, s)
    }

    pub fn update_snapshot(&self, player: u128, s: &Packed) -> Result<(), String> {
        let mut c = self.conn()?;
        c.exec_drop(
            self.q("UPDATE `%user_data_table%` SET `save_cause`=?,`pinned`=?,`data`=? WHERE `player_uuid`=? AND `version_uuid`=? LIMIT 1;"),
            (s.save_cause.clone(), s.pinned, s.to_bytes(self.compress), util::uuid_string(player), s.id.clone()),
        )
        .map_err(err)
    }

    pub fn delete_snapshot(&self, player: u128, version: u128) -> Result<bool, String> {
        let mut c = self.conn()?;
        c.exec_drop(
            self.q("DELETE FROM `%user_data_table%` WHERE `player_uuid`=? AND `version_uuid`=? LIMIT 1;"),
            (util::uuid_string(player), util::uuid_string(version)),
        )
        .map_err(err)?;
        Ok(c.affected_rows() > 0)
    }

    /// `pinSnapshot` / `unpinSnapshot`: the pinned flag in the column and in the document.
    pub fn set_pinned(&self, player: u128, version: u128, pinned: bool) -> Result<bool, String> {
        let Some(stored) = self.snapshot(player, version)? else {
            return Ok(false);
        };
        let mut s = stored.snapshot?;
        s.pinned = pinned;
        self.update_snapshot(player, &s)?;
        Ok(true)
    }

    pub fn ping(&self) -> Result<Duration, String> {
        let start = std::time::Instant::now();
        let mut c = self.conn()?;
        c.query_drop("SELECT 1").map_err(err)?;
        Ok(start.elapsed())
    }
}

/// The checks a snapshot read from the database or Redis must pass before it is applied.
pub fn usable(stored: Result<Packed, String>, platform: &Platform) -> Result<Packed, String> {
    let s = stored?;
    s.validate(platform).map_err(|e| e.message().to_owned())?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statements_use_configured_tables() {
        let t = crate::config::TableNames::default();
        let s = schema(&t);
        assert!(s[2].contains("CREATE TABLE IF NOT EXISTS `husksync_users`"));
        assert!(s[4].contains("REFERENCES `husksync_users` (`uuid`)"));
        assert_eq!(tables("`%user_data_table%`", &t), "`husksync_user_data`");
    }

    #[test]
    fn timestamps_are_wall_time_in_the_zone() {
        let t = util::parse_timestamp("2026-11-06T06:00:00.250+01:00").unwrap();
        assert_eq!(
            parse_zone("+00:00").to_value(&t),
            Value::Date(2026, 11, 6, 5, 0, 0, 250_000)
        );
        assert_eq!(parse_zone("+13:00").to_value(&t), Value::Date(2026, 11, 6, 18, 0, 0, 250_000));
        assert!(matches!(parse_zone("local"), Zone::Local));
    }
}
