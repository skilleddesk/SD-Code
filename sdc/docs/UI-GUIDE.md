# SDC UI Guide

Extracted from the code on 2026-10-17 (P0-7); the code is the source of truth, this file is how to find it. New UI (P0-8 onward) extends what is listed here: existing components, existing tokens, both languages. Regenerate the tables by reading `app/src/styles/tokens.css`, `app/tailwind.config.ts` and the string files named below.

## Rules the code already enforces

- **No raw colour in a component.** `app/eslint.config.js` bans hex colours in `src/panels` and `src/modals` (spec 8.1): use a token class (`bg-bg-raised`, `text-text-secondary`, `border-border-subtle`, `text-state-error`, `bg-diff-add-bg`).
- **Colour never carries meaning alone** (state tokens come with a label or icon).
- **Contrast is measured**, not assumed: `pnpm --filter @sdc/app smoke:a11y` (axe, WCAG 2.0/2.1 A+AA) fails on any serious or critical violation on the opening screen; muted text is 5.3:1 on the dark panel, white on `--accent-fill` 5.7:1.
- **Two themes:** dark is `:root`; `[data-theme="light"]` re-declares only what changes (see the table: *light* column empty = same as dark).
- **Right-to-left** locales (ar, ur) mirror the layout; code and terminals stay left-to-right.
- **Fonts:** Geist (UI), Geist Mono (code), Hind Siliguri (Bengali) via `@fontsource*`.

## Design tokens (`src/styles/tokens.css`)

| token | dark | light |
|---|---|---|
| `--accent` | `#A78BFA` | `#6D28D9` |
| `--accent-fill` | `#7C3AED` | `#6D28D9` |
| `--accent-fill-hover` | `#7339E6` | `#5B21B6` |
| `--accent-glow` | `rgba(167, 139, 250, 0.36)` | `rgba(109, 40, 217, 0.22)` |
| `--accent-hover` | `#C4B5FD` | `#5B21B6` |
| `--accent-subtle` | `rgba(167, 139, 250, 0.12)` | `rgba(109, 40, 217, 0.08)` |
| `--aurora` | `(radial gradients)` | `(own gradients)` |
| `--base` | `150ms` | |
| `--base-size` | `13px` | |
| `--bg-active` | `rgba(255, 255, 255, 0.09)` | `rgba(30, 20, 70, 0.08)` |
| `--bg-base` | `#0A0912` | `#F4F3F9` |
| `--bg-glass` | `rgba(23, 20, 37, 0.72)` | `rgba(255, 255, 255, 0.78)` |
| `--bg-hover` | `rgba(255, 255, 255, 0.055)` | `rgba(30, 20, 70, 0.045)` |
| `--bg-input` | `rgba(6, 5, 12, 0.5)` | `rgba(255, 255, 255, 0.9)` |
| `--bg-overlay` | `rgba(23, 20, 37, 0.94)` | `rgba(255, 255, 255, 0.95)` |
| `--bg-panel` | `rgba(21, 18, 34, 0.62)` | `rgba(255, 255, 255, 0.6)` |
| `--bg-raised` | `rgba(255, 255, 255, 0.035)` | `rgba(255, 255, 255, 0.72)` |
| `--border-default` | `rgba(255, 255, 255, 0.1)` | `rgba(30, 20, 70, 0.12)` |
| `--border-focus` | `#A78BFA` | `#7C3AED` |
| `--border-strong` | `rgba(255, 255, 255, 0.18)` | `rgba(30, 20, 70, 0.22)` |
| `--border-subtle` | `rgba(255, 255, 255, 0.065)` | `rgba(30, 20, 70, 0.07)` |
| `--diff-add-bg` | `rgba(52, 211, 153, 0.1)` | `rgba(4, 120, 87, 0.08)` |
| `--diff-add-text` | `#6EE7B7` | `#047857` |
| `--diff-remove-bg` | `rgba(248, 113, 113, 0.1)` | `rgba(220, 38, 38, 0.08)` |
| `--diff-remove-text` | `#FCA5A5` | `#B91C1C` |
| `--ease` | `cubic-bezier(.4, 0, .2, 1)` | |
| `--ease-out` | `cubic-bezier(0, 0, .2, 1)` | |
| `--edge-light` | `rgba(255, 255, 255, 0.06)` | `rgba(255, 255, 255, 0.9)` |
| `--fast` | `90ms` | |
| `--font-mono` | `'Geist Mono Variable', 'Hind Siliguri', ui-monospace, Menlo,` | |
| `--font-ui` | `'Geist Variable', 'Hind Siliguri', system-ui, -apple-system,` | |
| `--grad-brand` | `linear-gradient(135deg, #7C3AED 0%, #C026D3 58%, #EA580C 100` | |
| `--grad-brand-soft` | `linear-gradient(135deg, rgba(124, 58, 237, 0.16), rgba(192, ` | `linear-gradient(135deg, rgba(124, 58, 237, 0.1), rgba(192, 3` |
| `--grad-brand-text` | `linear-gradient(100deg, #C4B5FD 0%, #F0ABFC 50%, #FDBA74 100` | `linear-gradient(100deg, #6D28D9 0%, #C026D3 50%, #EA580C 100` |
| `--grad-host-local` | `linear-gradient(135deg, #2D2550, #43316E)` | `linear-gradient(135deg, #8B5CF6, #6D28D9)` |
| `--grad-host-vps` | `linear-gradient(135deg, #4C1D4F, #6B2752)` | `linear-gradient(135deg, #DB2777, #9D174D)` |
| `--grad-preview` | `linear-gradient(135deg, #17142A, #24142A)` | `linear-gradient(135deg, #EEEAFB, #F8E8F3)` |
| `--grad-preview-glow` | `radial-gradient(ellipse at top left, rgba(167, 139, 250, 0.2` | |
| `--grad-thumb` | `linear-gradient(135deg, #2D2550, #5B2352)` | `linear-gradient(135deg, #DDD6FE, #FBCFE8)` |
| `--green` | `#34D399` | `#047857` |
| `--green-subtle` | `rgba(52, 211, 153, 0.12)` | `rgba(4, 120, 87, 0.08)` |
| `--letter-spacing` | `-0.006em` | |
| `--line-height` | `1.5` | |
| `--orange` | `#FB923C` | `#C2410C` |
| `--orange-subtle` | `rgba(251, 146, 60, 0.12)` | `rgba(194, 65, 12, 0.08)` |
| `--purple` | `#F472B6` | `#DB2777` |
| `--purple-subtle` | `rgba(244, 114, 182, 0.12)` | `rgba(219, 39, 119, 0.08)` |
| `--r-full` | `9999px` | |
| `--r-lg` | `12px` | |
| `--r-md` | `9px` | |
| `--r-sm` | `6px` | |
| `--r-xl` | `16px` | |
| `--r-xs` | `4px` | |
| `--red` | `#F87171` | `#DC2626` |
| `--red-bright` | `#FECACA` | `#B91C1C` |
| `--red-subtle` | `rgba(248, 113, 113, 0.12)` | `rgba(220, 38, 38, 0.08)` |
| `--shadow-lg` | `0 20px 48px -10px rgba(0, 0, 0, 0.65), 0 0 0 1px rgba(255, 2` | `0 20px 48px -12px rgba(30, 20, 70, 0.18), 0 0 0 1px rgba(30,` |
| `--shadow-md` | `0 8px 24px -6px rgba(0, 0, 0, 0.55), 0 0 0 1px rgba(255, 255` | `0 8px 24px -8px rgba(30, 20, 70, 0.14), 0 0 0 1px rgba(30, 2` |
| `--shadow-panel` | `0 10px 40px -18px rgba(0, 0, 0, 0.7), inset 0 1px 0 var(--ed` | `0 10px 40px -20px rgba(30, 20, 70, 0.18), inset 0 1px 0 var(` |
| `--shadow-sm` | `0 1px 2px rgba(0, 0, 0, 0.45)` | `0 1px 2px rgba(30, 20, 70, 0.06)` |
| `--shadow-xl` | `0 36px 90px -12px rgba(0, 0, 0, 0.75), 0 0 0 1px rgba(255, 2` | `0 36px 90px -16px rgba(30, 20, 70, 0.25), 0 0 0 1px rgba(30,` |
| `--slow` | `220ms` | |
| `--spring` | `cubic-bezier(.34, 1.56, .64, 1)` | |
| `--state-error` | `#F87171` | `#DC2626` |
| `--state-idle` | `#6B6684` | `#8E8AA6` |
| `--state-running` | `#A78BFA` | `#6D28D9` |
| `--state-success` | `#34D399` | `#047857` |
| `--state-waiting` | `#FBBF24` | `#B45309` |
| `--text-faint` | `#4A4660` | `#C9C6D8` |
| `--text-muted` | `#8A85A3` | `#6B6787` |
| `--text-on-accent` | `#FFFFFF` | |
| `--text-primary` | `#EFEDF7` | `#17142A` |
| `--text-secondary` | `#B0ABC6` | `#4B4766` |

73 tokens; 53 are re-declared for light.

Tailwind aliases (`app/tailwind.config.ts`) map every colour token to a class namespace: `bg.*`, `border.*`, `text.*`, `accent.*`, `purple|green|orange|red.*`, `state.*`, `diff.*`; shape and motion follow spec 8.3/8.4.

## Components

Shared primitives (`src/panels/ui/`):

- `panels/ui/Badge.tsx`
- `panels/ui/BrandLogo.tsx`
- `panels/ui/Field.tsx`
- `panels/ui/HostIcon.tsx`
- `panels/ui/IconButton.tsx`
- `panels/ui/InstallButton.tsx`
- `panels/ui/Section.tsx`
- `panels/ui/button.ts`
- `panels/ui/status.ts`

Named surfaces the plan's new UI is modelled on (where to copy from):

| component | file | new UI that follows it |
|---|---|---|
| ModelSelector | `panels/prompt/ModelSelector.tsx` | mode badge, model class S/M/L, effective context |
| TrustChip | `kernel/TrustChip.tsx` | mode badge, step model chip, proof badge |
| PlanCard | `panels/turns/PlanCard.tsx` | step list with model chip and reason |
| QuestionCard | `panels/prompt/QuestionCard.tsx` | cloud-step permission, provider problem card, Select & Instruct input |
| ResearchCard | `panels/prompt/ResearchCard.tsx` | reading progress |
| CostMeter | `kernel/CostMeter.tsx` | budget gauge, receipt |
| PolicyEditor | `kernel/PolicyEditor.tsx` | Rotation Chain list (up/down, toggle) |
| ProofTab | `kernel/ProofTab.tsx` | Working Memory tab (the Proof-of-Work badge itself is `kernel/ProofBadge.tsx`, P0-15) |
| TimeMachineTab | `panels/right/TimeMachineTab.tsx` | one Time Machine point per step transaction |
| PreviewDiff / VerifyTab | `panels/right/` | Diff Review |
| Palette / slash commands | `overlays/Palette.tsx, panels/prompt/slash.ts` | @ picker |
| Settings tabs | `kernel/SettingsTabs.tsx, kernel/AgentSettings.tsx` | Settings → Agent → Local Rotation |
| Provider Hub | `modals/ProviderHub.tsx` | recommended chain card, hardware proposal |

All component trees: `src/panels/{main,prompt,rail,right,sidebar,statusbar,tabs,topbar,turns,ui}`, `src/modals`, `src/overlays`, `src/kernel`, `src/layout`.

## Strings (English and Bengali)

`strings.ts` is the one place words live, in English. Every other language is a **pack** of the same shape in `src/locales/<id>.ts` (`bn, hi, ar, ur, es, pt, fr, id, zh`); a key missing from a pack falls back to English, so half-translated screens still work. Functions (`(n) => ...`) are translated as functions, so plurals and word order stay the translator's. Add a new string to `strings.ts` and to `locales/bn.ts` in the same change. Feature string files: `agentStrings.ts`, `kernelStrings.ts`, `anywhereStrings.ts`, `researchStrings.ts`. The language is read once at start; switching reloads the window.

## Screens and their visual baselines

`pnpm --filter @sdc/app smoke:visual` photographs the built window in headless Chromium at four views and compares with `app/tests/visual/*.png`:

| view | size | scheme |
|---|---|---|
| desktop-dark | 1280×800 | dark |
| desktop-light | 1280×800 | light |
| phone-dark | 390×844 | dark |
| phone-light | 390×844 | light |

The bottom-centre toast strip (the rows a fixed toast can occupy) is not compared: in a browser tab the app says it has no daemon in a toast whose timing varies run to run. The tolerance is 0.5% of pixels differing by more than 24 levels; two runs of one build differ by ~0.000% here. **Coverage is honest, not complete:** the opening screen (the first-launch *Welcome to SDC* dialog over the shell) is covered. Screens that need a daemon, a project or a connected provider are not on a path a headless browser can reach without them; each new screen in P0-15/P2-2/P3-9 adds its own view when it has a way to be reached without a backend. A change that is intended: `UPDATE_VISUAL=1 pnpm --filter @sdc/app smoke:visual`, review the PNG diff in the commit.

### Component fixtures

A component that cannot be reached without a backend can still have a photograph: its unit test renders the real component with `renderToStaticMarkup` and writes `app/tests/visual/fixtures/<name>.html`; `smoke:visual` serves that page with the built stylesheet and photographs it in both themes (`<name>-dark`, `<name>-light`, 640×260). Today: `proof-badge` (the Proof-of-Work badge and receipt, closed, open and failed). A shot is only compared after a second shot of the same build agrees with it, because the app fades things by the clock.

## How to build a new surface (checklist)

1. Find the nearest component in the table above and copy its structure, classes and spacing.
2. Token classes only; no hex, no inline colour.
3. Strings in `strings.ts` **and** `locales/bn.ts`.
4. Phone width: the view must work at 390px (no horizontal scroll, 44px touch targets on the mobile layout).
5. `smoke:a11y` stays at 0 serious/critical; add `aria-label`s to icon buttons, keep focus traps in dialogs.
6. Add or update a visual view if the surface is reachable without a backend.

