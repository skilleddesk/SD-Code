# SDC Anywhere - DNS records for `sdc.skilleddesk.com`

Status: Phase 0 proposal. **You** add or delete records; nothing here is applied by Claude Code, and no `wrangler` command was run.
Scope rule: only the hostname `sdc.skilleddesk.com` and names below it. `skilleddesk.com` is the company's main domain.

## 1. Known facts (from you)

| Item | Value |
| --- | --- |
| DNS host | Cloudflare (zone `skilleddesk.com`) |
| Main site VPS | `109.199.108.216` (records are DNS only) |
| Existing root records | `A @`, `A www`, `A skilledguard`; `MX mx.sendknot.com`; SPF `v=spf1 include:spf.sendknot.com -all`; DKIM TXT selector `sk-cef511db`; DMARC `p=none` |
| Existing record to be replaced at deploy | `A sdc -> 109.199.108.216` (DNS only) |

## 2. Do not touch

Never add, edit or delete any of these, in any phase:

- `A @` (root), `A www`, `A skilledguard`
- root `MX` (`mx.sendknot.com`)
- root SPF TXT (`v=spf1 include:spf.sendknot.com -all`)
- the DKIM TXT for selector `sk-cef511db` (name as shown in your Cloudflare dashboard)
- root `_dmarc` TXT
- any CAA record on the root (if present, only *read* it, see check list)
- Cloudflare zone settings that apply to the whole zone (SSL mode, HSTS, "Always Use HTTPS" rules, Page Rules/Redirect Rules that match `*skilleddesk.com/*`). A zone-wide rule can change the main site; check, do not edit.

## 3. Records to add

### 3.1 Web: `sdc.skilleddesk.com`

No manual record. Add a **Custom Domain** to the Worker (or Pages project) in the Cloudflare dashboard (Workers & Pages -> project -> Settings -> Domains & Routes -> Add -> Custom domain -> `sdc.skilleddesk.com`). Cloudflare creates the DNS record itself (proxied). This works only because the zone is on Cloudflare, and only if no other record exists for that exact name, which is why step 4.3 comes first.

The PC needs no DNS record and no open port.

### 3.2 Email

The provider is not yet confirmed (DESIGN.md OQ-14). The key file `H:\emailapi.txt` contains one bare value, 58 characters, lowercase letters, digits and underscores, with no key name and no vendor prefix such as `re_`, `SG.` or `xkeysib-`. That shape does not identify a vendor. Your root records (`mx.sendknot.com`, `spf.sendknot.com`, selector `sk-...`) show SendKnot is your mail platform, and SendKnot's site describes REST API keys created in a console and per-domain verification with SPF, DKIM and DMARC. So:

**Branch A - the key is a SendKnot API key and `skilleddesk.com` is verified in SendKnot (most likely).**
- **Propose NO new email DNS records.**
- Send as `SDC <notify@skilleddesk.com>`. Root SPF, DKIM and DMARC already cover that domain.
- If, in the SendKnot console, the verified sending domain turns out to be `sdc.skilleddesk.com` (DKIM record under `sdc`), use `notify@sdc.skilleddesk.com` instead; still no new records unless the console lists unpublished ones.
- API auth: HTTPS REST with the key in an authorization header; exact header name and endpoint are `<FROM_DASHBOARD>`/provider docs, to be confirmed in Phase 1 before code. The key is only ever stored with `wrangler secret put EMAIL_API_KEY` (Phase 1) and in a gitignored `.dev.vars`; it is referred to by name only.

**Branch B - the key belongs to a different provider.** Records go **only under `sdc`**. Root SPF uses `-all` (hard fail): any mail whose envelope sender is `@skilleddesk.com` and is not from `spf.sendknot.com` will fail SPF and may be rejected, so the new provider must never send with a root-domain envelope sender or From domain. Use `From: SDC <notify@sdc.skilleddesk.com>` and a return-path under `sdc`.

All values are provider-specific; mark in the dashboard, copy exactly:

| Type | Name | Content | Proxy | Purpose |
| --- | --- | --- | --- | --- |
| TXT | `sdc` | `v=spf1 include:<FROM_DASHBOARD> -all` | n/a | SPF for the sdc subdomain only. This is a separate SPF record from the root one; it does not edit it |
| CNAME or TXT | `<DKIM_SELECTOR>._domainkey.sdc` | `<FROM_DASHBOARD>` | **DNS only (grey cloud)** | DKIM |
| CNAME | `<RETURN_PATH_HOST>.sdc` (often `bounce.sdc` or `send.sdc`) | `<FROM_DASHBOARD>` | **DNS only (grey cloud)** | return-path / bounce handling |
| MX | `<RETURN_PATH_HOST>.sdc` | `<FROM_DASHBOARD>`, priority `<FROM_DASHBOARD>` | n/a | only if the provider asks for it for bounces |
| TXT | `_dmarc.sdc` | `v=DMARC1; p=none; rua=mailto:<REPORT_ADDRESS>` | n/a | DMARC starts at `p=none`; `<REPORT_ADDRESS>` is a mailbox you choose |

Notes: (1) if the provider shows a hostname as full `x._domainkey.sdc.skilleddesk.com`, enter only the part before `.skilleddesk.com` in Cloudflare. (2) Never orange-cloud a DKIM or return-path CNAME. (3) The root DMARC already exists with `p=none`; `_dmarc.sdc` is optional because the root policy applies to subdomains, add it only if you want separate reports. (4) Move DMARC to `quarantine` only after Phase 2 mail has shown PASS for a few weeks, and only on the `sdc` record.

## 4. Deploy-time order (when we go live, not in Phase 0)

1. Deploy the Worker on its `*.workers.dev` URL first and test everything there. The old `sdc` A record is untouched so far.
2. **Record the old state:** screenshot or note exactly `A  sdc  109.199.108.216  DNS only  TTL <as shown>`. Confirm nothing real is served from the VPS at `sdc.skilleddesk.com` (open it, check nginx/Apache vhost for that name).
3. **You delete** the `A sdc` record in the Cloudflare DNS dashboard. (Cloudflare will not create the Custom Domain while a conflicting record exists.)
4. Immediately add the Custom Domain `sdc.skilleddesk.com` to the Worker/Pages project. Cloudflare creates the new proxied record. Certificate issuance is automatic; wait until the domain shows Active.
5. Add the email records of branch B only if branch B applies; wait for the provider's dashboard to show Verified.
6. Run the test checklist (section 6).

Expected window between steps 3 and 4: a few minutes, during which `sdc.skilleddesk.com` does not resolve to anything useful. It does not affect the root site or `www`.

Passkey note: the WebAuthn RP ID will be `sdc.skilleddesk.com`; changing the hostname later invalidates all passkeys. Do not go live on a throwaway hostname with passkeys enabled.

## 5. Rollback

If anything is wrong after step 4:

1. Dashboard -> Worker/Pages -> Domains -> remove the custom domain `sdc.skilleddesk.com` (this also removes its auto-created record).
2. Re-add exactly: `Type A | Name sdc | IPv4 109.199.108.216 | Proxy status DNS only (grey cloud) | TTL as you recorded in step 2`.
3. Delete only the new email records you added under `sdc` (never anything in section 2).
4. Verify: `nslookup sdc.skilleddesk.com` returns `109.199.108.216`; root site and `www` unchanged.

Nothing in the root zone is changed by the deploy or by the rollback, so there is nothing to restore at the root.

## 6. Test checklist

Web
- [ ] `nslookup sdc.skilleddesk.com` returns Cloudflare addresses (proxied) after step 4
- [ ] `https://sdc.skilleddesk.com` loads with a valid certificate; WebSocket upgrade works
- [ ] `https://skilleddesk.com`, `https://www.skilleddesk.com` and `skilledguard` behave exactly as before
- [ ] `curl -sI https://skilleddesk.com` shows whether HSTS `includeSubDomains` is set; if it is, `sdc` must be HTTPS only (it will be)
- [ ] root CAA records (if any) allow Cloudflare's CAs (Google Trust Services, Let's Encrypt, SSL.com, etc.); if a CAA record forbids them, certificate issuance for `sdc` fails. Read-only check

Email (after the first real send)
- [ ] Send one magic-link mail to a Gmail address
- [ ] Gmail -> three dots -> **Show original**: `SPF: PASS`, `DKIM: PASS`, `DMARC: PASS`
- [ ] The `DKIM` line shows the expected `d=` domain and the expected selector (`sk-cef511db` in branch A)
- [ ] Mail lands in Inbox, not Spam; the mail has no details of any action (plan section 5.10)
- [ ] Send one to an address that does not exist and confirm the bounce arrives where the provider reports it
- [ ] Root site's own mail (sent through SendKnot) still passes the same three checks

## 7. Open items for you

- Which provider owns `emailapi.txt`'s key (OQ-14)?
- What is served today at `sdc.skilleddesk.com` on `109.199.108.216` (OQ-15)?
- Are DKIM and the return-path for SendKnot published under the root or under `sdc`, if the console says so?
