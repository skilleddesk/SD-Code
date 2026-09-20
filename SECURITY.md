# Security Policy

**Do not report a vulnerability in a public issue, pull request or discussion.**
Use the private channel below; a public report puts every user at risk before
there is a fix.

---

## Reporting a vulnerability

**Preferred — GitHub private advisory.** Open the repository's **Security** tab
and choose **Report a vulnerability**. This opens a private thread only the
maintainers can see, and it keeps the report, the discussion and the eventual
advisory in one place.

**Alternative — email** `<OWNER-CONTACT-EMAIL>` with `SDC security` in the
subject line. This address is a placeholder until the first public release, so if
you get no reply within a week, use the GitHub advisory route instead.

### What to include

The more of this you can give, the faster the report can be acted on:

* what an attacker gains, and what they need to start (local access, a malicious
  repository, a hostile host, a compromised engine CLI, nothing at all);
* the exact version — the About tab (Settings → About) shows the app, daemon and
  protocol versions, and `git rev-parse HEAD` identifies the build;
* the platform (Windows, Linux, macOS) and how you run SDC;
* reproduction steps, minimal and ordered, with what you expected versus what
  happened;
* the smallest proof you can produce — a screenshot, a log excerpt, a diff of a
  state file. Redact tokens, keys, paths that identify you and the contents of
  your own code before sending anything.

You do not need a working exploit, a CVSS score or a polished write-up. A precise
description of a reachable weakness is worth more than a clever payload.

### What we will do

| Step | Target |
| --- | --- |
| Acknowledge your report | 3 business days |
| Initial assessment and severity, or a request for more detail | 7 business days |
| Fix for a critical issue (credential exposure, remote code execution) | 7 days from triage |
| Fix for a high-severity issue | 30 days from triage |
| Fix for a medium-severity issue | 90 days from triage |

These are targets, not guarantees — SDC is small and unfunded, and this policy
says so rather than quietly missing a date. You will get an update at each step,
including when a fix slips.

### Coordinated disclosure

We ask for **90 days** before publishing, or sooner if a fix ships earlier and
you agree. If a fix is going to take longer we will say so and give you a reason,
and we will not ask for silence without an end date. If we go silent, you are
entitled to publish — a report that dies in a private thread protects nobody.

Credit in the advisory and the release notes is offered by default. Tell us if
you would rather stay anonymous.

### No bug bounty

There is no monetary reward for security reports. Please do not spend effort on
the assumption that there is one.

---

## Scope

### In scope

Anything that breaks one of the promises SDC is built on. In descending order of
severity:

1. **Credential compromise.** Disclosure, exfiltration or misuse of a provider
   API key, a subscription session or an SSH credential; a key written to a log,
   a crash report, a checkpoint, the local database or a remote host; a key sent
   to a party it does not belong to.
2. **Unconsented egress.** Code, file contents, prompts, attachments or telemetry
   leaving the machine without the user having seen and allowed it. Spec
   principle P4 is that SDC never lies about what it did; a silent upload is that
   lie.
3. **Permission-broker bypass.** A mutating or destructive action — write,
   delete, `rm -rf`, `git push --force`, credential rotation — performed without
   the approval the broker is supposed to require, or performed under a label
   that understates it.
4. **Undo integrity.** A checkpoint that cannot be restored, a "rewind" that
   restores something other than what it claims, or a redo that resurrects state
   the user believed was gone. P5 promises reversibility; a broken undo is worse
   than no undo.
5. **Remote execution or authentication failure on a host.** Anything in the
   daemon or its transport that lets one machine act on another without the
   pinned host key and the expected authentication — including host-key
   substitution, a missing re-pin prompt, or command injection through a path, an
   argument or a file name.
6. **Sandbox or allowlist escape.** Escaping the declared working directory,
   following a symlink out of it, reading a file the deny-list blocks (`.env`,
   `*.pem`, `id_rsa`), or executing an unlisted command.
7. **Local data exposure.** Chat history, prompts or file excerpts readable by
   another local user, another application, or anything that receives a file
   dump — and the same for the SQLite index.
8. **Supply chain.** A dependency, a build step, a bundled binary or the release
   pipeline being subverted in a way that ships code the source does not
   contain.

### Out of scope

* Vulnerabilities in the third-party engines and CLIs SDC drives (`claude`,
  `codex`, `gemini`, Ollama), in the model providers, or in the underlying
  libraries. Report those upstream; if SDC's handling turns one of them into a
  problem for users, that part *is* in scope.
* Attacks that require a machine that is already compromised, an administrator
  who is already hostile, or physical access to an unlocked session.
* Social engineering of maintainers or users, and phishing that does not involve
  SDC's own artefacts.
* Missing hardening with no demonstrated consequence — a header that is absent, a
  dependency that is old, a compiler flag that could be stricter. A report that
  shows the consequence is welcome.
* Denial of service through unrealistic input volume, unless it corrupts state or
  loses data.
* The contents of the master specification and the design documents, which are
  public by intent and are not a security boundary.

---

## Current state of the product

Being precise about this saves everyone time. As of this policy:

* the repository contains a **UI preview** — the desktop shell, the panels, the
  stores and the design system — and a **stub daemon** that prints `sdcd ok`;
* there is **no daemon protocol, no filesystem capability, no network egress and
  no credential storage** implemented yet, so several classes above cannot be
  triggered today. Report them anyway if the *design* invites them: a design flaw
  found now costs a conversation, and found after implementation it costs a
  migration;
* there are **no releases and no signed builds**, so only the tip of `main` is
  supported. The version table starts listing versions once there is a release
  channel;
* nothing in this repository needs a secret in order to build it. If you find
  something that looks like it does, that is itself worth reporting.

## What we do to reduce the chance of a report

* `.env`, `*.key`, `*.pem` and local databases are ignored by git; `.env.example`
  documents the variable names and holds no values.
* Every push and pull request runs **gitleaks** over the full history
  (`.github/workflows/secret-scan.yml`), never just the newest commit — a secret
  removed in a later commit is still a published secret.
* The same workflow fails if a credential-shaped file (`.env`, `*.key`, `*.pem`,
  `*.db`) is tracked at all, ignore rules notwithstanding.
* Credentials are designed to live in the operating system's keychain rather than
  in a file in this repository or in the application's own store (spec §9.10).
* Secrets in prompts and logs are designed to be redacted, and `.env`, `*.pem`
  and `id_rsa` blocked from being sent, before anything leaves the machine.

If one of these mechanisms is the thing you are testing, its failure *is* the
finding — say so in the report.

---

## Safe harbour

If you make a good-faith effort to comply with this policy, we will not pursue or
support legal action against you for the research, and we will say so publicly if
asked. Good faith means:

* you only touch accounts, data and machines you own or have explicit permission
  to test;
* you stop as soon as you have proof, and you do not exfiltrate, retain or share
  other people's data;
* you do not degrade anyone's service, and you do not use the finding to extort,
  threaten or embarrass;
* you give us the timeframes above before publishing.

Research that stays inside those lines is welcome, including research on the
parts that are not built yet.
