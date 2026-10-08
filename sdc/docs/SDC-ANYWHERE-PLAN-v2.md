# SDC Anywhere: Master Plan v2

> **যেকোনো জায়গা থেকে, যেকোনো browser দিয়ে, কোনো app install ছাড়া, নিজের পুরো SDC চালানো।**
> PC-র local ফাইল হোক বা VPS-এর, দেখা, বাছাই, খোলা, বদলানো, upload/download, command চালানো, AI-কে নির্দেশ দেওয়া, permission দেওয়া, সব এক জায়গা থেকে।
>
> Domain: `sdc.skilleddesk.com` · Desktop: Windows / macOS / Linux · Remote: যেকোনো আধুনিক browser
> Owner: SkilledDesk · লক্ষ্য version: 0.17 → 0.21 · এই ফাইল v1-কে সম্পূর্ণ প্রতিস্থাপন করে।

---

## সূচিপত্র

0. সারসংক্ষেপ ও v1 থেকে কী বদলালো
1. অটল নীতি (কখনো ভাঙা যাবে না)
2. ব্যবহারকারীর যাত্রা: ৬টা বাস্তব উদাহরণ
3. সাত দিক থেকে বিশ্লেষণ ও সিদ্ধান্ত
4. Architecture
5. ফিচার-নকশা (module অনুযায়ী)
6. নিরাপত্তা মডেল
7. Performance: নকশা, আনুমানিক সংখ্যা, সীমা
8. Protocol
9. Policy
10. Domain ও DNS
11. Secret ব্যবস্থাপনা
12. Repo কাঠামো
13. ফেজ পরিকল্পনা ও "Done" শর্ত
14. Test ও benchmark
15. ঝুঁকি ও খোলা প্রশ্ন

---

## 0. সারসংক্ষেপ

SDC Anywhere হলো SDC-র একটা web রূপ, যা `sdc.skilleddesk.com`-এ চলে। Phone, tablet, বন্ধুর laptop, যেকোনো browser থেকে ঢুকে আপনি আপনার PC-র SDC-কে পুরোপুরি চালাতে পারবেন। PC নিজে সব কাজ করে (ফাইল, AI, VPS-এর SSH); browser শুধু নিরাপদ "জানালা" ও "রিমোট কন্ট্রোল"।

### v1 থেকে যা নতুন যুক্ত হলো
| বিষয় | v1 | v2 |
|---|---|---|
| Permission approve | ✅ | ✅ (উন্নত) |
| **Host বাছাই (PC / যেকোনো VPS)** | ❌ | ✅ |
| **File Explorer: local + VPS** | ❌ | ✅ |
| **ফাইল খোলা, খোঁজা, বদলানো** | ❌ | ✅ |
| **ফাইল বাছাই করে AI-কে দেওয়া (@file)** | ❌ | ✅ |
| **Upload / Download, PC↔VPS copy** | ❌ | ✅ |
| **Guarded Terminal (command চালানো)** | আংশিক | ✅ পূর্ণ |
| **Live Preview (dev server phone-এ দেখা)** | ❌ | ✅ |
| **Performance বিশ্লেষণ ও লক্ষ্য** | ❌ | ✅ |
| **নিরাপত্তা স্তর (View / Operate / Critical)** | ❌ | ✅ |

---

## 1. অটল নীতি

1. **PC-তে কোনো inbound port খোলা হবে না।** Daemon শুধু বাইরে connection করে।
2. **Server কখনো plaintext দেখবে না।** ফাইল, command, chat, সব end-to-end encrypted।
3. **VPS-এর SSH key কখনো PC ছাড়বে না।** Browser কখনো VPS-এ সরাসরি যুক্ত হয় না; PC-র daemon মাঝখানে থাকে।
4. **Remote থেকে আসা প্রতিটা পরিবর্তন Trust Kernel দিয়ে যাবে:** policy → checkpoint → audit।
5. **Passkey + trusted device signature ছাড়া কোনো remote পরিবর্তন কার্যকর নয়।**
6. **Remote থেকে নিষিদ্ধ:** policy বদলানো, device pair করা, remote চালু/বন্ধ, keychain পড়া, audit মোছা।
7. **Secret কখনো log, commit বা output-এ যাবে না।**
8. **`remote.enabled` default `false`।** Remote বন্ধ থাকলে desktop SDC আগের মতোই চলবে।
9. **Remote শুধু যোগ করে, কিছু ভাঙে না।** Relay down হলেও desktop SDC পুরোপুরি কাজ করবে।

---

## 2. ব্যবহারকারীর যাত্রা: ৬টা উদাহরণ

### উদাহরণ ১: বাইরে থেকে permission
আপনি দোকানে। PC-তে AI client-এর VPS-এ migration চালাতে চায়। Phone-এ notification: "SDC: অনুমতি দরকার"। খুললে card: command, কোন VPS, কেন, কী বদলাবে, rollback আছে কি না। আপনি `--dry-run` যোগ করে **Edit করে Allow** চাপলেন, fingerprint দিলেন। ১ সেকেন্ডের কম সময়ে PC-তে চলতে শুরু করল।

### উদাহরণ ২: VPS-এর ফাইল বাছাই করে AI-কে কাজ দেওয়া
Bus-এ বসে মনে পড়ল client-এর site-এ checkout page ভাঙা। Browser-এ:
1. **Host:** `client-vps-1` বাছাই করলেন
2. **Files:** `/var/www/shop/app/checkout/` খুললেন, `page.tsx` আর `api.ts` select করলেন
3. **Chat:** "এই দুটো ফাইলে checkout-এর bug ঠিক করো, আগে test চালাও" লিখে পাঠালেন
4. PC-র SDC SSH দিয়ে VPS-এ কাজ শুরু করল; আপনি live দেখছেন
5. Diff এলো, Verify PASS, আপনি Allow দিলেন

ফাইলের পুরো content phone-এ আসেনি, শুধু path গেছে। AI ফাইল পড়েছে PC→VPS পথে। তাই দ্রুত ও কম data খরচ।

### উদাহরণ ৩: PC-র local ফাইল দেখা ও ছোট পরিবর্তন
বন্ধুর laptop থেকে ঢুকলেন (আগে থেকে trusted নয়)। Email magic link দিয়ে login, কিন্তু device untrusted, তাই phone থেকে ৬-অঙ্কের code মিলিয়ে অনুমোদন দিলেন। এখন `H:\SDC\sdc\docs\README.md` খুলে একটা লাইন বদলালেন। Save চাপলে PC-তে checkpoint নিয়ে পরিবর্তন হলো, audit-এ লেখা থাকলো "বন্ধুর laptop থেকে"।

### উদাহরণ ৪: Phone থেকে ফাইল পাঠানো
Client WhatsApp-এ একটা logo পাঠিয়েছে। Phone থেকে upload করলেন → PC-র `.sdc/inbox/` এ গেল (quarantine) → chat-এ "এই logo `public/logo.png` হিসেবে বসাও" → AI সরাল।

### উদাহরণ ৫: Terminal থেকে command
VPS-এর log দেখতে Terminal খুলে `tail -n 100 /var/log/nginx/error.log` লিখলেন। এটা read-only, তাই সঙ্গে সঙ্গে চলল। তারপর `systemctl restart nginx` লিখলেন। এটা ঝুঁকিপূর্ণ, তাই card এলো, fingerprint চাইল, তারপর চলল।

### উদাহরণ ৬: Live preview
AI নতুন homepage বানিয়েছে, PC-তে `localhost:3000`-এ চলছে। Phone-এ **Preview** tab খুলে সত্যিকারের page দেখলেন, encrypted পথে, কোনো public URL ছাড়া।

---

## 3. সাত দিক থেকে বিশ্লেষণ ও সিদ্ধান্ত

### 3.1 ব্যবহারকারী (UX)
| প্রশ্ন | সিদ্ধান্ত | কারণ |
|---|---|---|
| Install লাগবে? | না; PWA, Home Screen ঐচ্ছিক | User-এর মূল চাহিদা |
| Login | Passkey (password নেই) + email magic link শুধু প্রথমবার/recovery | Phishing-প্রতিরোধী, মনে রাখার কিছু নেই |
| Layout | Mobile-first: নিচে ৫টা tab: **Inbox · Chat · Files · Terminal · More** | এক হাতে চালানো; desktop browser-এ split view |
| Host বদল | উপরে সবসময় Host selector (💻 My PC, 🖥 client-vps-1, …) | Local আর VPS একই অভিজ্ঞতা, SDC-র বিদ্যমান নীতির সাথে মেলে |
| দেখতে কেমন | SDC desktop-এর design tokens ও component | একই brand, একই অভ্যাস |
| ভাষা | SDC-র ১০টা ভাষা, RTL | বিদ্যমান শক্তি |

### 3.2 নিরাপত্তা
মূল কথা: **ক্ষমতা ধাপে ধাপে খোলে** (§6)। দেখা সহজ, বদলানো কঠিন, বিপজ্জনক কাজ আরও কঠিন। এতে নিরাপত্তা ও সুবিধা দুটোই থাকে।
**বাদ:** browser-এ SSH key রাখা (চুরির ঝুঁকি), PC-তে port খোলা, শুধু email link দিয়ে কাজ।

### 3.3 প্রযুক্তি
| বিকল্প | রায় | কারণ |
|---|---|---|
| Cloudflare Pages + Workers + Durable Objects + D1 | ✅ প্রধান | কাছের edge server, WebSocket সস্তা, DDoS সুরক্ষা, রক্ষণাবেক্ষণ কম |
| WebRTC DataChannel (P2P) বড় ফাইলের জন্য | ✅ ফেজ ৪ | Relay-এর চাপ ও খরচ কমায়; না পারলে relay fallback |
| নিজের VPS-এ Rust relay | 🔁 fallback | Transport trait দিয়ে বদলযোগ্য |
| PC-তে public tunnel | ❌ | আক্রমণের দরজা |

### 3.4 প্ল্যাটফর্ম (Windows / macOS / Linux)
| কাজ | Windows | macOS | Linux |
|---|---|---|---|
| Idle detect | `GetLastInputInfo` | IOKit `HIDIdleTime` | logind `IdleHint` |
| Sleep আটকানো | `SetThreadExecutionState` | `IOPMAssertionCreateWithName` | `systemd-inhibit` / D-Bus |
| Secret | Credential Manager | Keychain | Secret Service |
| Path নিয়ম | `\`, drive letter, case-insensitive, long path (`\\?\`) | case-insensitive (সাধারণত) | case-sensitive |

Path সবসময় daemon-এ canonicalize হবে; browser শুধু daemon-এর দেওয়া opaque `path_id` + display path ব্যবহার করবে। এতে OS-ভেদে পার্থক্য browser-এ পৌঁছায় না।

### 3.5 Performance
সংক্ষেপে: **"ভারী কাজ PC/VPS-এ, phone-এ শুধু ফল।"** বিস্তারিত §7।

### 3.6 ব্যবসা ও প্রতিযোগিতা
বাজারের AI coding tool সাধারণত হয় cloud-এ code চালায়, নয়তো local-only। SDC Anywhere-এর জায়গা:
> **"Code আপনার মেশিনে থাকে, নিয়ন্ত্রণ আপনার পকেটে, আর প্রতিটা সিদ্ধান্ত cryptographically প্রমাণযোগ্য।"**

আলাদা করে যা: WYSIWYS signing, Edit-before-Allow, Blast-radius preview, নিজের ভাষায় risk, local+VPS এক explorer-এ, Guarded Terminal, E2E Preview Tunnel, Two-Person Rule, Proof Pack-এ remote সিদ্ধান্ত।

### 3.7 খরচ ও পরিচালনা
- Text/approval traffic খুব হালকা, তাই খরচ নগণ্য।
- বড় ফাইল relay দিয়ে গেলে খরচ বাড়ে, তাই P2P (ফেজ ৪) ও **path-only attach** (উদাহরণ ২)।
- Email-এর জন্য SPF/DKIM/DMARC বাধ্যতামূলক।
- নির্দিষ্ট দাম provider-এর সাইটে যাচাই করতে হবে।

---

## 4. Architecture

```
 ┌─────────────── আপনার PC (Win/Mac/Linux) ───────────────┐
 │ SDC Desktop (Tauri) ◄─SDCP─► sdcd                        │
 │                              ├─ Trust Kernel             │
 │                              ├─ ssh/ (russh, SFTP) ──────┼──► VPS-1, VPS-2 …
 │                              ├─ fs/, pty/, agent/ …      │    (key কখনো PC ছাড়ে না)
 │                              └─ remote/  ← নতুন          │
 │                                  ├─ session & crypto     │
 │                                  ├─ capability gate      │
 │                                  ├─ channel mux          │
 │                                  └─ relay_client ────────┼──┐ শুধু outbound WSS
 └──────────────────────────────────────────────────────────┘  │
                                                               ▼
 ┌──────────── sdc.skilleddesk.com (Cloudflare) ────────────────────┐
 │ Pages: web app (PWA)                                              │
 │ Worker: /api (account, WebAuthn, magic link, push subscription)   │
 │ Durable Object "Hub" (প্রতি account): sockets, routing,           │
 │     encrypted mailbox (TTL 24h), escalation timer                 │
 │ D1: account, device public key, push endpoint (কোনো content নয়)  │
 │ Queue → Email provider · Web Push (VAPID) · TURN (ফেজ ৪)          │
 └───────────────────────────────┬───────────────────────────────────┘
                                 │ WSS (E2E) / WebRTC (ফেজ ৪)
                      ┌──────────▼───────────┐
                      │ যেকোনো Browser        │
                      │ passkey + device key │
                      └──────────────────────┘
```

### 4.1 স্তর (Layers)
| স্তর | কাজ |
|---|---|
| **Transport** | WSS (প্রধান), WebRTC DataChannel (বড় ফাইল, ফেজ ৪) |
| **Secure session** | Device↔Daemon E2E: HPKE (RFC 9180) দিয়ে session key, তারপর AES-256-GCM; প্রতি দিক আলাদা key, sequence number, rekey প্রতি ১ ঘণ্টা বা ১ GB |
| **Channel mux** | এক connection-এ অনেক channel: `control`, `stream`, `fs`, `xfer`, `pty`, `preview`; প্রতিটার নিজস্ব priority ও flow-control |
| **Capability gate** | প্রতিটা request কোন স্তরের (View/Operate/Critical), তা যাচাই |
| **Trust Kernel** | বিদ্যমান: policy, checkpoint, audit |

### 4.2 Channel priority (performance-এর চাবি)
`control` (approval, kill) > `pty` > `stream` > `fs` > `preview` > `xfer`
বড় download চললেও Kill বা Approve কখনো আটকাবে না।

---

## 5. ফিচার-নকশা

### 5.1 Access, login ও pairing
**প্রথম device (QR):** Desktop → Settings → SDC Anywhere → "Device যোগ" → QR (`/pair#token.fingerprint`, গোপন অংশ fragment-এ) → phone-এ passkey + non-extractable device key → দুই দিকে ৬-অঙ্কের SAS code → desktop-এ নিশ্চিত। **Trust-এর উৎস daemon, server নয়।**

**নতুন device (magic link):** email → magic link (১০ মিনিট, একবার; link খোলায় নয়, button চাপায় খরচ) → login হলেও untrusted → desktop বা অন্য trusted device থেকে SAS মিলিয়ে অনুমোদন।

**অস্থায়ী device (বন্ধুর laptop):** "Guest session": সর্বোচ্চ ২ ঘণ্টা, browser বন্ধ করলে key মুছে যায়, ডিফল্ট শুধু View।

**Recovery:** ১০টা recovery code; শুধু login দেয়, trust নয়।

**Device তালিকা:** নাম, browser/OS, শেষ ব্যবহার, অবস্থান (আনুমানিক দেশ), Revoke।

### 5.2 Host selector
- Daemon জানে কোন কোন host আছে: `local` + `ssh/`-এ সংরক্ষিত VPS-গুলো
- প্রতিটা host-এর অবস্থা: 🟢 connected · 🟡 connecting · 🔴 offline · 🔒 2FA দরকার
- Host বদলালে Files, Terminal, Preview সেই host-এর হয়ে যায়
- VPS-এ 2FA লাগলে SDC-র বিদ্যমান TOTP ব্যবস্থা কাজ করে; না থাকলে phone-এ code চাইবে (encrypted পথে)

### 5.3 Remote Workspace (File Explorer)

#### কোন ফাইল দেখা যাবে
- শুধু **অনুমোদিত root**-এর ভিতরে: SDC-তে যোগ করা project folder + VPS-এর project path (policy-তে `remote.fs.roots`)
- Root-এর বাইরে যাওয়া যাবে না: daemon `canonicalize` করে, symlink দিয়ে বাইরে গেলে বাতিল, `..` বাতিল
- **Protected path** (`.env`, `wp-config.php`, key, backup): তালিকায় 🔒 দেখায়; খোলা যায় শুধু Critical স্তরে ও policy অনুমতি দিলে; download কখনো নয়

#### কাজসমূহ
| কাজ | কীভাবে হয় | স্তর |
|---|---|---|
| Folder দেখা | Lazy load, প্রতি পাতা ২০০ item, cursor দিয়ে পরের পাতা | View |
| ফাইল খোলা | প্রথম ৫১২ KB, বাকিটা scroll করলে range অনুযায়ী; CodeMirror 6 syntax highlight; ছবি/PDF preview | View |
| খোঁজা | নাম অনুযায়ী (fuzzy) + content (`ripgrep`; VPS-এ `rg`, না থাকলে `grep`) — ফল stream হয়ে আসে | View |
| Git অবস্থা | কোন ফাইল modified/new | View |
| **Select করে AI-কে দেওয়া** | Multi-select → "Chat-এ যোগ" → prompt-এ `@path` হিসেবে যায়; **content phone-এ আসে না** | Operate |
| Edit ও Save | Web editor-এ বদল → diff → checkpoint → লেখা → audit | Operate |
| নতুন ফাইল/folder, rename, move | Action হিসেবে Trust Kernel দিয়ে | Operate |
| Delete | Blast radius দেখিয়ে, checkpoint সহ | Critical |
| Upload (phone → PC/VPS) | Chunked, encrypted → `.sdc/inbox/<তারিখ>/` (quarantine) → পরে সরানো | Operate |
| Download (PC/VPS → phone) | Chunked, resumable; protected ফাইল নিষিদ্ধ | Operate |
| **PC ↔ VPS copy** | Daemon SFTP দিয়ে সরাসরি; ফাইল phone দিয়ে যায় না | Operate |

#### Transfer engine
- ২৫৬ KB chunk, প্রতিটা আলাদা encrypted, **BLAKE3** hash দিয়ে যাচাই
- Resumable: network কাটলে যেখান থেকে থেমেছিল সেখান থেকে
- একসাথে ৪টা chunk (parallel), `xfer` channel সবচেয়ে কম priority
- Default সীমা: upload ১০০ MB, download ৫০০ MB (policy-তে বদলানো যায়)
- Upload-এ file type ও size যাচাই; executable inbox-এ গেলেও আপনা-আপনি চলবে না

### 5.4 Chat ও Prompt
- চলমান session-এ বার্তা বা নতুন session
- `@file` (Files থেকে বাছাই বা টাইপ করে autocomplete), `/` command (`/research`, `/compact` ইত্যাদি)
- Engine ও model বাছাই (desktop-এর মতো, শুধু connected model)
- Voice: browser-এ রেকর্ড → encrypted → PC-র local whisper.cpp → text (audio cloud-এ যায় না)
- Intent Engine-এর "এটাই কি বুঝেছি?" card phone-এ

### 5.5 Approval Center
প্রতিটা card-এ: command/action, host, cwd, risk, **blast radius**, নিজের ভাষায় ব্যাখ্যা, কারণ, rollback আছে কি না, খরচ, মেয়াদ।

| Button | কাজ |
|---|---|
| Allow once | শুধু এটা |
| Allow ৩০ মিনিট | একই ধরনের কাজ (pattern + cwd + host) |
| **Edit করে Allow** | Command বদলানো → আবার policy/risk → নতুন card → sign |
| Deny + কারণ | কারণটা AI-র কাছে নির্দেশ হিসেবে যায় |
| Deny + Pause | কাজ থেমে থাকে |

**WYSIWYS:** passkey-র challenge = `action_hash` (envelope-এর SHA-256)। PC-তে এক অক্ষর অমিল হলে বাতিল।

### 5.6 Guarded Terminal
সাধারণ terminal Trust Kernel এড়িয়ে যায়, তাই SDC-র terminal দুই রূপে চলবে:

**১. Guarded mode (ডিফল্ট):**
- আপনি লাইন লিখে Enter দিলে daemon command-টা `tree-sitter-bash` (Windows-এ PowerShell/cmd parser) দিয়ে বিশ্লেষণ করে
- Read-only (`ls`, `cat`, `tail`, `git status`, `df`) → সঙ্গে সঙ্গে চলে
- পরিবর্তনকারী (`npm install`, `git commit`) → Operate window থাকলে চলে, checkpoint সহ
- ঝুঁকিপূর্ণ (`rm -rf`, `systemctl restart`, `DROP`, `chmod 777`) → card + fingerprint
- Policy-র denied command → বাতিল

**২. Raw mode (vim, htop-এর মতো interactive):**
- Policy-তে `allow_raw_terminal = true` থাকলে, Critical স্তরে, সময়সীমা সহ
- পুরো session asciicast হিসেবে রেকর্ড হয়ে audit-এ যায়

**দ্রুত অনুভূতির জন্য:** mosh-এর মতো **predictive local echo**। টাইপ করা অক্ষর সঙ্গে সঙ্গে দেখায় (হালকা দাগে), server নিশ্চিত করলে স্বাভাবিক হয়। ফলে network lag টাইপিংয়ে অনুভূত হয় না।

### 5.7 Live Preview Tunnel (E2E)
- PC বা VPS-এর dev server (`localhost:3000`) phone-এ দেখা
- Browser-এর Service Worker `/preview/<id>/*` request ধরে E2E channel দিয়ে daemon-এ পাঠায়; daemon local port-এ forward করে উত্তর ফেরত দেয়
- কোনো public URL হয় না, server কিছু দেখে না
- শুধু policy-তে অনুমোদিত port (`remote.preview.ports`), শুধু localhost target
- HMR WebSocket: ফেজ ৪-এ (প্রথমে manual reload)

### 5.8 নিয়ন্ত্রণ
⏸ Pause / ▶ Resume · 🛑 Kill (কোনো step-up নেই, থামানো সবসময় সহজ) · ⏪ Rewind (Critical) · 🚀 Safe Deploy / Rollback (Critical, Two-Person প্রযোজ্য হলে) · ✅ Verify চালানো · 📄 Proof Pack দেখা

### 5.9 Multi-machine Dashboard
একাধিক PC (অফিস + বাসা) ও সব VPS এক screen-এ: কোনটা কাজ করছে, কোনটা অনুমতির অপেক্ষায়, আজকের খরচ, Health Watch অবস্থা।

### 5.10 Notification
- **ক্রম:** Browser খোলা থাকলে live → না থাকলে Web Push → ৬০ সেকেন্ডে সাড়া না পেলে Email
- Push/Email-এ কোনো বিবরণ নেই, শুধু "অনুমতি দরকার" + link `/a/<request_id>`
- Quiet hours, digest, শুধু critical
- iOS-এ push পেতে Home Screen-এ যোগ করতে হয় (iOS-এর সীমা); না করলে email কাজ করবেই

---

## 6. নিরাপত্তা মডেল

### 6.1 ক্ষমতার চার স্তর
| স্তর | কী করা যায় | কীভাবে খোলে | মেয়াদ |
|---|---|---|---|
| **0 · Notify** | শুধু খবর পাওয়া | কিছুই লাগে না | — |
| **1 · View** | Stream, ফাইল দেখা, খোঁজা, diff, log, Kill | Trusted device + session শুরুতে passkey | ১৫ মিনিট নিষ্ক্রিয়তায় লক |
| **2 · Operate** | Edit, upload/download, prompt, low/medium command, Allow | Passkey → "Operate window" ৫ মিনিট; এর মধ্যে low/medium কাজ device key দিয়ে sign | ৫ মিনিট |
| **3 · Critical** | High-risk command, delete, deploy, rewind, protected file, raw terminal | **প্রতিবার নতুন passkey** (+ Two-Person প্রযোজ্য হলে) | একবার |

**কারণ:** প্রতিটা ছোট কাজে fingerprint চাইলে মানুষ বিরক্ত হয়ে নিরাপত্তা বন্ধ করে দেয়। আবার সব খোলা রাখলে ঝুঁকি। স্তর দিয়ে দুটোর ভারসাম্য।

### 6.2 হুমকি ও সুরক্ষা
| হুমকি | সুরক্ষা |
|---|---|
| Email hack / link forward | Link-এ কোনো গোপন তথ্য নেই; untrusted device কিছু দেখে না |
| Phishing | Passkey শুধু আসল domain-এ |
| Relay/server hack | E2E; server শুধু ciphertext |
| Server বার্তা বদলায় | AEAD + sequence number + WYSIWYS; অমিল হলে বাতিল |
| দেখানো এক, চলল আরেক | action_hash-এ sign |
| Replay | Nonce + ৫ মিনিট মেয়াদ + একবার |
| Phone চুরি | Passkey ছাড়া কিছু নয়; Revoke |
| Path traversal (`../../etc/passwd`) | Daemon canonicalize + root যাচাই + symlink বাধা |
| Secret ফাইল চুরি | Protected path: Critical স্তর + download নিষিদ্ধ |
| Upload-এ ক্ষতিকর ফাইল | Quarantine inbox, আপনা-আপনি চলে না, size/type সীমা |
| Terminal দিয়ে Trust Kernel এড়ানো | Guarded mode; raw mode Critical + রেকর্ড |
| Preview দিয়ে internal network-এ ঢোকা | শুধু localhost + অনুমোদিত port |
| Server খারাপ JS দেয় | Strict CSP, SRI, code-pinning Service Worker, release hash desktop-এ দৃশ্যমান |
| Session cookie চুরি | Device-bound: প্রতিটা request device key-এ sign |
| Brute force | Rate limit, lock, alert |

> সৎ সতর্কতা: ১০০% hack-proof বলে কিছু নেই। এই নকশায় ক্ষতি করতে আক্রমণকারীর একসাথে লাগবে আপনার trusted device + আপনার fingerprint/মুখ। Server পুরো দখল করলেও ফাইল পড়তে বা command চালাতে পারবে না।

---

## 7. Performance

### 7.1 নকশার মূলনীতি
1. **ভারী কাজ কাছে:** AI, ফাইল পড়া, search, build, সব PC/VPS-এ। Phone-এ যায় শুধু ফল।
2. **Path পাঠাও, content নয়:** AI-কে ফাইল দিতে শুধু path যায়।
3. **Lazy ও paginated:** ফোল্ডার ২০০ করে, ফাইল ৫১২ KB করে।
4. **Priority channel:** Kill/Approve কখনো download-এর পিছনে আটকায় না।
5. **Event log থেকে resume:** SDC-র append-only log আছে, তাই network কাটলে browser `last_seq` পাঠায়, শুধু বাকি অংশ আসে।
6. **PC↔VPS কাজ phone ছুঁয়ে যায় না।**
7. **Predictive echo** terminal-এ।
8. **Compression:** ফাইল chunk ও stream encrypt করার আগে zstd দিয়ে compress (text ৩-৫ গুণ ছোট); secret-মিশ্রিত কিছু হলে compression বন্ধ (CRIME-ধরনের আক্রমণ এড়াতে)।

### 7.2 আনুমানিক সংখ্যা

**অনুমান:** User Dhaka-তে 4G-তে; PC Dhaka-তে broadband (upload ~২০ Mbps); VPS সিঙ্গাপুর; Cloudflare-এর কাছের edge। Durable Object একটা নির্দিষ্ট region-এ থাকে, তাই phone↔DO ও PC↔DO প্রতিটা ৪০-১২০ ms RTT ধরা হলো। **এগুলো আনুমানিক লক্ষ্য; ফেজ শেষে benchmark দিয়ে মাপা হবে (§14)।**

| কাজ | আনুমানিক সময় | লক্ষ্য (p95) | মন্তব্য |
|---|---|---|---|
| Approval request phone-এ পৌঁছানো (page খোলা) | ১০০-২৫০ ms | < ৫০০ ms | |
| Web Push পৌঁছানো | ১-৫ s | < ১০ s | FCM/APNs-এর উপর নির্ভর |
| Email পৌঁছানো | ৫-৬০ s | < ৯০ s | Provider ও Gmail-এর উপর নির্ভর |
| Allow চাপার পর PC-তে কাজ শুরু | ১৫০-৩০০ ms (+ fingerprint-এর মানুষের সময়) | < ৫০০ ms | |
| AI stream event দেখা | ১০০-২৫০ ms | < ৫০০ ms | |
| Folder খোলা (২০০ item, PC) | ১৫০-৩৫০ ms | < ৭০০ ms | |
| Folder খোলা (VPS) | ২৫০-৬০০ ms | < ১.২ s | PC↔VPS SSH RTT যোগ হয় |
| ১০০ KB ফাইল খোলা | ২০০-৫০০ ms | < ১ s | compression সহ |
| Content search (মাঝারি repo) | প্রথম ফল ৩০০ ms-১ s | প্রথম ফল < ২ s | ফল stream হয়ে আসে |
| ১০ MB download | ৫-১০ s | — | PC-র upload গতিই সীমা (~২.৫ MB/s) |
| ১০ MB upload phone থেকে | ৩-১৫ s | — | Mobile network-এর উপর নির্ভর |
| Terminal echo | অনুভূত ~০ ms (predictive), নিশ্চিত ১০০-২৫০ ms | — | |
| Preview page প্রথম load | ২-৬ s | < ৮ s | page-এর আকারের উপর নির্ভর |
| Network কাটার পর ফেরা (resume) | ১-৩ s | < ৫ s | `last_seq` থেকে |
| Encryption খরচ | প্রতি বার্তা < ১ ms | — | Hardware AES |

### 7.3 Resource ব্যবহার (আনুমানিক)
| কোথায় | ব্যবহার |
|---|---|
| PC daemon (remote চালু, idle) | অতিরিক্ত RAM ~১০-৩০ MB, CPU প্রায় শূন্য (heartbeat প্রতি ৩০ s) |
| PC daemon (সক্রিয় stream) | CPU সামান্য (encrypt + compress) |
| Phone data: ১ ঘণ্টা session দেখা | ~২-১০ MB (text stream compressed) |
| Phone battery | Page বন্ধ থাকলে কোনো connection নেই, শুধু push; খোলা থাকলে সাধারণ web app-এর মতো |
| Server (প্রতি account) | একটা DO যথেষ্ট; হাজারো বার্তা/মিনিট সামলাতে পারে |

### 7.4 সীমা ও সমাধান
| সীমা | সমাধান |
|---|---|
| WebSocket বার্তার আকার সীমিত | ২৫৬ KB chunk |
| PC-র home internet upload ধীর | Path-only attach; P2P (ফেজ ৪); PC↔VPS সরাসরি |
| PC sleep বা বন্ধ | কাজ চলাকালীন sleep আটকানো; বন্ধ থাকলে "PC offline" দেখায়; ঐচ্ছিক Wake-on-LAN (ফেজ ৫) |
| DO-এর region দূরে হলে latency বেশি | Account তৈরির সময় user-এর কাছের region-এ (location hint) |
| iOS push-এর সীমা | Email fallback |
| বড় repo-তে search ধীর | ফল stream, সীমা (প্রথম ৫০০ match), ripgrep |

---

## 8. Protocol (SDCP remote)

সব বার্তা E2E envelope-এর ভিতরে: `{ch, seq, type, body}`।

| Channel | বার্তা |
|---|---|
| `control` | `remote.hello`, `remote.resume{last_seq}`, `capability.unlock`, `capability.lock`, `control.pause/resume/kill/rewind`, `approval.requested/decision/resolved/expired` |
| `stream` | `stream.subscribe{session}`, `stream.event`, `session.list`, `session.create` |
| `host` | `host.list`, `host.status`, `host.connect`, `host.totp_needed` |
| `fs` | `fs.list{host,path_id,cursor}`, `fs.stat`, `fs.read{range}`, `fs.search{query,mode}`, `fs.search.result`, `fs.write.propose{diff}`, `fs.op{create/rename/move/delete}`, `fs.git_status` |
| `xfer` | `xfer.begin{dir,size,blake3}`, `xfer.chunk{idx}`, `xfer.ack`, `xfer.resume`, `xfer.done`, `xfer.copy{from_host,to_host}` |
| `pty` | `pty.open{host,mode}`, `pty.input`, `pty.output`, `pty.command.check`, `pty.resize`, `pty.close` |
| `preview` | `preview.open{host,port}`, `preview.request`, `preview.response` |
| `chat` | `chat.send{text,attachments:[path_id]}`, `chat.intent_card`, `voice.chunk` |

Approval envelope (canonical JSON, hash = `action_hash`):
```jsonc
{ "request_id": "apr_…", "turn_id": "…", "host": "client-vps-1", "cwd": "/var/www/shop",
  "action": "shell.exec", "args": ["pnpm","prisma","migrate","deploy"],
  "file_hashes": {"prisma/schema.prisma": "blake3:…"}, "risk": "high",
  "blast_radius": {"db_tables": 2, "files": 0, "services": []},
  "reason_localized": "…", "rollback": "chk_812", "est_cost_usd": 0.04,
  "expires_at": "…" }
```

---

## 9. Policy (`.sdc/policy.toml`)

```toml
[remote]
enabled = false
notify_when = "idle"              # always | idle | away_mode
idle_minutes = 5
escalate_to_email_after_sec = 60
approval_timeout_sec = 1800
on_timeout = "pause"              # pause | deny
quiet_hours = "01:00-07:00"
redact_in_notifications = true
view_idle_lock_minutes = 15
operate_window_minutes = 5
max_scoped_grant_minutes = 60
guest_session_max_minutes = 120
two_person_for = []               # e.g. ["deploy.prod"]
remote_forbidden = ["policy.edit","remote.pair","remote.toggle","keychain.read","audit.erase"]

[remote.fs]
roots = ["<project roots from SDC>"]   # default: registered projects only
protected_read = "critical"       # critical | never
max_upload_mb = 100
max_download_mb = 500
upload_dir = ".sdc/inbox"

[remote.terminal]
mode = "guarded"                  # guarded | off
allow_raw_terminal = false
record_raw_sessions = true

[remote.preview]
enabled = true
ports = [3000, 5173, 8080]
```

---

## 10. Domain ও DNS

**আপনার PC-র জন্য কোনো DNS record বা port forwarding লাগবে না।** লাগবে শুধু website ও email-এর জন্য।

### 10.1 Website (`sdc.skilleddesk.com`)
| অবস্থা | কী করতে হবে |
|---|---|
| `skilleddesk.com`-এর DNS Cloudflare-এ | Pages/Worker-এ Custom Domain যোগ করুন; Cloudflare নিজে record বানাবে (Proxied ✅) |
| DNS অন্য জায়গায় | `CNAME sdc → <project>.pages.dev`; তবে Worker API একই hostname-এ চালাতে DNS Cloudflare-এ নেওয়া সবচেয়ে সহজ |
| নিজের VPS (fallback) | `A sdc → <IPv4>` (+ `AAAA` থাকলে), VPS-এ Caddy (auto TLS) |
| ঐচ্ছিক | `CAA` record (শুধু নির্দিষ্ট CA সার্টিফিকেট দিতে পারবে) |

Cloudflare proxy চালু রাখা যাবে; WebSocket চলে; E2E-এর কারণে proxy content পড়তে পারে না।

### 10.2 Email (`notify@sdc.skilleddesk.com`)
| Record | কাজ |
|---|---|
| SPF (TXT) | কে পাঠাতে পারে |
| DKIM (TXT/CNAME) | Signature |
| DMARC (TXT `_dmarc.sdc`) | নকল mail আটকানো |
| Bounce/Return-path (MX/CNAME) | Provider অনুযায়ী |

সঠিক মান আসবে provider থেকে। Claude Code `DNS-RECORDS.md`-এ তালিকা বানাবে; বসাবেন আপনি।

### 10.3 Passkey
RP ID = `sdc.skilleddesk.com`। পরে domain বদলালে passkey নতুন করে বানাতে হবে, তাই এটা স্থায়ী রাখুন।

---

## 11. Secret ব্যবস্থাপনা (`emailapi.txt`)

- অবস্থান: `H:\emailapi.txt` (repo-র বাইরে)
- Email key থাকবে **শুধু server-এ** (`wrangler secret put EMAIL_API_KEY`); local dev-এ `.dev.vars` (gitignored)
- Claude Code:
  1. ফাইল পড়ে provider চিনবে (`re_` = Resend, `SG.` = SendGrid, `xkeysib-` = Brevo, SMTP host/user/pass, ইত্যাদি)
  2. মান কখনো output, log, commit বা অন্য ফাইলে লিখবে না; শুধু নাম উল্লেখ করবে
  3. SMTP হলে HTTP API-ওয়ালা provider-এর পরামর্শ দেবে
  4. `.gitignore` ও secret-scan CI-তে pattern যোগ করবে
- Setup শেষে user ফাইল মুছবেন। H:\ root-এর অন্য key ফাইলও (alibabaapi.txt, deepseakapi.txt, pass.txt, TAVILY_API_KEY.txt) keychain-এ সরিয়ে মুছে ফেলা উচিত।

---

## 12. Repo কাঠামো

```
sdc/
├─ sdcd/src/remote/                  ← নতুন (Rust)
│   ├─ mod.rs, identity.rs, pairing.rs, device_registry.rs
│   ├─ crypto.rs            # HPKE, AEAD session, rekey, sig verify, nonce store
│   ├─ session.rs           # capability levels, operate window, idle lock
│   ├─ mux.rs               # channels, priority, flow control, resume(last_seq)
│   ├─ relay_client.rs      # outbound WSS, backoff, heartbeat
│   ├─ envelope.rs          # ActionEnvelope, canonical JSON, action_hash
│   ├─ approval_router.rs   # fan-out, first-valid-wins, scoped grants, timeout
│   ├─ blast_radius.rs      # tree-sitter-bash/PowerShell parse, effect estimate
│   ├─ fs_gateway.rs        # roots, canonicalize, protected, list/read/search/write (local + SFTP)
│   ├─ xfer.rs              # chunked, BLAKE3, resumable, quarantine inbox, PC↔VPS copy
│   ├─ pty_gateway.rs       # guarded/raw terminal, command check, asciicast
│   ├─ preview.rs           # localhost-only HTTP forward
│   ├─ remote_input.rs      # chat/voice → Intent Engine → Trust Kernel
│   └─ os/{idle,sleep}_{windows,macos,linux}.rs
├─ sdcd/src/trust/          ← hook: permission broker → approval_router
├─ protocol/schema/remote/  ← নতুন বার্তা + TS types
├─ app/src/…/settings/Remote*   ← Settings → SDC Anywhere (devices, QR, policy, release hash)
├─ cloud/                   ← নতুন (TypeScript, Cloudflare)
│   ├─ wrangler.toml
│   ├─ src/worker.ts        # account, WebAuthn, magic link, push subscribe
│   ├─ src/hub.ts           # Durable Object: routing, mailbox, escalation
│   ├─ src/email/           # provider adapter
│   └─ migrations/          # D1
├─ web/                     ← নতুন (React + Vite PWA; app-এর tokens/components)
│   ├─ src/routes/{login,pair,inbox,chat,files,terminal,preview,hosts,devices,more}
│   ├─ src/crypto/          # WebCrypto device key, HPKE, AEAD
│   ├─ src/transport/       # mux, resume, priority
│   └─ src/sw.ts            # code pinning, push, preview proxy
├─ _verify/remote/          ← benchmark ও probe script
└─ docs/remote/
    ├─ SDC-ANYWHERE-PLAN-v2.md   ← এই ফাইল
    ├─ DESIGN.md, THREAT-MODEL.md, DNS-RECORDS.md, PERF.md
```

---

## 13. ফেজ পরিকল্পনা

### ফেজ ০: নকশা (code নয়)
`DESIGN.md`, `THREAT-MODEL.md`, `DNS-RECORDS.md`, `PERF.md` (benchmark পদ্ধতি); emailapi.txt থেকে provider চেনা।
**Done:** user লিখবেন "Phase 0 approved"।

### ফেজ ১ (0.17): নিরাপদ ভিত্তি
Identity, pairing (QR + SAS), E2E session, mux + resume, capability স্তর, relay + Hub + D1, passkey login, Approval Center (Allow once / Deny + কারণ), live stream, Kill, desktop Settings ও badge, OS idle/sleep।
**Done:** phone browser থেকে approve করলে PC-তে কাজ এগোয়; ভুল sig / replay / মেয়াদোত্তীর্ণ / revoked / hash অমিল, সব বাতিল (test সহ); relay বন্ধ থাকলে desktop স্বাভাবিক।

### ফেজ ২ (0.18): Remote Workspace (দেখা)
Host selector (local + VPS), File Explorer (list, open, search, git status), protected path, path-only `@file` attach, Chat (নতুন prompt, engine/model বাছাই), Edit-before-Allow, scoped allow, magic link + নতুন device অনুমোদন, guest session, Web Push + email escalation।
**Done:** উদাহরণ ২ শুরু থেকে শেষ phone দিয়ে করা যায়; path traversal test পাস।

### ফেজ ৩ (0.19): Remote Workspace (বদলানো)
Web editor + save (checkpoint), file op, upload (quarantine) / download, PC↔VPS copy, transfer engine, Guarded Terminal + predictive echo, blast-radius preview, নিজের ভাষায় risk, Pause/Resume/Rewind/Deploy।
**Done:** উদাহরণ ৩, ৪, ৫ কাজ করে; ১০০ MB upload network কাটার পরও resume হয়।

### ফেজ ৪ (0.20): উন্নত
E2E Preview Tunnel, WebRTC P2P transfer (+ TURN), Multi-machine dashboard, Two-Person Rule, code-pinning Service Worker + release hash, raw terminal, voice, Approval Learning, Away Mission Mode।
**Done:** উদাহরণ ৬; P2P থাকলে বড় ফাইল relay ছাড়া যায়।

### ফেজ ৫ (0.21+): ভবিষ্যৎ
Post-quantum hybrid HPKE (X25519 + ML-KEM-768), MLS (team), Wake-on-LAN, ঐচ্ছিক Telegram/WhatsApp notification।

---

## 14. Test ও Benchmark

- **Rust:** envelope hash স্থিতিশীলতা, sig/nonce/expiry, capability স্তর, path traversal ও symlink escape (তিন OS-এ), protected path, chunk resume, guarded command classifier
- **Cloud:** Miniflare/vitest: routing, mailbox TTL, rate limit, magic link একবার, GET-এ token খরচ না হওয়া
- **Web:** Playwright + WebAuthn virtual authenticator: pair → login → approve → file open → edit → upload → terminal
- **আক্রমণ test:** "খারাপ relay" mock যে বার্তা বদলায়/পুরনো পাঠায় → PC বাতিল করে; untrusted device কিছুই দেখে না; remote_forbidden বাতিল
- **Benchmark (`_verify/remote/`):** §7.2-এর প্রতিটা সারি মাপা (p50/p95), network throttling (4G profile, ১% packet loss), ফল `PERF.md`-এ; প্রতি release-এ আবার
- **CI:** Windows/macOS/Linux; accessibility (screen reader, RTL, বড় touch target)
- CHANGELOG, README, SETUP হালনাগাদ

---

## 15. ঝুঁকি ও খোলা প্রশ্ন

| বিষয় | মন্তব্য |
|---|---|
| Web app-এর JS server থেকে আসে | Code pinning + hash যাচাই কমায়, পুরো দূর করে না; ভবিষ্যতে native app বিকল্প |
| PC বন্ধ থাকলে কিছুই করা যায় না | স্বাভাবিক সীমা; VPS-এ সরাসরি daemon বসানো ভবিষ্যৎ বিকল্প (sdcd VPS-এও চলতে পারে) |
| Multi-user account (SkilledDesk) বনাম একক user | ফেজ ০-তে ঠিক করতে হবে: শুরুতে প্রতি user আলাদা account, team ফেজ ৪-এ |
| Cloudflare নির্ভরতা | Transport trait দিয়ে VPS relay-তে বদলযোগ্য |
| Pricing | Remote ফিচার কোন tier-এ, agency pilot-এর পরে |