# hmod port: husksync
Parity: 69% now / 75% with our PRs (200/290 items now, 217/290 with PRs; method: audit counts of commands, nodes, config keys, message keys and behaviours, estimated; items blocked by engine API count as not done)
Round 100 (blind, no compile; walk of the whole original: commands and sub-commands with their arguments and permission
nodes, tab completion, config keys, every locale key, the listeners of BukkitEventListener / BukkitLockedEventListener /
PaperEventListener / BukkitDeathEventListener)
- Fixed: number-key swap (menu click mode 2, buttons 0-8) swaps the menu slot with the viewer's hotbar; the off-hand key F (button 40) swaps with the viewer's off hand (`PlayerSlot -106`). `edit_click` now takes the viewer.
- Fixed: double click (mode 6) gathers the cursor's kind from every menu slot as far as the cursor takes it.
- Fixed: drags (mode 5) are collected from their start, added slots and end; the end shares the cursor's stack over the slots (left drag evenly, right drag one each), `drag_share` / `drag_carry`.
- Fixed: Redis over TLS (`redis.credentials.use_ssl`: `rediss://`, the `tls-rustls` feature of the redis crate, system certificates) instead of an error at start.
- Fixed: Redis Sentinel (`redis.sentinel.master` / `nodes` / `password`): the primary is found through the sentinels (`SentinelClient`, sentinel nodes with the sentinel password, TLS with the credentials' `use_ssl`); the status line "Using Redis Sentinel" reads it.
- Fixed: with save_on_death on and `items_to_save: DROPS`, the drops past the 41 inventory slots are removed from the death's drops, as HuskSync's `handlePlayerDeath` does (they are not dropped in the world either).
- Checked, matches: all 140 locale keys are in `messages-en-gb.yml` and are read; `data_manager_status` stays out (its key is misspelt in the stock locale, so the original shows no line); command labels and aliases (`husksync`, `userdata`/`playerdata`, `inventory`/`invsee`/`openinv`, `enderchest`/`echest`/`openechest`), console-only `forceupgrade`/`migrate`; permission defaults; locked-event list (projectile, drop, pickup, interact, armour stand, place, break, open, inventory open, damage, command blacklist).
- Left open, API: clicks in a locked player's own inventory (`GAPS_UPSTREAM` line); the throw of a menu item and the quitting player's cursor item (no item drop API, GAPS line); the advancement announcement of a locked player (`player_advancement_done` cannot cancel the message, GAPS line); map state for `persist_locked_maps` (no map state API, GAPS line); HTTP for the update checker and the mc-logs upload (GAPS line); advancements (feature `horizon_advancements`, open PR).
- Left open, not done: POSTGRES and MONGO (the `postgres` crate is not in the registry this build uses; a blind driver would be a guess); `cancel_packets`, `enable_plan_hook`, `event_priorities`, `check_for_updates` (not applicable on Horizon, see README).
- Tests (blind, not run): `menu_clicks_carried_out_are_the_vanilla_kinds_with_their_buttons`, `drag_shares_the_cursor_evenly_on_a_left_drag_and_one_each_on_a_right_one` (lib.rs).
- Unsure for the compile round: `ItemPlace::insert` / `extract` results with an off-hand `PlayerSlot`; `RedisConnectionInfo::set_db` (i64) and `SentinelNodeConnectionInfo::set_tls_mode` names (checked against redis 1.8.2 sources only); `redis` features `tls-rustls` + `sentinel` pull `rustls-native-certs` 0.8.

Checklist round 100
- [x] commands: about, status, reload, update (no network), dump (confirm writes a file), forceupgrade, migrate (stub)
- [x] userdata: view, list (pages), delete, save, restore, pin, dump (web writes a file, file)
- [x] inventory, enderchest (+ aliases), editable with the edit node; the viewer's clicks in every mode except throws (API gap)
- [x] permission nodes and defaults (husksync 0, the rest level 2, console-only 4, edit nodes 2)
- [x] tab completion of player names, versions, page numbers, dump types; migrator names
- [x] config keys: all read; defaults as Settings.java; unused (see README not-supported list)
- [x] messages: every key of the locale file; hard-coded port-only lines listed in README
- [x] listeners: join, quit (cursor drop: API gap), death (drops truncation fixed), world save, server stop, command blacklist while locked, locked events
- [ ] inventory click of a locked player (API gap)
- [ ] advancement announcement of a locked player (API gap)
- [ ] item drop for throws and the quit cursor (API gap)
- [ ] map state (API gap)
- [ ] HTTP for the update checker and dump uploads (API gap)
- [ ] POSTGRES, MONGO (no driver)
- [ ] advancements sync (feature `horizon_advancements`, open PR)
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

Round 2 (combined SDK, 2026-11-25): cargo features `next-engine` and `horizon_advancements` removed (code is normal now); `container_click` cancels a locked player's clicks; advancement sync on; new `maps.rs` persists locked maps (inventory, ender chest, shulker boxes, bundles) in the map tables and rebuilds them on apply (`level_map_data`, `level_new_map`, `level_set_map_locked`); `throw_click` carries out Q/outside throws with `player_drop_stack`; `drop_cursor` drops a leaving player's cursor stack; `/husksync dump confirm` also uploads the status lines to mc-logs (feature `net`, ureq). Still open: advancement announcement cancel (engine), Postgres/Mongo, event API.
