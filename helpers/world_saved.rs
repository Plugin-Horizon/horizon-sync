// The world saved: WORLD_SAVE snapshots of everyone on (save_on_world_save).
fn horizonsync_world_saved(game: &mut crate::game::Game<'_>) {
    crate::game::horizonsync::world_saved(game);
}
