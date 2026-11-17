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
- Permission nodes (Boolean, declared in `permission_nodes()`):
  - `horizonsync.command.horizonsync` -> 2; `horizonsync.command.horizonsync.about` -> 0
  - `horizonsync.command.horizonsync.{status,reload,update,dump}` -> 2
  - `horizonsync.command.userdata` -> 2; `horizonsync.command.userdata.{view,list,delete,save,restore,pin,dump}` -> 2
  - `horizonsync.command.inventory`, `horizonsync.command.enderchest` -> 2
  - `horizonsync.command.inventory.edit`, `horizonsync.command.enderchest.edit` -> 2
- Gaps: HMOD-GAP(player-advancements-statistics): Horizon main has no per-player advancement or statistics read or write (only `advancement(id)` lookups). The leave snapshot and the join apply no longer carry advancements or statistics; the mod loses that part of the sync. The leave-time file read of `advancements/<uuid>.json` and `stats/<uuid>.json` (`read_saved`) still runs as before.
- Unsure: (1) `horizonsync.command.horizonsync` default is 2, while the old bare command was level 0 with an unset node (granted). The bare command is still usage level 0, so the core still answers it; `about` defaults to 0 as before. (2) Error from a permission query is a denial, which also applies to `about` (the old code granted on unset). (3) `mod_api::MenuReplace` and `mod_api::InteractionResult` are assumed to be re-exported at the mod_api root (they come from the vocab item module through the glob re-export; not compiled).
