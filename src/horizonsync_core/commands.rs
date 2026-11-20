//! HuskSync's commands: `/husksync`, `/userdata` (alias `/playerdata`), `/inventory`
//! (`/openinv`; `/invsee` is left to InvSee++) and `/enderchest` (`/echest`, `/openechest`).
//!
//! Permission nodes (`husksync.command.*`) are checked by the glue with the typed node query
//! (default level 2, as HuskSync's `Permission.Default.IF_OP`; `/husksync` and `/husksync about` are for
//! everyone, as `Permission.Default.TRUE`). Every text of a command comes from the locale file.

use crate::horizonsync_core::data::{EnderChest, Inventory};
use crate::horizonsync_core::engine::{Action, Engine, Sender};
use crate::horizonsync_core::player;
use crate::horizonsync_core::messages::Messages;
use crate::horizonsync_core::snapshot::{cause_display, keys, Packed};
use crate::horizonsync_core::util;
use serde_json::{json, Value};
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
    let husksync: &[(&str, u8)] = &[
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
        ("migrate help <migrator:word>", 4),
        ("migrate start <migrator:word>", 4),
        ("migrate set <migrator:word> <args:text>", 4),
    ];
    add("husksync", husksync);
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

/// The command group a label belongs to (`husksync.command.<group>`), aliases included.
fn group(label: &str) -> Option<&'static str> {
    match label {
        "husksync" => Some("husksync"),
        "userdata" | "playerdata" => Some("userdata"),
        "inventory" | "openinv" => Some("inventory"),
        "enderchest" | "echest" | "openechest" => Some("enderchest"),
        _ => None,
    }
}

/// The node a command is registered with: the first usage's node decides for the whole tree.
pub fn command_node(label: &str) -> Option<String> {
    group(&label.to_lowercase()).map(|g| format!("husksync.command.{g}"))
}

/// The node a command line needs and its default level (HuskSync's nodes: a sub-command has its own
/// node, `husksync.command.userdata.save`; the bare `/husksync` and `about` use the command's own node,
/// which is for everyone).
pub fn permission_for(line: &str) -> Option<(String, u8)> {
    let mut words = line.trim().trim_start_matches('/').split_whitespace();
    let group = group(&words.next()?.to_lowercase())?;
    let sub = words.next().map(|w| w.to_lowercase());
    let known: &[&str] = match group {
        "husksync" => &["status", "reload", "update", "dump"],
        "userdata" => &["view", "list", "delete", "save", "restore", "pin", "dump"],
        _ => &[],
    };
    let level = if group == "husksync" && !matches!(sub.as_deref(), Some(s) if known.contains(&s)) { 0 } else { OP };
    let node = match sub.as_deref() {
        Some(s) if known.contains(&s) => format!("husksync.command.{group}.{s}"),
        _ => format!("husksync.command.{group}"),
    };
    Some((node, level))
}

/// The node that lets a viewer edit the items of `/inventory` or `/enderchest` (None for any other
/// command): `husksync.command.inventory.edit`, `husksync.command.enderchest.edit`.
pub fn edit_permission_for(line: &str) -> Option<String> {
    let first = line.trim().trim_start_matches('/').split_whitespace().next()?.to_lowercase();
    let group = group(&first)?;
    matches!(group, "inventory" | "enderchest").then(|| format!("husksync.command.{group}.edit"))
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
        "husksync" => husksync(engine, &sender, &args),
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
        engine.say(sender.uuid, "error_no_permission", &[]);
        return false;
    }
    true
}

/// One text of the locale file as plain words (HuskSync's `getRawLocale` for a short word).
fn word(engine: &Engine, key: &str, args: &[&str]) -> String {
    engine.cfg().messages.lines(key, args).into_iter().map(|l| l.plain).collect::<Vec<_>>().join(" ")
}

/// A text that is not in the locale file (HuskSync builds these in code), as MiniMessage.
fn say_mini(engine: &Engine, to: Option<u128>, mini: &str) {
    engine.say_lines(to, Messages::render(mini));
}

fn husksync(engine: &Arc<Engine>, sender: &Sender, args: &[String]) {
    let to = sender.uuid;
    match args.first().map(|s| s.to_lowercase()).as_deref() {
        None | Some("about") => {
            for (plain, json) in about_lines() {
                engine.tell_component(to, plain, json);
            }
        }
        // Console only (HuskSync's `setCondition(isConsole)`).
        Some("forceupgrade" | "migrate") if to.is_some() => {
            engine.say(to, "error_console_command_only", &[]);
        }
        Some("forceupgrade") => {
            // `plugin.getLegacyConverter()` is empty without the v2 converter: HuskSync returns silently.
            log::info!("horizonsync: forceupgrade: the legacy (v2) converter is not part of the Horizon port");
        }
        Some("migrate") => match args.get(1).map(|s| s.to_lowercase()).as_deref() {
            None => {
                // No migrators (MPDB, legacy) are ported: the list HuskSync would print is empty.
                engine.tell(
                    None,
                    "Please choose a migrator, then run \"husksync migrate start <migrator>\"\n\
                     List of available migrators:\nMigrator ID / Migrator Name:",
                );
            }
            // The migrator argument fails to parse when no migrator has that name.
            Some("help" | "start" | "set") => {
                engine.tell(None, "Unknown or incomplete command: no migrators are available on Horizon.");
            }
            Some(_) => {
                engine.tell(None, "Unknown or incomplete command: use help, start or set.");
            }
        },
        // Denied: `op` has told the sender already.
        Some("status" | "reload" | "update" | "dump") if !op(engine, sender) => {}
        Some("status") => {
            let engine2 = engine.clone();
            engine.run_job(move |_| {
                engine2.say(to, "system_status_header", &[]);
                for (plain, json) in status_lines(&engine2) {
                    engine2.tell_component(to, plain, json);
                }
            });
        }
        Some("reload") => match reload(engine) {
            Ok(()) => engine.say(to, "reload_complete", &[]),
            Err(e) => {
                log::error!("horizonsync: failed to reload the plugin: {e}");
                say_mini(
                    engine,
                    to,
                    "<red>Error:</red> <gold>Failed to reload the plugin. Check console for errors.</gold>",
                );
            }
        },
        Some("update") => {
            // No update checker on Horizon (HuskSync asks Modrinth/Hangar): `up_to_date` would be a guess.
            say_mini(
                engine,
                to,
                "<bold><color:#00fb9a>HuskSync</color></bold> <color:#00fb9a>| Update checks are not part of the Horizon port.</color>",
            );
        }
        Some("dump") => match args.get(1).map(|s| s.to_lowercase()).as_deref() {
            None => engine.say(to, "system_dump_confirm", &[]),
            Some("confirm") => {
                engine.say(to, "system_dump_started", &[]);
                // The mc-logs upload is not ported: the status lines go to a local file in `dumps/`, nothing leaves the server.
                let engine2 = engine.clone();
                engine.run_job(move |_| {
                    let text: String = status_lines(&engine2).into_iter().map(|(plain, _)| plain + "\n").collect();
                    match write_system_dump(&engine2, &text) {
                        Ok(name) => {
                            engine2.say(to, "system_dump_ready", &[]);
                            let mut line = part(&name, Some("gray"), false, true, None, None);
                            line["click_event"] = json!({ "action": "copy_to_clipboard", "value": name });
                            engine2.tell_component(to, name.clone(), line);
                        }
                        Err(e) => {
                            log::error!("horizonsync: failed to write the system dump: {e}");
                            say_mini(
                                &engine2,
                                to,
                                "<color:#ff3300>Error:</color> <color:#ff7e5e>Failed to write the system dump. Check console for errors.</color>",
                            );
                        }
                    }
                });
            }
            Some(_) => usage(engine, to, "/husksync dump [confirm]"),
        },
        Some(_) => usage(engine, to, "/husksync <about|status|reload|update|dump>"),
    }
}

/// One text part for a component line: its words, colour (name or `#rrggbb`), bold and underline,
/// hover text and click.
fn part(text: &str, color: Option<&str>, bold: bool, underlined: bool, hover: Option<(&str, &str)>, click: Option<(&str, &str)>) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("text".into(), json!(text));
    if let Some(c) = color {
        m.insert("color".into(), json!(c));
    }
    if bold {
        m.insert("bold".into(), json!(true));
    }
    if underlined {
        m.insert("underlined".into(), json!(true));
    }
    if let Some((text, color)) = hover {
        m.insert("hover_event".into(), json!({ "action": "show_text", "value": { "text": text, "color": color } }));
    }
    if let Some((action, value)) = click {
        let field = if action == "open_url" { "url" } else { "command" };
        m.insert("click_event".into(), json!({ "action": action, field: value }));
    }
    Value::Object(m)
}

/// A line of parts: its plain words and the component.
fn line_of(parts: Vec<(String, Value)>) -> (String, Value) {
    let plain: String = parts.iter().map(|(t, _)| t.as_str()).collect();
    let extra: Vec<Value> = parts.into_iter().map(|(_, v)| v).collect();
    (plain, json!({ "text": "", "extra": extra }))
}

/// The `/husksync about` menu (`HuskSyncCommand.buildAboutMenu`), line by line.
fn about_lines() -> Vec<(String, Value)> {
    const PRIMARY: &str = "#00fb9a";
    const GRAY: &str = "#808080";
    let credit = |name: &str, what: &str| (name.to_owned(), part(name, Some("gray"), false, false, Some((what, GRAY)), None));
    let credits = |title: &str, list: Vec<(String, Value)>| {
        let mut parts = vec![(format!("{title}: "), part(&format!("{title}: "), Some(PRIMARY), true, false, None, None))];
        for (i, (name, v)) in list.into_iter().enumerate() {
            if i > 0 {
                parts.push((", ".into(), part(", ", None, false, false, None, None)));
            }
            parts.push((name, v));
        }
        line_of(parts)
    };
    let link = |text: &str, url: &str, color: &str| {
        (text.to_owned(), part(text, Some(color), false, true, Some(("Click to open link", GRAY)), Some(("open_url", url))))
    };
    let author = ("William278".to_owned(), part("William278", Some(PRIMARY), false, false, Some(("Click to visit website", GRAY)), Some(("open_url", "https://william278.net"))));
    let version = format!(" v{}", env!("CARGO_PKG_VERSION"));
    let blank = || (String::new(), json!({ "text": "" }));
    vec![
        line_of(vec![
            ("HuskSync".into(), part("HuskSync", Some(PRIMARY), true, false, None, None)),
            (version.clone(), part(&version, Some("gray"), false, false, None, None)),
        ]),
        line_of(vec![(
            "A modern, cross-server player data synchronization system".into(),
            part("A modern, cross-server player data synchronization system", Some(GRAY), false, false, None, None),
        )]),
        blank(),
        credits("Author", vec![author]),
        credits("Maintainers", vec![credit("DaUltraMarine", "Code")]),
        credits(
            "Contributors",
            vec![
                credit("CatTeaA", "Code"),
                credit("HarvelsX", "Code"),
                credit("HookWoods", "Code"),
                credit("mfnalex", "Code"),
                credit("MrPippi", "Code"),
                credit("Preva1l", "Code"),
                credit("quiquelhappy", "Code"),
                credit("hanbings", "Code (Fabric porting)"),
                credit("Stampede2011", "Code (Fabric mixins)"),
                credit("VinerDream", "Code"),
                credit("WaterArchery", "Code"),
            ],
        ),
        credits(
            "Translators",
            vec![
                credit("Namiu", "Japanese (ja-jp)"),
                credit("anchelthe", "Spanish (es-es)"),
                credit("Melonzio", "Spanish (es-es)"),
                credit("Ceddix", "German (de-de)"),
                credit("Pukejoy_1", "Bulgarian (bg-bg)"),
                credit("mateusneresrb", "Brazilian Portuguese (pt-br)"),
                credit("小蔡", "Traditional Chinese (zh-tw)"),
                credit("Ghost-chu", "Simplified Chinese (zh-cn)"),
                credit("DJelly4K", "Simplified Chinese (zh-cn)"),
                credit("Thourgard", "Ukrainian (uk-ua)"),
                credit("xF3d3", "Italian (it-it)"),
                credit("cada3141", "Korean (ko-kr)"),
                credit("Wirayuda5620", "Indonesian (id-id)"),
                credit("WinTone01", "Turkish (tr-tr)"),
                credit("IbanEtchep", "French (fr-fr)"),
                credit("Ris", "Hungarian (hu-hu)"),
                credit("Antonioo", "Polish (pl-pl)"),
            ],
        ),
        blank(),
        line_of(vec![
            link("⛏ Documentation", "https://william278.net/docs/husksync", "gray"),
            ("  ".into(), part("  ", Some("gray"), false, false, None, None)),
            link("❌ Issues", "https://github.com/WiIIiam278/HuskSync/issues", "#ff9f0f"),
            ("  ".into(), part("  ", Some("gray"), false, false, None, None)),
            link("⭐ Discord", "https://discord.gg/tVYhJfyDWG", "#6773f5"),
        ]),
    ]
}

/// `StatusLine`: "• Name: value", names as `WordUtils.capitalizeFully(name().replace('_', ' '))`, in
/// `StatusLine`'s colours (grey names, green yes and enabled, red no and disabled).
fn status_lines(engine: &Engine) -> Vec<(String, Value)> {
    const GREEN: &str = "green";
    const RED: &str = "red";
    let cfg = engine.cfg();
    let yes = |b: bool| vec![(if b { "Yes" } else { "No" }.to_owned(), Some(if b { GREEN } else { RED }))];
    let local = |h: &str| yes(matches!(h, "127.0.0.1" | "0.0.0.0" | "localhost" | "::1"));
    let plain = |s: String| vec![(s, None)];
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
    let mut types: Vec<(String, Option<&str>)> = Vec::new();
    for k in [
        "inventory", "ender_chest", "potion_effects", "advancements", "location", "statistics", "health",
        "hunger", "attributes", "experience", "game_mode", "flight_status", "persistent_data",
    ] {
        let on = cfg.synchronization.features.enabled(&format!("husksync:{k}"));
        if !types.is_empty() {
            types.push((", ".into(), None));
        }
        types.push((format!("husksync:{k} {}", if on { '✔' } else { '❌' }), Some(if on { GREEN } else { RED })));
    }
    let rows: Vec<(&str, Vec<(String, Option<&str>)>)> = vec![
        ("Plugin Version", plain(format!("v{}", env!("CARGO_PKG_VERSION")))),
        ("Server Version", plain("Horizon".into())),
        ("Language", plain(cfg.language.clone())),
        ("Minecraft Version", plain(engine.platform.minecraft_version.clone())),
        ("Java Version", plain("N/A (native server)".into())),
        ("Java Vendor", plain("N/A (native server)".into())),
        ("Server Name", plain(engine.platform.server_name.clone())),
        ("Cluster Id", plain(if cfg.cluster_id.is_empty() { "None".into() } else { cfg.cluster_id.clone() })),
        ("Sync Mode", plain(mode)),
        ("Delay Latency", plain(format!("{}ms", cfg.synchronization.network_latency_milliseconds))),
        ("Database Type", plain(kind)),
        ("Is Database Local", local(&cfg.database.host)),
        ("Redis Version", plain(engine.redis.version())),
        ("Using Redis Sentinel", yes(!cfg.redis.sentinel.master.trim().is_empty())),
        ("Redis Database", plain(cfg.redis.database.to_string())),
        ("Using Redis User", yes(!cfg.redis.user.is_empty())),
        ("Using Redis Password", yes(!cfg.redis.password.is_empty())),
        ("Redis Using Ssl", yes(cfg.redis.use_ssl)),
        ("Redis Latency", plain(ms(engine.redis.ping()))),
        ("Is Redis Local", local(&cfg.redis.host)),
        ("Locked User Handler", plain("HorizonEventListener".into())),
        ("Data Types", types),
    ];
    rows.into_iter()
        .map(|(name, value)| {
            let mut parts = vec![
                ("• ".to_owned(), part("• ", None, false, false, None, None)),
                (name.to_owned(), part(name, Some("#848484"), false, false, None, None)),
                (":".to_owned(), part(":", None, false, false, None, None)),
                (" ".to_owned(), part(" ", Some("white"), false, false, None, None)),
            ];
            for (text, color) in value {
                parts.push((text.clone(), part(&text, color, false, false, None, None)));
            }
            line_of(parts)
        })
        .collect()
}

/// `/husksync reload`: read the config file again. As in HuskSync, connections and the like keep
/// their start-up values until a restart.
fn reload(engine: &Engine) -> Result<(), String> {
    // `engine.dir` is `<server>/config/horizonsync`: the server folder is two levels up.
    let server_dir = engine.dir.parent().and_then(|p| p.parent()).unwrap_or(&engine.dir);
    let mut fresh = crate::horizonsync_core::config::Config::load(server_dir, &engine.dir)?;
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

/// `error_invalid_syntax`: the usage is a click-to-suggest.
fn usage(engine: &Engine, to: Option<u128>, usage: &str) {
    engine.say(to, "error_invalid_syntax", &[usage]);
}

/// A version argument that is present but not a UUID: the lookup fails, it does not fall back to the latest snapshot.
fn not_a_version(arg: Option<&String>) -> bool {
    arg.is_some_and(|v| util::parse_uuid(v).is_none())
}

/// `DateTimeFormatter.ofLocalizedDateTime(MEDIUM, SHORT)` in the en-GB locale: `8 Nov 2026, 05:20`.
fn time_medium(ts: &str) -> String {
    util::parse_timestamp(ts).map(|t| t.format("%-d %b %Y, %H:%M").to_string()).unwrap_or_else(|| ts.to_owned())
}

/// `DateTimeFormatter.ofLocalizedDateTime(LONG, MEDIUM)` in the en-GB locale: `8 November 2026 at 05:20:33`.
fn time_long(ts: &str) -> String {
    util::parse_timestamp(ts).map(|t| t.format("%-d %B %Y at %H:%M:%S").to_string()).unwrap_or_else(|| ts.to_owned())
}

/// `SaveCause.getLocale`: the `save_cause_*` text of the locale file, the display name for other causes.
fn cause_text(messages: &Messages, cause: &str) -> String {
    match messages.raw(&format!("save_cause_{}", cause.to_lowercase()), &[]) {
        Some(text) => text,
        None => cause_display(cause),
    }
}

/// `error_invalid_data`: the details are a hover.
fn invalid_data(engine: &Engine, to: Option<u128>, reason: &str) {
    log::warn!("horizonsync: invalid snapshot: {reason}");
    engine.say(to, "error_invalid_data", &[reason]);
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
        engine.say(to, "error_invalid_player", &[]);
        return;
    };
    let version = args.get(2).and_then(|v| util::parse_uuid(v));
    let need_version = |what: &str| -> Option<u128> {
        if version.is_none() {
            usage(engine, to, &format!("/userdata {what} <username> <version_uuid>"));
        }
        version
    };
    let player_id = util::uuid_string(player);
    match sub.as_str() {
        "view" => {
            if not_a_version(args.get(2)) {
                return engine.say(to, "error_invalid_version_uuid", &[]);
            }
            let stored = match version {
                Some(v) => engine.db.snapshot(player, v),
                None => engine.db.latest_snapshot(player),
            };
            match stored {
                Ok(Some(s)) => match crate::horizonsync_core::db::usable(s.snapshot, &engine.platform) {
                    Ok(s) => overview(engine, sender, &s, &name, player),
                    Err(e) => invalid_data(engine, to, &e),
                },
                Ok(None) if version.is_some() => engine.say(to, "error_invalid_version_uuid", &[]),
                Ok(None) => engine.say(to, "error_no_data_to_display", &[]),
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
                    let version_id = util::uuid_string(v);
                    let short = version_id.split('-').next().unwrap_or("").to_owned();
                    engine.say(to, "data_deleted", &[&short, &version_id, &name, &player_id]);
                }
                Ok(false) => engine.say(to, "error_invalid_version_uuid", &[]),
                Err(e) => engine.tell(to, format!("Error: {e}")),
            }
        }
        "save" => {
            if engine.online_name(player).is_none() {
                engine.say(to, "error_invalid_player", &[]);
                return;
            }
            engine.save_command(player, &name, to);
        }
        "restore" => {
            let Some(v) = need_version("restore") else { return };
            let stored = match engine.db.snapshot(player, v) {
                Ok(Some(s)) => s,
                Ok(None) => return engine.say(to, "error_invalid_version_uuid", &[]),
                Err(e) => return engine.tell(to, format!("Error: {e}")),
            };
            let mut s = match crate::horizonsync_core::db::usable(stored.snapshot, &engine.platform) {
                Ok(s) => s.copy(),
                Err(e) => return invalid_data(engine, to, &e),
            };
            // Restored players get at least one health point.
            player::set_health_at_least_one(&mut s.data);
            s.save_cause = "BACKUP_RESTORE".into();
            s.pinned = engine.cfg().synchronization.auto_pin("BACKUP_RESTORE");
            engine.save_and_update(player, &s);
            engine.say(to, "data_restored", &[&name, &player_id, s.short_id(), &s.id]);
        }
        "pin" => {
            let Some(v) = need_version("pin") else { return };
            match engine.db.snapshot(player, v) {
                Ok(Some(stored)) => match stored.snapshot {
                    Ok(s) => {
                        let pin = !s.pinned;
                        match engine.db.set_pinned(player, v, pin) {
                            Ok(_) => engine.say(
                                to,
                                if pin { "data_pinned" } else { "data_unpinned" },
                                &[s.short_id(), &s.id, &name, &player_id],
                            ),
                            Err(e) => engine.tell(to, format!("Error: {e}")),
                        }
                    }
                    Err(e) => engine.tell(to, format!("Error: {e}")),
                },
                Ok(None) => engine.say(to, "error_invalid_version_uuid", &[]),
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
                        return engine.say(to, "error_invalid_version_uuid", &[]);
                    };
                    match engine.db.snapshot(player, v).ok().flatten().and_then(|s| s.snapshot.ok()) {
                        Some(s) => s,
                        None => return engine.say(to, "error_invalid_version_uuid", &[]),
                    }
                }
                None => {
                    let found = engine
                        .online_user_data(player, "DUMP_COMMAND")
                        .or_else(|| engine.db.latest_snapshot(player).ok().flatten().and_then(|s| s.snapshot.ok()));
                    match found {
                        Some(s) => s,
                        None => return engine.say(to, "error_no_data_to_display", &[]),
                    }
                }
            };
            if web {
                engine.tell(to, "Web dumps are not available on Horizon; writing a file instead.");
            }
            match dump_to_file(engine, &s, &name) {
                Ok(path) => {
                    engine.say(to, "data_dumped", &[s.short_id(), &name]);
                    // The file name, underlined, copied on a click (`ClickEvent.copyToClipboard`).
                    let mut line = part(&path, Some("gray"), false, true, None, None);
                    line["click_event"] = json!({ "action": "copy_to_clipboard", "value": path });
                    engine.tell_component(to, path.clone(), line);
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

/// `/husksync dump confirm`: the status lines as a text file in `dumps/` (HuskSync uploads them to mc-logs).
/// The status lines carry no secrets (passwords and users only as Yes/No).
fn write_system_dump(engine: &Engine, text: &str) -> Result<String, String> {
    let dir = engine.dir.join("dumps");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = util::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let file = dir.join(format!("system_{stamp}.txt"));
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    Ok(file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/// The keys of the advancements a snapshot has completed, recipes left out
/// (`Advancements.getCompletedExcludingRecipes`).
fn completed_advancements(adv: &Value) -> Vec<String> {
    // HuskSync stores the list itself; an older build of this port wrote it under `completed`.
    adv.get("completed")
        .unwrap_or(adv)
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.get("key").and_then(|k| k.as_str()))
                .filter(|k| !k.starts_with("minecraft:recipe"))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// `DataSnapshotOverview.generateAdvancementPreview`: up to nine keys, then "and n more…".
fn advancement_preview(messages: &Messages, completed: &[String]) -> String {
    const PREVIEW_SIZE: usize = 8;
    let mut lines: Vec<String> = Vec::new();
    for (i, key) in completed.iter().enumerate() {
        lines.push(key.clone());
        if i >= PREVIEW_SIZE {
            break;
        }
    }
    if completed.len() > PREVIEW_SIZE {
        let remaining = (completed.len() - PREVIEW_SIZE).to_string();
        lines.push(
            messages
                .raw("data_manager_advancements_preview_remaining", &[&remaining])
                .unwrap_or_else(|| format!("+{remaining}…")),
        );
    }
    lines.join("\n")
}

fn overview(engine: &Engine, sender: &Sender, s: &Packed, name: &str, player: u128) {
    let to = sender.uuid;
    let id = &s.id;
    let player_id = util::uuid_string(player);
    engine.say(to, "data_manager_title", &[s.short_id(), id, name, &player_id]);
    engine.say(to, "data_manager_timestamp", &[&time_medium(&s.timestamp), &s.timestamp]);
    if s.pinned {
        engine.say(to, "data_manager_pinned", &[]);
    }
    let cause = cause_text(&engine.cfg().messages, &s.save_cause);
    engine.say(to, "data_manager_cause", &[&cause]);
    engine.say(to, "data_manager_server", &[s.server_name.as_deref().unwrap_or("N/A")]);
    // `data_manager_status` (health, hunger, level, game mode): HuskSync's en-gb.yml misspells its key
    // (`data_manger_status`), so the stock locale never shows the line; neither does this port.
    engine.say(to, "data_manager_size", &[&format!("{:.2}KiB", s.file_size(engine.platform.compress) as f64 / 1024.0)]);
    // Advancements and statistics, when the snapshot has both.
    let get = |k: &str| s.data.get(k).and_then(|v| serde_json::from_str::<Value>(v).ok());
    if let (Some(adv), Some(stats)) = (get(keys::ADVANCEMENTS), get(keys::STATISTICS)) {
        let completed = completed_advancements(&adv);
        let generic = stats.get("generic");
        let ticks = ["play_time", "minecraft:play_time"]
            .iter()
            .find_map(|k| generic.and_then(|g| g.get(*k)).and_then(|v| v.as_i64()))
            .unwrap_or(0);
        let preview = advancement_preview(&engine.cfg().messages, &completed);
        engine.say(
            to,
            "data_manager_advancements_statistics",
            &[&completed.len().to_string(), &preview, &format!("{:.2}", ticks as f64 / 20.0 / 60.0 / 60.0)],
        );
    }
    if sender.edit_all {
        engine.say(to, "data_manager_item_buttons", &[name, id]);
    }
    engine.say(to, "data_manager_management_buttons", &[name, id]);
    if sender.dump {
        engine.say(to, "data_manager_system_buttons", &[name, id]);
    }
}

/// `DataSnapshotList` / `PaginatedList`: newest first, six to a page, circled numbers, a footer
/// with previous/next arrows and page jumpers, all from the locale file.
fn list(engine: &Engine, to: Option<u128>, player: u128, name: &str, page: usize) {
    let all = match engine.db.all_snapshots(player) {
        Ok(v) => v,
        Err(e) => return engine.tell(to, format!("Error: {e}")),
    };
    if all.is_empty() {
        return engine.say(to, "error_no_data_to_display", &[]);
    }
    const PER_PAGE: usize = 6;
    const ICONS: &str = "①②③④⑤⑥⑦⑧⑨⑩⑪⑫⑬⑭⑮⑯⑰⑱⑲⑳";
    let pages = all.len().div_ceil(PER_PAGE);
    let page = page.clamp(1, pages);
    let start = (page - 1) * PER_PAGE;
    let end = (start + PER_PAGE).min(all.len());
    let compress = engine.platform.compress;
    let command = format!("/userdata list {name}");
    let m = engine.cfg().messages.clone();
    let mut out: Vec<crate::horizonsync_core::messages::Line> = Vec::new();
    // `data_list_title`: "%1%'s user data snapshots: (%2%-%3% of %4%)\n".
    out.extend(m.lines("data_list_title", &[name, &(start + 1).to_string(), &end.to_string(), &all.len().to_string()]));
    for (i, stored) in all[start..end].iter().enumerate() {
        let n = start + i + 1;
        let icon = if n <= 20 { ICONS.chars().nth(n - 1).map(String::from).unwrap_or_default() } else { n.to_string() };
        let version = util::uuid_string(util::parse_uuid(&stored.version_uuid).unwrap_or(0));
        let raw = match &stored.snapshot {
            Ok(s) => m.raw(
                "data_list_item",
                &[
                    &icon,
                    name,
                    &s.id,
                    s.short_id(),
                    if s.pinned { "※" } else { "  " },
                    &time_medium(&s.timestamp),
                    &time_long(&s.timestamp),
                    &cause_text(&m, &s.save_cause),
                    &format!("{:.2}KiB", s.file_size(compress) as f64 / 1024.0),
                    "",
                ],
            ),
            Err(e) => {
                log::warn!("horizonsync: unreadable snapshot {}: {e}", stored.version_uuid);
                let short = version.get(..8).unwrap_or(&version).to_owned();
                m.raw(
                    "data_list_item_invalid",
                    &[&icon, name, &version, &short, "  ", "?", "?", "?", "?", e],
                )
            }
        };
        match raw {
            Some(raw) => out.extend(Messages::render(&raw)),
            None => out.extend(Messages::render(&format!("• {version}"))),
        }
    }
    // `list_footer`: "\n%1%Page %2%/%3%%4%   %5%".
    let previous = if page > 1 {
        m.raw("list_previous_page_button", &[&(page - 1).to_string(), &format!("{command} {}", page - 1)]).unwrap_or_default()
    } else {
        String::new()
    };
    let next = if page < pages {
        m.raw("list_next_page_button", &[&(page + 1).to_string(), &format!("{command} {}", page + 1)]).unwrap_or_default()
    } else {
        String::new()
    };
    let group = (page - 1) / 10;
    let first = group * 10 + 1;
    let last = (first + 9).min(pages);
    let buttons: Vec<String> = (first..=last)
        .map(|p| {
            if p == page {
                m.raw("list_page_jumper_current_page", &[&p.to_string()]).unwrap_or_default()
            } else {
                m.raw("list_page_jumper_button", &[&p.to_string(), &format!("{command} {p}")]).unwrap_or_default()
            }
        })
        .collect();
    let separator = m.raw("list_page_jumper_separator", &[]).unwrap_or_else(|| " ".into());
    let jumpers = m.raw("list_page_jumpers", &[&buttons.join(&separator)]).unwrap_or_default();
    if let Some(footer) = m.raw("list_footer", &[&previous, &page.to_string(), &pages.to_string(), &next, &jumpers]) {
        out.extend(Messages::render(&footer));
    }
    engine.say_lines(to, out);
}

fn items(engine: &Arc<Engine>, sender: &Sender, args: &[String], kind: Kind) {
    let to = sender.uuid;
    let Some(viewer) = sender.uuid else {
        return engine.say(None, "error_in_game_command_only", &[]);
    };
    let label = match kind {
        Kind::Inventory => "inventory",
        Kind::EnderChest => "enderchest",
    };
    let Some(name) = args.first() else {
        return usage(engine, to, &format!("/{label} <username> [version_uuid]"));
    };
    let Some((player, name)) = find_user(engine, name) else {
        return engine.say(to, "error_invalid_player", &[]);
    };
    let cause = match kind {
        Kind::Inventory => "INVENTORY_COMMAND",
        Kind::EnderChest => "ENDERCHEST_COMMAND",
    };
    if not_a_version(args.get(1)) {
        return engine.say(to, "error_invalid_version_uuid", &[]);
    }
    let snapshot = match args.get(1).and_then(|v| util::parse_uuid(v)) {
        Some(v) => match engine.db.snapshot(player, v) {
            Ok(Some(s)) => s.snapshot,
            _ => return engine.say(to, "error_invalid_version_uuid", &[]),
        },
        None => match engine
            .online_user_data(player, cause)
            .map(Ok)
            .or_else(|| engine.db.latest_snapshot(player).ok().flatten().map(|s| s.snapshot))
        {
            Some(s) => s,
            None => return engine.say(to, "error_no_data_to_display", &[]),
        },
    };
    let s = match crate::horizonsync_core::db::usable(snapshot, &engine.platform) {
        Ok(s) => s,
        Err(e) => return invalid_data(engine, to, &e),
    };
    let fallback = util::data_version_of(&s.minecraft_version);
    let (title_key, opened_key, rows, items) = match kind {
        Kind::Inventory => {
            let Some(inv) = s.data.get(keys::INVENTORY).and_then(|v| Inventory::deserialize(v, fallback).ok()) else {
                return engine.say(to, "error_no_data_to_display", &[]);
            };
            ("inventory_viewer_menu_title", "inventory_viewer_opened", 5, inv.slots)
        }
        Kind::EnderChest => {
            let Some(e) = s.data.get(keys::ENDER_CHEST).and_then(|v| EnderChest::deserialize(v, fallback).ok()) else {
                return engine.say(to, "error_no_data_to_display", &[]);
            };
            ("ender_chest_viewer_menu_title", "ender_chest_viewer_opened", 3, e.slots)
        }
    };
    let title = word(engine, title_key, &[&name]);
    engine.say(to, opened_key, &[&name, &time_medium(&s.timestamp)]);
    // Only the player's latest data is editable, and only with the `.edit` permission.
    let edit = (sender.edit && args.get(1).is_none()).then(|| crate::horizonsync_core::engine::MenuEdit {
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
        assert_eq!(permission_for("/userdata save Bob"), Some(("husksync.command.userdata.save".into(), OP)));
        assert_eq!(permission_for("openinv Bob"), Some(("husksync.command.inventory".into(), OP)));
        // `about` and the bare command use the command's own node, which is for everyone.
        assert_eq!(permission_for("husksync about"), Some(("husksync.command.husksync".into(), 0)));
        assert_eq!(permission_for("husksync"), Some(("husksync.command.husksync".into(), 0)));
        assert_eq!(command_node("playerdata").as_deref(), Some("husksync.command.userdata"));
        assert_eq!(command_node("echest"), Some("husksync.command.enderchest".into()));
        assert_eq!(permission_for("lp info"), None);
        assert_eq!(permission_for("horizonsync status"), None);
        assert_eq!(permission_for("hsync status"), None);
        assert_eq!(permission_for("husksync dump confirm"), Some(("husksync.command.husksync.dump".into(), OP)));
        assert_eq!(permission_for("husksync status"), Some(("husksync.command.husksync.status".into(), OP)));
    }

    #[test]
    fn edit_nodes_and_about_menu() {
        assert_eq!(edit_permission_for("/openinv Bob").as_deref(), Some("husksync.command.inventory.edit"));
        assert_eq!(edit_permission_for("echest Bob").as_deref(), Some("husksync.command.enderchest.edit"));
        assert_eq!(edit_permission_for("userdata save Bob"), None);
        let about = about_lines();
        assert_eq!(about[0].0, format!("HuskSync v{}", env!("CARGO_PKG_VERSION")));
        assert_eq!(about[3].0, "Author: William278");
        assert_eq!(about[3].1["extra"][1]["click_event"]["action"], "open_url");
        assert_eq!(about[3].1["extra"][1]["click_event"]["url"], "https://william278.net");
    }

    #[test]
    fn times_and_causes_read_like_husksync() {
        assert_eq!(time_medium("2026-11-08T05:20:33.123+13:00"), "8 Nov 2026, 05:20");
        assert_eq!(time_long("2026-11-08T05:20:33.123+13:00"), "8 November 2026 at 05:20:33");
        let m = Messages::default();
        assert_eq!(cause_text(&m, "SAVE_COMMAND"), "save command");
        assert_eq!(cause_text(&m, "API"), "API");
        assert_eq!(cause_text(&m, "DEATH"), "death");
        assert_eq!(cause_text(&m, "SOME_PLUGIN"), "Some Plugin");
    }

    #[test]
    fn advancement_preview_lists_nine_keys_and_the_rest() {
        let m = Messages::default();
        let done: Vec<String> = (1..=12).map(|i| format!("minecraft:adv{i}")).collect();
        let preview = advancement_preview(&m, &done);
        let lines: Vec<&str> = preview.split('\n').collect();
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0], "minecraft:adv1");
        assert_eq!(lines[8], "minecraft:adv9");
        assert_eq!(lines[9], "and 4 more…");
        assert_eq!(advancement_preview(&m, &done[..2]), "minecraft:adv1\nminecraft:adv2");
        let adv = serde_json::json!([{"key": "minecraft:recipes/a"}, {"key": "minecraft:story/root"}]);
        assert_eq!(completed_advancements(&adv), vec!["minecraft:story/root".to_owned()]);
    }

    #[test]
    fn dump_and_console_only_usages() {
        let all = usages(&[]);
        assert!(all.iter().any(|(u, l)| u == "husksync dump confirm" && *l == OP));
        assert!(all.iter().any(|(u, l)| u == "husksync migrate" && *l == 4));
        assert!(all.iter().any(|(u, l)| u == "husksync migrate start <migrator:word>" && *l == 4));
        assert!(all.iter().any(|(u, _)| u == "userdata dump <username:player_name> <version:word> file"));
        assert!(!all.iter().any(|(u, _)| u.contains("[type:word]")));
    }

    #[test]
    fn usages_cover_husksync_commands_and_respect_disabled() {
        let all = usages(&[]);
        assert!(all.iter().any(|(u, l)| u == "husksync" && *l == 0));
        assert!(!all.iter().any(|(u, _)| u.starts_with("horizonsync") || u.starts_with("hsync")));
        assert!(all.iter().any(|(u, l)| u.starts_with("userdata restore") && *l == OP));
        assert!(all.iter().any(|(u, _)| u.starts_with("openechest")));
        let some = usages(&["inventory".into()]);
        assert!(!some.iter().any(|(u, _)| u.starts_with("inventory ")));
    }
}
