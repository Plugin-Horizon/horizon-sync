// /horizonsync, /userdata, /inventory, /enderchest and their aliases.
fn horizonsync_register_commands(commands: &mut Vec<crate::mod_api::NativeCommand>) {
    crate::game::horizonsync::register_commands(commands);
}
