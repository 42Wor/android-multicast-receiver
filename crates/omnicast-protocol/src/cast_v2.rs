//! Minimal Google Cast V2 framing + JSON namespace handlers.
//!
//! Wire format: big-endian u32 length + protobuf `CastMessage`.

use serde_json::{json, Value};
use thiserror::Error;

pub const NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
pub const NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
pub const NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";

#[derive(Debug, Error)]
pub enum CastV2Error {
    #[error("invalid cast frame: {0}")]
    Frame(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct CastMessage {
    pub protocol_version: i32,
    pub source_id: String,
    pub destination_id: String,
    pub namespace: String,
    pub payload_type: i32,
    pub payload_utf8: String,
}

impl CastMessage {
    pub fn json_reply(&self, payload: Value) -> Self {
        Self {
            protocol_version: 0,
            source_id: "receiver-0".into(),
            destination_id: self.source_id.clone(),
            namespace: self.namespace.clone(),
            payload_type: 0, // STRING
            payload_utf8: payload.to_string(),
        }
    }

    pub fn payload_json(&self) -> Result<Value, CastV2Error> {
        Ok(serde_json::from_str(&self.payload_utf8)?)
    }
}

/// Encode a CastMessage into a length-prefixed wire frame.
pub fn encode_frame(msg: &CastMessage) -> Vec<u8> {
    let body = encode_cast_message(msg);
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// Try to pull one complete frame from `buf`. Returns `(message, consumed_bytes)`.
pub fn try_decode_frame(buf: &[u8]) -> Result<Option<(CastMessage, usize)>, CastV2Error> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len == 0 || len > 1024 * 1024 {
        return Err(CastV2Error::Frame(format!("invalid frame length {len}")));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let msg = decode_cast_message(&buf[4..4 + len])?;
    Ok(Some((msg, 4 + len)))
}

/// Handle one inbound CastMessage; return zero or more reply frames.
pub fn handle_message(msg: &CastMessage) -> Result<Vec<Vec<u8>>, CastV2Error> {
    let payload = match msg.payload_json() {
        Ok(v) => v,
        Err(_) => {
            return Ok(Vec::new());
        }
    };
    let msg_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    match msg.namespace.as_str() {
        NS_CONNECTION => {
            // CONNECT / CLOSE — no mandatory reply; channel is considered open.
            Ok(Vec::new())
        }
        NS_HEARTBEAT if msg_type == "PING" => {
            let reply = msg.json_reply(json!({ "type": "PONG" }));
            Ok(vec![encode_frame(&reply)])
        }
        NS_RECEIVER if msg_type == "GET_STATUS" => {
            let request_id = payload.get("requestId").cloned().unwrap_or(json!(1));
            let reply = msg.json_reply(json!({
                "requestId": request_id,
                "type": "RECEIVER_STATUS",
                "status": {
                    "applications": [],
                    "isActiveInput": true,
                    "isStandBy": false,
                    "userEq": {},
                    "volume": {
                        "controlType": "attenuation",
                        "level": 1.0,
                        "muted": false,
                        "stepInterval": 0.05
                    }
                }
            }));
            Ok(vec![encode_frame(&reply)])
        }
        NS_RECEIVER if msg_type == "LAUNCH" || msg_type == "STOP" => {
            // Acknowledge with a status snapshot so senders don't hang.
            let request_id = payload.get("requestId").cloned().unwrap_or(json!(1));
            let reply = msg.json_reply(json!({
                "requestId": request_id,
                "type": "RECEIVER_STATUS",
                "status": {
                    "applications": [],
                    "isActiveInput": true,
                    "isStandBy": false,
                    "userEq": {},
                    "volume": {
                        "controlType": "attenuation",
                        "level": 1.0,
                        "muted": false,
                        "stepInterval": 0.05
                    }
                }
            }));
            Ok(vec![encode_frame(&reply)])
        }
        _ => Ok(Vec::new()),
    }
}

fn encode_cast_message(msg: &CastMessage) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint_field(&mut out, 1, msg.protocol_version as u64);
    write_string_field(&mut out, 2, &msg.source_id);
    write_string_field(&mut out, 3, &msg.destination_id);
    write_string_field(&mut out, 4, &msg.namespace);
    write_varint_field(&mut out, 5, msg.payload_type as u64);
    if !msg.payload_utf8.is_empty() {
        write_string_field(&mut out, 6, &msg.payload_utf8);
    }
    out
}

fn decode_cast_message(data: &[u8]) -> Result<CastMessage, CastV2Error> {
    let mut msg = CastMessage {
        protocol_version: 0,
        source_id: String::new(),
        destination_id: String::new(),
        namespace: String::new(),
        payload_type: 0,
        payload_utf8: String::new(),
    };
    let mut i = 0;
    while i < data.len() {
        let (tag, n) = read_varint(&data[i..])?;
        i += n;
        let field = (tag >> 3) as u32;
        let wire = (tag & 0x7) as u8;
        match (field, wire) {
            (1, 0) => {
                let (v, n) = read_varint(&data[i..])?;
                i += n;
                msg.protocol_version = v as i32;
            }
            (2, 2) => {
                let (s, n) = read_string(&data[i..])?;
                i += n;
                msg.source_id = s;
            }
            (3, 2) => {
                let (s, n) = read_string(&data[i..])?;
                i += n;
                msg.destination_id = s;
            }
            (4, 2) => {
                let (s, n) = read_string(&data[i..])?;
                i += n;
                msg.namespace = s;
            }
            (5, 0) => {
                let (v, n) = read_varint(&data[i..])?;
                i += n;
                msg.payload_type = v as i32;
            }
            (6, 2) => {
                let (s, n) = read_string(&data[i..])?;
                i += n;
                msg.payload_utf8 = s;
            }
            (7, 2) => {
                // binary payload — skip
                let (len, n) = read_varint(&data[i..])?;
                i += n + len as usize;
            }
            (_, 0) => {
                let (_, n) = read_varint(&data[i..])?;
                i += n;
            }
            (_, 1) => {
                i += 8;
            }
            (_, 2) => {
                let (len, n) = read_varint(&data[i..])?;
                i += n + len as usize;
            }
            (_, 5) => {
                i += 4;
            }
            _ => {
                return Err(CastV2Error::Frame(format!(
                    "unsupported wire type {wire} for field {field}"
                )));
            }
        }
    }
    Ok(msg)
}

fn write_varint_field(out: &mut Vec<u8>, field: u32, value: u64) {
    write_varint(out, ((field as u64) << 3) | 0);
    write_varint(out, value);
}

fn write_string_field(out: &mut Vec<u8>, field: u32, value: &str) {
    write_varint(out, ((field as u64) << 3) | 2);
    write_varint(out, value.len() as u64);
    out.extend_from_slice(value.as_bytes());
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_varint(data: &[u8]) -> Result<(u64, usize), CastV2Error> {
    let mut value = 0u64;
    let mut shift = 0;
    for (i, byte) in data.iter().copied().enumerate() {
        if i >= 10 {
            return Err(CastV2Error::Frame("varint too long".into()));
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
        shift += 7;
    }
    Err(CastV2Error::Frame("truncated varint".into()))
}

fn read_string(data: &[u8]) -> Result<(String, usize), CastV2Error> {
    let (len, n) = read_varint(data)?;
    let len = len as usize;
    if data.len() < n + len {
        return Err(CastV2Error::Frame("truncated string".into()));
    }
    let s = String::from_utf8_lossy(&data[n..n + len]).into_owned();
    Ok((s, n + len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_connect_like_message() {
        let original = CastMessage {
            protocol_version: 0,
            source_id: "sender-0".into(),
            destination_id: "receiver-0".into(),
            namespace: NS_CONNECTION.into(),
            payload_type: 0,
            payload_utf8: r#"{"type":"CONNECT"}"#.into(),
        };
        let frame = encode_frame(&original);
        let (decoded, consumed) = try_decode_frame(&frame).unwrap().unwrap();
        assert_eq!(consumed, frame.len());
        assert_eq!(decoded.source_id, "sender-0");
        assert_eq!(decoded.destination_id, "receiver-0");
        assert_eq!(decoded.namespace, NS_CONNECTION);
        assert_eq!(decoded.payload_utf8, original.payload_utf8);
    }

    #[test]
    fn ping_gets_pong() {
        let ping = CastMessage {
            protocol_version: 0,
            source_id: "sender-0".into(),
            destination_id: "receiver-0".into(),
            namespace: NS_HEARTBEAT.into(),
            payload_type: 0,
            payload_utf8: r#"{"type":"PING"}"#.into(),
        };
        let replies = handle_message(&ping).unwrap();
        assert_eq!(replies.len(), 1);
        let (msg, _) = try_decode_frame(&replies[0]).unwrap().unwrap();
        assert_eq!(msg.source_id, "receiver-0");
        assert_eq!(msg.destination_id, "sender-0");
        assert!(msg.payload_utf8.contains("PONG"));
    }
}
