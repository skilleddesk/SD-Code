// The email adapter. Configured entirely by variables, so the provider (OQ-14) can be settled without a code change:
//
//   EMAIL_PROVIDER   'sendknot' | 'resend' | 'generic'   (unset = email is off; nothing is sent and nothing fails)
//   EMAIL_API_KEY    a Worker secret. Read here, put in one request header, never logged, never returned.
//   EMAIL_FROM       e.g. `SDC <notify@skilleddesk.com>`
//   EMAIL_API_URL    'generic' only (and an override for 'resend'): the HTTPS endpoint that takes the JSON below
//   EMAIL_AUTH_HEADER / EMAIL_AUTH_SCHEME   'generic' only: default `Authorization` / `Bearer`
//
// 'generic' posts `{ from, to, subject, text, html }` as JSON, which is what most transactional providers accept (Resend,
// and many others). A provider with another shape gets its own small case here once it is known; none is guessed.

export interface EmailEnv {
  EMAIL_PROVIDER?: string;
  EMAIL_API_KEY?: string;
  EMAIL_FROM?: string;
  EMAIL_API_URL?: string;
  EMAIL_AUTH_HEADER?: string;
  EMAIL_AUTH_SCHEME?: string;
}

export interface Mail {
  to: string;
  subject: string;
  text: string;
  html: string;
}

export type MailResult = { sent: true } | { sent: false; reason: 'not_configured' | 'rejected' | 'unreachable' };

const RESEND_URL = 'https://api.resend.com/emails';
const SENDKNOT_URL = 'https://app.sendknot.com/v1/send';

/** True when every variable a send needs is present. The key's value is never inspected beyond "is there one". */
export function emailConfigured(env: EmailEnv): boolean {
  if (!env.EMAIL_PROVIDER || !env.EMAIL_API_KEY || !env.EMAIL_FROM) return false;
  if (env.EMAIL_PROVIDER === 'resend' || env.EMAIL_PROVIDER === 'sendknot') return true;

  // HTTPS only, except for a provider on this very machine (the end-to-end test runs a mail catcher there).
  return env.EMAIL_PROVIDER === 'generic' && /^(https:\/\/|http:\/\/(127\.0\.0\.1|localhost)(:\d+)?\/)/.test(env.EMAIL_API_URL ?? '');
}

/** A plain, conservative address check: one `@`, no spaces or control characters, a dot in the domain, sane length. */
export function validAddress(address: string): boolean {
  return address.length <= 254 && /^[^\s@<>(),;:"\\[\]]{1,64}@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/.test(address);
}

/** A key pasted through some tools arrives with a byte-order mark or a trailing newline. Neither belongs in a header. */
export function cleanKey(key: string): string {
  return key.replace(/^﻿/, '').trim();
}

export async function sendMail(rawEnv: EmailEnv, mail: Mail, fetcher: typeof fetch = fetch): Promise<MailResult> {
  const env: EmailEnv = { ...rawEnv, EMAIL_API_KEY: rawEnv.EMAIL_API_KEY === undefined ? undefined : cleanKey(rawEnv.EMAIL_API_KEY) };

  if (!emailConfigured(env)) return { sent: false, reason: 'not_configured' };
  // A header value must be printable ASCII; anything else would make the request fail with a message that quotes the key.
  if (!/^[\x21-\x7e]+$/.test(env.EMAIL_API_KEY!)) return { sent: false, reason: 'not_configured' };
  if (!validAddress(mail.to)) return { sent: false, reason: 'rejected' };

  const url =
    env.EMAIL_PROVIDER === 'resend' ? (env.EMAIL_API_URL ?? RESEND_URL) : env.EMAIL_PROVIDER === 'sendknot' ? (env.EMAIL_API_URL ?? SENDKNOT_URL) : env.EMAIL_API_URL!;
  // SendKnot's documented example uses `text_body` and `stream`; only documented fields are sent (no HTML part), so the mails are plain text.
  const payload =
    env.EMAIL_PROVIDER === 'sendknot'
      ? { from: env.EMAIL_FROM, to: [mail.to], subject: mail.subject, text_body: mail.text, stream: 'transactional' }
      : { from: env.EMAIL_FROM, to: [mail.to], subject: mail.subject, text: mail.text, html: mail.html };
  const header = env.EMAIL_PROVIDER === 'generic' ? (env.EMAIL_AUTH_HEADER || 'Authorization') : 'Authorization';
  const scheme = env.EMAIL_PROVIDER === 'generic' ? (env.EMAIL_AUTH_SCHEME ?? 'Bearer') : 'Bearer';
  const value = scheme ? `${scheme} ${env.EMAIL_API_KEY}` : env.EMAIL_API_KEY!;

  try {
    const response = await fetcher(url, {
      method: 'POST',
      headers: { [header]: value, 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });

    // The response body is not read or kept: an error page from a provider can echo request headers.
    return response.ok ? { sent: true } : { sent: false, reason: 'rejected' };
  } catch {
    return { sent: false, reason: 'unreachable' };
  }
}

const escapeHtml = (text: string) => text.replace(/[&<>"']/g, (char) => `&#${char.charCodeAt(0)};`);

/** The nudge: "something needs you" and a link. No command, file name or host (plan 5.10). */
export function approvalMail(to: string, link: string): Mail {
  return {
    to,
    subject: 'SDC needs your answer',
    text: `Something on your computer is waiting for your answer.\n\nOpen SDC: ${link}\n\nIf you were not expecting this, you can ignore it.`,
    html: `<p>Something on your computer is waiting for your answer.</p><p><a href="${escapeHtml(link)}">Open SDC</a></p><p>If you were not expecting this, you can ignore it.</p>`,
  };
}

/** The sign-in link. Opening it changes nothing; the button on the page it opens spends it (plan 5.1). */
export function magicMail(to: string, link: string): Mail {
  return {
    to,
    subject: 'Your SDC sign-in link',
    text: `Use this link within 10 minutes to add this browser to your SDC:\n\n${link}\n\nIt works once. After you open it you will still have to approve this browser on your computer, so the link alone gives nobody access.\n\nIf you did not ask for it, ignore this email.`,
    html: `<p>Use this link within 10 minutes to add this browser to your SDC:</p><p><a href="${escapeHtml(link)}">Continue to SDC</a></p><p>It works once. After you open it you will still have to approve this browser on your computer, so the link alone gives nobody access.</p><p>If you did not ask for it, ignore this email.</p>`,
  };
}
