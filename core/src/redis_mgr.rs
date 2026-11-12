//! `RedisManager`: HuskSync's keys and pub/sub channels on Redis, byte for byte.
//!
//! Keys (`RedisKeyType.getKeyPrefix` + `:<uuid>`), with the default empty cluster id:
//! `husksync:::latest_snapshot:<uuid>` (a packed snapshot, 1 year TTL),
//! `husksync:::server_switch:<uuid>` (empty, 10 s TTL, DELAY mode),
//! `husksync:::data_checkout:<uuid>` (the server name holding the player, LOCKSTEP mode).
//! Channels (`RedisMessage.Type.getMessageChannel`): `husksync:::update_user_data`,
//! `husksync:::request_user_data`, `husksync:::return_user_data`, `husksync:::check_in_petition`.
//! Messages are Gson JSON: `{"targetServer":..,"target_uuid":..,"payload":[signed bytes]}`.

use crate::config::Config;
use crate::snapshot::Packed;
use crate::util;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// `RedisManager.KEY_NAMESPACE` (with its trailing colon, which the format doubles).
const KEY_NAMESPACE: &str = "husksync:";
pub const TTL_1_YEAR: u64 = 60 * 60 * 24 * 7 * 52;
pub const TTL_10_SECONDS: u64 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyType {
    LatestSnapshot,
    ServerSwitch,
    DataCheckout,
}

impl KeyType {
    fn name(self) -> &'static str {
        match self {
            KeyType::LatestSnapshot => "latest_snapshot",
            KeyType::ServerSwitch => "server_switch",
            KeyType::DataCheckout => "data_checkout",
        }
    }
}

pub fn key_prefix(kind: KeyType, cluster: &str) -> String {
    format!("{}:{}:{}", KEY_NAMESPACE, cluster.to_lowercase(), kind.name())
}

pub fn key(kind: KeyType, uuid: u128, cluster: &str) -> String {
    format!("{}:{}", key_prefix(kind, cluster), util::uuid_string(uuid))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    UpdateUserData,
    RequestUserData,
    ReturnUserData,
    CheckInPetition,
}

impl MessageType {
    pub const ALL: [MessageType; 4] = [
        MessageType::UpdateUserData,
        MessageType::RequestUserData,
        MessageType::ReturnUserData,
        MessageType::CheckInPetition,
    ];

    pub fn channel(self, cluster: &str) -> String {
        let name = match self {
            MessageType::UpdateUserData => "update_user_data",
            MessageType::RequestUserData => "request_user_data",
            MessageType::ReturnUserData => "return_user_data",
            MessageType::CheckInPetition => "check_in_petition",
        };
        format!("{}:{}:{}", KEY_NAMESPACE, cluster.to_lowercase(), name)
    }

    pub fn from_channel(channel: &str, cluster: &str) -> Option<MessageType> {
        Self::ALL.into_iter().find(|t| t.channel(cluster).eq_ignore_ascii_case(channel))
    }
}

/// `RedisMessage` as Gson writes it (byte[] is a JSON array of signed bytes).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Message {
    #[serde(rename = "targetServer", default, skip_serializing_if = "Option::is_none")]
    pub target_server: Option<String>,
    #[serde(rename = "target_uuid", default, skip_serializing_if = "Option::is_none")]
    pub target_uuid: Option<String>,
    #[serde(default, with = "signed_bytes")]
    pub payload: Vec<u8>,
}

mod signed_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(v.iter().map(|b| *b as i8))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let v: Option<Vec<i16>> = Option::deserialize(d)?;
        Ok(v.unwrap_or_default().into_iter().map(|b| b as i8 as u8).collect())
    }
}

impl Message {
    pub fn to_player(uuid: u128, payload: Vec<u8>) -> Self {
        Message { target_server: None, target_uuid: Some(util::uuid_string(uuid)), payload }
    }

    pub fn to_server(server: &str, payload: Vec<u8>) -> Self {
        Message { target_server: Some(server.to_owned()), target_uuid: None, payload }
    }

    pub fn target(&self) -> Option<u128> {
        self.target_uuid.as_deref().and_then(util::parse_uuid)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }
}

fn percent(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub struct Redis {
    client: redis::Client,
    cluster: String,
    server: String,
    compress: bool,
    max_retries: u32,
    backoff: Duration,
    timeout: Duration,
}

fn bytes_of(v: redis::Value) -> Option<Vec<u8>> {
    match v {
        redis::Value::BulkString(b) => Some(b),
        redis::Value::SimpleString(s) => Some(s.into_bytes()),
        redis::Value::VerbatimString { text, .. } => Some(text.into_bytes()),
        _ => None,
    }
}

impl Redis {
    pub fn connect(config: &Config, server_name: &str) -> Result<Redis, String> {
        let r = &config.redis;
        if r.use_ssl {
            return Err("redis.use_ssl is not supported by this port (the redis crate is built without TLS)".into());
        }
        let auth = match (r.user.is_empty(), r.password.is_empty()) {
            (true, true) => String::new(),
            (true, false) => format!(":{}@", percent(&r.password)),
            (false, true) => format!("{}@", percent(&r.user)),
            (false, false) => format!("{}:{}@", percent(&r.user), percent(&r.password)),
        };
        let url = format!("redis://{auth}{}:{}/{}", r.host, r.port, r.database);
        let client = redis::Client::open(url.as_str()).map_err(|e| format!("redis: {e}"))?;
        let me = Redis {
            client,
            cluster: config.cluster_id.clone(),
            server: server_name.to_owned(),
            compress: config.synchronization.compress_data,
            max_retries: r.max_retries.max(1),
            backoff: Duration::from_millis(r.retry_backoff_millis),
            timeout: Duration::from_millis(r.connection_timeout.max(500)),
        };
        me.ping()?;
        Ok(me)
    }

    pub fn cluster(&self) -> &str {
        &self.cluster
    }

    fn connection(&self) -> Result<redis::Connection, String> {
        let c = self
            .client
            .get_connection_with_timeout(self.timeout)
            .map_err(|e| format!("redis: {e}"))?;
        let _ = c.set_read_timeout(Some(self.timeout));
        let _ = c.set_write_timeout(Some(self.timeout));
        Ok(c)
    }

    /// `tryExecute`: up to `max_retries` attempts with a growing backoff.
    fn run<T>(&self, what: &str, mut f: impl FnMut(&mut redis::Connection) -> redis::RedisResult<T>) -> Result<T, String> {
        let mut last = String::new();
        for attempt in 0..self.max_retries {
            match self.connection().and_then(|mut c| f(&mut c).map_err(|e| e.to_string())) {
                Ok(v) => return Ok(v),
                Err(e) => last = e,
            }
            if attempt + 1 < self.max_retries {
                std::thread::sleep(self.backoff * (attempt + 1));
            }
        }
        Err(format!("redis ({what}): {last}"))
    }

    pub fn ping(&self) -> Result<Duration, String> {
        let start = std::time::Instant::now();
        self.run("ping", |c| redis::cmd("PING").query::<redis::Value>(c))?;
        Ok(start.elapsed())
    }

    fn get(&self, k: &str) -> Result<Option<Vec<u8>>, String> {
        self.run("get", |c| redis::cmd("GET").arg(k).query::<redis::Value>(c))
            .map(bytes_of)
    }

    fn del(&self, k: &str) -> Result<i64, String> {
        self.run("del", |c| redis::cmd("DEL").arg(k).query::<i64>(c))
    }

    /// `getUserData`: reads and consumes the cached latest snapshot.
    pub fn take_user_data(&self, uuid: u128) -> Result<Option<Result<Packed, String>>, String> {
        let k = key(KeyType::LatestSnapshot, uuid, &self.cluster);
        let Some(bytes) = self.get(&k)? else {
            return Ok(None);
        };
        self.del(&k)?;
        Ok(Some(Packed::from_bytes(&bytes, self.compress)))
    }

    /// `setUserData`: caches the snapshot unless the cached one is newer. False when the write
    /// could not be made.
    pub fn set_user_data(&self, uuid: u128, s: &Packed) -> bool {
        let k = key(KeyType::LatestSnapshot, uuid, &self.cluster);
        let result = (|| -> Result<(), String> {
            if let Some(existing) = self.get(&k)? {
                if let Ok(existing) = Packed::from_bytes(&existing, self.compress) {
                    if let (Some(a), Some(b)) = (existing.time(), s.time()) {
                        if a > b {
                            log::debug!("horizonsync: kept a newer cached snapshot for {}", util::uuid_string(uuid));
                            return Ok(());
                        }
                    }
                }
            }
            let bytes = s.to_bytes(self.compress);
            self.run("setex", |c| {
                redis::cmd("SETEX").arg(&k).arg(TTL_1_YEAR).arg(&bytes).query::<redis::Value>(c)
            })?;
            Ok(())
        })();
        match result {
            Ok(()) => true,
            Err(e) => {
                log::warn!("horizonsync: {e}");
                false
            }
        }
    }

    /// Whether a cached snapshot exists (without consuming it), for `/userdata restore`.
    pub fn has_user_data(&self, uuid: u128) -> bool {
        self.get(&key(KeyType::LatestSnapshot, uuid, &self.cluster)).ok().flatten().is_some()
    }

    pub fn clear_user_data(&self, uuid: u128) {
        if let Err(e) = self.del(&key(KeyType::LatestSnapshot, uuid, &self.cluster)) {
            log::warn!("horizonsync: {e}");
        }
    }

    pub fn set_checked_out(&self, uuid: u128, checked_out: bool) {
        let k = key(KeyType::DataCheckout, uuid, &self.cluster);
        let r = if checked_out {
            self.run("set", |c| redis::cmd("SET").arg(&k).arg(self.server.as_bytes()).query::<redis::Value>(c))
                .map(|_| ())
        } else {
            self.del(&k).map(|_| ())
        };
        if let Err(e) = r {
            log::warn!("horizonsync: {e}");
        }
    }

    pub fn checked_out(&self, uuid: u128) -> Option<String> {
        self.get(&key(KeyType::DataCheckout, uuid, &self.cluster))
            .ok()
            .flatten()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    }

    /// `clearUsersCheckedOutOnServer`: every checkout key that names this server.
    pub fn clear_checkouts_here(&self) {
        let pattern = format!("{}*", key_prefix(KeyType::DataCheckout, &self.cluster));
        let keys = self.run("keys", |c| redis::cmd("KEYS").arg(&pattern).query::<Vec<Vec<u8>>>(c));
        let Ok(keys) = keys else { return };
        for k in keys {
            let k = String::from_utf8_lossy(&k).into_owned();
            if self.get(&k).ok().flatten().as_deref() == Some(self.server.as_bytes()) {
                let _ = self.del(&k);
            }
        }
    }

    pub fn set_server_switch(&self, uuid: u128) {
        let k = key(KeyType::ServerSwitch, uuid, &self.cluster);
        if let Err(e) = self.run("setex", |c| {
            redis::cmd("SETEX").arg(&k).arg(TTL_10_SECONDS).arg(&[] as &[u8]).query::<redis::Value>(c)
        }) {
            log::warn!("horizonsync: {e}");
        }
    }

    /// `getUserServerSwitch`: reads and consumes the key.
    pub fn take_server_switch(&self, uuid: u128) -> bool {
        let k = key(KeyType::ServerSwitch, uuid, &self.cluster);
        match self.get(&k) {
            Ok(Some(_)) => {
                let _ = self.del(&k);
                true
            }
            _ => false,
        }
    }

    pub fn publish(&self, kind: MessageType, m: &Message) {
        let channel = kind.channel(&self.cluster);
        let json = m.to_json();
        if let Err(e) = self.run("publish", |c| redis::cmd("PUBLISH").arg(&channel).arg(&json).query::<i64>(c)) {
            log::warn!("horizonsync: {e}");
        }
    }

    pub fn version(&self) -> String {
        self.run("info", |c| redis::cmd("INFO").query::<redis::Value>(c))
            .ok()
            .and_then(bytes_of)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .and_then(|info| {
                info.lines()
                    .find_map(|l| l.strip_prefix("redis_version:").map(|v| v.trim().to_owned()))
            })
            .unwrap_or_else(|| "unknown".into())
    }

    /// Subscribes to the four channels and hands each message to `on_message` until `stop`
    /// returns true; reconnects after errors.
    pub fn listen(&self, stop: &dyn Fn() -> bool, on_message: &dyn Fn(MessageType, Message)) {
        let channels: Vec<String> = MessageType::ALL.iter().map(|t| t.channel(&self.cluster)).collect();
        while !stop() {
            let mut con = match self.client.get_connection_with_timeout(self.timeout) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("horizonsync: redis subscribe: {e}");
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            let mut pubsub = con.as_pubsub();
            let _ = pubsub.set_read_timeout(Some(Duration::from_millis(500)));
            if let Err(e) = channels.iter().try_for_each(|c| pubsub.subscribe(c.as_str())) {
                log::warn!("horizonsync: redis subscribe: {e}");
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            while !stop() {
                match pubsub.get_message() {
                    Ok(msg) => {
                        let Some(kind) = MessageType::from_channel(msg.get_channel_name(), &self.cluster) else {
                            continue;
                        };
                        let text = String::from_utf8_lossy(msg.get_payload_bytes());
                        match Message::from_json(&text) {
                            Ok(m) => on_message(kind, m),
                            Err(e) => log::warn!("horizonsync: bad redis message: {e}"),
                        }
                    }
                    Err(e) if e.is_timeout() => continue,
                    Err(e) => {
                        log::warn!("horizonsync: redis subscription lost: {e}");
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_match_husksync() {
        let u = util::parse_uuid("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap();
        assert_eq!(
            key(KeyType::LatestSnapshot, u, ""),
            "husksync:::latest_snapshot:069a79f4-44e9-4726-a5be-fca90e38aaf5"
        );
        assert_eq!(key(KeyType::DataCheckout, u, "Main"), "husksync::main:data_checkout:069a79f4-44e9-4726-a5be-fca90e38aaf5");
        assert_eq!(MessageType::UpdateUserData.channel(""), "husksync:::update_user_data");
        assert_eq!(
            MessageType::from_channel("husksync:::check_in_petition", ""),
            Some(MessageType::CheckInPetition)
        );
    }

    #[test]
    fn messages_match_gson() {
        let u = util::parse_uuid("069a79f4-44e9-4726-a5be-fca90e38aaf5").unwrap();
        let m = Message::to_player(u, vec![0x41, 0xff]);
        assert_eq!(
            m.to_json(),
            r#"{"target_uuid":"069a79f4-44e9-4726-a5be-fca90e38aaf5","payload":[65,-1]}"#
        );
        let back = Message::from_json(r#"{"targetServer":"lobby","payload":[104,-56]}"#).unwrap();
        assert_eq!(back.target_server.as_deref(), Some("lobby"));
        assert_eq!(back.payload, vec![104, 200]);
        assert_eq!(Message::to_server("x", vec![]).to_json(), r#"{"targetServer":"x","payload":[]}"#);
    }

    #[test]
    fn percent_encodes_credentials() {
        assert_eq!(percent("p@ss:w/rd"), "p%40ss%3Aw%2Frd");
    }
}
