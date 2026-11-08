// A player is placed in the world: lock them and fetch their data (Redis, then the database).
// A fake player (a mod's bot with no client) has no data to sync: no lock, no fetch, no database.
fn horizonsync_joined(game: &mut crate::game::Game<'_>, player: u128, name: &str) {
    if game.native_is_player_fake(player) {
        return;
    }
    horizonsync_core::on_join(player, name);
}
