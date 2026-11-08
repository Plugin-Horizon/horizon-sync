// blacklisted_commands_while_locked: a player whose data is loading can't run commands.
fn horizonsync_lock_command(player: u128, command: &str, canceled: &mut bool) {
    if crate::game::horizonsync::refuse_command(player, command) {
        *canceled = true;
    }
}
