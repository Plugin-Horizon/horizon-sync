//! Locked-map persistence (`BukkitMapHandler`, `persist_locked_maps`): a locked filled map in a stack is saved with
//! its pixels in the map tables and a `husksync:persisted_locked_map` tag, and a map with that tag is rebuilt
//! on the server a snapshot is applied on (the server's own id when it is the origin, else the bound id).
//!
//! The map file format is the vanilla `data/<id>.dat` NBT (gzip) the original's map library writes; the table
//! column holds Gson's `{"data":[bytes]}` of it, Snappy-compressed when `compress_data` is on.

use crate::horizonsync_core::nbt::Nbt;
use crate::horizonsync_core::snappy;
use std::io::{Read, Write};

/// `BukkitMapHandler.MAP_DATA_KEY`.
pub const MAP_DATA_KEY: &str = "husksync:persisted_locked_map";
const MAP_ORIGIN_KEY: &str = "origin";
const MAP_ID_KEY: &str = "id";
const CUSTOM_DATA: &str = "minecraft:custom_data";
const MAP_ID: &str = "minecraft:map_id";

/// A map's pixels and placement, the part of the map file this port keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct MapPixels {
    /// 128 x 128 colour ids, row by row.
    pub colors: Vec<u8>,
    pub dimension: String,
    pub scale: i8,
    pub x_center: i32,
    pub z_center: i32,
}

/// A map of this server as the game reports it.
pub struct LocalMap {
    pub locked: bool,
    pub pixels: MapPixels,
}

/// What the item walk needs from the database.
pub trait MapStore {
    fn read(&self, server: &str, id: i32) -> Option<MapPixels>;
    fn write(&self, server: &str, id: i32, pixels: &MapPixels);
    fn bound_id(&self, from_server: &str, from_id: i32, to_server: &str) -> Option<i32>;
    fn bind(&self, from_server: &str, from_id: i32, to_server: &str, to_id: i32);
}

/// What the item walk needs from the game.
pub trait MapWorld {
    fn local(&mut self, id: i32) -> Option<LocalMap>;
    /// A new locked map with these pixels (`generateRenderedMap`: scale closest, no tracking); its id.
    fn create(&mut self, pixels: &MapPixels) -> Option<i32>;
}

/// Calls `f` on `item` and on every stack inside it (shulker box contents, bundles).
pub fn walk(item: &mut Nbt, f: &mut dyn FnMut(&mut Nbt)) {
    f(item);
    let Some(Nbt::Compound(components)) = item.get_mut("components") else {
        return;
    };
    if let Some(Nbt::List(entries)) = components.get_mut("minecraft:container") {
        for entry in entries {
            if let Some(inner) = entry.get_mut("item") {
                walk(inner, f);
            }
        }
    }
    if let Some(Nbt::List(stacks)) = components.get_mut("minecraft:bundle_contents") {
        for inner in stacks {
            walk(inner, f);
        }
    }
}

fn is_map(item: &Nbt) -> bool {
    item.str_at("id") == Some("minecraft:filled_map")
}

fn map_id_of(item: &Nbt) -> Option<i32> {
    item.get("components")?.i64_at(MAP_ID).map(|v| v as i32)
}

fn persisted_tag(item: &Nbt) -> Option<(String, i32)> {
    let tag = item.get("components")?.get(CUSTOM_DATA)?.get(MAP_DATA_KEY)?;
    Some((tag.str_at(MAP_ORIGIN_KEY)?.to_owned(), tag.i64_at(MAP_ID_KEY)? as i32))
}

/// `persistMapView` for one stack and what it holds.
pub fn persist(item: &mut Nbt, server: &str, store: &dyn MapStore, world: &mut dyn MapWorld) {
    walk(item, &mut |stack| {
        if !is_map(stack) || persisted_tag(stack).is_some() {
            return;
        }
        let Some(id) = map_id_of(stack) else { return };
        let Some(local) = world.local(id) else { return };
        if !local.locked {
            return;
        }
        let mut tag = Nbt::compound();
        tag.put(MAP_ORIGIN_KEY, Nbt::String(server.to_owned()));
        tag.put(MAP_ID_KEY, Nbt::Int(id));
        let Some(Nbt::Compound(components)) = stack.get_mut("components") else { return };
        let custom = components.entry(CUSTOM_DATA.to_owned()).or_insert_with(Nbt::compound);
        custom.put(MAP_DATA_KEY, tag);
        if store.read(server, id).is_none() {
            store.write(server, id, &local.pixels);
        }
    });
}

/// `applyMapView` for one stack and what it holds.
pub fn apply(item: &mut Nbt, server: &str, store: &dyn MapStore, world: &mut dyn MapWorld) {
    walk(item, &mut |stack| {
        if !is_map(stack) {
            return;
        }
        let Some((origin, original)) = persisted_tag(stack) else { return };
        let is_origin = origin == server;
        let bound = store.bound_id(&origin, original, server).or(if is_origin { Some(original) } else { None });
        let id = match bound {
            Some(id) if world.local(id).is_some() => Some(id),
            Some(id) => {
                let Some(pixels) = store.read(&origin, original) else { return };
                let created = world.create(&pixels);
                if let Some(new) = created {
                    if new != id {
                        store.bind(&origin, original, server, new);
                    }
                }
                created
            }
            None => {
                let Some(pixels) = store.read(&origin, original) else { return };
                let created = world.create(&pixels);
                if let Some(new) = created {
                    store.bind(&origin, original, server, new);
                }
                created
            }
        };
        if let (Some(id), Some(components)) = (id, stack.get_mut("components")) {
            components.put(MAP_ID, Nbt::Int(id));
        }
    });
}

/// The vanilla map file (`{data:{...},DataVersion}`), gzip, as the map library writes it.
pub fn encode(p: &MapPixels, data_version: i32) -> Vec<u8> {
    fn name(out: &mut Vec<u8>, kind: u8, key: &str) {
        out.push(kind);
        out.extend((key.len() as u16).to_be_bytes());
        out.extend(key.as_bytes());
    }
    let mut raw = Vec::new();
    name(&mut raw, 10, "");
    name(&mut raw, 10, "data");
    name(&mut raw, 7, "colors");
    raw.extend((p.colors.len() as i32).to_be_bytes());
    raw.extend(&p.colors);
    name(&mut raw, 8, "dimension");
    raw.extend((p.dimension.len() as u16).to_be_bytes());
    raw.extend(p.dimension.as_bytes());
    name(&mut raw, 1, "scale");
    raw.push(p.scale as u8);
    name(&mut raw, 3, "xCenter");
    raw.extend(p.x_center.to_be_bytes());
    name(&mut raw, 3, "zCenter");
    raw.extend(p.z_center.to_be_bytes());
    for key in ["locked", "trackingPosition", "unlimitedTracking"] {
        name(&mut raw, 1, key);
        raw.push(u8::from(key == "locked"));
    }
    for key in ["banners", "frames"] {
        name(&mut raw, 9, key);
        raw.push(10);
        raw.extend(0i32.to_be_bytes());
    }
    raw.push(0); // end of `data`
    name(&mut raw, 3, "DataVersion");
    raw.extend(data_version.to_be_bytes());
    raw.push(0); // end of root
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = enc.write_all(&raw);
    enc.finish().unwrap_or_default()
}

/// The pixels of a map file in either form (gzip or plain), or none for anything else.
pub fn decode(bytes: &[u8]) -> Option<MapPixels> {
    let mut raw = Vec::new();
    if bytes.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(bytes).take(64 << 20).read_to_end(&mut raw).ok()?;
    } else {
        raw = bytes.to_vec();
    }
    let mut i = 0usize;
    if *raw.first()? != 10 {
        return None;
    }
    i += 1;
    let n = u16::from_be_bytes(raw.get(i..i + 2)?.try_into().ok()?) as usize;
    i += 2 + n;
    let root = read_tag(10, &raw, &mut i, 0)?;
    let data = root.get("data")?;
    let colors = match data.get("colors")? {
        Nbt::ByteArray(v) => v.iter().map(|b| *b as u8).collect::<Vec<u8>>(),
        _ => return None,
    };
    Some(MapPixels {
        colors,
        dimension: data.str_at("dimension").unwrap_or("minecraft:overworld").to_owned(),
        scale: data.i64_at("scale").unwrap_or(0) as i8,
        x_center: data.i64_at("xCenter").unwrap_or(0) as i32,
        z_center: data.i64_at("zCenter").unwrap_or(0) as i32,
    })
}

fn read_tag(kind: u8, b: &[u8], i: &mut usize, depth: u32) -> Option<Nbt> {
    fn take<'a>(b: &'a [u8], i: &mut usize, n: usize) -> Option<&'a [u8]> {
        let s = b.get(*i..i.checked_add(n)?)?;
        *i += n;
        Some(s)
    }
    fn int(b: &[u8], i: &mut usize) -> Option<i32> {
        Some(i32::from_be_bytes(take(b, i, 4)?.try_into().ok()?))
    }
    fn len(b: &[u8], i: &mut usize) -> Option<usize> {
        usize::try_from(int(b, i)?).ok()
    }
    fn text(b: &[u8], i: &mut usize) -> Option<String> {
        let n = u16::from_be_bytes(take(b, i, 2)?.try_into().ok()?) as usize;
        Some(String::from_utf8_lossy(take(b, i, n)?).into_owned())
    }
    if depth > 64 {
        return None;
    }
    Some(match kind {
        1 => Nbt::Byte(take(b, i, 1)?[0] as i8),
        2 => Nbt::Short(i16::from_be_bytes(take(b, i, 2)?.try_into().ok()?)),
        3 => Nbt::Int(int(b, i)?),
        4 => Nbt::Long(i64::from_be_bytes(take(b, i, 8)?.try_into().ok()?)),
        5 => Nbt::Float(f32::from_be_bytes(take(b, i, 4)?.try_into().ok()?)),
        6 => Nbt::Double(f64::from_be_bytes(take(b, i, 8)?.try_into().ok()?)),
        7 => {
            let n = len(b, i)?;
            Nbt::ByteArray(take(b, i, n)?.iter().map(|v| *v as i8).collect())
        }
        8 => Nbt::String(text(b, i)?),
        9 => {
            let inner = take(b, i, 1)?[0];
            let n = len(b, i)?;
            let mut list = Vec::new();
            for _ in 0..n {
                list.push(read_tag(inner, b, i, depth + 1)?);
            }
            Nbt::List(list)
        }
        10 => {
            let mut map = Nbt::compound();
            loop {
                let k = take(b, i, 1)?[0];
                if k == 0 {
                    break;
                }
                let key = text(b, i)?;
                map.put(&key, read_tag(k, b, i, depth + 1)?);
            }
            map
        }
        11 => {
            let n = len(b, i)?;
            let mut v = Vec::new();
            for _ in 0..n {
                v.push(int(b, i)?);
            }
            Nbt::IntArray(v)
        }
        12 => {
            let n = len(b, i)?;
            let mut v = Vec::new();
            for _ in 0..n {
                v.push(i64::from_be_bytes(take(b, i, 8)?.try_into().ok()?));
            }
            Nbt::LongArray(v)
        }
        _ => return None,
    })
}

/// The column value (`AdaptableMapData`): Gson's `{"data":[signed bytes]}` of the map file, Snappy when `compress`.
pub fn to_column(file: &[u8], compress: bool) -> Vec<u8> {
    let list: Vec<String> = file.iter().map(|b| (*b as i8).to_string()).collect();
    let json = format!("{{\"data\":[{}]}}", list.join(",")).into_bytes();
    if compress { snappy::compress(&json) } else { json }
}

/// The map file inside a column value, whichever of the two encodings it has.
pub fn from_column(column: &[u8]) -> Option<Vec<u8>> {
    let parse = |json: &[u8]| -> Option<Vec<u8>> {
        let v: serde_json::Value = serde_json::from_slice(json).ok()?;
        v.get("data")?.as_array()?.iter().map(|n| n.as_i64().map(|n| n as u8)).collect()
    };
    snappy::decompress(column).ok().and_then(|j| parse(&j)).or_else(|| parse(column))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    fn pixels() -> MapPixels {
        let mut colors = vec![0u8; 16384];
        colors[5] = 12;
        colors[16383] = 200;
        MapPixels { colors, dimension: "minecraft:overworld".into(), scale: 0, x_center: -64, z_center: 320 }
    }

    #[test]
    fn a_map_file_round_trips_through_the_column() {
        let p = pixels();
        let file = encode(&p, 4000);
        assert_eq!(decode(&file).unwrap(), p);
        for compress in [true, false] {
            let column = to_column(&file, compress);
            assert_eq!(decode(&from_column(&column).unwrap()).unwrap(), p);
        }
    }

    #[derive(Default)]
    struct Store {
        data: RefCell<HashMap<(String, i32), MapPixels>>,
        bindings: RefCell<HashMap<(String, i32, String), i32>>,
    }
    impl MapStore for Store {
        fn read(&self, s: &str, id: i32) -> Option<MapPixels> {
            self.data.borrow().get(&(s.to_owned(), id)).cloned()
        }
        fn write(&self, s: &str, id: i32, p: &MapPixels) {
            self.data.borrow_mut().insert((s.to_owned(), id), p.clone());
        }
        fn bound_id(&self, f: &str, id: i32, t: &str) -> Option<i32> {
            self.bindings.borrow().get(&(f.to_owned(), id, t.to_owned())).copied()
        }
        fn bind(&self, f: &str, id: i32, t: &str, to: i32) {
            self.bindings.borrow_mut().insert((f.to_owned(), id, t.to_owned()), to);
        }
    }
    struct World {
        maps: HashMap<i32, bool>,
        next: i32,
    }
    impl MapWorld for World {
        fn local(&mut self, id: i32) -> Option<LocalMap> {
            self.maps.get(&id).map(|locked| LocalMap { locked: *locked, pixels: pixels() })
        }
        fn create(&mut self, _: &MapPixels) -> Option<i32> {
            self.next += 1;
            self.maps.insert(self.next, true);
            Some(self.next)
        }
    }

    fn stack(id: i32) -> Nbt {
        let mut components = Nbt::compound();
        components.put(MAP_ID, Nbt::Int(id));
        let mut item = Nbt::compound();
        item.put("id", Nbt::String("minecraft:filled_map".into()));
        item.put("count", Nbt::Int(1));
        item.put("components", components);
        item
    }

    #[test]
    fn a_locked_map_is_saved_and_rebuilt_on_another_server() {
        let store = Store::default();
        let mut here = World { maps: HashMap::from([(7, true), (8, false)]), next: 100 };
        // A locked map inside a shulker box is found too; the unlocked one is left alone.
        let mut shulker = Nbt::compound();
        shulker.put("id", Nbt::String("minecraft:shulker_box".into()));
        let mut entry = Nbt::compound();
        entry.put("slot", Nbt::Int(0));
        entry.put("item", stack(7));
        let mut unlocked = Nbt::compound();
        unlocked.put("slot", Nbt::Int(1));
        unlocked.put("item", stack(8));
        let mut components = Nbt::compound();
        components.put("minecraft:container", Nbt::List(vec![entry, unlocked]));
        shulker.put("components", components);
        persist(&mut shulker, "alpha", &store, &mut here);
        let list = shulker.get("components").unwrap().get("minecraft:container").unwrap().as_list().unwrap().to_vec();
        assert!(persisted_tag(list[0].get("item").unwrap()).is_some());
        assert!(persisted_tag(list[1].get("item").unwrap()).is_none());
        assert_eq!(store.read("alpha", 7).unwrap(), pixels());

        // On the origin server the id stays; on another server a new map is made and bound.
        let mut copy = list[0].get("item").unwrap().clone();
        apply(&mut copy, "alpha", &store, &mut here);
        assert_eq!(map_id_of(&copy), Some(7));
        let mut there = World { maps: HashMap::new(), next: 40 };
        apply(&mut copy, "beta", &store, &mut there);
        assert_eq!(map_id_of(&copy), Some(41));
        assert_eq!(store.bound_id("alpha", 7, "beta"), Some(41));
        // The second time the bound map is reused.
        let mut again = list[0].get("item").unwrap().clone();
        apply(&mut again, "beta", &store, &mut there);
        assert_eq!(map_id_of(&again), Some(41));
        assert_eq!(there.next, 41);
    }
}
