// A locked player can't pick up items, open containers or use armour stands.
fn horizonsync_lock_player(player: u128, canceled: &mut bool) {
    if crate::game::horizonsync::locked(Some(player)) {
        *canceled = true;
    }
}
