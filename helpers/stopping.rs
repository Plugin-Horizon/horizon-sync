// An orderly stop begins, players still on: SERVER_SHUTDOWN snapshots, then wait for every save.
fn horizonsync_stopping(game: &mut crate::game::Game<'_>) {
    crate::game::horizonsync::stopping(game);
}
