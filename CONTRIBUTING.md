# Contributing to SDC (Skilleddesk Code)

## Read this first: this project is proprietary

SDC is **not open source**. See [LICENSE](LICENSE). The source is public so it can
be read, audited and reviewed; that is not the same as being licensed for reuse,
and no licence to copy, modify, redistribute or build on it is granted to anyone.

That has one direct consequence for contributions, and it is the most important
paragraph in this file:

> **By submitting anything to this repository — a pull request, a patch, a commit
> on a branch, a snippet in a review comment, a design or a document — you accept
> the contribution terms below.** If you do not accept them, do not submit.
> There is no need to agree with anything here in order to *read* the project;
> these terms bind you only once you send something in.

### Contribution terms

**1. Assignment of copyright.** You assign to SkilledDesk your entire right,
title and interest, throughout the world and for the full term of protection, in
and to every contribution you submit, including all copyright and all rights of
renewal and extension.

**2. Rights granted with the assignment.** The assignment includes, without
limitation, the right for SkilledDesk to use, reproduce, modify, adapt, publish,
distribute, sublicense, transfer and otherwise exploit the contribution **under
any licence or terms it chooses, including proprietary and closed terms**, and to
enforce the copyright in it against others. If, in any jurisdiction, the
assignment in clause 1 is not effective for any reason, you instead grant
SkilledDesk an exclusive, perpetual, irrevocable, worldwide, royalty-free, fully
paid-up, sublicensable and transferable licence to do all of the above.

**3. Moral rights.** To the extent permitted by law, you waive, and agree not to
assert, any moral rights or similar rights in the contribution. Where a waiver is
not permitted, you agree not to exercise them in a way that interferes with
SkilledDesk's use of the contribution.

**4. No rights retained for reuse.** The assignment gives you no right in the
project. You may not reuse, publish or distribute your own contribution — or any
derivative of it that reproduces project material — in another product, service
or repository, and you keep only the right to refer to it factually.

**5. Your representations.** By submitting, you confirm that:

* the contribution is your original work, or you have the rights you are
  assigning;
* it contains no third-party code, text, image or asset unless you have said so
  explicitly and identified its licence and origin;
* submitting it does not breach any agreement you have, including an employment
  or client contract — if your employer holds rights in your work, get written
  permission first;
* if you used an AI assistant to produce any part of it, you will say so in the
  pull request, because you remain responsible for its provenance and for licence
  cleanliness. A contribution that cannot be shown to be free of third-party
  obligations will be rejected.

**6. No compensation.** Contributions are voluntary. There is no payment,
royalty, equity, employment relationship or partnership created by contributing.

**7. No obligation to accept.** Maintainers may decline, close or ignore any
submission for any reason, and may change or relicense the project without notice
to contributors.

**8. How you accept.** Opening a pull request is acceptance. Please put this
sentence in the pull request description, so the record is unambiguous:

```
I have read CONTRIBUTING.md and I assign copyright in this contribution to SkilledDesk.
```

### If you want to contribute but not assign copyright

Then please **open an issue instead of a pull request**. Describe the bug, the
idea or the design problem precisely enough that a maintainer can implement it,
and that description is all you have given us. Ideas and bug reports are welcome
from anyone, and clause 2 applies to the text you write in them.

### Invitation-only

At the time of writing, external pull requests are not accepted. The terms above
are what would make accepting them possible later: a project that cannot take
ownership of a patch cannot safely take the patch at all.

---

## Working in this repository

### Prerequisites and setup

`sdc/SETUP.md` is the fresh-machine guide (Node.js 20 LTS, pnpm 10, Rust stable
via rustup, Tauri CLI, MSVC Build Tools + WebView2 on Windows,
`webkit2gtk-4.1` on Linux). `sdc/README.md` explains the layout and why the
workspace is arranged the way it is.

```bash
pnpm install          # workspace root: app/ and sdcd/
pnpm dev              # Vite dev server only (browser, port 1420)
pnpm tauri:dev        # the real thing: builds the Rust shell and opens the window
```

### Before you call anything done

```bash
pnpm typecheck        # tsc, strict, for src/ and for the build tooling
pnpm lint             # ESLint 9; warnings count as noise, fix them
pnpm build            # tsc + vite build
pnpm test             # Vitest: the reducer, the command registry, the strings contract
pnpm sdcd:check       # cargo check for the daemon
pnpm sdcd:test        # cargo test for the daemon, VCR fixtures included
pnpm app:check        # cargo check for the Tauri bridge (no window, no bundle)
pnpm rust:clippy      # cargo clippy -- -D warnings - this one is a gate
pnpm rust:fmt         # cargo fmt - available, but see the note below
```

All of them have to be clean, with one honest exception. **The Rust crates are not
rustfmt-formatted.** They are written in a hand style that uses a blank line between
statements; `cargo fmt` disagrees with every file, which means running it would rewrite
the whole crate — every step's code, not just yours — inside an unrelated change. So:
run `rust:clippy` before you push, and if you want the crate formatted, open a
formatting-only pull request and say so in the title. Until then a review that asks
"why is this line long" is a review of this style, not of a rule.

### The app and the daemon are one system, and both halves are testable alone

`pnpm dev` (the browser) has no Tauri bridge, so `lib/sdcp.ts` falls back to the
stand-in in `lib/standin.ts`. It is not a mock and not a demo: it answers the same SDCP
envelopes and appends to the same event log the reducer folds, which is why a UI
flow proved in the browser is a UI flow that works against `sdcd`
(spec §3.1). Two consequences for a change:

* **A new method lands in three places or it is not done**: the handler in
  `sdcd/src/sdcp/methods.rs`, the contract in `protocol/types.ts`, and the
  stand-in in `app/src/lib/standin.ts` — in that order, because the daemon is the
  definition and the other two are views of it.
* **Events are the only state.** Nothing writes UI state directly; a handler
  pushes an event and `app/src/store/reducer.ts` folds it. If you find yourself
  wanting to set state from an answer, the event is missing.


### Conventions that are enforced or checked in review

* **User-visible copy lives in `sdc/app/src/strings.ts`** and nowhere else (spec
  §2.7). Components must not inline a sentence, a label or a tooltip. The file is
  grouped by surface, and it also carries the demo seed data, because that is
  user-visible copy too.
* **No hardcoded colours** (spec §8.1). Components read tokens through the
  Tailwind aliases in `app/tailwind.config.ts` — `bg-bg-raised`,
  `text-text-secondary`, `border-border-subtle`, `text-state-error`. ESLint
  rejects a raw hexadecimal literal in `src/panels` and `src/modals`. If a
  gradient or a shade has no token yet, add the token (in
  `app/src/styles/tokens.css` and `design/tokens.json`, which are kept in step by
  hand until the generator exists) rather than writing the value inline.
* **The prototype wins.** Where `docs/MASTER_SPEC.md` and
  `design/ui-prototype.html` disagree, the prototype is the UI source of truth
  (spec §7). A UI change is a change to the prototype first, then to the
  component — otherwise the next person diffs against the wrong picture.
* **Comments explain why.** The codebase documents intent, trade-offs and the
  spec section a rule comes from; it does not narrate what the next line does.
  Match that. A block comment that ends in "because the prototype does X" is
  worth more than a paragraph restating the code.
* **Strict TypeScript.** `strict`, `noUnusedLocals`, `noUnusedParameters`,
  `noImplicitReturns`, `verbatimModuleSyntax`. `any` needs a reason in a comment.
  Exported functions carry explicit return types.
* **Style.** 2-space indent, LF line endings, single quotes, trailing commas,
  around 100 columns — the settings `.editorconfig` and `.gitattributes` pin.
  Rust is `cargo fmt` output, no exceptions.
* **The stack is locked** (spec §4.1). A new runtime dependency is a design
  decision, not a convenience: raise it in an issue first and say what it
  replaces.

### Never commit a secret

This repository is public, and the CI job in `.github/workflows/secret-scan.yml`
scans for secrets on every push and pull request — over the full history, not
just the newest commit. The rules:

* no API keys, tokens, passwords, private keys or connection strings — not in
  code, a test, a fixture, a comment or a commit message;
* use `.env` for local values; it is ignored. `.env.example` is the file that
  gets committed, and it holds names and comments only;
* credential-shaped files — `.env`, `*.key`, `*.pem`, `*.db` — must never be
  tracked. The workflow fails if one is, even though `.gitignore` should have
  caught it;
* if you must paste a log or a config into an issue, redact it first. Blur,
  replace or delete the value; do not trust a screenshot's resolution to hide it;
* if you do commit a secret, **rotate it immediately** and tell a maintainer. A
  secret is compromised the moment it is pushed, whether or not the commit is
  later rewritten — history is public, forks keep their copy and mirrors exist.
  Removing the commit is clean-up, not a fix.

Run the scan yourself before a push if you have gitleaks installed:

```bash
gitleaks protect --staged --redact     # staged changes
gitleaks detect --redact               # the whole history
```

### Commits and branches

* Short-lived branches: `feat/host-switcher`, `fix/tab-close-focus`,
  `docs/security-policy`.
* An imperative subject under 72 characters; a body that says why, not what.
  Reference the spec section when a rule is being implemented: `§7.3`.
* One concern per commit. A formatting sweep mixed into a behaviour change makes
  the behaviour change unreviewable.

### Issues

Bugs and feature requests are welcome — see the invitation-only note above about
why a description is a contribution. Include what you ran, what you expected,
what happened, and the versions from Settings → About. Security problems do
**not** belong in an issue; follow [SECURITY.md](SECURITY.md).

## Code of conduct

Be straightforward and civil: criticise code, not people; assume the other person
had a reason for what they wrote and ask what it was. Harassment, personal
attacks, discrimination and deliberate disruption are not welcome, and maintainers
will close, block or ban without further discussion. If you see a problem, report
it privately to a maintainer rather than escalating it in a thread.

