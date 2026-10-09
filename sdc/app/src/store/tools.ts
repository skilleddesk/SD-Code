import { useEffect, useRef, useState } from 'react';

import type { ToolJob } from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { withProviders } from './reducer';
import { useAppStore } from './store';

/**
 * Installs run by the daemon (0.21) - `tool.install`, `ollama.pull` - followed until they end.
 *
 * The daemon does the work on its own thread and keeps the job; the window only asks how it is going,
 * so closing a dialog mid-install loses nothing: opening it again picks the same job up (`tool.install`
 * for a tool that is already installing answers with the running job).
 */

/** The ids the daemon can install itself (`host/tools.rs` TOOLS). */
export const INSTALLABLE = new Set(['node', 'claude', 'codex', 'gemini', 'ollama', 'ripgrep', 'git', 'browser']);

/** Roughly what each install downloads - said on its button before the press (the daemon's `TOOLS` sizes). */
export function toolSize(id: string): string | undefined {
  const mac = typeof navigator !== 'undefined' && /Mac/i.test(navigator.userAgent);
  const sizes: Record<string, string> = {
    node: '~30 MB',
    claude: '~60 MB',
    codex: '~50 MB',
    gemini: '~40 MB',
    ollama: mac ? '~30 MB' : '~1.5 GB',
    ripgrep: '~2 MB',
    git: '~60 MB',
    browser: '~100 MB',
  };

  return sizes[id];
}

export type JobStart = () => Promise<ToolJob>;

export interface JobView {
  job: ToolJob | null;
  /** The call that starts (or rejoins) the job failed before it began. */
  error: string | null;
  start: () => void;
}

/** Follows one job from `begin` to its end; `onDone` hears a success once. */
export function useJob(begin: JobStart, onDone?: (job: ToolJob) => void): JobView {
  const [job, setJob] = useState<ToolJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<number | null>(null);
  const done = useRef(onDone);

  done.current = onDone;

  useEffect(
    () => () => {
      if (timer.current !== null) {
        window.clearTimeout(timer.current);
      }
    },
    [],
  );

  const follow = (id: string): void => {
    timer.current = window.setTimeout(() => {
      void sdcpCall('tool.status', { jobId: id }).then(
        (next) => {
          setJob(next);

          if (next.state === 'running') {
            follow(id);
          } else if (next.state === 'done') {
            refreshProviders();
            done.current?.(next);
          }
        },
        () => follow(id),
      );
    }, 700);
  };

  const start = (): void => {
    setError(null);

    void begin().then(
      (first) => {
        setJob(first);

        if (first.state === 'running') {
          follow(first.id);
        }
      },
      (reason: unknown) => setError(isSdcpError(reason) ? reason.message : String(reason)),
    );
  };

  return { job, error, start };
}

/** The provider cards say whether their CLI is installed: after an install they are asked again. */
function refreshProviders(): void {
  void sdcpCall('provider.list', {}).then(
    ({ providers }) => useAppStore.setState((state) => withProviders(state, providers)),
    () => undefined,
  );
}

/** The tools SDC can install on a server, into ~/.sdc/tools (`host/tools.rs` remote_installable). */
export const REMOTE_INSTALLABLE = new Set(['node', 'claude', 'codex', 'gemini']);

/** Installs a tool here, or on the server `hostId` names. */
export const installTool = (id: string, hostId?: string): JobStart => () =>
  sdcpCall('tool.install', hostId === undefined || hostId === 'local' ? { id } : { id, hostId });

export const pullModel = (model: string): JobStart => () => sdcpCall('ollama.pull', { model });

/** `12.3 MB of 31.0 MB`, or what is known of it. */
export function sizeOf(bytes: number): string {
  if (bytes >= 1024 ** 3) {
    return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  }

  return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
}

/** The fraction done, or `null` while the size is unknown. */
export function fraction(job: ToolJob): number | null {
  return job.total !== null && job.total > 0 ? Math.min(1, job.done / job.total) : null;
}
