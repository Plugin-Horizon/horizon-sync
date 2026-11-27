//! HuskSync's PostgreSQL statements (`postgres_schema.sql` and `PostgresDatabase`); `pg_store.rs` runs them.
//! Uuids are bound as text and cast in the statement, rows read them back as text.

use crate::horizonsync_core::config::TableNames;

fn fill(q: &str, t: &TableNames) -> String {
    q.replace("%users_table%", &t.users)
        .replace("%user_data_table%", &t.user_data)
        .replace("%map_data_table%", &t.map_data)
        .replace("%map_ids_table%", &t.map_ids)
}

/// The schema, in order.
pub fn schema(t: &TableNames) -> Vec<String> {
    [
        "CREATE TABLE IF NOT EXISTS \"%users_table%\" (uuid uuid NOT NULL PRIMARY KEY, username varchar(16) NOT NULL)",
        "CREATE INDEX IF NOT EXISTS \"%users_table%_username\" ON \"%users_table%\" (username)",
        "CREATE TABLE IF NOT EXISTS \"%user_data_table%\" (version_uuid uuid NOT NULL UNIQUE, player_uuid uuid NOT NULL, \
\"timestamp\" timestamp NOT NULL, save_cause varchar(32) NOT NULL, pinned boolean NOT NULL DEFAULT FALSE, \
data bytea NOT NULL, PRIMARY KEY (version_uuid, player_uuid), \
FOREIGN KEY (player_uuid) REFERENCES \"%users_table%\" (uuid) ON DELETE CASCADE)",
        "CREATE TABLE IF NOT EXISTS \"%map_data_table%\" (server_name varchar(32) NOT NULL, map_id integer NOT NULL, \
data bytea NOT NULL, PRIMARY KEY (server_name, map_id))",
        "CREATE TABLE IF NOT EXISTS \"%map_ids_table%\" (from_server_name varchar(32) NOT NULL, from_id integer NOT NULL, \
to_server_name varchar(32) NOT NULL, to_id integer NOT NULL, PRIMARY KEY (from_server_name, from_id, to_server_name), \
FOREIGN KEY (from_server_name, from_id) REFERENCES \"%map_data_table%\" (server_name, map_id) ON DELETE CASCADE)",
    ]
    .iter()
    .map(|q| fill(q, t))
    .collect()
}

/// A uuid parameter: bound as text and cast, so no `uuid` type support is needed in the client.
const U1: &str = "CAST($1::text AS uuid)";

pub fn user_by_uuid(t: &TableNames) -> String {
    fill(&format!("SELECT username FROM \"%users_table%\" WHERE uuid={U1}"), t)
}
pub fn user_by_name(t: &TableNames) -> String {
    fill("SELECT uuid::text, username FROM \"%users_table%\" WHERE LOWER(username)=LOWER($1)", t)
}
/// `$1` uuid, `$2` name.
pub fn insert_user(t: &TableNames) -> String {
    fill("INSERT INTO \"%users_table%\" (uuid, username) VALUES (CAST($1::text AS uuid), $2) ON CONFLICT (uuid) DO UPDATE SET username=EXCLUDED.username", t)
}
pub fn latest_snapshot(t: &TableNames) -> String {
    fill(&format!("SELECT version_uuid::text AS version_uuid, data FROM \"%user_data_table%\" WHERE player_uuid={U1} ORDER BY \"timestamp\" DESC LIMIT 1"), t)
}
/// `$2` is a `text[]` of causes.
pub fn latest_snapshot_with_causes(t: &TableNames) -> String {
    fill(&format!("SELECT version_uuid::text AS version_uuid, data FROM \"%user_data_table%\" WHERE player_uuid={U1} AND save_cause = ANY($2::text[]) ORDER BY \"timestamp\" DESC LIMIT 1"), t)
}
pub fn all_snapshots(t: &TableNames) -> String {
    fill(&format!("SELECT version_uuid::text AS version_uuid, data FROM \"%user_data_table%\" WHERE player_uuid={U1} ORDER BY \"timestamp\" DESC"), t)
}
/// `$2` version uuid.
pub fn snapshot(t: &TableNames) -> String {
    fill(&format!("SELECT version_uuid::text AS version_uuid, data FROM \"%user_data_table%\" WHERE player_uuid={U1} AND version_uuid=CAST($2::text AS uuid) LIMIT 1"), t)
}
pub fn unpinned_count(t: &TableNames) -> String {
    fill(&format!("SELECT COUNT(version_uuid) FROM \"%user_data_table%\" WHERE player_uuid={U1} AND pinned=false"), t)
}
/// `rotateSnapshots`: `$2` is how many of the oldest unpinned snapshots go.
pub fn rotate_snapshots(t: &TableNames) -> String {
    fill(&format!("DELETE FROM \"%user_data_table%\" WHERE version_uuid IN (SELECT version_uuid FROM \"%user_data_table%\" WHERE player_uuid={U1} AND pinned IS FALSE ORDER BY \"timestamp\" ASC LIMIT $2)"), t)
}
/// `rotateLatestSnapshot`: `$2` is the cut-off wall time.
pub fn rotate_latest(t: &TableNames) -> String {
    fill(&format!("DELETE FROM \"%user_data_table%\" WHERE version_uuid IN (SELECT version_uuid FROM \"%user_data_table%\" WHERE player_uuid={U1} AND \"timestamp\" > $2 AND pinned IS FALSE ORDER BY \"timestamp\" ASC LIMIT 1)"), t)
}
/// `$1` player, `$2` version, `$3` wall time, `$4` cause, `$5` pinned, `$6` data.
pub fn insert_snapshot(t: &TableNames) -> String {
    fill("INSERT INTO \"%user_data_table%\" (player_uuid, version_uuid, \"timestamp\", save_cause, pinned, data) VALUES (CAST($1::text AS uuid), CAST($2::text AS uuid), $3, $4, $5, $6)", t)
}
/// `$1` cause, `$2` pinned, `$3` data, `$4` player, `$5` version.
pub fn update_snapshot(t: &TableNames) -> String {
    fill("UPDATE \"%user_data_table%\" SET save_cause=$1, pinned=$2, data=$3 WHERE player_uuid=CAST($4::text AS uuid) AND version_uuid=CAST($5::text AS uuid)", t)
}
pub fn delete_snapshot(t: &TableNames) -> String {
    fill("DELETE FROM \"%user_data_table%\" WHERE player_uuid=CAST($1::text AS uuid) AND version_uuid=CAST($2::text AS uuid)", t)
}
pub fn map_file(t: &TableNames) -> String {
    fill("SELECT data FROM \"%map_data_table%\" WHERE server_name=$1 AND map_id=$2 LIMIT 1", t)
}
pub fn save_map(t: &TableNames) -> String {
    fill("INSERT INTO \"%map_data_table%\" (server_name, map_id, data) VALUES ($1, $2, $3) ON CONFLICT (server_name, map_id) DO UPDATE SET data=EXCLUDED.data", t)
}
pub fn bound_map_id(t: &TableNames) -> String {
    fill("SELECT to_id FROM \"%map_ids_table%\" WHERE from_server_name=$1 AND from_id=$2 AND to_server_name=$3 LIMIT 1", t)
}
pub fn set_map_binding(t: &TableNames) -> String {
    fill("INSERT INTO \"%map_ids_table%\" (from_server_name, from_id, to_server_name, to_id) VALUES ($1, $2, $3, $4) ON CONFLICT (from_server_name, from_id, to_server_name) DO UPDATE SET to_id=EXCLUDED.to_id", t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statements_use_configured_tables() {
        let mut t = TableNames::default();
        t.user_data = "my_data".into();
        assert!(schema(&t).iter().any(|q| q.contains("\"my_data\"")));
        assert!(schema(&t).iter().all(|q| !q.contains('%')));
        assert!(insert_snapshot(&t).contains("\"my_data\"") && !insert_snapshot(&t).contains('%'));
        assert_eq!(schema(&t).len(), 5);
    }

    /// Every statement numbers its parameters `$1..$n` without gaps, and no placeholder is left over.
    #[test]
    fn parameters_are_numbered_and_filled() {
        let t = TableNames::default();
        let all = [
            (user_by_uuid(&t), 1), (user_by_name(&t), 1), (insert_user(&t), 2), (latest_snapshot(&t), 1),
            (latest_snapshot_with_causes(&t), 2), (all_snapshots(&t), 1), (snapshot(&t), 2), (unpinned_count(&t), 1),
            (rotate_snapshots(&t), 2), (rotate_latest(&t), 2), (insert_snapshot(&t), 6), (update_snapshot(&t), 5),
            (delete_snapshot(&t), 2), (map_file(&t), 2), (save_map(&t), 3), (bound_map_id(&t), 3), (set_map_binding(&t), 4),
        ];
        for (q, n) in all {
            assert!(!q.contains('%'), "{q}");
            for i in 1..=n {
                assert!(q.contains(&format!("${i}")), "{q} lacks ${i}");
            }
            assert!(!q.contains(&format!("${}", n + 1)), "{q} has too many");
        }
    }
}
