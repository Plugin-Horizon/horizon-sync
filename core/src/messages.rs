//! HuskSync's `plugins/HuskSync/messages-<language>.yml`: MiniMessage templates under `locales`,
//! turned into chat components (1.21.5+ field names, as `commands.rs` builds them).
//!
//! Supported MiniMessage: `<color:#rrggbb>` / `<color:name>` and the named colours (`<gray>`,
//! `<dark_gray>`...), `</color>` and every `</name>` closing tag, `<bold>` `<italic>` `<underlined>`
//! `<strikethrough>` `<obfuscated>` (and the short forms `<b>` `<i>` `<em>` `<u>` `<st>` `<obf>`),
//! `<gradient:#rrggbb:#rrggbb...>` (per character, hex stops only), `<hover:show_text:'...'>` (the
//! hover text is itself MiniMessage), `<click:run_command|suggest_command|open_url|copy_to_clipboard:'...'>`,
//! backslash escapes (`\<`, `\'`, `\\`), `%1%`..`%9%` placeholders, and a literal `\n` (two characters
//! in the file) as a line break: every line is one chat line.
//!
//! Not supported (the tag is dropped, its text kept): `<reset>`, `<rainbow>`, `<transition>`,
//! `<shadow>`, `<font>`, `<insertion>`, `<keybind>`, `<translate>`, `<newline>`, hover actions other
//! than `show_text`, and click actions other than the four above. Legacy `&` codes are plain text.
//! A real newline inside a block scalar (`|-`) becomes a space, as YAML folds it.

use crate::yaml;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// The built-in English texts (`messages-en-gb.yml`'s values), used when the file is missing or has
/// no such key. Only the keys this port sends are listed.
pub const DEFAULTS: &[(&str, &str)] = &[
    ("synchronization_complete", "<color:#00fb9a>⏵ Data synchronized!</color>"),
    ("synchronization_failed", "<color:#ff7e5e>⏵ Failed to synchronize your data! Please contact an administrator.</color>"),
    ("data_update_complete", "<color:#00fb9a>🔔 Your data has been updated!</color>"),
    ("data_update_failed", "<color:#ff7e5e>🔔 Failed to update your data! Please contact an administrator.</color>"),
    ("user_registration_complete", "<color:#00fb9a>⭐ User registration complete!</color>"),
];

/// The keys of `DEFAULTS`: the messages this port reads from the file (`check_original` reports the
/// others as not read).
pub fn used_keys() -> impl Iterator<Item = &'static str> {
    DEFAULTS.iter().map(|(k, _)| *k)
}

/// The locale file's texts.
#[derive(Clone, Debug, Default)]
pub struct Messages {
    locales: BTreeMap<String, String>,
}

/// One chat line: its plain words (logs, console) and the component.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub plain: String,
    pub component: Value,
}

impl Messages {
    /// `plugins/HuskSync/messages-<language>.yml`; the built-in texts when it is missing or unreadable.
    pub fn load(plugin_dir: &Path, language: &str) -> Messages {
        let file = plugin_dir.join(format!("messages-{language}.yml"));
        match std::fs::read_to_string(&file) {
            Ok(text) => match Messages::from_yaml(&text) {
                Ok(m) => m,
                Err(e) => {
                    log::error!("horizonsync: {}: {e}; built-in messages used", file.display());
                    Messages::default()
                }
            },
            Err(_) => Messages::default(),
        }
    }

    /// The `locales` map of a locale file.
    pub fn from_yaml(text: &str) -> Result<Messages, String> {
        let root = yaml::parse(text).map_err(|e| e.to_string())?;
        let mut locales = BTreeMap::new();
        if let Some(map) = root.get("locales") {
            for entry in map.entries() {
                if let Some(v) = entry.value.as_str() {
                    locales.insert(entry.key.clone(), v.to_owned());
                }
            }
        }
        Ok(Messages { locales })
    }

    /// The MiniMessage template for `key`: the file's, else the built-in one.
    pub fn template(&self, key: &str) -> Option<&str> {
        match self.locales.get(key) {
            Some(v) => Some(v.as_str()),
            None => DEFAULTS.iter().find(|(k, _)| *k == key).map(|(_, v)| *v),
        }
    }

    /// The chat lines of `key` with `%1%`, `%2%`... replaced by `args`. Empty for an unknown key.
    pub fn lines(&self, key: &str, args: &[&str]) -> Vec<Line> {
        let Some(template) = self.template(key) else { return Vec::new() };
        let mut out = Vec::new();
        for piece in template.split("\\n") {
            let filled = fill(piece, args);
            let segs = parse(&filled.replace(['\n', '\r'], " "));
            let plain: String = segs.iter().map(|s| s.text.as_str()).collect();
            if plain.is_empty() {
                continue;
            }
            out.push(Line { plain, component: component(&segs) });
        }
        out
    }
}

fn fill(template: &str, args: &[&str]) -> String {
    let mut out = template.to_owned();
    for (i, arg) in args.iter().enumerate().take(9) {
        out = out.replace(&format!("%{}%", i + 1), arg);
    }
    out
}

// ----- MiniMessage -----

type Rgb = (u8, u8, u8);

#[derive(Clone, Debug, Default, PartialEq)]
struct Style {
    color: Option<String>,
    bold: bool,
    italic: bool,
    underlined: bool,
    strikethrough: bool,
    obfuscated: bool,
    /// (action, value): `run_command`, `suggest_command`, `open_url` or `copy_to_clipboard`.
    click: Option<(String, String)>,
    /// The hover's MiniMessage source.
    hover: Option<String>,
    /// Index into the gradient list while inside `<gradient>`.
    gradient: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
struct Seg {
    text: String,
    style: Style,
}

const NAMED_COLORS: &[&str] = &[
    "black", "dark_blue", "dark_green", "dark_aqua", "dark_red", "dark_purple", "gold", "gray", "dark_gray",
    "blue", "green", "aqua", "red", "light_purple", "yellow", "white",
];

/// The tag's name and its arguments; `:` separates them, quotes keep a `:` inside an argument.
fn split_tag(content: &str) -> (String, Vec<String>) {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                cur.push(c);
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            '\'' => {
                quoted = !quoted;
                cur.push(c);
            }
            ':' if !quoted => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    let name = parts.remove(0).trim().to_lowercase();
    let args = parts.into_iter().map(|a| unquote(&a)).collect();
    (name, args)
}

fn unquote(arg: &str) -> String {
    let t = arg.trim();
    let inner = if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') { &t[1..t.len() - 1] } else { t };
    inner.replace("\\'", "'")
}

/// Finds the `>` that ends the tag starting at `start` (the `<`), skipping quoted text.
fn tag_end(chars: &[char], start: usize) -> Option<usize> {
    let mut quoted = false;
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '\'' => quoted = !quoted,
            '>' if !quoted => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse(src: &str) -> Vec<Seg> {
    let chars: Vec<char> = src.chars().collect();
    let mut gradients: Vec<Vec<Rgb>> = Vec::new();
    // (tag name, style it gives) per open tag; the bottom entry is the plain style.
    let mut stack: Vec<(String, Style)> = vec![(String::new(), Style::default())];
    let mut out: Vec<Seg> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;
    let flush = |buf: &mut String, out: &mut Vec<Seg>, style: &Style| {
        if !buf.is_empty() {
            out.push(Seg { text: std::mem::take(buf), style: style.clone() });
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], '<' | '\'' | '\\') {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c != '<' {
            buf.push(c);
            i += 1;
            continue;
        }
        let Some(end) = tag_end(&chars, i) else {
            buf.push(c);
            i += 1;
            continue;
        };
        let content: String = chars[i + 1..end].iter().collect();
        i = end + 1;
        let top = stack.last().map(|(_, s)| s.clone()).unwrap_or_default();
        if let Some(closing) = content.strip_prefix('/') {
            let name = closing.trim().to_lowercase();
            flush(&mut buf, &mut out, &top);
            if let Some(at) = stack.iter().rposition(|(n, _)| *n == name).filter(|at| *at > 0) {
                stack.truncate(at);
            }
            continue;
        }
        let (name, args) = split_tag(&content);
        let mut style = top.clone();
        let opens = match name.as_str() {
            "bold" | "b" => {
                style.bold = true;
                true
            }
            "italic" | "i" | "em" => {
                style.italic = true;
                true
            }
            "underlined" | "u" => {
                style.underlined = true;
                true
            }
            "strikethrough" | "st" => {
                style.strikethrough = true;
                true
            }
            "obfuscated" | "obf" => {
                style.obfuscated = true;
                true
            }
            "color" | "colour" => match args.first().and_then(|c| colour(c)) {
                Some(c) => {
                    style.color = Some(c);
                    true
                }
                None => false,
            },
            "hover" if args.first().map(String::as_str) == Some("show_text") && args.len() >= 2 => {
                style.hover = Some(args[1].clone());
                true
            }
            "click" => match (args.first(), args.get(1)) {
                (Some(action), Some(value))
                    if matches!(action.as_str(), "run_command" | "suggest_command" | "open_url" | "copy_to_clipboard") =>
                {
                    style.click = Some((action.clone(), value.clone()));
                    true
                }
                _ => false,
            },
            "gradient" => {
                let stops: Vec<Rgb> = args.iter().filter_map(|a| hex(a)).collect();
                if stops.len() >= 2 {
                    gradients.push(stops);
                    style.gradient = Some(gradients.len() - 1);
                    true
                } else {
                    false
                }
            }
            n if n.starts_with('#') => match colour(n) {
                Some(c) => {
                    style.color = Some(c);
                    true
                }
                None => false,
            },
            n if NAMED_COLORS.contains(&n) => {
                style.color = Some(n.to_owned());
                true
            }
            _ => false,
        };
        if opens {
            flush(&mut buf, &mut out, &top);
            stack.push((name, style));
        }
    }
    let top = stack.last().map(|(_, s)| s.clone()).unwrap_or_default();
    flush(&mut buf, &mut out, &top);
    expand_gradients(out, &gradients)
}

fn colour(c: &str) -> Option<String> {
    let c = c.trim();
    if hex(c).is_some() {
        return Some(c.to_lowercase());
    }
    NAMED_COLORS.contains(&c.to_lowercase().as_str()).then(|| c.to_lowercase())
}

fn hex(c: &str) -> Option<Rgb> {
    let h = c.trim().strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|x| x.is_ascii_hexdigit()) {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((v(0)?, v(2)?, v(4)?))
}

fn hex_of(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

/// Colours each character of a gradient span, then merges neighbours with the same style.
fn expand_gradients(segs: Vec<Seg>, gradients: &[Vec<Rgb>]) -> Vec<Seg> {
    let mut total = vec![0usize; gradients.len()];
    for s in &segs {
        if let Some(g) = s.style.gradient {
            total[g] += s.text.chars().count();
        }
    }
    let mut seen = vec![0usize; gradients.len()];
    let mut out: Vec<Seg> = Vec::new();
    for seg in segs {
        let Some(g) = seg.style.gradient else {
            push_merged(&mut out, seg);
            continue;
        };
        for ch in seg.text.chars() {
            let k = seen[g];
            seen[g] += 1;
            let mut style = seg.style.clone();
            style.gradient = None;
            style.color = Some(hex_of(lerp(&gradients[g], k, total[g])));
            push_merged(&mut out, Seg { text: ch.to_string(), style });
        }
    }
    out
}

fn push_merged(out: &mut Vec<Seg>, seg: Seg) {
    match out.last_mut() {
        Some(last) if last.style == seg.style => last.text.push_str(&seg.text),
        _ => out.push(seg),
    }
}

fn lerp(stops: &[Rgb], k: usize, n: usize) -> Rgb {
    if n <= 1 {
        return stops[0];
    }
    let t = k as f64 / (n - 1) as f64 * (stops.len() - 1) as f64;
    let i = (t.floor() as usize).min(stops.len() - 2);
    let f = t - i as f64;
    let mix = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    let (a, b) = (stops[i], stops[i + 1]);
    (mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

/// The root component for a line: `{"text":"","extra":[...]}`.
fn component(segs: &[Seg]) -> Value {
    let extra: Vec<Value> = segs.iter().map(seg_json).collect();
    json!({ "text": "", "extra": extra })
}

fn seg_json(seg: &Seg) -> Value {
    let mut m = Map::new();
    m.insert("text".into(), Value::String(seg.text.clone()));
    let s = &seg.style;
    if let Some(c) = &s.color {
        m.insert("color".into(), Value::String(c.clone()));
    }
    for (on, key) in [
        (s.bold, "bold"),
        (s.italic, "italic"),
        (s.underlined, "underlined"),
        (s.strikethrough, "strikethrough"),
        (s.obfuscated, "obfuscated"),
    ] {
        if on {
            m.insert(key.into(), Value::Bool(true));
        }
    }
    if let Some((action, value)) = &s.click {
        let field = match action.as_str() {
            "open_url" => "url",
            "copy_to_clipboard" => "value",
            _ => "command",
        };
        m.insert("click_event".into(), json!({ "action": action, field: value }));
    }
    if let Some(raw) = &s.hover {
        let inner = parse(raw);
        m.insert("hover_event".into(), json!({ "action": "show_text", "value": component(&inner) }));
    }
    Value::Object(m)
}

/// The plain words of a component tree built by `component` (for logs and tests).
pub fn plain_of(component: &Value) -> String {
    let mut out = String::new();
    if let Some(t) = component.get("text").and_then(Value::as_str) {
        out.push_str(t);
    }
    if let Some(extra) = component.get("extra").and_then(Value::as_array) {
        for e in extra {
            out.push_str(&plain_of(e));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(src: &str) -> Vec<Seg> {
        parse(src)
    }

    #[test]
    fn colour_tags_and_closing() {
        let segs = first("<color:#00fb9a>⏵ Data synchronized!</color> done");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "⏵ Data synchronized!");
        assert_eq!(segs[0].style.color.as_deref(), Some("#00fb9a"));
        assert_eq!(segs[1].text, " done");
        assert_eq!(segs[1].style.color, None);
    }

    #[test]
    fn named_colours_and_unclosed_tags_run_to_the_end() {
        let segs = first("<gray>a<dark_gray>b");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].style.color.as_deref(), Some("gray"));
        assert_eq!(segs[1].style.color.as_deref(), Some("dark_gray"));
    }

    #[test]
    fn decorations_nest() {
        let segs = first("<bold><italic>x</italic>y</bold>z");
        assert!(segs[0].style.bold && segs[0].style.italic);
        assert!(segs[1].style.bold && !segs[1].style.italic);
        assert!(!segs[2].style.bold);
    }

    #[test]
    fn gradient_is_coloured_per_character() {
        let segs = first("<gradient:#000000:#ffffff>abc</gradient>");
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].style.color.as_deref(), Some("#000000"));
        assert_eq!(segs[1].style.color.as_deref(), Some("#808080"));
        assert_eq!(segs[2].style.color.as_deref(), Some("#ffffff"));
    }

    #[test]
    fn hover_and_click_become_component_events() {
        let lines = Messages::default().lines("user_registration_complete", &[]);
        assert_eq!(lines.len(), 1);
        let src = "<hover:show_text:'<gray>Version UUID:<dark_gray>%1%'><click:run_command:'/userdata view %2% x'>[Go]</click></hover>";
        let filled = fill(src, &["abc", "Name"]);
        let segs = parse(&filled);
        let c = component(&segs);
        let child = &c["extra"][0];
        assert_eq!(child["text"], "[Go]");
        assert_eq!(child["click_event"]["action"], "run_command");
        assert_eq!(child["click_event"]["command"], "/userdata view Name x");
        assert_eq!(child["hover_event"]["action"], "show_text");
        assert_eq!(plain_of(&child["hover_event"]["value"]), "Version UUID:abc");
    }

    #[test]
    fn escapes_unknown_tags_and_placeholders() {
        let segs = first(r"won\'t \<b> <reset>x");
        let text: String = segs.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "won't <b> x");
        assert_eq!(fill("%1% and %2%", &["a", "b"]), "a and b");
    }

    #[test]
    fn literal_backslash_n_splits_lines() {
        let m = Messages::from_yaml("locales:\n  k: first\\n<gray>second</gray>\n").unwrap();
        let lines = m.lines("k", &[]);
        assert_eq!(lines.iter().map(|l| l.plain.as_str()).collect::<Vec<_>>(), ["first", "second"]);
    }

    #[test]
    fn file_text_wins_and_defaults_fill_in() {
        let m = Messages::from_yaml("locales:\n  synchronization_complete: <red>Synced %1%</red>\n").unwrap();
        assert_eq!(m.lines("synchronization_complete", &["x"])[0].plain, "Synced x");
        assert_eq!(
            Messages::default().lines("synchronization_complete", &[])[0].plain,
            "⏵ Data synchronized!"
        );
        assert!(m.lines("no_such_key", &[]).is_empty());
    }

    #[test]
    fn fixture_in_the_shape_of_a_locale_file() {
        let text = "# comment\nlocales:\n  data_update_failed: <color:#ff7e5e>🔔 Failed to update.</color>\n  reload_complete: |-\n    <bold>HuskSync</bold> | Reloaded\n    <italic>A restart is needed.</italic>\n";
        let m = Messages::from_yaml(text).unwrap();
        assert_eq!(m.lines("data_update_failed", &[])[0].plain, "🔔 Failed to update.");
        // A block scalar keeps its line breaks in the template; `lines` turns them into spaces.
        assert_eq!(m.template("reload_complete"), Some("<bold>HuskSync</bold> | Reloaded\n<italic>A restart is needed.</italic>"));
        assert_eq!(m.lines("reload_complete", &[])[0].plain, "HuskSync | Reloaded A restart is needed.");
    }
}
