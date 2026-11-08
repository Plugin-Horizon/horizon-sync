// An editable /inventory or /enderchest menu closed: the items in it are the player's data.
fn horizonsync_menu_closed(player: u128, window: i32, slots: &mut Vec<Option<horizon_client::items::ItemStack>>) {
    crate::game::horizonsync::menu_input_closed(player, window, slots);
}
