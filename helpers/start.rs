// A dedicated server starts: read config/horizonsync/config.toml, connect to MariaDB and Redis.
fn horizonsync_start(dedicated: bool) {
    crate::game::horizonsync::start(dedicated);
}
