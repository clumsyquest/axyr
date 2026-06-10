//! The engine's end of the wire: a [`ProbeLink`] that executes every primitive
//! on a remote agent over one WebSocket.
//!
//! One dedicated thread owns the socket (see [`serve_socket`]): it forwards
//! queued requests, routes replies back, and buffers the telemetry the agent
//! pushes. [`RemoteLink`] itself never touches the socket, so the engine's
//! owner loop stays single-threaded and oblivious to the transport.

use std::io::{ErrorKind, Read, Write};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::Value;
use tungstenite::{Message, WebSocket};

use crate::link::ProbeLink;
use crate::wire::{self, AgentFrame, Op, Request};

/// A primitive in flight: the op to send and where to deliver its reply.
/// Opaque outside this module — it only exists to be handed to [`serve_socket`].
pub struct Pending {
    op: Op,
    reply: Sender<Result<Value, String>>,
}

/// How long to wait for a primitive's reply. Flash rewrites the whole image
/// (and re-verifies), so it gets a far longer leash.
const REPLY_TIMEOUT: Duration = Duration::from_secs(15);
const FLASH_TIMEOUT: Duration = Duration::from_secs(180);

pub struct RemoteLink {
    req_tx: Sender<Pending>,
    telemetry_rx: Receiver<Vec<u8>>,
    next_id: u64,
}

impl RemoteLink {
    /// Wire a new link to a socket thread. Returns the link plus the channel
    /// ends the socket thread consumes (pass them to [`serve_socket`]).
    pub fn new() -> (Self, Receiver<Pending>, Sender<Vec<u8>>) {
        let (req_tx, req_rx) = channel();
        let (tel_tx, telemetry_rx) = channel();
        (Self { req_tx, telemetry_rx, next_id: 0 }, req_rx, tel_tx)
    }

    fn request(&mut self, op: Op) -> Result<Value, String> {
        let timeout = match op {
            Op::Flash { .. } => FLASH_TIMEOUT,
            _ => REPLY_TIMEOUT,
        };
        self.next_id += 1;
        let (reply, rx) = channel();
        self.req_tx
            .send(Pending { op, reply })
            .map_err(|_| "agent link is down".to_string())?;
        rx.recv_timeout(timeout)
            .map_err(|_| "agent did not reply (link lost?)".to_string())?
    }
}

impl ProbeLink for RemoteLink {
    fn poll_telemetry(&mut self, out: &mut Vec<u8>) -> Result<(), String> {
        // The agent pushes; we only drain what already arrived. Never blocks.
        while let Ok(chunk) = self.telemetry_rx.try_recv() {
            out.extend_from_slice(&chunk);
        }
        Ok(())
    }

    fn resync_telemetry(&mut self) {
        let _ = self.request(Op::ResyncTelemetry);
    }

    fn read_word(&mut self, address: u64) -> Result<u32, String> {
        let v = self.request(Op::ReadWord { addr: address })?;
        v.as_u64().map(|x| x as u32).ok_or_else(|| format!("bad read_word reply: {v}"))
    }

    fn read_words(&mut self, address: u64, out: &mut [u32]) -> Result<(), String> {
        let v = self.request(Op::ReadWords { addr: address, count: out.len() })?;
        let arr = v.as_array().ok_or_else(|| format!("bad read_words reply: {v}"))?;
        if arr.len() != out.len() {
            return Err(format!("read_words: got {} of {} words", arr.len(), out.len()));
        }
        for (slot, item) in out.iter_mut().zip(arr) {
            *slot = item.as_u64().ok_or("bad word in reply")? as u32;
        }
        Ok(())
    }

    fn read_cstring(&mut self, address: u64, max: usize) -> Result<String, String> {
        let v = self.request(Op::ReadCstring { addr: address, max })?;
        v.as_str().map(String::from).ok_or_else(|| format!("bad read_cstring reply: {v}"))
    }

    fn read_coredump(&mut self, base: u64) -> Result<Option<Vec<u8>>, String> {
        let v = self.request(Op::ReadCoredump { base })?;
        match v.get("data_b64").and_then(Value::as_str) {
            None => Ok(None),
            Some(b64) => B64
                .decode(b64)
                .map(Some)
                .map_err(|e| format!("bad coredump payload: {e}")),
        }
    }

    fn status(&mut self) -> Result<String, String> {
        let v = self.request(Op::Status)?;
        v.as_str().map(String::from).ok_or_else(|| format!("bad status reply: {v}"))
    }

    fn reset(&mut self) -> Result<(), String> {
        self.request(Op::Reset).map(|_| ())
    }

    fn flash(&mut self, elf: &[u8]) -> Result<(), String> {
        self.request(Op::Flash { elf_b64: B64.encode(elf) }).map(|_| ())
    }
}

/// Own the agent socket: forward queued requests, route replies, buffer
/// telemetry pushes. Runs until the socket dies; the engine sees the loss as
/// request errors (and an eventual reconnect replaces the whole session).
///
/// The stream must have a short read timeout set, so the loop can interleave
/// request writes between reads.
pub fn serve_socket<S: Read + Write>(
    mut ws: WebSocket<S>,
    req_rx: Receiver<Pending>,
    tel_tx: Sender<Vec<u8>>,
) {
    let mut pending: Option<Sender<Result<Value, String>>> = None;
    let mut next_id: u64 = 0;

    loop {
        // Requests are sequential (one owner loop drives them), so at most one
        // is in flight; forward it as soon as it shows up.
        if pending.is_none()
            && let Ok(p) = req_rx.try_recv()
        {
            next_id += 1;
            let text = wire::encode(&Request { id: next_id, op: p.op });
            if ws.send(Message::Text(text.into())).is_err() {
                let _ = p.reply.send(Err("agent link is down".to_string()));
                return;
            }
            pending = Some(p.reply);
        }

        match ws.read() {
            Ok(Message::Text(text)) => match wire::decode::<AgentFrame>(text.as_str()) {
                Ok(AgentFrame::Telemetry { data_b64 }) => {
                    if let Ok(bytes) = B64.decode(&data_b64)
                        && !bytes.is_empty()
                    {
                        let _ = tel_tx.send(bytes);
                    }
                }
                Ok(AgentFrame::Reply { ok, err, .. }) => {
                    if let Some(reply) = pending.take() {
                        let _ = reply.send(match err {
                            Some(e) => Err(e),
                            None => Ok(ok.unwrap_or(Value::Null)),
                        });
                    }
                }
                Err(e) => eprintln!("engine: bad frame from agent: {e}"),
            },
            Ok(Message::Close(_)) => {
                eprintln!("engine: agent disconnected");
                return;
            }
            Ok(_) => {} // ping/pong/binary: ignored
            Err(tungstenite::Error::Io(e))
                if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
            Err(e) => {
                eprintln!("engine: agent socket error: {e}");
                if let Some(reply) = pending.take() {
                    let _ = reply.send(Err("agent link is down".to_string()));
                }
                return;
            }
        }
    }
}
