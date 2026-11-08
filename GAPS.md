# Gaps in Horizon (base `a589a3dd9`) for HorizonSync (the HuskSync-compatible player data sync)

Each line: the hook or helper that is missing, what it is for, and what the port does today.

## Player state (the core of the port)

- [x] (`player_logging_out` †: `native_player_logging_out(player, name)` + `native_player_compound`) `native_player_leaving(player: u128, name: &str, compound: &Tag)` (or `Guest` readable in
  `native_player_logged_out`) — needed for the DISCONNECT snapshot, taken from the player as they
  leave (Bukkit's `PlayerQuitEvent` still has the live player) — was: raw HEAD injection on
  `Game::save_guest`; now the DISCONNECT snapshot comes from `player_logging_out` (not heard when the server stops, where `server_stopping` saves everyone).
- [x] (hooks-all: native_player_compound) `Game::native_player_compound(uuid) -> Option<Tag>` (the `playerdata::player_tag` of a placed
  guest) — needed for every capture (world save, `/userdata save`, cross-server `/inventory`) —
  today: the glue module calls `horizon_client::lan::playerdata::player_tag` on `host.hub.guests`.
- [x] (hooks-all: native_set_inventory) `Game::native_set_inventory(uuid, kind: NativeInventoryKind, slots: Vec<Option<ItemStack>>, selected: Option<usize>)`
  (writes all 41 / 27 slots, clears cursor and 2x2 grid, syncs the client) — needed to apply
  `husksync:inventory` / `ender_chest` — today: the glue writes `Guest.inventory/armor/offhand/
  ender_items/selected/carried/craft_grid` directly and relies on the tick's container sync.
- [x] (hooks-all: native_set_player_status) `Game::native_set_player_status(uuid, health: Option<f32>, food: Option<(i32, f32, f32)>, xp: Option<(i32, i32, f32)>)`
  — needed for health/hunger/experience — today: direct writes to `Guest.health/food/saturation/
  xp_*` and `motion.health.food.exhaustion`.
- [x] (`native_set_player_status` with health 0 kills, †) `Game::native_kill_player(uuid)` or `native_set_player_status` accepting health 0 as death —
  needed when a snapshot holds a dead player (`setHealth(0)` on Bukkit) — today: not applied.
- [x] (hooks-all: native_set_effects (show_icon still not settable)) `Game::native_set_effects(uuid, effects: &[(String, i32, i32, bool, bool, bool)])` (replace all,
  with `show_icon`) — needed for `husksync:potion_effects` — today: `PlayerEffects::from_saved`
  written into the guest; `show_icon` is always true in Horizon (`PlayerEffects::flags`).
- [x] (hooks-all: native_set_attributes (live transient modifiers still not read, HS-3)) `Game::native_set_attributes(uuid, saved: &Tag)` and the live attribute instances with
  transient modifiers in a read helper — needed for `husksync:attributes` (Bukkit sees transient
  equipment modifiers, the saved list does not) — today: `PlayerMotion::apply_saved_attributes`
  on the guest's body, or `carried_attributes` when it has none.
- [x] (hooks-all: native_set_flight) `Guest.may_fly` (an `allowFlight` independent of the game mode) + `Game::native_set_flight(uuid,
  allow: bool, flying: bool)` — needed for `husksync:flight_status` in survival (`Player.setAllowFlight`)
  — today: only `flying` of creative/spectator players is set; survival allow-flight is lost.
- [ ] `Game::native_set_game_mode(uuid, mode)` — needed for `husksync:game_mode` — today:
  `native_run_command("gamemode <mode> <name>")`.
- [x] (hooks-all: native_teleport_player) `Game::native_teleport_player(uuid, dimension, pos, yaw, pitch)` — needed for `husksync:location`
  — today: `native_run_command("execute in <dim> run tp <name> x y z yaw pitch")` (lands next tick).
- [x] (hooks-all: native_player_persistent_data + setter (kept as NeoForgeData, mapped to BukkitValues on capture)) `Game::native_player_persistent_data(uuid) -> Option<Tag>` / `native_set_player_persistent_data`
  (Bukkit's `BukkitValues`) — needed for `husksync:persistent_data` — today: read from the player
  compound, written into `Guest.saved` (the loaded compound Horizon writes back).
- [x] (`player_advancements/set_player_advancements/player_stats/set_player_stats`, † `player_records`; `core/src/records.rs` converts vanilla JSON and HuskSync's lists) Read/write of a player's advancements (`Guest.advancements`) and statistics in vanilla's JSON
  form (`advancements/<uuid>.json`, `stats/<uuid>.json`) — needed for `husksync:advancements`,
  `husksync:statistics` — today: carried over unchanged from the last applied snapshot.
- [x] (`Host::respawn_player`, †; the apply plan waits up to 200 ticks for the respawn) `native_respawn_player(uuid)` — needed for "a player who joins dead is respawned" — today: not done.

## Lock while data loads

- [x] (`item_dropped_v2`, † `cancel`) `native_item_dropped` with `canceled: &mut bool` — needed for `PlayerDropItemEvent` cancel —
  today: a locked player can still drop items (not possible).
- [x] (hooks-all: player_launch_projectile_v2) `native_projectile_launch(player, kind, canceled)` cancellable for players — needed for
  `ProjectileLaunchEvent` (ender pearls unless `allow_pearl_spawning_while_locked`) — today: not possible.
- [x] (hooks-all: inventory_click + inventory_drag) `native_inventory_click(player, window, slot, button, mode, canceled)` for vanilla menus —
  needed for `InventoryClickEvent` cancel — today: not possible (only native menus report clicks).
- [x] (hooks-all: block_place) `native_place_block` cancellable (or document that cancelling `native_use_item_on` covers it) —
  needed for `BlockPlaceEvent` — today: `use_item_on` is claimed, which also stops placing.
- [x] (`living_incoming_damage_v3`, †: `victim_uuid`) `native_living_incoming_damage` giving the victim's UUID for players — needed to match the
  locked player — today: the glue keeps an entity-id → locked map refreshed each tick.

## Commands and permissions

- [x] (hooks-all: native_has_permission + NativeCommand::permission) permission nodes for native
  commands — today: `horizonsync.command.*` per sub-command via `core::permission_for`, checked with
  `native_has_permission` (default level 2; `about` is 0); operators pass unless a mod denies.
- [x] (`Text::with_component` with `click_event` / `hover_event`; the snapshot overview buttons are clickable; list footer arrows and page jumpers are still plain text) A rich-text helper (`Text` from MiniMessage/MineDown or JSON with click/hover events) for
  `native_tell` — needed for HuskSync's clickable snapshot lists and buttons — today: plain text.
- [x] (`open_menu_with_input` + `menu_input_closed` †, `horizonsync.command.inventory.edit` / `enderchest.edit`; no `edit` sub-command: the menu is editable for who holds the node, on the latest data) Editable native menus: `native_menu_closed` handing the menu's final slots (or the mod moving
  items between cursor and menu) — needed for `/inventory` and `/enderchest` editing — today: view only.
  The public docs name the sub-commands `/inventory edit` and `/enderchest edit`, nodes
  `horizonsync.command.inventory.edit` and `horizonsync.command.enderchest.edit`; not registered here.
- [ ] `/horizonsync migrate` (console-only legacy migration) and a web dump for `/horizonsync dump` and
  `/userdata dump` — today: `dump` answers that web dumps are not available; `migrate` is not registered.
- [x] Permission nodes per sub-command as the public docs list them (`horizonsync.command.userdata.save`,
  `horizonsync.command.horizonsync.dump`, ...) — today: checked per sub-command; `.*` wildcards are left to the
  permissions mod.

## Crates and platform

- [x] (`snap` 1.1.2, `snappy` feature) `snap` (Snappy) in Horizon's `Cargo.lock` / `horizon-mod-deps` — needed for `compress_data`
  — was: the core's own raw-Snappy codec; now `snap::raw`.
- [ ] `redis` with `tls-rustls` in `horizon-mod-deps` — needed for `redis.use_ssl` — today: refused at start.
- [x] (`getrandom` 0.3) `uuid` with `v4` (or `getrandom`) in the lock — needed for snapshot ids — today: v4 ids from
  `RandomState`-keyed SipHash (unique, not cryptographic).
- [x] (`Value::from(NaiveDateTime)`) `mysql` with `chrono` feature — needed to bind `DATETIME` directly — today: `Value::Date` by hand.
- [ ] Map hooks (a filled map's id and colours on save/load; `component(stack, "minecraft:map_id")`, `new_map`, `paint_map`, `send_map` write maps, but nothing READS a map's colours: capturing a locked map has no source) — needed for
  HuskSync's locked map sync (`persist_locked_maps`) — today: not started.

## Messages (round 3)

- [ ] Clickable / hoverable text: HuskSync's snapshot overview, list, buttons and page footer use MiniMessage
  `click`/`hover` events. `native_tell` takes plain text only, so the port prints the labels with the command each
  button runs in parentheses, and the page arrows/jumpers as text. Needs a `native_tell` that takes a text
  component with click events (and colours: all messages are uncoloured today).
- [ ] `/inventory` and `/enderchest` editing (`horizonsync.command.inventory.edit`, save on close) needs a
  menu-close hook that hands back the container contents.
- [ ] Web dumps (`mc-logs` upload) and `/horizonsync dump confirm` system dumps are not ported (nothing is uploaded).
- [ ] `/horizonsync migrate` (MPDB, legacy) and `forceupgrade` (v2 converter) are console-only stubs.
