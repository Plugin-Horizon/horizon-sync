// A locked player (data still loading) can't break, place, use, click or hit.
// The acting player is read from the game (a binding may only name a hook parameter).
fn horizonsync_lock_acting(game: &crate::game::Game<'_>, canceled: &mut bool) {
    if crate::game::horizonsync::locked(game.lan_acting_uuid) {
        *canceled = true;
    }
}
