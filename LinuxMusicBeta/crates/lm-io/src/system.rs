//! System services around the device (interfaces.md, H2, U8).
//!
//! - [`SleepGuard`]: while playing, recording or exporting, the machine must not go to sleep (H2). On Linux, the
//!   `systemd-logind` inhibitor ("sleep:idle"), asked on the D-Bus thread and polled from the pump (the main thread
//!   never waits on D-Bus); released when no longer needed. Mac (later): `beginActivity`.
//! - [`device_error`]: the open errors as the user needs them (U8): no permission explains the `audio` group; a busy
//!   card says who has it after ReserveDevice1 failed.

use std::fmt;

use lm_base::Error;

/// What the sleep inhibitor is doing
#[derive(Debug)]
enum Inhibit {
    Off,
    #[cfg(target_os = "linux")]
    Asking(lm_sys_linux::Reply<lm_sys_linux::SleepInhibitor>),
    #[cfg(target_os = "linux")]
    Held(lm_sys_linux::SleepInhibitor),
    Failed,
}

/// Keeps the system awake while the session plays, records or exports (H2). Driven from the pump: [`SleepGuard::set`]
/// with the wish, [`SleepGuard::poll`] to learn the outcome. Never blocks
pub struct SleepGuard {
    #[cfg(target_os = "linux")]
    bus: Option<std::sync::Arc<lm_sys_linux::SystemBus>>,
    state: Inhibit,
    wanted: bool,
    why: String,
    /// A failure not reported yet (one notice per session, H2)
    failure: Option<Error>,
    reported: bool,
}

impl fmt::Debug for SleepGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SleepGuard").field("state", &self.state).field("wanted", &self.wanted).finish()
    }
}

impl SleepGuard {
    /// On Linux, through the session's D-Bus thread (`None`: no bus, the guard reports it once)
    #[cfg(target_os = "linux")]
    pub fn new(bus: Option<std::sync::Arc<lm_sys_linux::SystemBus>>) -> Self {
        Self { bus, state: Inhibit::Off, wanted: false, why: String::new(), failure: None, reported: false }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn new() -> Self {
        Self { state: Inhibit::Off, wanted: false, why: String::new(), failure: None, reported: false }
    }

    /// Whether the system should stay awake, and why (people see it: "Recording")
    pub fn set(&mut self, wanted: bool, why: &str) {
        self.wanted = wanted;
        why.clone_into(&mut self.why);
        self.drive();
    }

    /// True while the system is kept awake
    pub fn is_held(&self) -> bool {
        #[cfg(target_os = "linux")]
        if matches!(self.state, Inhibit::Held(_)) {
            return true;
        }
        false
    }

    /// From the pump: advances the request; returns the failure once (`linux.logind.unavailable`,
    /// `linux.dbus.unavailable`, `io.sleep.unsupported`)
    pub fn poll(&mut self) -> Option<Error> {
        self.drive();
        if self.reported {
            return None;
        }
        let f = self.failure.take();
        if f.is_some() {
            self.reported = true;
        }
        f
    }

    fn drive(&mut self) {
        #[cfg(target_os = "linux")]
        {
            let state = std::mem::replace(&mut self.state, Inhibit::Off);
            self.state = match (state, self.wanted) {
                (Inhibit::Off, true) => match &self.bus {
                    Some(bus) => Inhibit::Asking(bus.inhibit_sleep("LinuxMusic", &self.why)),
                    None => {
                        self.failure.get_or_insert_with(|| Error::new("linux.dbus.unavailable").arg("bus", "system"));
                        Inhibit::Failed
                    }
                },
                (Inhibit::Asking(reply), wanted) => match reply.try_take() {
                    None => Inhibit::Asking(reply),
                    Some(Ok(lock)) if wanted => Inhibit::Held(lock),
                    Some(Ok(lock)) => {
                        drop(lock);
                        Inhibit::Off
                    }
                    Some(Err(e)) => {
                        self.failure.get_or_insert(e);
                        Inhibit::Failed
                    }
                },
                (Inhibit::Held(lock), false) => {
                    drop(lock);
                    Inhibit::Off
                }
                // A failed inhibitor is not asked again in a loop; a new session asks again
                (Inhibit::Failed, _) => Inhibit::Failed,
                (other, _) => other,
            };
        }
        #[cfg(not(target_os = "linux"))]
        {
            if self.wanted && matches!(self.state, Inhibit::Off) {
                self.failure.get_or_insert_with(|| Error::new("io.sleep.unsupported"));
                self.state = Inhibit::Failed;
            }
        }
    }
}

/// The open errors of a device as the user needs them (U8): `io.device.permission_denied` (with `audio_group`: 1 if
/// the user is already in the `audio` group, so the fix is to log in again), `io.device.busy` (with `owner` when
/// ReserveDevice1 told who refused), `io.device.not_found`; anything else as it came
pub fn device_error(e: Error, device: &str) -> Error {
    match e.key() {
        "alsa.permission_denied" => {
            #[cfg(target_os = "linux")]
            let in_group = i64::from(lm_sys_linux::in_audio_group().unwrap_or(false));
            #[cfg(not(target_os = "linux"))]
            let in_group = 0i64;
            Error::new("io.device.permission_denied").arg("device", device).arg("audio_group", in_group)
        }
        "alsa.device_busy" => Error::new("io.device.busy").arg("device", device),
        "linux.reserve.refused" => {
            let owner = e
                .label
                .args
                .iter()
                .find(|(k, _)| *k == "owner")
                .map(|(_, v)| v.clone())
                .unwrap_or(lm_base::Arg::Text(String::new()));
            Error::new("io.device.busy").arg("device", device).arg("owner", owner)
        }
        "alsa.device_not_found" => Error::new("io.device.not_found").arg("device", device),
        _ => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_errors_explain_themselves() {
        let e = device_error(Error::new("alsa.permission_denied"), "hw:0");
        assert_eq!(e.key(), "io.device.permission_denied");
        assert!(e.label.args.iter().any(|(k, _)| *k == "audio_group"));
        let e = device_error(Error::new("linux.reserve.refused").arg("owner", "PipeWire"), "hw:0");
        assert_eq!(e.key(), "io.device.busy");
        assert!(e.label.args.iter().any(|(k, v)| *k == "owner" && *v == lm_base::Arg::Text("PipeWire".into())));
        assert_eq!(device_error(Error::new("alsa.xrun"), "hw:0").key(), "alsa.xrun");
    }
}
