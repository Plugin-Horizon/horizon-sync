# hmod port: husksync
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
