//! HuskSync's PostgreSQL statements (`postgres_schema.sql` and `PostgresDatabase`), ready for a client.
//! No PostgreSQL client crate is in the workspace yet (report to the main session: `postgres = "0.19"`);
//! the `Store` implementation on top of these strings is the only piece missing.

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

pub fn user_by_uuid(t: &TableNames) -> String {
    fill("SELECT username FROM \"%users_table%\" WHERE uuid=$1", t)
}
pub fn user_by_name(t: &TableNames) -> String {
    fill("SELECT uuid, username FROM \"%users_table%\" WHERE LOWER(username)=LOWER($1)", t)
}
pub fn insert_user(t: &TableNames) -> String {
    fill("INSERT INTO \"%users_table%\" (uuid, username) VALUES ($1, $2) ON CONFLICT (uuid) DO UPDATE SET username=EXCLUDED.username", t)
}
pub fn latest_snapshot(t: &TableNames) -> String {
    fill("SELECT version_uuid, \"timestamp\", data FROM \"%user_data_table%\" WHERE player_uuid=$1 ORDER BY \"timestamp\" DESC LIMIT 1", t)
}
pub fn all_snapshots(t: &TableNames) -> String {
    fill("SELECT version_uuid, \"timestamp\", data FROM \"%user_data_table%\" WHERE player_uuid=$1 ORDER BY \"timestamp\" DESC", t)
}
pub fn insert_snapshot(t: &TableNames) -> String {
    fill("INSERT INTO \"%user_data_table%\" (player_uuid, version_uuid, \"timestamp\", save_cause, pinned, data) VALUES ($1, $2, $3, $4, $5, $6)", t)
}
pub fn delete_snapshot(t: &TableNames) -> String {
    fill("DELETE FROM \"%user_data_table%\" WHERE player_uuid=$1 AND version_uuid=$2", t)
}
pub fn set_pinned(t: &TableNames) -> String {
    fill("UPDATE \"%user_data_table%\" SET pinned=$3 WHERE player_uuid=$1 AND version_uuid=$2", t)
}
pub fn save_map(t: &TableNames) -> String {
    fill("INSERT INTO \"%map_data_table%\" (server_name, map_id, data) VALUES ($1, $2, $3) ON CONFLICT (server_name, map_id) DO UPDATE SET data=EXCLUDED.data", t)
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
}
