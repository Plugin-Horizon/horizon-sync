# Status (2026-11-08, written blind)

The core crate (`core/`) compiles without warnings and its 38 unit tests pass (`cargo test`, offline,
with Horizon's `Cargo.lock`), including 10 tests of the sync rules (LOCKSTEP, DELAY, staleness,
checkout fallback after ~5 s, locked leave, shutdown) against an in-memory database and Redis. The glue module and the 21 injections were written against Horizon `a589a3dd9` by
reading its source and have **not been compiled**: no engine build, no server, no database or Redis
run. Paper interoperability is argued from HuskSync's source, not observed.

Legend: `written` (complete as far as can be told without a build), `partly`, `not started`.

## Pinned to Horizon b13dbf0687b5 (main): hooks not on main

Dropped from the bundle (not in `native-hooks.json`, only pending fragments; not invented):

- needs upstream hook `block_place`: lock on placing a block. Partly covered by the `use_item_on` claim
  (`lock-use-on`), which runs before placement when a block is clicked; placing against air is not covered.
- needs upstream hook `inventory_click`: lock on vanilla menu clicks (`lock-click`).
- needs upstream hook `inventory_drag`: lock on vanilla menu drags (`lock-drag`).
- needs upstream hook `player_launch_projectile_v2`: lock on throwing a pearl or snowball (`lock-launch`).

Used although still pending fragments (each needs `"hooks": @API:<fragment>@` once stamped): `player_logging_out`, `item_dropped_v2`, `living_incoming_damage_v3`, `menu_input` (`menu_input_closed`).

The `snap` crate (Snappy), `getrandom` (ids) and `mysql`'s `chrono` binding are used as in Horizon's lock. The `mysql` crate is pinned to 28 (the version in Horizon's Cargo.lock, with `chrono`), not 27.

## Data format and storage

| HuskSync feature | Status | Notes |
|---|---|---|
| Snapshot JSON (`DataSnapshot.Packed`, format 5), id/pinned/timestamp/cause/server/version/platform | written | serde, Gson field names; ISO offset timestamps |
| Snappy compression (`compress_data`) | written | own raw-Snappy codec (no snappy crate in the lock); reads either encoding |
| Snapshot validation (newer MC version, newer format, legacy <4, platform type) | written | legacy v1/v2 conversion not ported |
| `husksync:inventory` (41 slots, held slot), `husksync:ender_chest` | written | SNBT writer/reader, NBT-API layout, overflow placement as `upgradeItemStacks` |
| Item upgrade from older DataVersions | written | Horizon DataFixer in the glue (untested) |
| `husksync:potion_effects` | written | ambient effects left out on capture, as Bukkit |
| `husksync:health` | written | 0 health kills the player (`set_player_status`); health scale stored as 20/false, not applied |
| `husksync:hunger` | written | exhaustion goes to the live body only |
| `husksync:experience` | written | |
| `husksync:game_mode` | written | via `/gamemode` run as console |
| `husksync:flight_status` | partly | Horizon derives `mayfly` from the game mode: survival allow-flight is lost |
| `husksync:location` | written | via `execute in … run tp`; off in the reference setup |
| `husksync:attributes` | written | synced/ignored wildcards; only saved (permanent) modifiers are seen |
| `husksync:persistent_data` | written | the player compound's `BukkitValues`, kept by Horizon in `guest.saved` |
| `husksync:advancements`, `husksync:statistics` | written | captured from `player_advancements` / `player_stats` and applied with the setters (`records.rs`: vanilla JSON <-> HuskSync's lists; Bukkit statistic names for renamed stats such as play time are unverified); falls back to the carried data when the game gives none |
| Custom (API) data types of other plugins | partly | carried over unchanged |
| MariaDB/MySQL schema, users table, ensureUser | written | statements copied from `MySqlDatabase` |
| Snapshot CRUD, rotation, backup frequency, pin/unpin, auto-pin | written | |
| PostgreSQL, MongoDB | not started | not used here |
| Map data tables / locked map sync (`persist_locked_maps`) | not started | needs map item + map data hooks |

## Synchronisation

| Feature | Status | Notes |
|---|---|---|
| Lock on join until data applied | written | |
| LOCKSTEP: data_checkout key, poll with Fibonacci back-off, DB fallback | written | |
| DELAY: server_switch key, latency wait | written | |
| Redis staleness rule vs. shutdown-critical DB snapshots | written | |
| Check-in petitions | written | off by default |
| DISCONNECT save on leave (`player_logging_out` + `player_compound`) | written | |
| WORLD_SAVE saves (`world_saved` hook) | written | |
| SERVER_SHUTDOWN saves + bounded wait + verified DISCONNECT retries | written | |
| `update_user_data` / `request_user_data` / `return_user_data` messages | written | |
| Notifications (ACTION_BAR, CHAT; TOAST shown as action bar) | written | en-gb only, plain text |
| Save on death (`save_on_death`) | not started | off in the reference setup |
| Respawn a player who joins dead | written | `respawn_player`, the plan waits for the respawn |
| Lock: commands, break, left/right click, use, attack, interact, pickup, containers, armour stands, damage (victim UUID), item drop | written | damage via `living_incoming_damage_v3`, drop via `item_dropped_v2` |
| Lock: projectile/pearl launch, inventory clicks, packet cancelling | not started | still only pending fragments (see the list above) |
| Event API (PreSync, DataSave, SyncComplete), Plan hook | not started | no Java API on Horizon |

## Commands

| Command | Status | Notes |
|---|---|---|
| `/horizonsync about`, `status` | written | |
| `/horizonsync reload`, `update`, `dump` | partly | answer only; no live reload, no web dump |
| `/horizonsync migrate` | not started | console-only in HuskSync; `forceupgrade` is not in the public docs |
| Unknown `/horizonsync` sub-command | written | usage line (HuskSync's exact text not verified) |
| Config layout | written | HuskSync's nested `database.credentials`, `database.connection_pool`, `redis.credentials` are lifted at load |
| `/userdata view`, `list`, `delete`, `save`, `restore`, `pin` | written | the overview's View/Manage/System buttons are clickable (hover hint, run the command); list arrows and page jumpers are plain text |
| `/userdata dump` | partly | file dumps only (no web upload) |
| `/inventory`, `/enderchest` (and aliases) | written | editable for who holds `horizonsync.command.inventory.edit` / `enderchest.edit` (latest data only); closing saves a new INVENTORY_COMMAND / ENDERCHEST_COMMAND snapshot if the items changed |
| Permission nodes `horizonsync.command.*` | written | per sub-command via `native_has_permission`; default level 2 (`about` 0); `.*` wildcards only if the permissions mod grants them |
| `disabled_commands` | written | |
| Tab completion | written | one usage per sub-command path (26 usages with the aliases); `<username>` uses `player_name`, so online names complete; a malformed version UUID fails instead of falling back to the latest snapshot |

## Counts

written 32, partly 5, not started 6.

## Test first, once a build is possible

0. Does `playerdata::player_tag(guest)` include `BukkitValues` from `guest.saved` (persistent data)?
1. `horizon-mixins apply` + `cargo build -p horizon-client`: the glue module and the raw
   `save_guest` target are the likeliest to need fixes (paths, borrows, privacy).
2. Throwaway MariaDB + Redis on private ports, one throwaway Paper 26.3 with HuskSync 4.1 and one
   Horizon server with this mod on the same cluster: a player with a full inventory (enchanted,
   named, lore, damaged, potion, shulker box with contents, written book, player head), armour,
   off hand, ender chest, effects, XP, hunger switches Paper → Horizon → Paper; diff the item NBT.
3. LOCKSTEP switch timing: the checkout key is released by the leaving server before the joining
   one gives up (5 s); no duplication when switching fast.
4. Stop the Horizon server with players on: SERVER_SHUTDOWN snapshots present, checkouts cleared.
5. `/userdata list|view|restore|pin|delete` against snapshots written by Paper.
6. The lock: a player who joins while Redis is slow can't move items, break blocks or run commands.
