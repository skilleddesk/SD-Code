# Windows containment (Phase 4.10)

**What it is:** every command the Command Runner starts on Windows is put in a **Job Object** (`sdcd/src/guard/winjob.rs`).

| Enforced by the kernel | Not enforced |
|---|---|
| at most 256 live processes (a fork bomb stops) | writing outside the project |
| 4 GiB committed memory for the whole tree | using the network |
| the whole process tree ends when the command's job is closed (also when SDC itself dies) | reading other files |
| no clipboard, desktop switching, display settings, log-off, or other programs' handles | |

`guard::sandbox::guarantees()` therefore says, on Windows: filesystem **none**, network **none**, limits **enforced**, with a note to run
checks inside **WSL2** (with bubblewrap) for real isolation. SDC does not route checks through WSL2 automatically yet; AppContainer is not
built. A caller may still refuse to run untrusted code on Windows.

## What was verified, and where
* The pure part (limits, tick conversion) has unit tests that run everywhere (`cargo test --lib winjob`).
* The Windows-only part was **type-checked for `x86_64-pc-windows-msvc` from a Mac** with a throwaway crate that includes `winjob.rs`
  (`cargo check --target x86_64-pc-windows-msvc --tests` - this is how a missing `Win32_System_Threading` feature was found and fixed).
  Type-checking is not running.
* **OWNER-TO-RUN on a Windows PC:**
  ```
  cargo test --lib winjob -- --nocapture            # a child in a job dies when the job is dropped
  cargo test --lib guard::runner                    # the runner's tests with the job around every child
  ```
