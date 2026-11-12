//! A small NBT tree and its SNBT text form.
//!
//! HuskSync keeps item stacks as SNBT: the text `CompoundTag.toString()` prints on Paper, read
//! back with Mojang's `TagParser`. The writer prints the shape of Mojang's `StringTagVisitor`
//! (typed suffixes, `[B;`/`[I;`/`[L;` arrays, compact, no spaces). Keys are written in the order the
//! tree holds them: a compound read from Paper is written back in Paper's order, and a new compound
//! in the order it was built. The reader takes that and the usual hand-written variants (unquoted
//! strings, `true`/`false`, either quote, escapes).

use indexmap::IndexMap;
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Nbt {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Nbt>),
    /// Keys keep the order they were read or inserted in (Paper's order survives a round trip).
    Compound(IndexMap<String, Nbt>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Nbt {
    pub fn compound() -> Self {
        Nbt::Compound(IndexMap::new())
    }

    pub fn get(&self, key: &str) -> Option<&Nbt> {
        match self {
            Nbt::Compound(map) => map.get(key),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Nbt> {
        match self {
            Nbt::Compound(map) => map.get_mut(key),
            _ => None,
        }
    }

    /// Puts `value` under `key`; a no-op on anything but a compound.
    pub fn put(&mut self, key: &str, value: Nbt) {
        if let Nbt::Compound(map) = self {
            map.insert(key.to_owned(), value);
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Nbt> {
        match self {
            Nbt::Compound(map) => map.shift_remove(key),
            _ => None,
        }
    }

    /// Any numeric tag as an i64 (`NumericTag.getAsLong`; floats truncate).
    pub fn as_i64(&self) -> Option<i64> {
        Some(match self {
            Nbt::Byte(v) => *v as i64,
            Nbt::Short(v) => *v as i64,
            Nbt::Int(v) => *v as i64,
            Nbt::Long(v) => *v,
            Nbt::Float(v) => *v as i64,
            Nbt::Double(v) => *v as i64,
            _ => return None,
        })
    }

    pub fn as_f64(&self) -> Option<f64> {
        Some(match self {
            Nbt::Byte(v) => *v as f64,
            Nbt::Short(v) => *v as f64,
            Nbt::Int(v) => *v as f64,
            Nbt::Long(v) => *v as f64,
            Nbt::Float(v) => *v as f64,
            Nbt::Double(v) => *v,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Nbt::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Nbt]> {
        match self {
            Nbt::List(v) => Some(v),
            _ => None,
        }
    }

    pub fn i64_at(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Nbt::as_i64)
    }

    pub fn f64_at(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Nbt::as_f64)
    }

    pub fn str_at(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Nbt::as_str)
    }

    /// The SNBT text, as Paper's `CompoundTag.toString()` writes it.
    pub fn to_snbt(&self) -> String {
        let mut out = String::new();
        write_snbt(self, &mut out);
        out
    }
}

fn simple_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+'))
}

/// `StringTag.quoteAndEscape`: double quotes unless the text holds one and no single quote.
fn quote(s: &str, out: &mut String) {
    let q = if s.contains('"') && !s.contains('\'') { '\'' } else { '"' };
    out.push(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push(q);
}

/// Java's `Float.toString` / `Double.toString` shape: always a fraction or an exponent.
fn java_number(v: f64, out: &mut String) {
    if v.is_nan() {
        out.push_str("NaN");
    } else if v.is_infinite() {
        out.push_str(if v > 0.0 { "Infinity" } else { "-Infinity" });
    } else {
        let s = format!("{v:?}");
        // Rust's Debug prints `1.0`, `0.1`, `1e-7`; the parsers on both sides read all of them.
        out.push_str(&s);
    }
}

fn java_float(v: f32, out: &mut String) {
    if v.is_finite() {
        let _ = write!(out, "{v:?}");
    } else {
        java_number(v as f64, out);
    }
}

fn write_snbt(tag: &Nbt, out: &mut String) {
    match tag {
        Nbt::Byte(v) => {
            let _ = write!(out, "{v}b");
        }
        Nbt::Short(v) => {
            let _ = write!(out, "{v}s");
        }
        Nbt::Int(v) => {
            let _ = write!(out, "{v}");
        }
        Nbt::Long(v) => {
            let _ = write!(out, "{v}L");
        }
        Nbt::Float(v) => {
            java_float(*v, out);
            out.push('f');
        }
        Nbt::Double(v) => {
            java_number(*v, out);
            out.push('d');
        }
        Nbt::ByteArray(v) => {
            out.push_str("[B;");
            for (i, b) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{b}B");
            }
            out.push(']');
        }
        Nbt::IntArray(v) => {
            out.push_str("[I;");
            for (i, b) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{b}");
            }
            out.push(']');
        }
        Nbt::LongArray(v) => {
            out.push_str("[L;");
            for (i, b) in v.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{b}L");
            }
            out.push(']');
        }
        Nbt::String(s) => quote(s, out),
        Nbt::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_snbt(item, out);
            }
            out.push(']');
        }
        Nbt::Compound(map) => {
            out.push('{');
            // Paper's SNBT (as HuskSync stores it) lists a compound's keys sorted, by Java's
            // String order (UTF-16 units; the same as bytes for these ASCII keys).
            let mut entries: Vec<(&String, &Nbt)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            for (i, (k, v)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if simple_key(k) {
                    out.push_str(k);
                } else {
                    quote(k, out);
                }
                out.push(':');
                write_snbt(v, out);
            }
            out.push('}');
        }
    }
}

/// Parses SNBT; the whole text must be one value (surrounding whitespace allowed).
pub fn parse_snbt(text: &str) -> Result<Nbt, String> {
    let mut p = Parser {
        s: text.as_bytes(),
        text,
        i: 0,
        depth: 0,
    };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("trailing data at {}", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
    depth: u32,
}

const MAX_DEPTH: u32 = 512;

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        self.ws();
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at {}", c as char, self.i))
        }
    }

    fn value(&mut self) -> Result<Nbt, String> {
        self.ws();
        match self.peek() {
            Some(b'{') => self.compound(),
            Some(b'[') => self.list(),
            Some(b'"' | b'\'') => Ok(Nbt::String(self.quoted()?)),
            Some(_) => self.literal(),
            None => Err("unexpected end".into()),
        }
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("nested too deep".into());
        }
        Ok(())
    }

    fn compound(&mut self) -> Result<Nbt, String> {
        self.enter()?;
        self.expect(b'{')?;
        let mut map = IndexMap::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            self.depth -= 1;
            return Ok(Nbt::Compound(map));
        }
        loop {
            self.ws();
            let key = match self.peek() {
                Some(b'"' | b'\'') => self.quoted()?,
                _ => self.unquoted()?.to_owned(),
            };
            if key.is_empty() {
                return Err(format!("expected a key at {}", self.i));
            }
            self.expect(b':')?;
            let v = self.value()?;
            map.insert(key, v);
            self.ws();
            match self.peek() {
                Some(b',') => {
                    self.i += 1;
                    self.ws();
                    if self.peek() == Some(b'}') {
                        self.i += 1;
                        break;
                    }
                }
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(format!("expected ',' or '}}' at {}", self.i)),
            }
        }
        self.depth -= 1;
        Ok(Nbt::Compound(map))
    }

    fn list(&mut self) -> Result<Nbt, String> {
        self.enter()?;
        self.expect(b'[')?;
        // Typed arrays: [B; ...], [I; ...], [L; ...]
        if self.s.len() > self.i + 1
            && matches!(self.s[self.i], b'B' | b'I' | b'L')
            && self.s[self.i + 1] == b';'
        {
            let kind = self.s[self.i];
            self.i += 2;
            let mut vals = Vec::new();
            self.ws();
            if self.peek() == Some(b']') {
                self.i += 1;
            } else {
                loop {
                    let v = self.value()?;
                    vals.push(v.as_i64().ok_or_else(|| format!("not a number at {}", self.i))?);
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            break;
                        }
                        _ => return Err(format!("expected ',' or ']' at {}", self.i)),
                    }
                }
            }
            self.depth -= 1;
            return Ok(match kind {
                b'B' => Nbt::ByteArray(vals.into_iter().map(|v| v as i8).collect()),
                b'I' => Nbt::IntArray(vals.into_iter().map(|v| v as i32).collect()),
                _ => Nbt::LongArray(vals),
            });
        }
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            self.depth -= 1;
            return Ok(Nbt::List(items));
        }
        loop {
            items.push(self.value()?);
            self.ws();
            match self.peek() {
                Some(b',') => {
                    self.i += 1;
                    self.ws();
                    if self.peek() == Some(b']') {
                        self.i += 1;
                        break;
                    }
                }
                Some(b']') => {
                    self.i += 1;
                    break;
                }
                _ => return Err(format!("expected ',' or ']' at {}", self.i)),
            }
        }
        self.depth -= 1;
        Ok(Nbt::List(items))
    }

    fn quoted(&mut self) -> Result<String, String> {
        let q = self.s[self.i];
        self.i += 1;
        let mut out = String::new();
        let start = self.i;
        let mut run = start;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c == q {
                out.push_str(&self.text[run..self.i]);
                self.i += 1;
                return Ok(out);
            }
            if c == b'\\' {
                out.push_str(&self.text[run..self.i]);
                self.i += 1;
                let e = self.peek().ok_or("unterminated escape")?;
                self.i += 1;
                match e {
                    b'\\' => out.push('\\'),
                    b'"' => out.push('"'),
                    b'\'' => out.push('\''),
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    b'b' => out.push('\u{8}'),
                    b'f' => out.push('\u{c}'),
                    b's' => out.push(' '),
                    b'x' => out.push(self.hex_char(2)?),
                    b'u' => out.push(self.hex_char(4)?),
                    b'U' => out.push(self.hex_char(8)?),
                    other => return Err(format!("bad escape '\\{}'", other as char)),
                }
                run = self.i;
                continue;
            }
            self.i += 1;
        }
        Err("unterminated string".into())
    }

    fn hex_char(&mut self, n: usize) -> Result<char, String> {
        let end = self.i + n;
        let h = self.text.get(self.i..end).ok_or("short escape")?;
        let v = u32::from_str_radix(h, 16).map_err(|_| "bad hex escape")?;
        self.i = end;
        Ok(char::from_u32(v).unwrap_or('\u{fffd}'))
    }

    fn unquoted(&mut self) -> Result<&str, String> {
        let start = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+') {
                self.i += 1;
            } else {
                break;
            }
        }
        Ok(&self.text[start..self.i])
    }

    fn literal(&mut self) -> Result<Nbt, String> {
        let start = self.i;
        let word = self.unquoted()?;
        if word.is_empty() {
            return Err(format!("unexpected character at {start}"));
        }
        Ok(parse_literal(word))
    }
}

/// An unquoted word: a typed number, a boolean, or a string.
fn parse_literal(word: &str) -> Nbt {
    match word {
        "true" => return Nbt::Byte(1),
        "false" => return Nbt::Byte(0),
        "NaNf" => return Nbt::Float(f32::NAN),
        "Infinityf" => return Nbt::Float(f32::INFINITY),
        "-Infinityf" => return Nbt::Float(f32::NEG_INFINITY),
        "NaNd" | "NaN" => return Nbt::Double(f64::NAN),
        "Infinityd" | "Infinity" => return Nbt::Double(f64::INFINITY),
        "-Infinityd" | "-Infinity" => return Nbt::Double(f64::NEG_INFINITY),
        _ => {}
    }
    let (body, suffix) = match word.as_bytes()[word.len() - 1] {
        c @ (b'b' | b'B' | b's' | b'S' | b'l' | b'L' | b'f' | b'F' | b'd' | b'D') => {
            (&word[..word.len() - 1], Some(c.to_ascii_lowercase()))
        }
        _ => (word, None),
    };
    let int_like = !body.is_empty()
        && body
            .trim_start_matches(['-', '+'])
            .bytes()
            .all(|b| b.is_ascii_digit())
        && body.trim_start_matches(['-', '+']).len() > 0;
    let float_like = !body.is_empty() && body.parse::<f64>().is_ok() && !body.contains("inf");
    match suffix {
        Some(b'b') if int_like => body.parse::<i64>().map(|v| Nbt::Byte(v as i8)).ok(),
        Some(b's') if int_like => body.parse::<i64>().map(|v| Nbt::Short(v as i16)).ok(),
        Some(b'l') if int_like => body.parse::<i64>().map(Nbt::Long).ok(),
        Some(b'f') if float_like => body.parse::<f32>().map(Nbt::Float).ok(),
        Some(b'd') if float_like => body.parse::<f64>().map(Nbt::Double).ok(),
        None if int_like => body.parse::<i32>().map(Nbt::Int).ok(),
        None if float_like && (body.contains('.') || body.contains(['e', 'E'])) => {
            body.parse::<f64>().map(Nbt::Double).ok()
        }
        _ => None,
    }
    .unwrap_or_else(|| Nbt::String(word.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_typed_values() {
        let text = r#"{DataVersion:5023,Slot:3,components:{"minecraft:custom_name":'{"text":"A \'b\'"}',"minecraft:damage":7},count:1,f:1.5f,d:-0.25d,id:"minecraft:diamond_sword",l:9L,arr:[B;1B,-2B],ia:[I;1,2],la:[L;3L],s:4s,list:[1,2]}"#;
        let tag = parse_snbt(text).unwrap();
        assert_eq!(tag.i64_at("DataVersion"), Some(5023));
        assert_eq!(tag.get("f"), Some(&Nbt::Float(1.5)));
        assert_eq!(tag.get("s"), Some(&Nbt::Short(4)));
        assert_eq!(tag.get("arr"), Some(&Nbt::ByteArray(vec![1, -2])));
        let name = tag
            .get("components")
            .unwrap()
            .str_at("minecraft:custom_name")
            .unwrap();
        assert_eq!(name, r#"{"text":"A 'b'"}"#);
        let again = parse_snbt(&tag.to_snbt()).unwrap();
        assert_eq!(again, tag);
    }

    #[test]
    fn writes_mojang_shape() {
        let mut c = Nbt::compound();
        c.put("id", Nbt::String("minecraft:stone".into()));
        c.put("count", Nbt::Int(3));
        c.put("minecraft:weird key", Nbt::Byte(1));
        assert_eq!(
            c.to_snbt(),
            r#"{count:3,id:"minecraft:stone","minecraft:weird key":1b}"#
        );
    }

    #[test]
    fn writes_keys_sorted_as_paper_does() {
        // Key order of a live Paper snapshot (checked on real rows, 2026-11-11): sorted.
        let text = r#"{held_item_slot:2,items:{size:43,items:[{Slot:40,DataVersion:5023,id:"minecraft:shield",count:1}]}}"#;
        assert_eq!(
            parse_snbt(text).unwrap().to_snbt(),
            r#"{held_item_slot:2,items:{items:[{DataVersion:5023,Slot:40,count:1,id:"minecraft:shield"}],size:43}}"#
        );
    }

    #[test]
    fn reads_java_floats_and_words() {
        let t = parse_snbt("{a:1.0E-5f,b:true,c:hello,d:1.0,e:-7}").unwrap();
        assert!(matches!(t.get("a"), Some(Nbt::Float(v)) if (*v - 1e-5).abs() < 1e-9));
        assert_eq!(t.get("b"), Some(&Nbt::Byte(1)));
        assert_eq!(t.get("c"), Some(&Nbt::String("hello".into())));
        assert_eq!(t.get("d"), Some(&Nbt::Double(1.0)));
        assert_eq!(t.get("e"), Some(&Nbt::Int(-7)));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_snbt("{a:").is_err());
        assert!(parse_snbt("{a:1}}").is_err());
        assert!(parse_snbt(&"[".repeat(600)).is_err());
    }
}
