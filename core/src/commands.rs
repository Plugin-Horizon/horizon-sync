//! HuskSync's commands: `/horizonsync`, `/userdata` (alias `/playerdata`), `/inventory`
//! (`/openinv`; `/invsee` is left to InvSee++) and `/enderchest` (`/echest`, `/openechest`).
//!
//! Permission nodes (`horizonsync.command.*`) are checked by the glue with `native_has_permission`
//! (default level 2, as HuskSync's `Permission.Default.IF_OP`; `/horizonsync about` is for everyone).
//! Operators pass unless a permissions mod denies the node.

use crate::data::{EnderChest, Inventory};
use crate::engine::{Action, Engine, Sender};
use crate::player;
use crate::snapshot::{cause_display, keys, Packed};
use crate::util;
use std::sync::Arc;

pub const OP: u8 = 2;

/// The usages the game registers (`NativeCommand` syntax) with their levels.
pub fn usages(disabled: &[String]) -> Vec<(String, u8)> {
    let mut out: Vec<(String, u8)> = Vec::new();
    let mut add = |name: &str, rest: &[(&str, u8)]| {
        if disabled.iter().any(|d| d.eq_ignore_ascii_case(name)) {
            return;
        }
        for (r, level) in rest {
            out.push((if r.is_empty() { name.to_owned() } else { format!("{name} {r}") }, *level));
        }
    };
    let horizonsync: &[(&str, u8)] = &[
        ("", 0),
        ("about", 0),
        ("status", OP),
        ("dump", OP),
        ("dump confirm", OP),
        ("reload", OP),
        ("update", OP),
        // Console only in HuskSync: hidden from players.
        ("forceupgrade", 4),
        ("migrate", 4),
    ];
    add("horizonsync", horizonsync);
    add("hsync", horizonsync);
    let userdata: &[(&str, u8)] = &[
        ("view <username:player_name> [version:word]", OP),
        ("list <username:player_name> [page:integer]", OP),
        ("delete <username:player_name> <version:word>", OP),
        ("save <username:player_name>", OP),
        ("restore <username:player_name> <version:word>", OP),
        ("pin <username:player_name> <version:word>", OP),
        ("dump <username:player_name>", OP),
        ("dump <username:player_name> <version:word> web", OP),
        ("dump <username:player_name> <version:word> file", OP),
    ];
    add("userdata", userdata);
    add("playerdata", userdata);
    let items: &[(&str, u8)] = &[("<username:player_name> [version:word]", OP)];
    for name in ["inventory", "openinv", "enderchest", "echest", "openechest"] {
        add(name, items);
    }
    out
}

/// The command group a label belongs to (`horizonsync.command.<group>`), aliases included.
fn group(label: &str) -> Option<&'static str> {
    match label {
        "horizonsync" | "hsync" => Some("horizonsync"),
        "userdata" | "playerdata" => Some("userdata"),
        "inventory" | "openinv" => Some("inventory"),
        "enderchest" | "echest" | "openechest" => Some("enderchest"),
        _ => None,
    }
}

/// The node a command is registered with: the first usage's node decides for the whole tree.
pub fn command_node(label: &str) -> Option<String> {
    group(&label.to_lowercase()).map(|g| format!("horizonsync.command.{g}"))
}

/// The node a command line needs and its default level (HuskSync's `plugin.yml` nodes: a
/// sub-command has its own node, `horizonsync.command.userdata.save`; `about` is for everyone).
pub fn permission_for(line: &str) -> Option<(String, u8)> {
    let mut words = line.trim().trim_start_matches('/').split_whitespace();
    let group = group(&words.next()?.to_lowercase())?;
    let sub = words.next().map(|w| w.to_lowercase());
    let known: &[&str] = match group {
        "horizonsync" => &["about", "status", "reload", "update", "dump"],
        "userdata" => &["view", "list", "delete", "save", "restore", "pin", "dump"],
        _ => &[],
    };
    let level = if group == "horizonsync" && matches!(sub.as_deref(), None | Some("about")) { 0 } else { OP };
    let node = match sub.as_deref() {
        Some(s) if known.contains(&s) => format!("horizonsync.command.{group}.{s}"),
        _ => format!("horizonsync.command.{group}"),
    };
    Some((node, level))
}

/// The node that lets a viewer edit the items of `/inventory` or `/enderchest` (None for any other
/// command): `horizonsync.command.inventory.edit`, `horizonsync.command.enderchest.edit`.
pub fn edit_permission_for(line: &str) -> Option<String> {
    let first = line.trim().trim_start_matches('/').split_whitespace().next()?.to_lowercase();
    let group = group(&first)?;
    matches!(group, "inventory" | "enderchest").then(|| format!("horizonsync.command.{group}.edit"))
}

enum Kind {
    Inventory,
    EnderChest,
}

/// Runs one of our commands; false when the line is not ours.
pub fn perform(engine: &Arc<Engine>, sender: Sender, line: &str) -> bool {
    let mut words = line.trim().trim_start_matches('/').split_whitespace();
    let Some(label) = words.next().map(|w| w.to_lowercase()) else { return false };
    let args: Vec<String> = words.map(str::to_owned).collect();
    if engine.cfg().disabled_commands.iter().any(|d| d.eq_ignore_ascii_case(&label)) {
        return false;
    }
    match label.as_str() {
        "horizonsync" | "hsync" => horizonsync(engine, &sender, &args),
        "userdata" | "playerdata" => {
            if !op(engine, &sender) {
                return true;
            }
            let engine = engine.clone();
            engine.clone().run_job(move |_| userdata(&engine, &sender, &args));
        }
        "inventory" | "openinv" | "enderchest" | "echest" | "openechest" => {
            if !op(engine, &sender) {
                return true;
            }
            let kind = if label.contains("ch") { Kind::EnderChest } else { Kind::Inventory };
            let engine = engine.clone();
            engine.clone().run_job(move |_| items(&engine, &sender, &args, kind));
        }
        _ => return false,
    }
    true
}

fn op(engine: &Engine, sender: &Sender) -> bool {
    if sender.uuid.is_some() && sender.level < OP {
        engine.tell(sender.uuid, "Error: You do not have permission to execute this command");
        return false;
    }
    true
}

fn horizonsync(engine: &Arc<Engine>, sender: &Sender, args: &[String]) {
    let to = sender.uuid;
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("about") => {
            for line in about_lines() {
                engine.tell(to, line);
            }
        }
        // Console only (HuskSync's `setCondition(isConsole)`).
        Some("forceupgrade" | "migrate") if to.is_some() => {
            engine.tell(to, "Error: That command can only be run through console");
        }
        Some("forceupgrade") => {
            // `plugin.getLegacyConverter()` is empty without the v2 converter: HuskSync returns silently.
            log::info!("horizonsync: forceupgrade: the legacy (v2) converter is not part of the Horizon port");
        }
        Some("migrate") => {
            // No migrators (MPDB, legacy) are ported: the list HuskSync would print is empty.
            engine.tell(
                None,
                "Please choose a migrator, then run \"horizonsync migrate start <migrator>\"\n\
                 List of available migrators:\nMigrator ID / Migrator Name:",
            );
        }
        // Denied: `op` has told the sender already.
        Some("status" | "reload" | "update" | "dump") if !op(engine, sender) => {}
        Some("status") => {
            let engine2 = engine.clone();
            engine.run_job(move |_| {
                engine2.tell(to, "HorizonSync | System status report:");
                for line in status_lines(&engine2) {
                    engine2.tell(to, line);
                }
            });
        }
        Some("reload") => match reload(engine) {
            Ok(()) => {
                engine.tell(to, "HorizonSync | Reloaded config and message files.");
                engine.tell(to, "⚠ Ensure config files are up-to-date on all servers!");
                engine.tell(to, "A restart is needed for config changes to take effect.");
            }
            Err(e) => {
                log::error!("horizonsync: failed to reload the plugin: {e}");
                engine.tell(to, "Error: Failed to reload the plugin. Check console for errors.");
            }
        },
        Some("update") => {
            // No update checker on Horizon (HuskSync asks Modrinth/Hangar).
            engine.tell(to, "HorizonSync | Update checks are not part of the Horizon port.");
        }
        Some("dump") => match args.get(1).map(|s| s.to_lowercase()).as_deref() {
            None => {
                for line in [
                    "HorizonSync | Prepare a system dump? This will include:",
                    "• Your latest server logs and HorizonSync config files",
                    "• Current plugin system status information",
                    "• Information about your Java & Minecraft server environment",
                    "• A list of other currently installed plugins",
                    "To confirm, use: /horizonsync dump confirm",
                ] {
                    engine.tell(to, line);
                }
            }
            Some("confirm") => {
                engine.tell(to, "HorizonSync | Preparing system status dump, please wait…");
                // The dump service (mc-logs) upload is not ported; nothing leaves the server.
                engine.tell(to, "Error: System status dumps are not available on Horizon.");
            }
            Some(_) => usage(engine, to, "/horizonsync dump [confirm]"),
        },
        Some(_) => {
            engine.tell(to, "Error: Incorrect syntax. Usage: /horizonsync <about|status|reload|update|dump>");
        }
    }
}

/// The `/horizonsync about` text.
fn about_lines() -> Vec<String> {
    vec![
        format!("HorizonSync v{}", env!("CARGO_PKG_VERSION")),
        "Player data sync for the Horizon server: inventory, ender chest, health and more follow a player across servers.".into(),
        String::new(),
        "Compatible with the HuskSync data format (database and Redis), so Horizon and Paper servers can share one cluster.".into(),
        "Derived from HuskSync by William278 and contributors (Apache License 2.0); see NOTICE.".into(),
    ]
}

/// `StatusLine`: "• Name: value", names as `WordUtils.capitalizeFully(name().replace('_', ' '))`.
fn status_lines(engine: &Engine) -> Vec<String> {
    let cfg = engine.cfg();
    let yes = |b: bool| if b { "Yes" } else { "No" };
    let local = |h: &str| yes(matches!(h, "127.0.0.1" | "0.0.0.0" | "localhost" | "::1"));
    let ms = |r: Result<std::time::Duration, String>| match r {
        Ok(d) => format!("{}ms", d.as_millis()),
        Err(e) => format!("error ({e})"),
    };
    let mode = cfg.synchronization.mode.to_lowercase();
    let mode = mode.get(..1).map(|f| f.to_uppercase() + &mode[1..]).unwrap_or_default();
    let kind = match cfg.database.kind.to_uppercase().as_str() {
        "MARIADB" => "MariaDB".to_owned(),
        "MYSQL" => "MySQL".to_owned(),
        other => other.to_owned(),
    };
    let types = [
        "inventory", "ender_chest", "potion_effects", "advancements", "location", "statistics", "health",
        "hunger", "attributes", "experience", "game_mode", "flight_status", "persistent_data",
    ]
    .iter()
    .map(|k| {
        let on = cfg.synchronization.features.enabled(&format!("husksync:{k}"));
        format!("husksync:{k} {}", if on { '✔' } else { '❌' })
    })
    .collect::<Vec<_>>()
    .join(", ");
    let rows: Vec<(&str, String)> = vec![
        ("Plugin Version", format!("v{}", env!("CARGO_PKG_VERSION"))),
        ("Server Version", "Horizon".into()),
        ("Language", cfg.language.clone()),
        ("Minecraft Version", engine.platform.minecraft_version.clone()),
        ("Java Version", "N/A (native server)".into()),
        ("Java Vendor", "N/A (native server)".into()),
        ("Server Name", engine.platform.server_name.clone()),
        ("Cluster Id", if cfg.cluster_id.is_empty() { "None".into() } else { cfg.cluster_id.clone() }),
        ("Sync Mode", mode),
        ("Delay Latency", format!("{}ms", cfg.synchronization.network_latency_milliseconds)),
        ("Database Type", kind),
        ("Is Database Local", local(&cfg.database.host).into()),
        ("Redis Version", engine.redis.version()),
        ("Using Redis Sentinel", "No".into()),
        ("Redis Database", cfg.redis.database.to_string()),
        ("Using Redis User", yes(!cfg.redis.user.is_empty()).into()),
        ("Using Redis Password", yes(!cfg.redis.password.is_empty()).into()),
        ("Redis Using Ssl", yes(cfg.redis.use_ssl).into()),
        ("Redis Latency", ms(engine.redis.ping())),
        ("Is Redis Local", local(&cfg.redis.host).into()),
        ("Locked User Handler", "HorizonEventListener".into()),
        ("Data Types", types),
    ];
    rows.into_iter().map(|(k, v)| format!("• {k}: {v}")).collect()
}

/// `/horizonsync reload`: read the config file again. As in HuskSync, connections and the like keep
/// their start-up values until a restart.
fn reload(engine: &Engine) -> Result<(), String> {
    // `engine.dir` is `<server>/config/horizonsync`: the server folder is two levels up.
    let server_dir = engine.dir.parent().and_then(|p| p.parent()).unwrap_or(&engine.dir);
    let mut fresh = crate::config::Config::load(server_dir, &engine.dir)?;
    if fresh.server_name.trim().is_empty() {
        fresh.server_name = engine.platform.server_name.clone();
    }
    *engine.config.write().unwrap_or_else(|e| e.into_inner()) = fresh;
    Ok(())
}

/// A player by name: on this server first, then the users table.
fn find_user(engine: &Engine, name: &str) -> Option<(u128, String)> {
    if let Some(u) = engine.online_by_name(name) {
        return Some(u);
    }
    if let Some(u) = util::parse_uuid(name) {
        if let Ok(Some(n)) = engine.db.user_by_uuid(u) {
            return Some((u, n));
        }
    }
    engine.db.user_by_name(name).ok().flatten()
}

fn usage(engine: &Engine, to: Option<u128>, usage: &str) {
    engine.tell(to, format!("Error: Incorrect syntax. Usage: {usage}"));
}

/// A version argument that is present but not a UUID: the lookup fails, it does not fall back to the latest snapshot.
fn not_a_version(arg: Option<&String>) -> bool {
    arg.is_some_and(|v| util::parse_uuid(v).is_none())
}

/// `DateTimeFormatter.ofLocalizedDateTime(MEDIUM, SHORT)` in the en-GB locale: `8 Nov 2026, 05:20`.
fn time_medium(ts: &str) -> String {
    util::parse_timestamp(ts).map(|t| t.format("%-d %b %Y, %H:%M").to_string()).unwrap_or_else(|| ts.to_owned())
}

/// `SaveCause.getLocale`: the `save_cause_*` text of en-gb.yml, the display name for other causes.
fn cause_text(cause: &str) -> String {
    match cause.to_lowercase().as_str() {
        "disconnect" => "disconnect".into(),
        "world_save" => "world save".into(),
        "death" => "death".into(),
        "server_shutdown" => "server shutdown".into(),
        "save_command" => "save command".into(),
        "dump_command" => "dump command".into(),
        "inventory_command" => "inventory command".into(),
        "enderchest_command" => "enderchest command".into(),
        "backup_restore" => "backup restore".into(),
        "api" => "API".into(),
        "mpdb_migration" => "MPDB migration".into(),
        "legacy_migration" => "legacy migration".into(),
        "converted_from_v2" => "converted from v2".into(),
        _ => cause_display(cause),
    }
}

/// `error_invalid_data`: the details are a hover in HuskSync, so they go to the log here.
fn invalid_data(engine: &Engine, to: Option<u128>, reason: &str) {
    log::warn!("horizonsync: invalid snapshot: {reason}");
    engine.tell(to, "Error: Failed to unpack user data as the snapshot is invalid or corrupt. (Details…)");
}

fn userdata(engine: &Arc<Engine>, sender: &Sender, args: &[String]) {
    let to = sender.uuid;
    let sub = args.first().map(|s| s.to_lowercase()).unwrap_or_default();
    // An unknown sub-command gets the usage line before any player lookup.
    if !["view", "list", "delete", "save", "restore", "pin", "dump"].contains(&sub.as_str()) {
        return usage(engine, to, "/userdata <view|list|delete|save|restore|pin|dump> <username> [version_uuid]");
    }
    let Some(name) = args.get(1) else {
        usage(engine, to, "/userdata <view|list|delete|save|restore|pin|dump> <username> [version_uuid]");
        return;
    };
    let Some((player, name)) = find_user(engine, name) else {
        engine.tell(to, "Error: Could not find a player by that name.");
        return;
    };
    let version = args.get(2).and_then(|v| util::parse_uuid(v));
    let need_version = |what: &str| -> Option<u128> {
        if version.is_none() {
            usage(engine, to, &format!("/userdata {what} <username> <version_uuid>"));
        }
        version
    };
    match sub.as_str() {
        "view" => {
            if not_a_version(args.get(2)) {
                return engine.tell(to, "Error: Could not find any user data for that version UUID.");
            }
            let stored = match version {
                Some(v) => engine.db.snapshot(player, v),
                None => engine.db.latest_snapshot(player),
            };
            match stored {
                Ok(Some(s)) => match crate::db::usable(s.snapshot, &engine.platform) {
                    Ok(s) => overview(engine, to, &s, &name, player),
                    Err(e) => invalid_data(engine, to, &e),
                },
                Ok(None) if version.is_some() => engine.tell(to, "Error: Could not find any user data for that version UUID."),
                Ok(None) => engine.tell(to, "Error: Could not find any user data to display."),
                Err(e) => engine.tell(to, format!("Error: {e}")),
            }
        }
        "list" => {
            let page = args.get(2).and_then(|p| p.parse::<usize>().ok()).unwrap_or(1).max(1);
            list(engine, to, player, &name, page);
        }
        "delete" => {
            let Some(v) = need_version("delete") else { return };
            match engine.db.delete_snapshot(player, v) {
                Ok(true) => {
                    engine.redis.clear_user_data(player);
                    engine.tell(to, format!("❌ Successfully deleted user data snapshot {} for {name}.", &util::uuid_string(v)[..8]));
                }
                Ok(false) => engine.tell(to, "Error: Could not find any user data for that version UUID."),
                Err(e) => engine.tell(to, format!("Error: {e}")),
            }
        }
        "save" => {
            if engine.online_name(player).is_none() {
                engine.tell(to, "Error: Could not find a player by that name.");
                return;
            }
            engine.save_command(player, &name, to);
        }
        "restore" => {
            let Some(v) = need_version("restore") else { return };
            let stored = match engine.db.snapshot(player, v) {
                Ok(Some(s)) => s,
                Ok(None) => return engine.tell(to, "Error: Could not find any user data for that version UUID."),
                Err(e) => return engine.tell(to, format!("Error: {e}")),
            };
            let mut s = match crate::db::usable(stored.snapshot, &engine.platform) {
                Ok(s) => s.copy(),
                Err(e) => return invalid_data(engine, to, &e),
            };
            // Restored players get at least one health point.
            player::set_health_at_least_one(&mut s.data);
            s.save_cause = "BACKUP_RESTORE".into();
            s.pinned = engine.cfg().synchronization.auto_pin("BACKUP_RESTORE");
            engine.save_and_update(player, &s);
            engine.tell(to, format!("⏪ Successfully restored {name}'s current user data from snapshot {}.", s.short_id()));
        }
        "pin" => {
            let Some(v) = need_version("pin") else { return };
            match engine.db.snapshot(player, v) {
                Ok(Some(stored)) => match stored.snapshot {
                    Ok(s) => {
                        let pin = !s.pinned;
                        match engine.db.set_pinned(player, v, pin) {
                            Ok(_) => engine.tell(
                                to,
                                format!(
                                    "※ Successfully {} user data snapshot {} for {name}.",
                                    if pin { "pinned" } else { "unpinned" },
                                    s.short_id()
                                ),
                            ),
                            Err(e) => engine.tell(to, format!("Error: {e}")),
                        }
                    }
                    Err(e) => engine.tell(to, format!("Error: {e}")),
                },
                Ok(None) => engine.tell(to, "Error: Could not find any user data for that version UUID."),
                Err(e) => engine.tell(to, format!("Error: {e}")),
            }
        }
        "dump" => {
            // `dump <user>` (latest, web) or `dump <user> <version> <web|file>`.
            let usage_dump = "/userdata dump <username> [version_uuid] [web|file]";
            if args.len() == 3 || args.len() > 4 {
                return usage(engine, to, usage_dump);
            }
            let web = match args.get(3).map(|t| t.to_lowercase()).as_deref() {
                None | Some("web") => true,
                Some("file") => false,
                Some(_) => return usage(engine, to, usage_dump),
            };
            let s = match args.get(2) {
                Some(_) => {
                    let Some(v) = version else {
                        return engine.tell(to, "Error: Could not find any user data for that version UUID.");
                    };
                    match engine.db.snapshot(player, v).ok().flatten().and_then(|s| s.snapshot.ok()) {
                        Some(s) => s,
                        None => return engine.tell(to, "Error: Could not find any user data for that version UUID."),
                    }
                }
                None => {
                    let found = engine
                        .online_user_data(player, "DUMP_COMMAND")
                        .or_else(|| engine.db.latest_snapshot(player).ok().flatten().and_then(|s| s.snapshot.ok()));
                    match found {
                        Some(s) => s,
                        None => return engine.tell(to, "Error: Could not find any user data to display."),
                    }
                }
            };
            if web {
                engine.tell(to, "Web dumps are not available on Horizon; writing a file instead.");
            }
            match dump_to_file(engine, &s, &name) {
                Ok(path) => {
                    engine.tell(to, format!("☂ Successfully dumped user data snapshot {} for {name}. Click to view:", s.short_id()));
                    engine.tell(to, path);
                }
                Err(e) => log::error!("horizonsync: failed to dump user data: {e}"),
            }
        }
        _ => usage(engine, to, "/userdata <view|list|delete|save|restore|pin|dump> <username> [version_uuid]"),
    }
}

/// `UserDataDumper.toFile`: the snapshot JSON in `<config>/dumps/`.
fn dump_to_file(engine: &Engine, s: &Packed, name: &str) -> Result<String, String> {
    let dir = engine.dir.join("dumps");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = s.time().map(|t| t.format("%Y-%m-%d_%H-%M-%S").to_string()).unwrap_or_default();
    let file = dir.join(format!("{name}_{stamp}_{}.json", s.short_id()));
    let pretty = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(&file, pretty).map_err(|e| e.to_string())?;
    // Like HuskSync (`file.getName()`), show only the file name, never the server path.
    Ok(file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

fn overview(engine: &Engine, to: Option<u128>, s: &Packed, name: &str, _player: u128) {
    let id = &s.id;
    let mut lines = vec![
        format!("Viewing user data snapshot {} for {name}:", s.short_id()),
        format!("⌚ {}", time_medium(&s.timestamp)),
    ];
    if s.pinned {
        lines.push("※ Snapshot pinned".into());
    }
    lines.push(format!("⚑ {}", cause_text(&s.save_cause)));
    lines.push(format!("☁ {}", s.server_name.as_deref().unwrap_or("N/A")));
    // HuskSync's en-gb.yml misspells the status line's key (`data_manger_status`), so the line
    // (health, hunger, level, game mode) is never shown by the stock locale; neither is it here.
    lines.push(format!("⏏ {:.2}KiB", s.file_size(engine.platform.compress) as f64 / 1024.0));
    // Advancements and statistics, when the snapshot has both.
    let get = |k: &str| s.data.get(k).and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok());
    if let (Some(adv), Some(stats)) = (get(keys::ADVANCEMENTS), get(keys::STATISTICS)) {
        // HuskSync stores the list itself; an older build of this port wrote it under `completed`.
        let done = adv
            .get("completed")
            .unwrap_or(&adv)
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|x| !x.get("key").and_then(|k| k.as_str()).unwrap_or("").starts_with("minecraft:recipe"))
                    .count()
            })
            .unwrap_or(0);
        let generic = stats.get("generic");
        let ticks = ["play_time", "minecraft:play_time"]
            .iter()
            .find_map(|k| generic.and_then(|g| g.get(*k)).and_then(|v| v.as_i64()))
            .unwrap_or(0);
        lines.push(format!("⭐ Advancements: {done}   ⌛ Play Time: {:.2}ʜʀs", ticks as f64 / 20.0 / 60.0 / 60.0));
    }
    // Lines of `[label](hover) -> command` buttons, as HuskSync's `data_manager_*_buttons`.
    let rows: [(&str, Vec<(&str, &str, String)>); 3] = [
        (
            "View:",
            vec![
                ("[🪣 Inventory…]", "Click to view the inventory in this snapshot", format!("/inventory {name} {id}")),
                ("[⌀ Ender Chest…]", "Click to view the Ender Chest in this snapshot", format!("/enderchest {name} {id}")),
            ],
        ),
        (
            "Manage:",
            vec![
                ("[❌ Delete…]", "Click to delete this snapshot", format!("/userdata delete {name} {id}")),
                ("[⏪ Restore…]", "Click to restore the user to this snapshot", format!("/userdata restore {name} {id}")),
                ("[※ Pin/Unpin…]", "Click to pin or unpin this snapshot", format!("/userdata pin {name} {id}")),
            ],
        ),
        (
            "System:",
            vec![
                ("[⏷ File Dump…]", "Click to dump this snapshot to a file", format!("/userdata dump {name} {id} file")),
                ("[☂ Web Dump…]", "Click to dump this snapshot to the web", format!("/userdata dump {name} {id} web")),
            ],
        ),
    ];
    for l in lines {
        engine.tell(to, l);
    }
    for (head, buttons) in rows {
        let plain = format!(
            "{head} {}",
            buttons.iter().map(|(l, _, cmd)| format!("{l} ({cmd})")).collect::<Vec<_>>().join("   ")
        );
        engine.tell_component(to, plain, button_line(head, &buttons));
    }
}

/// A text component for a head word and buttons that run a command on click and show a hint on
/// hover (1.21.5+ component fields).
fn button_line(head: &str, buttons: &[(&str, &str, String)]) -> serde_json::Value {
    let mut extra = Vec::new();
    for (label, hover, command) in buttons {
        extra.push(serde_json::json!({ "text": " " }));
        extra.push(serde_json::json!({
            "text": label,
            "click_event": { "action": "run_command", "command": command },
            "hover_event": { "action": "show_text", "value": hover },
        }));
    }
    serde_json::json!({ "text": head, "extra": extra })
}


/// `DataSnapshotList` / `PaginatedList`: newest first, six to a page, circled numbers, a footer
/// with previous/next arrows and page jumpers.
fn list(engine: &Engine, to: Option<u128>, player: u128, name: &str, page: usize) {
    let all = match engine.db.all_snapshots(player) {
        Ok(v) => v,
        Err(e) => return engine.tell(to, format!("Error: {e}")),
    };
    if all.is_empty() {
        return engine.tell(to, "Error: Could not find any user data to display.");
    }
    const PER_PAGE: usize = 6;
    const ICONS: &str = "①②③④⑤⑥⑦⑧⑨⑩⑪⑫⑬⑭⑮⑯⑰⑱⑲⑳";
    let pages = all.len().div_ceil(PER_PAGE);
    let page = page.clamp(1, pages);
    let start = (page - 1) * PER_PAGE;
    let end = (start + PER_PAGE).min(all.len());
    engine.tell(to, format!("{name}'s user data snapshots: ({}-{} of {})", start + 1, end, all.len()));
    engine.tell(to, "");
    let compress = engine.platform.compress;
    for (i, stored) in all[start..end].iter().enumerate() {
        let n = start + i + 1;
        let icon = if n <= 20 { ICONS.chars().nth(n - 1).map(String::from).unwrap_or_default() } else { n.to_string() };
        let line = match &stored.snapshot {
            Ok(s) => format!(
                "{icon} {} {}   ⚑ {}   ⏏ {:.2}KiB",
                if s.pinned { "※" } else { "  " },
                time_medium(&s.timestamp),
                cause_text(&s.save_cause),
                s.file_size(compress) as f64 / 1024.0
            ),
            Err(e) => {
                log::warn!("horizonsync: unreadable snapshot {}: {e}", stored.version_uuid);
                format!("{icon}    {}", stored.version_uuid)
            }
        };
        engine.tell(to, line);
    }
    // `list_footer`: "\n%1%Page %2%/%3%%4%   %5%"
    let group = (page - 1) / 10;
    let first = group * 10 + 1;
    let last = (first + 9).min(pages);
    let jumpers = (first..=last).map(|p| p.to_string()).collect::<Vec<_>>().join(" ");
    engine.tell(to, "");
    engine.tell(
        to,
        format!(
            "{}Page {page}/{pages}{}   ({jumpers})",
            if page > 1 { "◀ " } else { "" },
            if page < pages { " ▶" } else { "" }
        ),
    );
}

fn items(engine: &Arc<Engine>, sender: &Sender, args: &[String], kind: Kind) {
    let to = sender.uuid;
    let Some(viewer) = sender.uuid else {
        return engine.tell(None, "Error: That command can only be used in-game.");
    };
    let label = match kind {
        Kind::Inventory => "inventory",
        Kind::EnderChest => "enderchest",
    };
    let Some(name) = args.first() else {
        return usage(engine, to, &format!("/{label} <username> [version_uuid]"));
    };
    let Some((player, name)) = find_user(engine, name) else {
        return engine.tell(to, "Error: Could not find a player by that name.");
    };
    let cause = match kind {
        Kind::Inventory => "INVENTORY_COMMAND",
        Kind::EnderChest => "ENDERCHEST_COMMAND",
    };
    if not_a_version(args.get(1)) {
        return engine.tell(to, "Error: Could not find any user data for that version UUID.");
    }
    let snapshot = match args.get(1).and_then(|v| util::parse_uuid(v)) {
        Some(v) => match engine.db.snapshot(player, v) {
            Ok(Some(s)) => s.snapshot,
            _ => return engine.tell(to, "Error: Could not find any user data for that version UUID."),
        },
        None => match engine
            .online_user_data(player, cause)
            .map(Ok)
            .or_else(|| engine.db.latest_snapshot(player).ok().flatten().map(|s| s.snapshot))
        {
            Some(s) => s,
            None => return engine.tell(to, "Error: Could not find any user data to display."),
        },
    };
    let s = match crate::db::usable(snapshot, &engine.platform) {
        Ok(s) => s,
        Err(e) => return invalid_data(engine, to, &e),
    };
    let fallback = util::data_version_of(&s.minecraft_version);
    let (title, rows, items) = match kind {
        Kind::Inventory => {
            let Some(inv) = s.data.get(keys::INVENTORY).and_then(|v| Inventory::deserialize(v, fallback).ok()) else {
                return engine.tell(to, "Error: Could not find any user data to display.");
            };
            (format!("{name}'s Inventory"), 5, inv.slots)
        }
        Kind::EnderChest => {
            let Some(e) = s.data.get(keys::ENDER_CHEST).and_then(|v| EnderChest::deserialize(v, fallback).ok()) else {
                return engine.tell(to, "Error: Could not find any user data to display.");
            };
            (format!("{name}'s Ender Chest"), 3, e.slots)
        }
    };
    let what = match kind {
        Kind::Inventory => "inventory",
        Kind::EnderChest => "Ender Chest",
    };
    engine.tell(to, format!("Viewing snapshot of {name}'s {what} as of ⌚ {}", time_medium(&s.timestamp)));
    // Only the player's latest data is editable, and only with the `.edit` permission.
    let edit = (sender.edit && args.get(1).is_none()).then(|| crate::engine::MenuEdit {
        viewer,
        target: player,
        target_name: name.clone(),
        ender_chest: matches!(kind, Kind::EnderChest),
    });
    engine.push(Action::OpenMenu { viewer, title, rows, items, edit });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_nodes_per_sub_command() {
        assert_eq!(permission_for("/userdata save Bob"), Some(("horizonsync.command.userdata.save".into(), OP)));
        assert_eq!(permission_for("openinv Bob"), Some(("horizonsync.command.inventory".into(), OP)));
        assert_eq!(permission_for("horizonsync about"), Some(("horizonsync.command.horizonsync.about".into(), 0)));
        assert_eq!(permission_for("horizonsync"), Some(("horizonsync.command.horizonsync".into(), 0)));
        assert_eq!(command_node("playerdata").as_deref(), Some("horizonsync.command.userdata"));
        assert_eq!(command_node("echest"), Some("horizonsync.command.enderchest".into()));
        assert_eq!(permission_for("lp info"), None);
        assert_eq!(permission_for("horizonsync dump confirm"), Some(("horizonsync.command.horizonsync.dump".into(), OP)));
    }

    #[test]
    fn edit_nodes_and_buttons() {
        assert_eq!(edit_permission_for("/openinv Bob").as_deref(), Some("horizonsync.command.inventory.edit"));
        assert_eq!(edit_permission_for("echest Bob").as_deref(), Some("horizonsync.command.enderchest.edit"));
        assert_eq!(edit_permission_for("userdata save Bob"), None);
        let line = button_line("View:", &[("[a]", "hint", "/inventory Bob 1".into())]);
        assert_eq!(line["extra"][1]["click_event"]["action"], "run_command");
        assert_eq!(line["extra"][1]["click_event"]["command"], "/inventory Bob 1");
    }

    #[test]
    fn times_and_causes_read_like_husksync() {
        assert_eq!(time_medium("2026-11-08T05:20:33.123+13:00"), "8 Nov 2026, 05:20");
        assert_eq!(cause_text("SAVE_COMMAND"), "save command");
        assert_eq!(cause_text("API"), "API");
        assert_eq!(cause_text("SOME_PLUGIN"), "Some Plugin");
    }

    #[test]
    fn dump_and_console_only_usages() {
        let all = usages(&[]);
        assert!(all.iter().any(|(u, l)| u == "horizonsync dump confirm" && *l == OP));
        assert!(all.iter().any(|(u, l)| u == "horizonsync migrate" && *l == 4));
        assert!(all.iter().any(|(u, _)| u == "userdata dump <username:player_name> <version:word> file"));
        assert!(!all.iter().any(|(u, _)| u.contains("[type:word]")));
    }

    #[test]
    fn usages_cover_husksync_commands_and_respect_disabled() {
        let all = usages(&[]);
        assert!(all.iter().any(|(u, l)| u == "horizonsync" && *l == 0));
        assert!(all.iter().any(|(u, l)| u.starts_with("userdata restore") && *l == OP));
        assert!(all.iter().any(|(u, _)| u.starts_with("openechest")));
        let some = usages(&["inventory".into()]);
        assert!(!some.iter().any(|(u, _)| u.starts_with("inventory ")));
    }
}
