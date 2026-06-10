//! The agent⇄engine wire protocol: JSON text frames over one WebSocket.
//!
//! The agent is **dumb on purpose**: it owns the probe and answers raw
//! primitive requests (read words, reset, flash bytes) — the exact surface of
//! [`crate::link::ProbeLink`] — and autonomously pushes telemetry chunks so the
//! engine never has to poll RTT over the WAN. All interpretation (ELF, DWARF,
//! SVD, devicetree) lives engine-side; the agent ships nothing worth
//! reverse-engineering.
//!
//! Frames are small JSON objects; bulk payloads (ELF, telemetry, coredump,
//! flash images) ride inside as base64. Requests are strictly sequential (the
//! engine's owner loop serializes them), so `id` is a sanity check, not a
//! routing key.

use serde::{Deserialize, Serialize};

/// Protocol version — bump on breaking changes; both ends check it in Hello.
pub const WIRE_VERSION: u32 = 1;

/// First message after connect, agent → engine: who/what is plugged in, plus
/// the build artifacts the engine's analysis needs (the ELF is the contract —
/// symbols, DWARF; the DTS gives the system map; the SOC string, the chip).
#[derive(Serialize, Deserialize, Debug)]
pub struct Hello {
    pub wire_version: u32,
    pub agent_version: String,
    /// Probe identity (name, serial), if one was listed.
    pub probe: Option<(String, Option<String>)>,
    /// The probe-rs target we attached to and how it was detected.
    pub chip: String,
    pub detection: String,
    /// True when attached as a generic Cortex-M (observation only).
    pub generic: bool,
    /// Silicon identity (CPUID / ST device id), when readable.
    pub cpuid: Option<u32>,
    pub devid: Option<u32>,
    /// Firmware build artifacts, inline.
    pub elf_b64: String,
    pub dts: Option<String>,
    /// Watched globals (AXYR_WATCH on the agent side).
    pub watch: Vec<String>,
}

/// Engine → agent: one primitive request.
#[derive(Serialize, Deserialize, Debug)]
pub struct Request {
    pub id: u64,
    pub op: Op,
}

/// The raw primitives — a 1:1 mirror of [`crate::link::ProbeLink`].
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    ReadWord { addr: u64 },
    ReadWords { addr: u64, count: usize },
    ReadCstring { addr: u64, max: usize },
    ReadCoredump { base: u64 },
    Status,
    Reset,
    Flash { elf_b64: String },
    ResyncTelemetry,
}

/// Agent → engine: every frame the agent sends after Hello.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentFrame {
    /// Reply to a [`Request`], carrying its `id`.
    Reply {
        id: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        ok: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        err: Option<String>,
    },
    /// Telemetry bytes drained from RTT, pushed on the agent's own cadence.
    Telemetry { data_b64: String },
}

pub fn encode<T: Serialize>(msg: &T) -> String {
    serde_json::to_string(msg).expect("wire message serializes")
}

pub fn decode<'a, T: Deserialize<'a>>(text: &'a str) -> Result<T, String> {
    serde_json::from_str(text).map_err(|e| format!("wire decode: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ops_roundtrip() {
        let req = Request { id: 7, op: Op::ReadWords { addr: 0x2000_0000, count: 16 } };
        let back: Request = decode(&encode(&req)).unwrap();
        assert_eq!(back.id, 7);
        match back.op {
            Op::ReadWords { addr, count } => {
                assert_eq!(addr, 0x2000_0000);
                assert_eq!(count, 16);
            }
            other => panic!("wrong op: {other:?}"),
        }
    }

    #[test]
    fn reply_skips_empty_fields() {
        let frame = AgentFrame::Reply { id: 1, ok: Some(serde_json::json!(42)), err: None };
        let text = encode(&frame);
        assert!(!text.contains("err"), "{text}");
        let back: AgentFrame = decode(&text).unwrap();
        match back {
            AgentFrame::Reply { id: 1, ok: Some(v), err: None } => assert_eq!(v, 42),
            other => panic!("wrong frame: {other:?}"),
        }
    }
}
