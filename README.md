# HorizonSync

HorizonSync -- player data sync for the Horizon server (https://github.com/horizon-rust/horizon, the
Rust server for Minecraft 26.3). Players keep their inventory, ender chest, health, hunger, experience,
effects, game mode, attributes and (optionally) location when they move between servers that share
one MariaDB/MySQL database and one Redis.

**Compatible with the HuskSync data format.** Snapshots, database tables and Redis keys follow the
format of [HuskSync](https://github.com/WiIIiam278/HuskSync) 4.1, so a Horizon server can join the same
cluster as Paper servers running HuskSync. The default table names (`husksync_users`,
`husksync_user_data`, ...), the database name and the `husksync:` data keys are kept on purpose for this
compatibility; the table names are configurable. HorizonSync includes code derived from HuskSync
(Apache-2.0); see [NOTICE](NOTICE) and [LICENSE](LICENSE).

**Status: written blind** (no engine build, no server run yet). The core crate builds and its unit
tests pass; the engine glue and injections have not been compiled. See [STATUS.md](STATUS.md) and
[GAPS.md](GAPS.md).

## What it does

- **Snapshots in HuskSync's format.** `DataSnapshot.Packed` JSON (format version 5, platform type
  `bukkit`, Minecraft version `26.3`), Snappy-compressed (`compress_data`), stored in
  `husksync_user_data` and cached in Redis exactly as HuskSync does. Paper reads what Horizon writes
  and the other way round.
- **Data types** (`synchronization.features`): `inventory`, `ender_chest`, `potion_effects`, `health`,
  `hunger`, `experience`, `game_mode`, `flight_status`, `attributes`, `persistent_data` and `location`
  are captured from and applied to the player. `advancements`, `statistics` and other plugins' types
  are carried over unchanged from the snapshot the player last received (Horizon does not read or
  write them yet), so a Paper server still finds them.
- **Server switches.** `LOCKSTEP` (the `data_checkout` key, check-in petitions) and `DELAY` (the
  `server_switch` key and the network latency wait), with HuskSync's Redis polling (Fibonacci tick
  back-off, database fallback after 100 ticks) and its staleness rule (a Redis snapshot older than
  five seconds loses to a newer `DISCONNECT`/`SERVER_SHUTDOWN` database snapshot).
- **Saves**: `DISCONNECT` on leave, `WORLD_SAVE` on every world save, `SERVER_SHUTDOWN` on stop (with
  the bounded wait and the verified retry of disconnect writes), `SAVE_COMMAND`, `BACKUP_RESTORE`,
  rotation (`max_user_data_snapshots`), `snapshot_backup_frequency`, auto-pinned causes.
- **Cross-server messages**: `update_user_data`, `request_user_data`/`return_user_data` (an operator on
  another server views a player who is here), `check_in_petition`.
- **The lock** while a player's data loads: commands (`blacklisted_commands_while_locked`), breaking,
  placing and using, attacking and interacting, picking up items, opening containers, armour stands,
  taking damage.
- **Commands** (operator level 2 for everything but `/horizonsync about`; `/horizonsync` has the alias `/hsync`; `/userdata`, `/inventory` and `/enderchest` keep their generic names; permission nodes are `horizonsync.command.*`):
  `/horizonsync [about|status|reload|update|dump [confirm]]` (`forceupgrade`/`migrate` are console-only stubs),
  `/userdata view|list|delete|save|restore|pin|dump` (alias `/playerdata`),
  `/inventory <player> [version]` (`/invsee`, `/openinv`) and `/enderchest <player> [version]`
  (`/echest`, `/openechest`), the last two as a read-only chest menu.

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

Not supported: `database.type` `POSTGRES`/`MONGO`, Redis Sentinel, Redis over TLS, map syncing.

## Build

A horizon-mixins bundle (this folder): one crate (`core`, using only crates in Horizon's
`Cargo.lock`: `mysql` 27 with `minimal-rust` + `rustls-tls-ring`, `redis` 1 without default features,
`serde`, `serde_json`, `toml`, `chrono`, `log`), one support module (`glue/horizonsync.rs`, placed in
`crates/horizon-client/src/game.rs` as `game::horizonsync`), and 21 HEAD injections (`helpers/`).

```sh
horizon-mixins apply --workspace <horizon source> --mods <folder containing this bundle> --out <stage>
cd <stage> && cargo build --profile fast -p horizon-client --no-default-features --locked
```

`horizon_revision` in `horizon-mixin.toml` pins the Horizon commit it was written against.
Unit tests: `cargo test` in `core/` with Horizon's `Cargo.lock` copied next to it.

## Licence

Apache License 2.0, see [LICENSE](LICENSE) (the licence of HuskSync, from which this is derived by reading
its source); the attribution is in [NOTICE](NOTICE).

## Contributing

Pull requests and improvements are always welcome! Open an issue or a PR -- direct pushes to main are disabled, every change goes through review.
