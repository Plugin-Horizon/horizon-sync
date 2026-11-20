# hmod port: husksync
Parity: before ~65% -> after ~66% (192/290 items; method: audit counts of commands, nodes, config keys, message keys and behaviours, estimated; items blocked by engine API count as not done)
Round 3 (blind, no compile)
- Fixed: a locked player launches no projectile. `right_click_item` (cancel before `Item.use`) refuses the items that launch one (bow, crossbow, trident, snowball, egg, ender pearl, potions, experience bottle, wind charge, fishing rod, firework rocket) through `core::refuse_launch`, so `allow_pearl_spawning_while_locked` still lets pearls through. A projectile from a dispenser or a mob is not a player's and is not in the original's lock either.
- Added: advancement sync behind the cargo feature `horizon_advancements` (off by default): `read_player` takes `player.advancements(host)` (vanilla's save JSON) into the snapshot, `apply` loads it with `player.set_advancements(host, json)`; the core's `records` already converts both ways. The two host calls (`player_advancements`, `player_set_advancements`) are in the open upstream pull request `gap-advancements`, not in the pinned SDK: the method names on `PlayerRef` are assumed from the other player calls.
- Left open, no engine API: clicks in a locked player's own inventory screen (no cancelling hook; `container_opened` is refused); advancements without the feature; map sync.
- Left open, out of scope here: update checker and web dump (network), POSTGRES and MONGO databases, Redis Sentinel and TLS (crates and a live test), the stock locale's misspelt `data_manager_status` line.
- Unsure: `RightClickItemEvent` field `click.player` / `click.stack`; `ItemStack::item().id(host)` returning a string-like with `as_str`.
Round 1
- Fixed: src/lib.rs imports: added `InteractionResult, MenuReplace, NodeDefault, PermissionGatherNodesEvent, PermissionLevel, PermissionNode, PermissionValueType`.
- Fixed: src/lib.rs `read_player`: `player.advancements` / `player.statistics` reads removed (2 errors), replaced by a HMOD-GAP comment.
- Fixed: src/lib.rs `apply`: `set_advancements` / `set_statistics` blocks removed (2 errors), replaced by a HMOD-GAP comment. `set_persistent_data` now checked with `.is_err()` (Result, 1 error); inventory and ender chest slot writes checked with `.is_err()` (2 errors).
- Fixed: src/lib.rs `respawn`: `player.respawn(host).is_ok()` (Result, 1 error).
- Fixed: src/lib.rs `run_mod_command` / `player_command` / `register_commands`: permission checks use `permission_bool` with `boolean_node(..)` and `.unwrap_or(false)`; `Command::op(..).with_node(&boolean_node(..))` (5 errors: permission_value, has_permission x3, with_node type).
- Fixed: src/lib.rs `open_menu` call: the 5th argument `MenuReplace::CloseFirst` (vanilla default) added (1 error).
- Fixed: src/lib.rs `entity_interact`: signature is `(host: HostSide<'_>, ..) -> Option<InteractionResult>`; a cancel (`Flow::Cancel`) maps to `Some(InteractionResult::Fail)` (1 error).
- Fixed: src/lib.rs `item_toss`: `ItemTossEvent` has no lifetime (1 error).
- Added: src/lib.rs `permission_gather_nodes` + `permission_nodes()` + `boolean_node()`; `run_mod_command` binds `(node, _)` (the `required` value is no longer used there).
- Permission nodes (Boolean, declared in `permission_nodes()`; renamed from `horizonsync.*` to HuskSync's `husksync.*` in the parity audit):
  - `husksync.command.husksync` -> 0 (the bare command and `about`)
  - `husksync.command.husksync.{status,reload,update,dump}` -> 2
  - `husksync.command.userdata` -> 2; `husksync.command.userdata.{view,list,delete,save,restore,pin,dump}` -> 2
  - `husksync.command.inventory`, `husksync.command.enderchest` -> 2
  - `husksync.command.inventory.edit`, `husksync.command.enderchest.edit` -> 2 (HuskSync leaves them undeclared: Bukkit's default for an unknown node is operators)
- Gaps: HMOD-GAP(player-advancements): Horizon main has no per-player advancement read or write (only `advancement(id)` lookups). Advancements are carried over from the last snapshot, not synced. (Statistics are synced since the audit: `player_stat`, `player_reset_stat`, `player_award_stat`.)
- Gaps: HMOD-GAP(locked-projectile-cancel): `player_launch_projectile` / `projectile_launch` only listen; a locked player's projectile is not refused. HMOD-GAP(locked-inventory-click): no hook cancels a click in the player's own inventory screen.
- Unsure: (1) `horizonsync.command.horizonsync` default is 2, while the old bare command was level 0 with an unset node (granted). The bare command is still usage level 0, so the core still answers it; `about` defaults to 0 as before. (2) Error from a permission query is a denial, which also applies to `about` (the old code granted on unset). (3) `mod_api::MenuReplace` and `mod_api::InteractionResult` are assumed to be re-exported at the mod_api root (they come from the vocab item module through the glob re-export; not compiled).

Round 4 (blind, system dump as a local file)
- Fixed: `/husksync dump confirm` writes the status lines (the same lines as `/husksync status`, no passwords) to `<config>/dumps/system_<time>.txt` and prints `system_dump_ready` with the file name on a click-to-copy line, in place of the "not available" error. No upload.
- Checked: `userdata dump ... web` already writes a file (`dump_to_file`), so no change there.
- Not done: `/husksync update` (the original asks the update checker; without network the answer would be a guess, so the port still says the checks are not part of Horizon); the locale messages that the port still writes as plain text (`Error: ...` lines from errors, the migrate stub lines).
- Tests (blind, not run): none for the file write (it needs an engine); the change is in `commands.rs` (`write_system_dump`).
