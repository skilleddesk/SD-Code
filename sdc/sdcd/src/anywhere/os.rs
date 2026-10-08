//! Two things the operating system knows and the daemon needs (plan 3.4):
//!
//! * **idle** - how long since the person touched the keyboard or mouse, so "tell my phone only when
//!   I am away from the desk" (`notify_when = "idle"`) can be honoured;
//! * **sleep** - keeping the machine awake while work is running or an approval is waiting, so a
//!   request is not stranded by the PC going to sleep.
//!
//! | OS      | idle                           | keep awake                                    |
//! | ------- | ------------------------------ | --------------------------------------------- |
//! | Windows | `GetLastInputInfo`             | `SetThreadExecutionState` on a dedicated thread |
//! | macOS   | `ioreg` `HIDIdleTime`          | `caffeinate -i -w <pid>`                       |
//! | Linux   | `loginctl` session `IdleHint`  | `systemd-inhibit ... sleep infinity`           |
//!
//! Every function degrades to "unknown" or "not held" rather than failing: an approval must still be
//! deliverable on a machine where one of these tools is missing.

use std::time::Duration;

/// Seconds since the last keyboard or mouse input, or `None` when the OS does not say.
pub fn idle_seconds() -> Option<u64> {
    imp::idle_seconds()
}

/// A hold on the machine's sleep. Dropping it releases the hold.
pub struct SleepGuard {
    inner: imp::Hold,
}

impl SleepGuard {
    /// Tries to keep the machine awake. `None` when this OS could not be asked.
    pub fn acquire() -> Option<Self> {
        imp::hold().map(|inner| Self { inner })
    }

    /// Whether the hold is real (the helper process or thread is running).
    pub fn is_active(&self) -> bool {
        self.inner.is_active()
    }
}

/// Whether the person has been away for at least `minutes`. Unknown idle time counts as "at the desk":
/// when in doubt, the desktop already shows the card.
pub fn away_for(minutes: u64) -> bool {
    idle_seconds().is_some_and(|seconds| Duration::from_secs(seconds) >= Duration::from_secs(minutes * 60))
}

#[cfg(windows)]
mod imp {
    use std::sync::mpsc::{channel, Sender};
    use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

    use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

    pub fn idle_seconds() -> Option<u64> {
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };

        // SAFETY: `info` is a correctly sized, initialised LASTINPUTINFO that lives for the call.
        let ok = unsafe { GetLastInputInfo(&mut info) };

        if ok == 0 {
            return None;
        }

        // SAFETY: GetTickCount takes no arguments and has no preconditions.
        let now = unsafe { GetTickCount() };

        /* Both are 32-bit tick counts that wrap together; the wrapping difference is right. */
        Some(u64::from(now.wrapping_sub(info.dwTime)) / 1000)
    }

    /// `SetThreadExecutionState` is per thread, so the hold lives on a thread of its own that waits for
    /// the release message and clears the state itself.
    pub struct Hold {
        release: Option<Sender<()>>,
        active: Arc<AtomicBool>,
    }

    pub fn hold() -> Option<Hold> {
        let (release, wait) = channel::<()>();
        let active = Arc::new(AtomicBool::new(false));
        let flag = active.clone();

        std::thread::Builder::new()
            .name("sdc-keep-awake".into())
            .spawn(move || {
                // SAFETY: plain flag arguments; the returned previous state is not needed.
                let previous = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };

                flag.store(previous != 0, Ordering::SeqCst);

                let _ = wait.recv();

                // SAFETY: as above; ES_CONTINUOUS alone clears the requirement.
                unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
                flag.store(false, Ordering::SeqCst);
            })
            .ok()?;

        Some(Hold { release: Some(release), active })
    }

    impl Hold {
        pub fn is_active(&self) -> bool {
            self.active.load(Ordering::SeqCst)
        }
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            if let Some(release) = self.release.take() {
                let _ = release.send(());
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::process::{Child, Command, Stdio};

    pub fn idle_seconds() -> Option<u64> {
        let output = Command::new("ioreg").args(["-c", "IOHIDSystem"]).output().ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text.lines().find(|line| line.contains("HIDIdleTime"))?;
        let nanos: u64 = line.rsplit('=').next()?.trim().parse().ok()?;

        Some(nanos / 1_000_000_000)
    }

    pub struct Hold(Child);

    pub fn hold() -> Option<Hold> {
        /* `-i` prevents idle sleep, `-w` ends when this process does, so a crash cannot leave it held. */
        Command::new("caffeinate").args(["-i", "-w", &std::process::id().to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok().map(Hold)
    }

    impl Hold {
        pub fn is_active(&self) -> bool {
            true
        }
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use std::process::{Child, Command, Stdio};

    pub fn idle_seconds() -> Option<u64> {
        /* `loginctl` reports when the session went idle; absent or "no" means the person is active. */
        let session = std::env::var("XDG_SESSION_ID").ok()?;
        let output = Command::new("loginctl").args(["show-session", &session, "-p", "IdleHint", "-p", "IdleSinceHint"]).output().ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let hint = text.lines().find_map(|line| line.strip_prefix("IdleHint="))?;

        if hint.trim() != "yes" {
            return Some(0);
        }

        let since: u64 = text.lines().find_map(|line| line.strip_prefix("IdleSinceHint="))?.trim().parse().ok()?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_micros() as u64;

        Some(now.saturating_sub(since) / 1_000_000)
    }

    pub struct Hold(Child);

    pub fn hold() -> Option<Hold> {
        Command::new("systemd-inhibit")
            .args(["--what=idle:sleep", "--who=SDC", "--why=An approval or a task is waiting", "--mode=block", "sleep", "infinity"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()
            .map(Hold)
    }

    impl Hold {
        pub fn is_active(&self) -> bool {
            true
        }
    }

    impl Drop for Hold {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    pub fn idle_seconds() -> Option<u64> {
        None
    }

    pub struct Hold;

    pub fn hold() -> Option<Hold> {
        None
    }

    impl Hold {
        pub fn is_active(&self) -> bool {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_time_is_a_plausible_number_or_unknown() {
        if let Some(seconds) = idle_seconds() {
            assert!(seconds < 365 * 24 * 3600, "{seconds} seconds idle is not plausible");
        }
    }

    #[cfg(windows)]
    #[test]
    fn the_sleep_hold_is_taken_and_released() {
        let guard = SleepGuard::acquire().expect("Windows can be asked to stay awake");

        for _ in 0..50 {
            if guard.is_active() {
                break;
            }

            std::thread::sleep(Duration::from_millis(20));
        }

        assert!(guard.is_active());
        drop(guard);
    }

    #[test]
    fn unknown_idle_time_is_not_treated_as_away() {
        /* away_for(0) is true only when the OS reported a time; a machine that cannot say stays "present". */
        if idle_seconds().is_none() {
            assert!(!away_for(0));
        }
    }
}
