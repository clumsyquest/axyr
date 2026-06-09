//! Non-intrusive telemetry over SEGGER RTT.
//!
//! RTT replaces the serial (UART) channel. The firmware writes telemetry to a
//! RAM ring buffer (no halt, a few cycles); the host drains it over the debug
//! probe. It is deterministic even while the core sleeps (WFI): the data is
//! buffered, so a read that misses a sleeping window simply retries and loses
//! nothing — unlike polling a variable, which would read a false zero.

use probe_rs::Session;
use probe_rs::rtt::Rtt;

/// A live RTT telemetry channel attached to the target.
pub struct Telemetry {
    rtt: Rtt,
}

impl Telemetry {
    /// Find the RTT control block in target RAM and attach to it. The scan needs
    /// an awake window, so the caller may want to retry while the core sleeps.
    pub fn attach(session: &mut Session) -> Result<Self, String> {
        let mut core = session.core(0).map_err(|e| format!("core: {e}"))?;
        let rtt = Rtt::attach(&mut core).map_err(|e| format!("attach RTT: {e}"))?;
        Ok(Self { rtt })
    }

    /// Drain whatever bytes are available on up-channel 0 into `buf`, returning
    /// how many were read (0 if none — just call again).
    pub fn read(&mut self, session: &mut Session, buf: &mut [u8]) -> Result<usize, String> {
        let mut core = session.core(0).map_err(|e| format!("core: {e}"))?;
        let channel = self
            .rtt
            .up_channels()
            .get_mut(0)
            .ok_or("no RTT up-channel")?;
        channel
            .read(&mut core, buf)
            .map_err(|e| format!("rtt read: {e}"))
    }
}
