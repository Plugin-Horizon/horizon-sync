// Every tick: apply arrived snapshots, answer capture requests, show messages and menus.
fn horizonsync_tick(game: &mut crate::game::Game<'_>) {
    crate::game::horizonsync::tick(game);
}
