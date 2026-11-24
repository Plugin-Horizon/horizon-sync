# HorizonSync

## Port status

rec100 audit (2026-11-24): 17/25 items done, see audit/rec100/husksync.md; the cargo feature `next-engine` (off) adds the locked inventory click cancel and advancement sync for engine main.

Compared with HuskSync 4.1.0 (estimate): **69 %** of the original's commands, permissions, config keys, messages and features work today; **75 %** once our open Horizon PRs are merged; **76 %** with the engine changes we are writing; **70 %** with the newest Horizon SDK. Estimated count: 200/290 items. Status: not yet compiled after the 2026-11-20 round.

HorizonSync -- player data sync for the Horizon server (https://github.com/arismoko/horizon, the
Rust server for Minecraft 26.3). Players keep their inventory, ender chest, health, hunger, experience,
effects, game mode, attributes and (optionally) location when they move between servers that share
one MariaDB/MySQL database and one Redis.

**Compatible with the HuskSync data format.** Snapshots, database tables and Redis keys follow the
format of [HuskSync](https://github.com/WiIIiam278/HuskSync) 4.1, so a Horizon server can join the same
cluster as Paper servers running HuskSync. The default table names (`husksync_users`,
`husksync_user_data`, ...), the database name and the `husksync:` data keys are kept on purpose for this
compatibility; the table names are configurable. HorizonSync includes code derived from HuskSync
(Apache-2.0); see [NOTICE](NOTICE) and [LICENSE](LICENSE).

> **Built for and working only with [arismoko/horizon](https://github.com/arismoko/horizon)** (the Horizon fork that carries the mod API). [horizon-rust/horizon](https://github.com/horizon-rust/horizon) is experimental for mods and not supported: the earlier version for its native hooks / horizon-mixins is frozen on the `horizon-plain` branch (work stopped on 2026-11-12).

## Which Horizon

This is a native trait mod: `horizon-mod.toml`, a `Cargo.toml` that depends on `horizon-mod-api`, `src/lib.rs` with the hooks, and the plugin logic in the `core/` crate (plain Rust, no engine needed for its tests).

- Pin: `[native] horizon = "54090ee36c4e2c7b43b66db864220c577b94b7e0"` in `horizon-mod.toml` (game `26.3`). That is a development build of arismoko/horizon `main` plus mod API additions that are not all merged upstream yet, so a stock `main` may lack some host methods this mod calls until they land.
- It compiled and loaded on that build and was smoke-tested there; it has not seen a long run with real players. Treat it as experimental.
- Feature descriptions below were written for the earlier horizon-rust version; where they name engine calls (`native_*`) or old file paths, read them as descriptions only. The Known gaps list is current.

## What it does

- **Snapshots in HuskSync's format.** `DataSnapshot.Packed` JSON (format version 5, platform type
  `bukkit`, Minecraft version `26.3`), Snappy-compressed (`compress_data`), stored in
  `husksync_user_data` and cached in Redis exactly as HuskSync does. Paper reads what Horizon writes
  and the other way round.
- **Data types** (`synchronization.features`): `inventory`, `ender_chest`, `potion_effects`, `health`,
  `hunger`, `experience`, `game_mode`, `flight_status`, `attributes`, `persistent_data` and `location`
  are captured from and applied to the player. `statistics` too (every key of the block, item, entity
  type and custom statistic registries is read with `player_stat`; applying resets each saved
  statistic and awards its count). `advancements` and other plugins' types are carried over unchanged
  from the snapshot the player last received (Horizon does not read or write them yet), so a Paper
  server still finds them.
- **Server switches.** `LOCKSTEP` (the `data_checkout` key, check-in petitions) and `DELAY` (the
  `server_switch` key and the network latency wait), with HuskSync's Redis polling (Fibonacci tick
  back-off, database fallback after 100 ticks) and its staleness rule (a Redis snapshot older than
  five seconds loses to a newer `DISCONNECT`/`SERVER_SHUTDOWN` database snapshot).
- **Redis**: plain, or TLS with `redis.credentials.use_ssl` (`rediss://`, the system's certificates); with `redis.sentinel.master` the primary is found through the sentinels (`redis.sentinel.nodes`, `password`).
- **Saves**: `DISCONNECT` on leave, `WORLD_SAVE` on every world save, `SERVER_SHUTDOWN` on stop (with
  the bounded wait and the verified retry of disconnect writes), `SAVE_COMMAND`, `BACKUP_RESTORE`,
  rotation (`max_user_data_snapshots`), `snapshot_backup_frequency`, auto-pinned causes, and `DEATH`
  (`save_on_death`: the drops, or with `ITEMS_TO_KEEP` the inventory when `keep_inventory` is on; the drops past the
  41 inventory slots are removed from the drops, as HuskSync does; a dead player's inventory is left empty when
  `sync_dead_players_changing_server` is off).
- **Cross-server messages**: `update_user_data`, `request_user_data`/`return_user_data` (an operator on
  another server views a player who is here), `check_in_petition`.
- **The lock** while a player's data loads: commands (`blacklisted_commands_while_locked`), breaking,
  placing and using, attacking and interacting, picking up items, opening containers, armour stands,
  taking damage.
- **Commands** (operator level 2 for everything but `/husksync` and `/husksync about`; permission nodes are `husksync.command.*`, as in HuskSync; every text comes from the locale file: the embedded `messages-en-gb.yml`, overridden key by key by `messages-<language>.yml`):
  `/husksync [about|status|reload|update|dump [confirm]]` (`forceupgrade`/`migrate` are console-only stubs),
  `/userdata view|list|delete|save|restore|pin|dump` (alias `/playerdata`),
  `/inventory <player> [version]` (`/invsee`, `/openinv`) and `/enderchest <player> [version]`
  (`/echest`, `/openechest`), the last two as a chest menu; with `husksync.command.inventory.edit` /
  `husksync.command.enderchest.edit` the player's latest data can be edited (left and right click pick
  up, put down and swap, shift-click moves between the menu and the viewer's inventory; drags, number
  keys, throws and double clicks do nothing) and is saved as a new snapshot when the menu closes.

## Item compatibility

HuskSync stores items with Item-NBT-API: each stack is Minecraft's own `ItemStack.CODEC` NBT
(`id`, `count`, `components`) plus `Slot` (int) and `DataVersion` (int), inside `{size:N, items:[...]}`,
written as SNBT text. Horizon encodes stacks with the same codec (`horizon_items::item_nbt`), so:

- **Same version (26.3, DataVersion 5023) both ways: compatible** for every component Horizon models
  (damage, enchantments, stored enchantments, repair cost, potion contents, dyed colour, trims, banner
  patterns, bundle contents, written books, ...) and for every component it does not model, which it
  keeps verbatim (`StackData.other`: custom name, lore, custom data / plugin item data, item model,
  profile, ...). Counts outside 1..99 read as 1 on both sides (codec rule).
- **Older snapshots** (items with a lower `DataVersion`, or legacy `tag`/`Count` stacks) are upgraded with
  Horizon's DataFixer port (`horizon_world::datafix`, `Ty::ItemStack`) before reading, as Paper does with
  `DataFixerUtil.fixUpRawItemData`. Items without a `DataVersion` use the snapshot's Minecraft version.
- **Not checked yet**: removed-component markers (`"!minecraft:food":{}`), non-finite float values
  (HuskSync itself skips such items), unpaired UTF-16 surrogates in text (lost in the SNBT round trip),
  and the exact component output of `canonical_other_component` against Paper's for each of the ~100
  vanilla components. A byte-for-byte SNBT match is not needed: both sides parse.

## Config

`config/horizonsync/config.toml` in the server folder, written with the defaults on the first start
([core/config.example.toml](core/config.example.toml)). Keys keep HuskSync's names (compatible data format); copy the values
from a Paper server of the same cluster. Additions: `server_name` (HuskSync's `server.yml`; empty
uses the server folder's name), `minecraft_version` and `platform_type` (what snapshots say they come
from; keep `bukkit` to share with Paper), `database.timestamp_zone` (the zone of the `timestamp`
column, `local` as a Paper JVM on the same machine) and `[worlds]` (the Bukkit world name and UUID per
dimension, for `location`).

Not supported: `database.type` `POSTGRES`/`MONGO` (no driver in the build), map syncing, `cancel_packets` (the lock is event hooks, not packet filters), `enable_plan_hook` (Plan is not on Horizon), `event_priorities` (no listener priorities in Horizon's hooks), `check_for_updates` (see the update checker under Known gaps).

## Build

Build it as a native trait mod with arismoko's launcher (see the "native mods" how-to in the arismoko/horizon docs):

```sh
horizon-launcher native-build <path to this folder> --server-output <dir> --version <tag> --trust-native-code
```

`--trust-native-code` is your consent to compile and run this code inside the server. The logic crate tests run without the engine: `(cd core && cargo test)`.

## Known gaps

What does not work yet on arismoko/horizon (each is marked `TODO(arismoko-api: ...)` in `src/lib.rs`):

- Off by default: set `enabled = true` in `config/husksync-mod.toml` after filling in the database section.
- Locked players can still click in their own inventory while their data loads (no hook cancels it); damage and projectile launches (the item use that starts one) are cancelled. The advancement announcement of a locked player is still shown, and a quitting player's item on the cursor is not dropped (no hook cancels the announcement, and there is no item drop API).
- Advancements are carried over unless the cargo feature `horizon_advancements` is on (it needs `player_advancements` / `player_set_advancements`, an open upstream pull request).
- Statistics are read by asking every registry key (`player_stat`), a few thousand calls per capture.
- `/husksync update` (no update checker, so no version answer), the mc-logs upload: `dump confirm` writes the status lines to `dumps/system_<time>.txt` instead, and `/userdata dump ... web` writes a file too. The `data_manager_status` line (the stock locale
  misspells its key), POSTGRES/MONGO and map syncing (`persist_locked_maps`) are not implemented.
- The editable menus carry out their clicks themselves (the game's chest menu moves nothing for a mod menu): pick up, put down, swap, shift-click, number keys, the off hand (F), double clicks and drags. Throws (Q, or a click outside the window) do nothing, as there is no item drop API. A hover text line break is kept only in hovers.
- The DISCONNECT snapshot depends on hook order and is not fully verified.
- Snapshot compatibility with the Java HuskSync plugin has to be checked on your data before you trust it: keep backups.

## Licence

Apache License 2.0, see [LICENSE](LICENSE) (the licence of HuskSync, from which this is derived by reading
its source); the attribution is in [NOTICE](NOTICE).
