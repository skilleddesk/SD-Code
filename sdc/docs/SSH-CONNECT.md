# SSH connect — how SDC gets into a VPS, layer by layer, with the code (0.7.13)

This is the *operational* companion to [`REMOTE.md`](REMOTE.md). REMOTE.md answers "why is it built this
way"; this file answers **"what exactly runs, in what order, and where does it stop"** — with the real code
of every step, plus the commands to run by hand, so you can check each layer yourself instead of trusting
SDC's summary.

Everything here happens on the **local** machine (the `ssh` client you already have) — SDC installs nothing
on the host and runs no server there. That single decision shapes the whole document.

---

## 1. The chain, end to end

```
window (Add a host / the host's card)            app/src/modals/AddHost.tsx
      │  intents: addHost · hostKey · trustHost · installHostKey        app/src/store/intents.ts
      ▼
SDCP call over the bridge                        app/src/lib/sdcp.ts → sdcd
      │  host.add → host.key → host.trust → ssh.key → host.doctor
      ▼
daemon methods                                   sdcd/src/sdcp/methods.rs
      │  parse → ensure key → scan → pin → install key → probe
      ▼
ssh layer                                        sdcd/src/ssh/{mod,hostkey,ops}.rs
      │
      ▼
the `ssh` binary on THIS machine                 C:\Windows\System32\OpenSSH\ssh.exe  (or /usr/bin/ssh)
      │
      ▼
the VPS: sshd → shell → the chat's folder, git, the CLIs
```

The three daemon modules, in the order a connection goes through them:

| module | what it owns | why it is separate |
| --- | --- | --- |
| `sdcd/src/ssh/mod.rs` | one hardened argument set (`Ssh::base_args`), `run`, `run_with_stdin`, `sh_quote`, `port_hint` | a connection with the right flags in one place and the wrong ones in another is how a tool ends up trusting a machine it has never seen |
| `sdcd/src/ssh/hostkey.rs` | `scan`, `fingerprint`, `pin_into`, `pinned_in`, `inspect`, `confirm`, `Trust` | the trust decision is a *person's*, and it needs its own vocabulary (unknown / pinned / changed) |
| `sdcd/src/auth/remote.rs` | `parse_target`, `key_path`, `public_key`, `ensure_key`, `install_key`, `prompt_in` | the target, the key and the one-time install — the part that is about *SDC getting in*, not about what happens afterwards |


---

## 2. Step 1 — the target: what a person types is parsed, not trusted

`sdcd/src/auth/remote.rs`:

```rust
/// Parses what a person types for a host.
///
/// Accepted, because all five are things people paste:
///
/// ```text
/// deploy@203.0.113.10
/// ssh deploy@203.0.113.10
/// ssh -p 8443 deploy@203.0.113.10
/// deploy@203.0.113.10 -p 8443
/// deploy@203.0.113.10:8443          ← the colon form a hosting panel prints (0.7.13)
/// ```
///
/// A bare hostname is an error rather than a guess: `ssh` needs a user to try, and inventing `root`
/// for somebody would be a change of meaning, not a convenience.
pub fn parse_target(input: &str) -> Result<SshTarget, String> { /* … */ }
```

The port is the part 0.7.0 dropped, and dropping it is why a host could never connect: the report pasted
`ssh -p 8443 deploy@203.0.113.10`, the daemon used the **whole string as a hostname** (so `ssh`
was asked for a machine called `ssh`) and the port was never used. It is stored now (migration
`0003-host-ssh`: `hosts.port`) and every call for that host carries `-p 8443`.

**Check it by hand** — this is the whole layer, and it takes two seconds:

```powershell
foreach ($p in 22,2222,8443) { "$p => $(Test-NetConnection 203.0.113.10 -Port $p -InformationLevel Quiet)" }
```

On the host in the original report: **22 → False, 8443 → True**. A target written without a port dials 22
and fails, which is why the daemon now says so out loud (`port_hint`, §7).

---

## 3. Step 2 — SDC's own key

`sdcd/src/auth/remote.rs`:

```rust
/// The path of the key SDC uses for hosts it adds: `~/.ssh/sdc_ed25519`.
pub fn key_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;

    Some(PathBuf::from(home).join(".ssh").join("sdc_ed25519"))
}

/// The public half, generated if it does not exist yet.
///
/// `ssh-keygen -t ed25519 -N ""` - the same command a person runs, so the key is a normal key: they can
/// see it with `ssh-add -l`, revoke it by deleting one line from `authorized_keys`, and use it from a
/// terminal too. No passphrase: a passphrase is something a *person* types, and an automated installer
/// that stored one would be storing a secret for no reason.
pub fn ensure_key() -> Result<String, String> { /* ssh-keygen -t ed25519 -N "" -C sdc -f <path> */ }
```

A normal key in a normal place, made once. `host.add` is the only caller that creates it — a remote
`fs.list` must not mint a key as a side effect of looking at a folder.

**Check it by hand:**

```powershell
ssh-keygen -lf "$env:USERPROFILE\.ssh\sdc_ed25519.pub"
# 256 SHA256:Qa+B67XMaUq4yOcvcymel5J5JIqKSF+ukXrHzCJT8MA sdc (ED25519)      ← the machine running SDC

---

## 4. Step 3 — the host key, and the one question SDC asks

`sdcd/src/ssh/hostkey.rs`. Two ways to ask, and the order matters — `ssh-keyscan` performs a key exchange
and **nothing else**, so the fingerprint is decided before a password or a key is ever offered:

```rust
pub fn scan(target: &SshTarget) -> Result<Vec<HostKey>, ErrorObject> {
    let keyscan = scan_with("ssh-keyscan", target);

    if let Ok(keys) = &keyscan {
        if !keys.is_empty() {
            return Ok(keys.clone());
        }
    }

    match scan_by_handshake(target) {
        Ok(keys) if !keys.is_empty() => Ok(keys),
        /* … both failed: the sentence is the handshake's, because that call is a real `ssh` connection … */
        Err(handshake) => { /* append ssh-keyscan's reason only when it differs */ }
    }
}
```

The fallback is not a nicety — it is what makes **your** host work: `ssh-keyscan` on this machine fails
against it (`choose_kex: unsupported KEX method sntrup761x25519-sha512@openssh.com`) while a real `ssh`
completes the same key exchange. A build without the fallback reports "did not present a host key" for a
machine that answers perfectly.

The decision is one enum:

```rust
pub enum Trust {
    Pinned(HostKey),                                       // a pin exists and the machine matches → connect
    Unknown(Vec<HostKey>),                                 // never seen → ask the person, send nothing
    Changed { pinned: Vec<String>, seen: Vec<HostKey> },   // the alarm: refuse, and name both
}
```

and `confirm` re-scans before storing, so **what was confirmed is what is pinned**:

```rust
pub fn confirm(target: &SshTarget, fingerprint_wanted: &str) -> Result<Vec<HostKey>, ErrorObject> {
    let seen = scan(target)?;

    if !seen.iter().any(|key| key.fingerprint == fingerprint_wanted) {
        return Err(ErrorObject::bad_request(format!(
            "{} now presents {} - not the {} that was confirmed. Nothing was pinned: …",
            target.user_host, /* … */
        )));
    }
    /* only the confirmed key is returned, never every key the machine offers */
}
```

The pins live in **SDC's own** file (`<data>/ssh/known_hosts`, i.e. `%APPDATA%\sdc\ssh\known_hosts`),
`0600` in a `0700` directory — never the user's `~/.ssh/known_hosts`.

**Check it by hand** (must match the fingerprint the card shows, character for character — and it is the
form that works on a server `ssh-keyscan` cannot negotiate with, which includes yours):

```powershell
ssh -p 8443 -o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=$env:TEMP\check `
    deploy@203.0.113.10 true
ssh-keygen -lf "$env:TEMP\check"
# 256 SHA256:Xk3v9Qm2b7EXAMPLEfingerprintNotARealKey0 203.0.113.10 (ED25519)   ← your host

# where ssh-keyscan *can* negotiate, the shorter form says the same thing:
ssh-keyscan -p 8443 203.0.113.10 | ssh-keygen -lf -
```

---

## 5. Step 4 — the hardened argument set (the security layer)

Every `ssh` SDC runs goes through one function (`sdcd/src/ssh/mod.rs`):

```rust
fn args_with(&self, interactive: bool) -> Result<Vec<String>, ErrorObject> {
    let mut args = self.target.port_args();          // -p 8443
    let pins = hostkey::known_hosts_path()?;

    let mut options = vec![
        "ConnectTimeout=10".to_string(),
        "IdentitiesOnly=yes".to_string(),
        "StrictHostKeyChecking=yes".to_string(),     // never accept-new
        "LogLevel=ERROR".to_string(),
    ];

    if interactive {                                 // ONLY the one-time key install
        options.push("BatchMode=no".to_string());
        options.push("NumberOfPasswordPrompts=1".to_string());
        options.push("PreferredAuthentications=publickey,keyboard-interactive,password".to_string());
    } else {
        options.push("BatchMode=yes".to_string());   // nothing may prompt
        options.push("ServerAliveInterval=15".to_string());
        options.push("ServerAliveCountMax=3".to_string());
        options.push("PreferredAuthentications=publickey".to_string());
        options.push("PasswordAuthentication=no".to_string());
    }

    for option in options {
        args.push("-o".to_string());
        args.push(option);
    }

    args.push("-o".to_string());
    args.push(format!("UserKnownHostsFile={}", pins.display()));

    /* The key SDC owns, when it exists. */
    if let Some(key) = key_path().filter(|path| path.exists()) {
        args.push("-i".to_string());
        args.push(key.display().to_string());
    }

    args.push(self.target.user_host.clone());

    Ok(args)
}
```

Three flags separate the install call from every other call: a prompt is allowed, exactly once,
**after** the pin — so a password never reaches a machine whose identity has not been decided.

**Check it by hand** — the same command the daemon runs, verbatim:

```powershell
ssh -p 8443 -o ConnectTimeout=10 -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes `
    -o UserKnownHostsFile="$env:APPDATA\sdc\ssh\known_hosts" `
    -o BatchMode=yes -o PreferredAuthentications=publickey -o PasswordAuthentication=no `
    -i "$env:USERPROFILE\.ssh\sdc_ed25519" deploy@203.0.113.10 true
```

Your host answers with the one sentence that matters:

```
deploy@203.0.113.10: Permission denied (keyboard-interactive).
```

That is **not** a broken connection: the transport, the key exchange and the host key are all fine. It is
the far side saying *"I have no key of yours in `authorized_keys`"* — which is §6.


---

## 6. Step 5 — the install (the step that needs your password, once)

`sdcd/src/auth/remote.rs` — the only `ssh` in the daemon that may answer a prompt, run on the daemon's own
PTY so the password can be typed into `ssh`'s own `password:` question:

```rust
pub fn install_key(pty: &PtyManager, target: &SshTarget, password: &str) -> Result<String, ErrorObject> {
    let public_key = ensure_key().map_err(ErrorObject::internal)?;
    let command = install_command(&public_key);          // the one-line authorized_keys append

    let mut args = crate::ssh::Ssh::new(target.clone()).install_args()?;
    args.push(command);

    let opened = pty.open("ssh", &args, None, None, None)?;
    /* … then, every 200 ms for ~30 s: read the transcript, and … */
    Prompt::Password if !typed && !password.is_empty() => {
        pty.write(&pty_id, &format!("{password}\n"))?;    // typed once, never stored
        typed = true;
    }
    /* … until the transcript carries the marker:  SDC-KEY-INSTALLED */
}
```

and the command itself is idempotent and prints its marker:

```rust
fn install_command(public_key: &str) -> String {
    format!(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && \
         (grep -qF '{public_key}' ~/.ssh/authorized_keys 2>/dev/null || echo '{public_key}' >> ~/.ssh/authorized_keys) && \
         chmod 600 ~/.ssh/authorized_keys && echo {MARKER}"
    )
}
```

**Check it by hand** — with your own password, in your own terminal, if you would rather not give it to
SDC at all:

```powershell
type "$env:USERPROFILE\.ssh\sdc_ed25519.pub" | ssh -p 8443 deploy@203.0.113.10 `
    "mkdir -p ~/.ssh && chmod 700 ~/.ssh && cat >> ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys"
```

**And then the probe must pass** — this is what flips the host from `offline` to `connected`:

```powershell
ssh -p 8443 -o IdentitiesOnly=yes -o PreferredAuthentications=publickey `
    -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$env:APPDATA\sdc\ssh\known_hosts" `
    -o BatchMode=yes -i "$env:USERPROFILE\.ssh\sdc_ed25519" deploy@203.0.113.10 "echo OK"
# OK    →  the host is connected, and every later call is passwordless
```


---

## 7. Where it stops — the failure table

| What you see | Cause | Fix |
| --- | --- | --- |
| `did not answer: Connection timed out` **+ `SDC dialed port 22…`** | the target had no port, and 22 is closed or filtered | add it with its port: `user@host:8443`, or `ssh -p 8443 user@host` |
| `did not answer: Connection refused` | nothing is listening on that port | check the port (compare with your own terminal) |
| `did not present a host key: choose_kex: unsupported KEX method …` | `ssh-keyscan` cannot negotiate with that server (OpenSSH 9.5 client vs OpenSSH 10.2 server) | **nothing** — a real `ssh` handshake is tried next and succeeds; the sentence only appears if *both* fail |
| `is reachable, and its host key is SHA256:… a key SDC has never seen` | first time SDC has seen this machine | check the fingerprint the sentence prints (the handshake form works even where `ssh-keyscan` cannot negotiate) and press **Trust and connect** |
| `its host key is not the one SDC pinned for it` | the machine presents a **different** key | stop and find out what changed the machine's keys — this is the alarm, and there is deliberately no "continue anyway" |
| `answered, but it does not accept SDC's key yet` | the transport is fine; `authorized_keys` has no SDC key | open the host's card (`Keys & doctor`) → **Install SDC's key**, password once — or paste the printed line by hand |
| `asks for a verification code` | the host uses 2FA | sign in from your own terminal and paste the line `ssh.key` shows — a daemon holding a one-time code would defeat the second factor |
| `ssh` / `ssh-keygen` `is not on this machine's PATH` | no OpenSSH client | Windows: Settings → Optional features → **OpenSSH Client** |

---

## 8. The window's side of it (so every failure has a button)

| Where | What it does | File |
| --- | --- | --- |
| **Add a host** | target + password + label → `host.add` | `app/src/modals/AddHost.tsx` |
| **Trust card** | the fingerprint, `Trust and connect` / `Re-pin and connect`; the password is re-sent here so it is spent **after** the pin | same |
| **`Install SDC's key`** (0.7.13) | the step after the pin: a password field, the button, and — for a 2FA host — the exact `authorized_keys` line to paste | same |
| **`Keys & doctor`** (switcher) | the way back to that card for a host added days ago | `app/src/modals/HostSwitcherPopover.tsx` |
| **Doctor rows** | `SSH to <address>` with `Trust` / `Re-pin` / `Install key`, then `Host key`, then the host's own tools | `sdcd/src/host/doctor.rs` |

The doctor's `ssh` row is where the fix comes from: a pin that is in place plus a probe that still failed
**is** the key install, and that is decided in exactly one place:

```rust
let ssh_fix = match (&trust, status.as_str()) {
    (_, "connected") => None,
    (Ok(crate::ssh::hostkey::Trust::Unknown(_)), _) => Some("Trust"),
    (Ok(crate::ssh::hostkey::Trust::Changed { .. }), _) => Some("Re-pin"),
    (Ok(crate::ssh::hostkey::Trust::Pinned(_)), _) => Some("Install key"),
    (Err(_), _) => None,     // a machine that is simply down gets no button
};
```

`Install SDC's key` sends the address **the row already shows** (`user@host:8443`) back to the daemon —
which is why the colon form has to be parseable — and `host.add` reuses the row, sees the pin, and goes
straight to the install and the probe:

```ts
export async function installHostKey(hostId: string, password: string): Promise<boolean> {
  const host = useAppStore.getState().hosts.find((candidate) => candidate.id === hostId);
  const answer = await addHost({ type: 'ssh', target: host.address, label: host.name, password });
  return answer !== null && answer.hostId === hostId;
}
```


---

## 9. The fifteen-second diagnosis

Run this on the machine that runs SDC, and you will know which layer is at fault before opening the window:

```powershell
$h = '203.0.113.10'; $p = 8443; $u = 'deploy'
$key = "$env:USERPROFILE\.ssh\sdc_ed25519"

"1. client   : $(ssh -V 2>&1)"
"2. port     : $(Test-NetConnection $h -Port $p -InformationLevel Quiet)"
"3. host key : $((ssh-keyscan -T 8 -p $p $h 2>$null | ssh-keygen -lf - 2>$null) -join ' | ')"
"4. SDC key  : $(ssh-keygen -lf "$key.pub")"
"5. probe    : " + (ssh -p $p -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=no `
                  -o UserKnownHostsFile=NUL -i $key "$u@$h" 'echo SDC-OK' 2>&1)
```

Reading it:

1. **no version** → install the OpenSSH client;
2. **False** → wrong port, or the host is not reachable from here;
3. **empty** → `ssh-keyscan` cannot negotiate with that server (SDC still handles it: a real handshake is
   the fallback); a fingerprint on stdout is what SDC shows on the card;
4. **no fingerprint** → SDC has not made its key yet (add a host once);
5. **`SDC-OK`** → the host accepts SDC's key and the whole chain works; **`Permission denied`** → §6, the
   install is the missing step.

For the daemon's side of the same thing, `_verify/probe-remote.mjs` drives the real methods over SDCP:

```powershell
node _verify/probe-remote.mjs 'deploy@203.0.113.10:8443' 7811
```

Against the host from the original report it prints, in order: `host.add` → the pin already in place →
`offline · "…does not accept SDC's key yet…"`, `host.key · matches=true pinned=true`,
`host.doctor · ssh=fail(Install key) hostkey=ok`, `ssh.key · exists=true`, `fs.list /` and `git.status /`
(each answering either data or the daemon's own sentence), then a real `shell.run` and
`pty.open`/`pty.close` whose failure sentence is the far side's own
(`Permission denied (keyboard-interactive)`). After the one-time install, the same run ends with the host
`connected`, a `shell.run` that printed `sdc-line-ok`, real rows from `fs.list`, and a background process
stopped by its **process group**.

---

## 10. The two rules this file exists to make checkable

* **Nothing is sent before the host key is decided.** No key is offered and no password is typed until the
  fingerprint on the card has been accepted (and `confirm` re-scans, so what was confirmed is what is
  pinned). A machine whose key *changed* is an error with both fingerprints named, and there is no
  "continue anyway" anywhere in the daemon.
* **SDC installs nothing on the host and runs nothing there but `ssh`.** The far side needs a shell; the
  engines need their own CLIs (`claude`/`codex`/`gemini`), and where one is missing the doctor says
  `not installed` and the `Install` fix opens the **Terminal on that host** so the person runs the
  installer themselves.

```
