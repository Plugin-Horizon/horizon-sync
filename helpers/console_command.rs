fn horizonsync_console_command(command: &str, ran: &mut bool) {
    crate::game::horizonsync::perform_console_command(command, ran);
}
