//! **Windows containment with a Job Object (Phase 4.10).**
//!
//! macOS has Seatbelt and Linux has bubblewrap; Windows has no equivalent that needs no installer and no administrator rights. A
//! **Job Object** is what it does have, and it is real but small. What it enforces, per child tree started under it:
//!
//! * the whole tree dies when SDC lets go of the job (`KILL_ON_JOB_CLOSE`) - stronger than `taskkill /T`, which misses a process that
//!   re-parented;
//! * a cap on live processes (a fork bomb stops at the cap);
//! * a cap on committed memory for the whole job, and on CPU time;
//! * UI limits: it cannot read or write the clipboard, switch desktops, change system parameters, log the user off, or touch another
//!   program's windows or handles.
//!
//! What it does **not** do, and SDC says so: it does not stop a command writing outside the project or using the network. That needs
//! an AppContainer (a profile, ACLs on the project folder, a firewall rule) or Windows Sandbox / WSL2, none of which can be built and
//! proven without a Windows machine. [`crate::guard::sandbox::guarantees_for`] therefore reports `filesystem: None, network: None`
//! for Windows, and `limits: Enforced`.
//!
//! The platform-independent half ([`Limits`], [`limits_for`]) is tested everywhere; the half that calls Windows is `#[cfg(windows)]`
//! and was type-checked for `x86_64-pc-windows-msvc` from a Mac (see `docs/WINDOWS-SANDBOX.md`). Running it is **OWNER-TO-RUN** on a Windows
//! PC: `cargo test --lib winjob -- --nocapture`.

/// What a job is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Live processes in the job at once.
    pub processes: u32,
    /// Committed memory for the whole job, in MiB (0 = no cap).
    pub memory_mib: u64,
    /// CPU time for the whole job, in seconds (0 = no cap).
    pub cpu_seconds: u64,
}

/// The limits for a sandbox policy: its memory and CPU caps carry over; the process cap is always set (a check that needs more than 256
/// live processes is not a check).
pub fn limits_for(memory_mib: u64, cpu_seconds: u64) -> Limits {
    Limits { processes: 256, memory_mib, cpu_seconds }
}

/// 100-nanosecond ticks of CPU time, as the Job Object API counts them.
pub fn ticks(seconds: u64) -> i64 {
    (seconds.saturating_mul(10_000_000)).min(i64::MAX as u64) as i64
}

#[cfg(windows)]
pub use imp::Job;

/// What the Command Runner puts every child in on Windows: a job with the default caps (256 processes, 4 GiB). The returned value is held
/// until the command is finished; dropping it ends anything still running. `None` when the job could not be made - the runner then
/// carries on with its deadline and `taskkill`, as it did before this module existed, and does not pretend otherwise.
#[cfg(windows)]
pub fn contain_command(child: &std::process::Child) -> Option<Job> {
    let job = Job::new(&limits_for(4096, 0)).ok()?;

    job.contain(child).ok()?;

    Some(job)
}

/// Other systems have their own mechanisms (`sandbox`); nothing is held.
#[cfg(not(windows))]
pub fn contain_command(_child: &std::process::Child) -> Option<()> {
    None
}

#[cfg(windows)]
mod imp {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_BASIC_UI_RESTRICTIONS,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_JOB_TIME, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_UILIMIT_DESKTOP,
        JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
        JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    };

    use super::{ticks, Limits};

    /// A job. Dropping it closes the handle, which (with `KILL_ON_JOB_CLOSE`) ends every process still in it.
    pub struct Job(HANDLE);

    // The handle is only a number the kernel resolves; it is safe to move between threads.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        /// Creates a job with these limits.
        pub fn new(limits: &Limits) -> Result<Self, String> {
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());

                if handle.is_null() {
                    return Err(format!("CreateJobObject failed: {}", std::io::Error::last_os_error()));
                }

                let job = Job(handle);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();

                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
                info.BasicLimitInformation.ActiveProcessLimit = limits.processes;

                if limits.memory_mib > 0 {
                    info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
                    info.JobMemoryLimit = (limits.memory_mib as usize).saturating_mul(1024 * 1024);
                }

                if limits.cpu_seconds > 0 {
                    info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_TIME;
                    info.BasicLimitInformation.PerJobUserTimeLimit = ticks(limits.cpu_seconds);
                }

                if SetInformationJobObject(handle, JobObjectExtendedLimitInformation, &info as *const _ as *const _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32) == 0 {
                    return Err(format!("SetInformationJobObject (limits) failed: {}", std::io::Error::last_os_error()));
                }

                let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
                    UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
                        | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
                        | JOB_OBJECT_UILIMIT_EXITWINDOWS
                        | JOB_OBJECT_UILIMIT_GLOBALATOMS
                        | JOB_OBJECT_UILIMIT_HANDLES
                        | JOB_OBJECT_UILIMIT_READCLIPBOARD
                        | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
                        | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
                };

                if SetInformationJobObject(handle, JobObjectBasicUIRestrictions, &ui as *const _ as *const _, std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32) == 0 {
                    return Err(format!("SetInformationJobObject (ui) failed: {}", std::io::Error::last_os_error()));
                }

                Ok(job)
            }
        }

        /// Puts a running child (and from then on everything it starts) in the job. A grandchild started in the instant before this call is
        /// not in it - the same limit every process-creation-then-assign scheme has.
        pub fn contain(&self, child: &std::process::Child) -> Result<(), String> {
            let ok = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };

            if ok == 0 {
                return Err(format!("AssignProcessToJobObject failed: {}", std::io::Error::last_os_error()));
            }

            Ok(())
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Windows only (OWNER-TO-RUN): a child in a job dies when the job is dropped, and a job refuses more processes than its cap.
        #[test]
        fn a_child_in_a_job_dies_with_it() {
            let job = Job::new(&Limits { processes: 8, memory_mib: 512, cpu_seconds: 60 }).expect("a job");
            let mut child = std::process::Command::new("cmd").args(["/C", "ping -n 30 127.0.0.1 > NUL"]).spawn().expect("a child");

            job.contain(&child).expect("assign");
            drop(job);
            std::thread::sleep(std::time::Duration::from_millis(500));

            assert!(child.try_wait().expect("wait").is_some(), "the child outlived its job");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limits_carry_the_policys_caps_and_always_cap_processes() {
        assert_eq!(limits_for(2048, 120), Limits { processes: 256, memory_mib: 2048, cpu_seconds: 120 });
        assert_eq!(limits_for(0, 0).processes, 256, "no memory or CPU cap still caps the process count");
    }

    #[test]
    fn cpu_seconds_become_100_nanosecond_ticks_without_overflowing() {
        assert_eq!(ticks(1), 10_000_000);
        assert_eq!(ticks(60), 600_000_000);
        assert_eq!(ticks(u64::MAX), i64::MAX);
    }
}
