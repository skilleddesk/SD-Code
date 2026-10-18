import { create } from 'zustand';

import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { strings } from '../strings';

/** The morning brief and the weekly client report (Phase 4.8): the daemon builds the pages; the window shows them. */
interface BriefState {
  open: boolean;
  busy: boolean;
  kind: 'brief' | 'client' | null;
  headline: string;
  html: string;
  path: string;
  error: string | null;
  daily: boolean;
}

export const useBrief = create<BriefState>()(() => ({ open: false, busy: false, kind: null, headline: '', html: '', path: '', error: null, daily: false }));

/** The window's language (the document's `lang`), so the page is written in it. */
const language = (): string => globalThis.document?.documentElement.lang || 'en';

export function openBrief(): void {
  useBrief.setState({ open: true });
  void loadBrief();
  void sdcpCall('settings.get', { key: 'brief.daily' }).then(
    (answer) => useBrief.setState({ daily: answer.value === 'on' }),
    () => undefined,
  );
}

export function closeBrief(): void {
  useBrief.setState({ open: false });
}

export async function loadBrief(): Promise<void> {
  useBrief.setState({ busy: true, error: null, kind: 'brief' });

  try {
    const answer = await sdcpCall('brief.today', { lang: language() });

    useBrief.setState({ busy: false, headline: answer.headline, html: answer.html, path: answer.htmlPath });
  } catch (reason) {
    useBrief.setState({ busy: false, error: isSdcpError(reason) ? reason.message : strings.brief.failed });
  }
}

export async function loadClientReport(): Promise<void> {
  useBrief.setState({ busy: true, error: null, kind: 'client' });

  try {
    const answer = await sdcpCall('report.client', { days: 7, lang: language() });

    useBrief.setState({ busy: false, headline: strings.brief.clientTitle, html: answer.html, path: answer.htmlPath });
  } catch (reason) {
    useBrief.setState({ busy: false, error: isSdcpError(reason) ? reason.message : strings.brief.failed });
  }
}

export async function setDaily(on: boolean): Promise<void> {
  try {
    await sdcpCall('settings.set', { key: 'brief.daily', value: on ? 'on' : 'off' });
    useBrief.setState({ daily: on });
  } catch {
    /* unchanged */
  }
}
