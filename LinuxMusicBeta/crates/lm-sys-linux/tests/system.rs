//! Limits, scheduling and memory locking on Linux, and the requests to the system bus (rtkit, logind), which skip
//! with a message when the machine has no system bus or no rtkit (CI containers).
#![cfg(target_os = "linux")]
#![allow(clippy::print_stderr)]

use lm_sys_linux::{
    LockedBuffer, SchedPolicy, SystemBus, current_policy, current_thread_id, in_audio_group, lock_all,
    lock_current_stack, make_current_thread_realtime, memlock_limit, prefault_stack, resource_limits,
};
use std::time::Duration;

fn is_root() -> bool {
    // SAFETY: plain system call
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn limits_and_groups_are_readable() {
    let limits = resource_limits().expect("limits");
    eprintln!("limits: {limits:?}; audio group: {:?}", in_audio_group());
    in_audio_group().expect("groups");
    assert!(current_thread_id().0 > 0);
}

#[test]
fn a_locked_buffer_within_the_limit_is_usable() {
    let mut buf = LockedBuffer::<f32>::zeroed(4096).expect("16 KiB within any default limit");
    assert!(buf.iter().all(|x| *x == 0.0));
    buf[4095] = 1.5;
    assert_eq!(buf[4095], 1.5);
    assert_eq!(buf.len(), 4096);
    assert!(buf.bytes_locked() >= 16 * 1024);
    let empty = LockedBuffer::<u8>::zeroed(0).expect("empty");
    assert!(empty.is_empty());
}

#[test]
fn locking_beyond_the_limit_fails_clearly() {
    let limit = memlock_limit().expect("limit");
    let Some(soft) = limit.soft else {
        eprintln!("SKIPPED: RLIMIT_MEMLOCK is unlimited here; nothing to exceed");
        return;
    };
    if is_root() || soft > 1 << 30 {
        eprintln!("SKIPPED: root (CAP_IPC_LOCK) or a limit too large to exceed in a test ({soft})");
        return;
    }
    let e = LockedBuffer::<u8>::zeroed(soft as usize * 2 + 4096).expect_err("twice the limit");
    assert_eq!(e.key(), "linux.memlock.denied", "{e}");
}

#[test]
fn lock_all_only_when_unlimited() {
    let limit = memlock_limit().expect("limit");
    match lock_all() {
        Ok(guard) => {
            assert!(limit.soft.is_none());
            drop(guard);
        }
        Err(e) => {
            assert!(limit.soft.is_some(), "unlimited, but: {e}");
            assert_eq!(e.key(), "linux.memlock.limited");
        }
    }
}

#[test]
fn an_audio_thread_stack_is_prefaulted_and_locked() {
    let result = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            prefault_stack(128 * 1024);
            lock_current_stack(64 * 1024).map(|lock| lock.bytes())
        })
        .expect("thread")
        .join()
        .expect("join");
    let limit = memlock_limit().expect("limit");
    match result {
        Ok(bytes) => assert!(bytes >= 64 * 1024),
        Err(e) => {
            assert!(limit.soft.is_some_and(|s| s < 128 * 1024) && !is_root(), "{e}");
            assert_eq!(e.key(), "linux.memlock.denied");
        }
    }
}

#[test]
fn realtime_is_granted_or_the_error_says_what_is_missing() {
    let bus = SystemBus::start().expect("bus thread");
    let outcome = std::thread::spawn(move || {
        let before = current_policy();
        let result = make_current_thread_realtime(10, Some(&bus), Duration::from_secs(10));
        let during = current_policy();
        let result = result.map(|guard| {
            let method = guard.method();
            drop(guard);
            (method, current_policy())
        });
        (before, during, result)
    })
    .join()
    .expect("join");
    match outcome {
        (before, during, Ok((method, after))) => {
            eprintln!("real time granted by {method:?}");
            assert!(matches!(during, SchedPolicy::Fifo(_)), "{during:?}");
            assert_eq!(after, before, "the guard did not restore the policy");
        }
        (_, _, Err(e)) => {
            eprintln!("real time denied here (expected on CI runners): {e}");
            assert_eq!(e.key(), "linux.realtime.denied");
            for arg in ["priority", "rtprio_limit", "audio_group", "rtkit"] {
                assert!(e.label.args.iter().any(|(name, _)| *name == arg), "{arg} missing in {e}");
            }
        }
    }
}

#[test]
fn rtkit_and_logind_answer_or_say_why_not() {
    let bus = SystemBus::start().expect("bus thread");
    match bus.rtkit_info().wait(Duration::from_secs(15)) {
        Ok(info) => {
            assert!(info.max_realtime_priority >= 1);
            assert!(info.rttime_usec_max > 0);
        }
        Err(e) => {
            eprintln!("SKIPPED rtkit: {e}");
            assert!(e.key().starts_with("linux.dbus.") || e.key() == "linux.rtkit.unavailable", "{e}");
        }
    }
    // The reply is polled by the main thread without blocking
    let reply = bus.inhibit_sleep("LinuxMusic tests", "Testing the inhibitor");
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let result = loop {
        if let Some(r) = reply.try_take() {
            break r;
        }
        assert!(std::time::Instant::now() < deadline, "no reply from the D-Bus thread");
        std::thread::sleep(Duration::from_millis(10));
    };
    match result {
        Ok(inhibitor) => drop(inhibitor),
        Err(e) => {
            eprintln!("SKIPPED logind: {e}");
            assert!(e.key().starts_with("linux.dbus.") || e.key() == "linux.logind.unavailable", "{e}");
        }
    }
    assert!(bus.shutdown(Duration::from_secs(10)), "the D-Bus thread did not stop");
}
