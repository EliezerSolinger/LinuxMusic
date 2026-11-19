use std::cell::Cell;
use std::marker::PhantomData;

/// What a thread is for (interfaces.md, I-1). Set once when the thread starts; debug checks and the profiler read it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Unknown,
    Main,
    Audio,
    Worker,
    Disk,
    Midi,
    Compile,
}

thread_local! {
    static ROLE: Cell<Role> = const { Cell::new(Role::Unknown) };
    static HAS_MAIN: Cell<bool> = const { Cell::new(false) };
}

/// Sets the role of the calling thread. The audio thread and the workers set `Audio`; threads made by the session set
/// theirs when they start
pub fn set_role(role: Role) {
    ROLE.with(|r| r.set(role));
}

/// The role of the calling thread
pub fn role() -> Role {
    ROLE.with(Cell::get)
}

/// Proof of being on the main thread (interfaces.md, I-1, S15).
///
/// At most one is alive per thread (a thread-local flag), so `cargo test`, which runs each test on its own thread, can
/// make one per test. It is neither `Send` nor `Sync`: it cannot reach another thread, so whatever holds it (the
/// session) stays on the thread that made it.
#[derive(Debug)]
pub struct Main {
    _not_send: PhantomData<*const ()>,
}

impl Main {
    /// The proof for this thread, or `None` if this thread already holds one. Marks the thread's role as `Main`
    pub fn acquire() -> Option<Main> {
        HAS_MAIN.with(|has| {
            if has.get() {
                None
            } else {
                has.set(true);
                set_role(Role::Main);
                Some(Main { _not_send: PhantomData })
            }
        })
    }
}

impl Drop for Main {
    fn drop(&mut self) {
        HAS_MAIN.with(|has| has.set(false));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_main_per_thread() {
        let first = Main::acquire();
        assert!(first.is_some());
        assert_eq!(role(), Role::Main);
        assert!(Main::acquire().is_none(), "a second proof on the same thread");
        drop(first);
        assert!(Main::acquire().is_some(), "free again once dropped");
    }

    #[test]
    fn each_thread_has_its_own() {
        let _here = Main::acquire().expect("this test's thread");
        let other = std::thread::spawn(|| Main::acquire().is_some()).join().expect("thread");
        assert!(other);
    }

    #[test]
    fn roles_are_per_thread() {
        set_role(Role::Disk);
        let other = std::thread::spawn(role).join().expect("thread");
        assert_eq!(other, Role::Unknown);
        assert_eq!(role(), Role::Disk);
    }
}
