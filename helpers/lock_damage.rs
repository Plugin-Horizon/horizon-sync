// A locked player takes no damage (BukkitLockedEventListener.onPlayerTakeDamage); the victim is
// named by their UUID (`living_incoming_damage_v3`), a mob's is none.
fn horizonsync_lock_damage(victim_uuid: Option<u128>, canceled: &mut bool) {
    if crate::game::horizonsync::locked(victim_uuid) {
        *canceled = true;
    }
}
