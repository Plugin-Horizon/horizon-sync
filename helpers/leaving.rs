// A player leaves with their state still readable: the DISCONNECT snapshot.
fn horizonsync_leaving(game: &mut crate::game::Game<'_>, player: u128) {
    // A fake player was never fetched: no snapshot to save for it.
    if game.native_is_player_fake(player) {
        return;
    }
    crate::game::horizonsync::leaving(game, player);
}
