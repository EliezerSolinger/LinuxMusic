//! The ReserveDevice1 protocol (U8; the user's decision of 12/11), as JACK speaks it: before opening an ALSA card in
//! exclusive mode, own the session-bus name `org.freedesktop.ReserveDevice1.Audio<N>`. If another program owns it
//! (PipeWire, PulseAudio), call its `RequestRelease(priority)`; if it agrees, it lets the card go and allowed its
//! name to be replaced, so we take the name over. While we hold the name, our object answers other programs'
//! `RequestRelease` with **no** (and counts the requests, so the session can tell the user).
//!
//! Protocol: <http://git.0pointer.net/reserve.git/tree/reserve.txt>.

use super::bus::dbus_error;
use lm_base::{Error, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use zbus::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};

const INTERFACE: &str = "org.freedesktop.ReserveDevice1";

pub(crate) fn bus_name(card: u32) -> String {
    format!("org.freedesktop.ReserveDevice1.Audio{card}")
}

pub(crate) fn object_path(card: u32) -> String {
    format!("/org/freedesktop/ReserveDevice1/Audio{card}")
}

/// Our object on the bus while we hold a card
pub(crate) struct ReserveDevice {
    pub priority: i32,
    pub application_name: String,
    pub device_name: String,
    pub release_requests: Arc<AtomicU32>,
}

#[zbus::interface(name = "org.freedesktop.ReserveDevice1")]
impl ReserveDevice {
    /// Another program wants the card. A recording program does not hand its card over in the middle of a take: the
    /// answer is no, and the request is counted
    async fn request_release(&self, priority: i32) -> bool {
        let _ = priority;
        self.release_requests.fetch_add(1, Ordering::Relaxed);
        false
    }

    #[zbus(property)]
    async fn priority(&self) -> i32 {
        self.priority
    }

    #[zbus(property)]
    async fn application_name(&self) -> String {
        self.application_name.clone()
    }

    #[zbus(property)]
    async fn application_device_name(&self) -> String {
        self.device_name.clone()
    }
}

fn failed(e: &zbus::Error) -> Error {
    dbus_error("linux.reserve.failed", e)
}

/// The current owner's `ApplicationName`, when it says
async fn owner_name(conn: &Connection, name: &str, path: &str) -> Option<String> {
    let msg = conn
        .call_method(Some(name), path, Some("org.freedesktop.DBus.Properties"), "Get", &(INTERFACE, "ApplicationName"))
        .await
        .ok()?;
    let v: zbus::OwnedVariant = msg.body().deserialize().ok()?;
    String::try_from(v).ok()
}

async fn take_name(conn: &Connection, name: &str, replace: bool) -> Result<RequestNameReply> {
    let mut flags = RequestNameFlags::DO_NOT_QUEUE;
    if replace {
        flags |= RequestNameFlags::REPLACE_EXISTING;
    }
    // zbus turns the bus's `Exists` reply into `Error::NameTaken`; here it is an answer, not a failure: the name has
    // an owner, who is asked to release it (or, after a yes, did not allow the replacement)
    match conn.request_name_with_flags(name, flags).await {
        Ok(reply) => Ok(reply),
        Err(zbus::Error::NameTaken) => Ok(RequestNameReply::Exists),
        Err(e) => Err(failed(&e)),
    }
}

/// Owns the name for `object`'s card, asking the owner to release it if needed
pub(crate) async fn acquire(
    conn: &Connection,
    name: &str,
    path: &str,
    object: ReserveDevice,
    priority: i32,
) -> Result<()> {
    // The object first, so a request arriving right after we own the name is answered
    // `false` means an object is already there (an earlier reservation of this card not yet released)
    if !conn.object_server().at(path, object).await.map_err(|e| failed(&e))? {
        return Err(Error::new("linux.reserve.failed").arg("detail", "the card is already reserved by this process"));
    }
    let result = negotiate(conn, name, path, priority).await;
    if result.is_err() {
        let _ = conn.object_server().remove::<ReserveDevice, _>(path).await;
    }
    result
}

async fn negotiate(conn: &Connection, name: &str, path: &str, priority: i32) -> Result<()> {
    // Twice at most: the owner may vanish between our two calls
    for _ in 0..2 {
        match take_name(conn, name, false).await? {
            RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => return Ok(()),
            RequestNameReply::InQueue => {
                return Err(Error::new("linux.reserve.failed").arg("detail", "queued despite DO_NOT_QUEUE"));
            }
            RequestNameReply::Exists => {}
        }
        let owner = owner_name(conn, name, path).await;
        let answer = conn.call_method(Some(name), path, Some(INTERFACE), "RequestRelease", &(priority,)).await;
        let agreed: bool = match answer {
            Ok(msg) => msg.body().deserialize().map_err(|e| failed(&e))?,
            // The owner went away meanwhile: try again
            Err(zbus::Error::MethodError(err, ..))
                if err.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown"
                    || err.as_str() == "org.freedesktop.DBus.Error.NameHasNoOwner" =>
            {
                continue;
            }
            Err(e) => return Err(failed(&e)),
        };
        if !agreed {
            let e = Error::new("linux.reserve.refused").arg("name", name);
            return Err(match owner {
                Some(o) => e.arg("owner", o),
                None => e,
            });
        }
        return match take_name(conn, name, true).await? {
            RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => Ok(()),
            // It said yes but did not allow the name to be replaced
            _ => Err(Error::new("linux.reserve.refused").arg("name", name).arg("detail", "the owner kept the name")),
        };
    }
    Err(Error::new("linux.reserve.failed").arg("name", name).arg("detail", "the owner changed twice"))
}

/// Gives the name back and removes our object
pub(crate) async fn release(conn: &Connection, name: &str, path: &str) -> Result<()> {
    conn.release_name(name).await.map_err(|e| failed(&e))?;
    conn.object_server().remove::<ReserveDevice, _>(path).await.map_err(|e| failed(&e))?;
    Ok(())
}
