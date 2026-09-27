import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  CircleDashed,
  Globe,
  LoaderCircle,
  Moon,
  Play,
  Plus,
  RotateCcw,
  ScanSearch,
  Send,
  Server,
  ShieldCheck,
  Trash2,
  Wand2,
  XCircle,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';

import type { ApprovalRecord, DeployRecord, DeployStep, Playbook, SiteRecord, XrayMap } from '../../../protocol/types';
import { currentLocale } from '../i18n';
import { openOutside } from '../lib/external';
import { strings } from '../strings';
import {
  checkHealth,
  createStaging,
  decideApproval,
  deploySite,
  detectSite,
  listApprovals,
  listDeploys,
  listPlaybooks,
  listSites,
  loadXray,
  pollApproval,
  previewRollback,
  rehearseMigration,
  removePlaybook,
  removeSite,
  restoreDatabase,
  rollbackDeploy,
  runPlaybook,
  savePlaybook,
  saveSite,
  scanHost,
  setGuardian,
  stopStaging,
} from '../store/kernelIntents';
import { useKernelUi, type AgencyTab } from '../store/kernelUi';
import { useModelStore } from '../store/model';
import { useAppStore } from '../store/store';
import { Modal } from '../modals/Modal';
import { Badge } from '../panels/ui/Badge';
import { BTN, BTN_DANGER, BTN_GHOST, BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../panels/ui/button';

/**
 * **The Agency hub** (0.12 - the plan's 0.14, 1.0 and 2.0 surfaces in one place): the sites SDC ships and
 * watches, and everything done to them.
 *
 *   Sites       Health Watch - uptime, response time, SSL days, disk, backup age, errors, last deploy,
 *               the Agency Ops Score with its reasons - and each site's own actions: Safe Deploy, a
 *               staging copy for the client, a migration rehearsed on a copy of the database
 *   Deploys     the pipeline view: every step with its log, the backup, one-click rollback with an Undo
 *               preview, the database restore (only on the word RESTORE), and "Fix with AI"
 *   Approvals   production deploys waiting for a second person, client sign-offs, the Guardian's fixes
 *   Playbooks   the same steps on many sites, each a Safe Deploy of its own
 *   X-ray       a read-only scan of an inherited server: map, document, risks
 *   Guardian    the Night Guardian per site, and what it did
 */
const k = strings.kernel.agency;

const inputClass = 'rounded-md border border-border-default bg-bg-input px-[8px] py-[5px] text-[12px] text-text-primary focus:border-border-focus';

function Title({ children }: { children: React.ReactNode }) {
  return <h3 className="mb-[8px] mt-[4px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{children}</h3>;
}

function StepIcon({ status }: { status: DeployStep['status'] }) {
  switch (status) {
    case 'pass':
      return <CheckCircle2 size={13} className="text-state-success" aria-hidden="true" />;
    case 'fail':
      return <XCircle size={13} className="text-state-error" aria-hidden="true" />;
    case 'running':
      return <LoaderCircle size={13} className="animate-spin text-accent motion-reduce:animate-none" aria-hidden="true" />;
    default:
      return <CircleDashed size={13} className="text-text-faint" aria-hidden="true" />;
  }
}

function stateTone(state: string): 'success' | 'warning' | 'muted' | 'accent' {
  if (state === 'success' || state === 'rolled_back') {
    return state === 'success' ? 'success' : 'warning';
  }

  return state === 'running' ? 'accent' : state === 'failed' || state === 'rollback_failed' ? 'warning' : 'muted';
}

/* ------------------------------------------------------------------------------------------------ */
/* Sites                                                                                            */
/* ------------------------------------------------------------------------------------------------ */

interface SiteDraft {
  siteId?: string;
  name: string;
  hostId: string;
  root: string;
  url: string;
  config: Record<string, unknown>;
}

function get(config: Record<string, unknown>, path: string): unknown {
  return path.split('.').reduce<unknown>((value, key) => (value !== null && typeof value === 'object' ? (value as Record<string, unknown>)[key] : undefined), config);
}

function set(config: Record<string, unknown>, path: string, value: unknown): Record<string, unknown> {
  const [head, ...rest] = path.split('.');

  if (head === undefined) {
    return config;
  }

  if (rest.length === 0) {
    return { ...config, [head]: value };
  }

  const inner = config[head];

  return { ...config, [head]: set(inner !== null && typeof inner === 'object' ? (inner as Record<string, unknown>) : {}, rest.join('.'), value) };
}

function SiteForm({ initial, onDone }: { initial: SiteDraft; onDone: () => void }) {
  const hosts = useAppStore((state) => state.hosts);
  const [draft, setDraft] = useState<SiteDraft>(initial);
  const [busy, setBusy] = useState(false);
  const text = (path: string): string => {
    const value = get(draft.config, path);

    return Array.isArray(value) ? value.join('\n') : value === null || value === undefined ? '' : String(value);
  };
  const setText = (path: string, value: string, list = false): void =>
    setDraft({ ...draft, config: set(draft.config, path, list ? value.split('\n').map((line) => line.trim()).filter((line) => line !== '') : value === '' ? null : value) });
  const flag = (path: string, fallback: boolean): boolean => {
    const value = get(draft.config, path);

    return typeof value === 'boolean' ? value : fallback;
  };

  return (
    <div className="flex flex-col gap-[10px] rounded-md border border-border-default bg-bg-raised p-[14px] text-[12px]">
      <div className="grid grid-cols-2 gap-[10px]">
        <label className="flex flex-col gap-[4px]">
          {k.form.name}
          <input className={inputClass} value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
        </label>
        <label className="flex flex-col gap-[4px]">
          {k.form.host}
          <select className={inputClass} value={draft.hostId} onChange={(event) => setDraft({ ...draft, hostId: event.target.value })}>
            {hosts.map((host) => (
              <option key={host.id} value={host.id}>
                {host.name}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-[4px]">
          {k.form.root}
          <input className={inputClass + ' font-mono'} dir="ltr" placeholder="/var/www/shop" value={draft.root} onChange={(event) => setDraft({ ...draft, root: event.target.value })} />
        </label>
        <label className="flex flex-col gap-[4px]">
          {k.form.url}
          <input className={inputClass + ' font-mono'} dir="ltr" placeholder="https://shop.example" value={draft.url} onChange={(event) => setDraft({ ...draft, url: event.target.value })} />
        </label>
      </div>

      <button
        type="button"
        className={BTN_SM + ' ' + BTN_SECONDARY + ' self-start'}
        disabled={draft.root.trim() === '' || busy}
        onClick={() => {
          setBusy(true);
          void detectSite(draft.hostId, draft.root).then((config) => {
            setBusy(false);

            if (config !== null) {
              setDraft({ ...draft, config: { ...config, ...draft.config, deploy: config.deploy, backup: config.backup, tools: config.tools, kind: config.kind } });
            }
          });
        }}
      >
        {busy ? <LoaderCircle size={11} className="animate-spin" aria-hidden="true" /> : <ScanSearch size={11} aria-hidden="true" />}
        {k.form.detect}
      </button>

      <div className="grid grid-cols-2 gap-[10px]">
        <label className="col-span-2 flex flex-col gap-[4px]">
          {k.form.steps}
          <textarea rows={4} className={inputClass + ' font-mono'} dir="ltr" value={text('deploy.steps')} onChange={(event) => setText('deploy.steps', event.target.value, true)} />
          <span className="text-[10.5px] text-text-muted">{k.form.stepsHelp}</span>
        </label>
        <label className="flex flex-col gap-[4px]">
          {k.form.restart}
          <input className={inputClass + ' font-mono'} dir="ltr" placeholder="pm2 reload all" value={text('deploy.restart')} onChange={(event) => setText('deploy.restart', event.target.value)} />
        </label>
        <label className="flex flex-col gap-[4px]">
          {k.form.backupDir}
          <input className={inputClass + ' font-mono'} dir="ltr" placeholder="~/.sdc/backups/…" value={text('backup.dir')} onChange={(event) => setText('backup.dir', event.target.value)} />
        </label>
        <label className="flex flex-col gap-[4px]">
          {k.form.dbKind}
          <select className={inputClass} value={text('backup.db.kind') || 'none'} onChange={(event) => setText('backup.db.kind', event.target.value)}>
            {['none', 'wordpress', 'mysql', 'postgres'].map((kind) => (
              <option key={kind} value={kind}>
                {k.form.db[kind as 'none']}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-[4px]">
          {k.form.dbName}
          <input className={inputClass + ' font-mono'} dir="ltr" value={text('backup.db.name')} onChange={(event) => setText('backup.db.name', event.target.value)} />
        </label>
        <label className="flex flex-col gap-[4px]">
          {k.form.healthText}
          <input className={inputClass} value={text('health.expectText')} onChange={(event) => setText('health.expectText', event.target.value)} />
        </label>
        <label className="flex flex-col gap-[4px]">
          {k.form.logFile}
          <input className={inputClass + ' font-mono'} dir="ltr" placeholder="/var/log/nginx/error.log" value={text('health.logFile')} onChange={(event) => setText('health.logFile', event.target.value)} />
        </label>
        <label className="flex flex-col gap-[4px]">
          {k.form.stagingUrl}
          <input className={inputClass + ' font-mono'} dir="ltr" value={text('staging.url')} onChange={(event) => setText('staging.url', event.target.value)} />
        </label>
        <label className="flex flex-col gap-[4px]">
          {k.form.stagingDb}
          <input className={inputClass + ' font-mono'} dir="ltr" value={text('staging.db')} onChange={(event) => setText('staging.db', event.target.value)} />
        </label>
      </div>

      <div className="flex flex-wrap gap-[16px]">
        {([
          ['production', true, k.form.production],
          ['requireApproval', false, k.form.requireApproval],
          ['health.enabled', true, k.form.watch],
        ] as const).map(([path, fallback, label]) => (
          <label key={path} className="flex items-center gap-[6px]">
            <input type="checkbox" className="accent-[var(--accent)]" checked={flag(path, fallback)} onChange={(event) => setDraft({ ...draft, config: set(draft.config, path, event.target.checked) })} />
            {label}
          </label>
        ))}
      </div>

      <div className="flex gap-[8px]">
        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY}
          disabled={draft.name.trim() === '' || draft.root.trim() === ''}
          onClick={() => void saveSite({ ...draft, ...(draft.siteId === undefined ? {} : { siteId: draft.siteId }) }).then((id) => (id !== null ? onDone() : undefined))}
        >
          {k.form.save}
        </button>
        <button type="button" className={BTN + ' ' + BTN_SECONDARY} onClick={onDone}>
          {k.form.cancel}
        </button>
      </div>
    </div>
  );
}

function HealthLine({ site }: { site: SiteRecord }) {
  const live = useAppStore((state) => state.kernel.health[site.id]);
  const report = live?.report ?? site.health ?? null;

  if (report === null) {
    return <p className="text-[11.5px] text-text-muted">{k.health.notChecked}</p>;
  }

  const up = report.http.ok;
  const score = live?.score ?? report.score?.score ?? null;
  const level = live?.level ?? report.score?.level ?? null;
  const reasons = live?.reasons ?? report.score?.reasons ?? [];

  return (
    <div className="flex flex-col gap-[6px]">
      <div className="flex flex-wrap items-center gap-[8px] text-[11.5px]">
        <Badge tone={up === true ? 'success' : up === false ? 'warning' : 'muted'} icon={Activity}>
          {up === true ? k.health.up : up === false ? k.health.down : k.health.unknown}
        </Badge>
        <span className="text-text-secondary">{report.http.detail}</span>
        {report.ssl?.days === null || report.ssl?.days === undefined ? null : <Badge tone={report.ssl.days < 14 ? 'warning' : 'neutral'}>{k.health.ssl(report.ssl.days)}</Badge>}
        {report.disk?.percent === null || report.disk?.percent === undefined ? null : <Badge tone={report.disk.percent >= 85 ? 'warning' : 'neutral'}>{k.health.disk(report.disk.percent)}</Badge>}
        {report.backup?.ageHours === null || report.backup?.ageHours === undefined ? (
          <Badge tone="warning">{k.health.noBackup}</Badge>
        ) : (
          <Badge tone={report.backup.ageHours > 48 ? 'warning' : 'neutral'}>{k.health.backup(Math.round(report.backup.ageHours))}</Badge>
        )}
        {report.errors === null ? null : <Badge tone={report.errors.count > 50 ? 'warning' : 'neutral'}>{k.health.errors(report.errors.count)}</Badge>}
      </div>
      {score === null || level === null ? null : (
        <p className="text-[11.5px] text-text-secondary" title={reasons.map((reason) => reason.text).join(' · ')}>
          <span className="font-semibold">{k.health.ops(score, k.health.level[level])}</span>
          {reasons.filter((reason) => reason.delta !== 0).length === 0 ? null : ` · ${reasons.filter((reason) => reason.delta !== 0).map((reason) => reason.text).join(' · ')}`}
        </p>
      )}
    </div>
  );
}

function StagingPanel({ site, onClose }: { site: SiteRecord; onClose: () => void }) {
  const status = useAppStore((state) => state.kernel.staging[site.id]);
  const [summary, setSummary] = useState('');
  const [changes, setChanges] = useState('');
  const [lang, setLang] = useState<string>(currentLocale());

  return (
    <div className="mt-[8px] flex flex-col gap-[8px] rounded-md border border-border-subtle bg-bg-base p-[10px] text-[12px]">
      <p className="text-text-muted">{k.staging.help}</p>
      <textarea rows={3} className={inputClass} placeholder={k.staging.summary} value={summary} onChange={(event) => setSummary(event.target.value)} dir="auto" />
      <textarea rows={3} className={inputClass} placeholder={k.staging.changes} value={changes} onChange={(event) => setChanges(event.target.value)} dir="auto" />
      <label className="flex items-center gap-[6px]">
        {k.staging.language}
        <select className={inputClass} value={lang} onChange={(event) => setLang(event.target.value)}>
          {['en', 'bn', 'hi', 'ar', 'es'].map((code) => (
            <option key={code} value={code}>
              {code}
            </option>
          ))}
        </select>
      </label>
      <div className="flex flex-wrap gap-[8px]">
        <button
          type="button"
          className={BTN_SM + ' ' + BTN_PRIMARY}
          disabled={status?.state === 'copying'}
          onClick={() => void createStaging({ siteId: site.id, summary, changes: changes.split('\n').filter((line) => line.trim() !== ''), lang })}
        >
          <Send size={11} aria-hidden="true" />
          {k.staging.create}
        </button>
        <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void stopStaging(site.id, false)}>
          {k.staging.stop}
        </button>
        <button type="button" className={BTN_SM + ' ' + BTN_GHOST} onClick={onClose}>
          {k.form.cancel}
        </button>
      </div>
      {status === undefined ? null : (
        <div className="text-[11.5px]">
          <span className={status.state === 'failed' ? 'text-state-error' : 'text-text-secondary'}>{k.staging.state[status.state]}</span>
          {status.error === undefined ? null : <p className="text-state-error">{status.error}</p>}
          {status.page === undefined ? null : (
            <button type="button" className="block font-mono text-accent hover:underline" dir="ltr" onClick={() => void openOutside(status.page ?? '')}>
              {status.page}
            </button>
          )}
          {status.state === 'ready' && status.php === false ? <p className="text-text-muted">{k.staging.noPhp}</p> : null}
        </div>
      )}
    </div>
  );
}

function ShadowPanel({ site, onClose }: { site: SiteRecord; onClose: () => void }) {
  const [command, setCommand] = useState('DB_DATABASE={db} php artisan migrate --force');
  const [runId, setRunId] = useState<string | null>(null);
  const run = useAppStore((state) => (runId === null ? undefined : state.kernel.shadow[runId]));

  return (
    <div className="mt-[8px] flex flex-col gap-[8px] rounded-md border border-border-subtle bg-bg-base p-[10px] text-[12px]">
      <p className="text-text-muted">{k.shadow.help}</p>
      <input className={inputClass + ' font-mono'} dir="ltr" value={command} onChange={(event) => setCommand(event.target.value)} />
      <div className="flex gap-[8px]">
        <button type="button" className={BTN_SM + ' ' + BTN_PRIMARY} disabled={run?.state === 'running'} onClick={() => void rehearseMigration(site.id, command).then(setRunId)}>
          <Play size={11} aria-hidden="true" />
          {k.shadow.run}
        </button>
        <button type="button" className={BTN_SM + ' ' + BTN_GHOST} onClick={onClose}>
          {k.form.cancel}
        </button>
      </div>
      {run === undefined ? null : run.state === 'running' ? (
        <p className="text-text-secondary">{k.shadow.running}</p>
      ) : (
        <div className="flex flex-col gap-[4px]">
          <p className={run.result?.passed === true ? 'text-state-success' : 'text-state-error'}>{run.result?.sentence}</p>
          {(run.result?.schemaChanges.length ?? 0) === 0 ? null : (
            <pre className="max-h-[160px] overflow-auto rounded-sm bg-bg-input p-[8px] font-mono text-[10.5px]" dir="ltr">
              {run.result?.schemaChanges.join('\n')}
            </pre>
          )}
          {(run.result?.output.length ?? 0) === 0 ? null : (
            <pre className="max-h-[160px] overflow-auto rounded-sm bg-bg-input p-[8px] font-mono text-[10.5px] text-text-muted" dir="ltr">
              {run.result?.output.join('\n')}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

function SitesTab({ onDeploys }: { onDeploys: () => void }) {
  const hosts = useAppStore((state) => state.hosts);
  const [sites, setSites] = useState<SiteRecord[] | null>(null);
  const [editing, setEditing] = useState<SiteDraft | null>(null);
  const [panel, setPanel] = useState<{ siteId: string; kind: 'staging' | 'shadow' } | null>(null);
  const reload = useCallback(() => void listSites().then(setSites), []);

  useEffect(() => reload(), [reload]);

  if (editing !== null) {
    return (
      <SiteForm
        initial={editing}
        onDone={() => {
          setEditing(null);
          reload();
        }}
      />
    );
  }

  return (
    <div className="flex flex-col gap-[10px]">
      <div className="flex items-center justify-between">
        <p className="text-[12px] text-text-muted">{k.sitesHelp}</p>
        <button
          type="button"
          className={BTN + ' ' + BTN_PRIMARY}
          onClick={() => setEditing({ name: '', hostId: hosts.find((host) => host.type === 'vps')?.id ?? 'local', root: '', url: '', config: { production: true } })}
        >
          <Plus size={12} aria-hidden="true" />
          {k.addSite}
        </button>
      </div>

      {sites === null ? <p className="text-[12px] text-text-muted">{k.loading}</p> : null}
      {sites !== null && sites.length === 0 ? <p className="rounded-md border border-dashed border-border-default p-[18px] text-center text-[12.5px] text-text-muted">{k.noSites}</p> : null}

      {(sites ?? []).map((site) => (
        <article key={site.id} className="rounded-md border border-border-subtle bg-bg-raised p-[12px]">
          <header className="mb-[6px] flex flex-wrap items-center gap-[8px]">
            <Globe size={14} className="text-accent" aria-hidden="true" />
            <span className="text-[13px] font-semibold text-text-primary">{site.name}</span>
            <span className="font-mono text-[11px] text-text-muted" dir="ltr">
              {site.url || site.root}
            </span>
            <Badge tone="muted" icon={Server}>
              {hosts.find((host) => host.id === site.hostId)?.name ?? site.hostId}
            </Badge>
            {site.lastDeploy === null || site.lastDeploy === undefined ? null : <Badge tone={stateTone(site.lastDeploy.state)}>{k.deployState[site.lastDeploy.state]}</Badge>}
          </header>
          <HealthLine site={site} />
          <div className="mt-[8px] flex flex-wrap gap-[6px]">
            <button type="button" className={BTN_SM + ' ' + BTN_PRIMARY} onClick={() => void deploySite(site.id).then((result) => (result?.deployId === undefined ? undefined : onDeploys()))}>
              <Play size={11} aria-hidden="true" />
              {k.deploy}
            </button>
            <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void checkHealth(site.id)}>
              <Activity size={11} aria-hidden="true" />
              {k.checkHealth}
            </button>
            <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => setPanel({ siteId: site.id, kind: 'staging' })}>
              <Send size={11} aria-hidden="true" />
              {k.sendForApproval}
            </button>
            <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => setPanel({ siteId: site.id, kind: 'shadow' })}>
              {k.rehearse}
            </button>
            <button
              type="button"
              className={BTN_SM + ' ' + BTN_SECONDARY}
              onClick={() => setEditing({ siteId: site.id, name: site.name, hostId: site.hostId, root: site.root, url: site.url, config: site.config })}
            >
              {k.edit}
            </button>
            <button type="button" className={BTN_SM + ' ' + BTN_DANGER} onClick={() => void removeSite(site.id).then(reload)}>
              <Trash2 size={11} aria-hidden="true" />
              {k.remove}
            </button>
          </div>
          {panel?.siteId === site.id && panel.kind === 'staging' ? <StagingPanel site={site} onClose={() => setPanel(null)} /> : null}
          {panel?.siteId === site.id && panel.kind === 'shadow' ? <ShadowPanel site={site} onClose={() => setPanel(null)} /> : null}
        </article>
      ))}
    </div>
  );
}

/* ------------------------------------------------------------------------------------------------ */
/* Deploys: the pipeline view                                                                       */
/* ------------------------------------------------------------------------------------------------ */

function DeploysTab() {
  const live = useAppStore((state) => state.kernel.deploys);
  const [stored, setStored] = useState<DeployRecord[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  const [preview, setPreview] = useState<{ path: string; change: string }[] | null>(null);
  const [confirmDb, setConfirmDb] = useState('');
  const liveCount = Object.keys(live).length;

  useEffect(() => void listDeploys().then(setStored), [liveCount]);

  /* The stored rows, with any live snapshot laid over its row (it is newer). */
  const deploys = useMemo(() => {
    const byId = new Map<string, DeployRecord>();

    for (const row of stored) {
      byId.set(row.id, row);
    }

    for (const snapshot of Object.values(live)) {
      const base = byId.get(snapshot.deployId);

      byId.set(snapshot.deployId, {
        id: snapshot.deployId,
        siteId: snapshot.siteId,
        kind: snapshot.kind,
        state: snapshot.state,
        steps: snapshot.steps,
        backup: snapshot.backup,
        note: snapshot.note,
        startedAt: base?.startedAt ?? snapshot.ts,
        finishedAt: base?.finishedAt ?? null,
      });
    }

    return [...byId.values()].sort((a, b) => b.startedAt.localeCompare(a.startedAt));
  }, [stored, live]);

  const deploy = deploys.find((row) => row.id === chosen) ?? deploys[0] ?? null;

  if (deploy === null) {
    return <p className="rounded-md border border-dashed border-border-default p-[18px] text-center text-[12.5px] text-text-muted">{k.noDeploys}</p>;
  }

  const failed = deploy.steps.find((step) => step.status === 'fail');

  return (
    <div className="grid grid-cols-[220px_1fr] gap-[12px] max-700:grid-cols-1">
      <ul className="flex max-h-[60vh] flex-col gap-[4px] overflow-y-auto">
        {deploys.map((row) => (
          <li key={row.id}>
            <button
              type="button"
              className={'w-full rounded-md border px-[8px] py-[6px] text-left text-[11.5px] ' + (row.id === deploy.id ? 'border-border-focus bg-accent-subtle' : 'border-border-subtle bg-bg-raised hover:bg-bg-hover')}
              onClick={() => {
                setChosen(row.id);
                setPreview(null);
              }}
            >
              <span className="block font-mono text-[10.5px] text-text-muted">{row.startedAt.slice(0, 16).replace('T', ' ')}</span>
              <span className="flex items-center gap-[6px]">
                <Badge tone={stateTone(row.state)}>{k.deployState[row.state]}</Badge>
                <span className="truncate">{row.kind}</span>
              </span>
            </button>
          </li>
        ))}
      </ul>

      <div className="flex flex-col gap-[10px] text-[12px]">
        <p className="text-text-primary">{deploy.note}</p>
        <ol className="flex flex-col gap-[6px]">
          {deploy.steps.map((step) => (
            <li key={step.id} className="rounded-md border border-border-subtle bg-bg-raised">
              <div className="flex items-center gap-[8px] px-[10px] py-[6px]">
                <StepIcon status={step.status} />
                <span className="min-w-0 flex-1 truncate">{step.name}</span>
                <span className="font-mono text-[10.5px] text-text-muted">{step.ms === null ? k.stepState[step.status] : `${(step.ms / 1000).toFixed(1)}s`}</span>
              </div>
              {step.tail.length === 0 ? null : (
                <pre className="max-h-[140px] overflow-auto border-t border-border-subtle bg-bg-input px-[10px] py-[6px] font-mono text-[10.5px] text-text-secondary" dir="ltr">
                  {step.tail.join('\n')}
                </pre>
              )}
            </li>
          ))}
        </ol>

        {deploy.backup === null ? null : (
          <p className="font-mono text-[10.5px] text-text-muted" dir="ltr">
            {k.backupAt(deploy.backup.files?.path ?? '—', deploy.backup.db?.path ?? '—')}
          </p>
        )}

        <div className="flex flex-wrap gap-[6px]">
          <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} disabled={deploy.backup?.files === null || deploy.backup === null} onClick={() => void previewRollback(deploy.id).then((result) => setPreview(result?.changes ?? null))}>
            {k.undoPreview}
          </button>
          <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} disabled={deploy.backup?.files === null || deploy.backup === null || deploy.state === 'running'} onClick={() => void rollbackDeploy(deploy.id)}>
            <RotateCcw size={11} aria-hidden="true" />
            {k.rollback}
          </button>
          {failed === undefined ? null : (
            <button
              type="button"
              className={BTN_SM + ' ' + BTN_SECONDARY}
              onClick={() => {
                useModelStore.getState().setDraft(k.fixPrompt(failed.name, failed.tail.join('\n')));
                useKernelUi.getState().closeAgency();
              }}
            >
              <Wand2 size={11} aria-hidden="true" />
              {k.fixWithAi}
            </button>
          )}
        </div>

        {preview === null ? null : (
          <div className="rounded-md border border-border-subtle p-[8px]">
            <Title>{k.previewTitle(preview.length)}</Title>
            <ul className="max-h-[180px] overflow-y-auto font-mono text-[10.5px]" dir="ltr">
              {preview.map((change) => (
                <li key={change.path}>
                  {k.change[change.change as 'added']} {change.path}
                </li>
              ))}
            </ul>
          </div>
        )}

        {deploy.backup?.db === null || deploy.backup === null ? null : (
          <div className="flex flex-wrap items-center gap-[6px] rounded-md border border-state-waiting p-[8px]">
            <AlertTriangle size={12} className="text-state-waiting" aria-hidden="true" />
            <span className="text-[11.5px] text-text-secondary">{k.dbRestoreHelp}</span>
            <input className={inputClass + ' w-[110px] font-mono'} placeholder="RESTORE" value={confirmDb} onChange={(event) => setConfirmDb(event.target.value)} />
            <button type="button" className={BTN_SM + ' ' + BTN_DANGER} disabled={confirmDb !== 'RESTORE'} onClick={() => void restoreDatabase(deploy.id).then(() => setConfirmDb(''))}>
              {k.restoreDb}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

/* ------------------------------------------------------------------------------------------------ */
/* Approvals, playbooks, X-ray, the Guardian                                                        */
/* ------------------------------------------------------------------------------------------------ */

function ApprovalsTab() {
  const live = useAppStore((state) => state.kernel.approvals);
  const [stored, setStored] = useState<ApprovalRecord[]>([]);
  const count = Object.keys(live).length;

  useEffect(() => void listApprovals().then(setStored), [count]);

  const approvals = useMemo(() => {
    const byId = new Map(stored.map((row) => [row.id, row]));

    for (const row of Object.values(live)) {
      byId.set(row.id, row);
    }

    return [...byId.values()].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  }, [stored, live]);

  if (approvals.length === 0) {
    return <p className="rounded-md border border-dashed border-border-default p-[18px] text-center text-[12.5px] text-text-muted">{k.noApprovals}</p>;
  }

  return (
    <ul className="flex flex-col gap-[8px]">
      {approvals.map((approval) => {
        const guardian = approval.kind === 'guardian-fix' ? (JSON.parse(approval.note || '{}') as { prompt?: string }) : null;

        return (
          <li key={approval.id} className="rounded-md border border-border-subtle bg-bg-raised p-[10px] text-[12px]">
            <div className="flex flex-wrap items-center gap-[8px]">
              <Badge tone={approval.state === 'pending' ? 'warning' : approval.state === 'approved' ? 'success' : 'muted'}>{k.approvalState[approval.state]}</Badge>
              <span className="font-semibold">{k.approvalKind[approval.kind as 'deploy'] ?? approval.kind}</span>
              <span className="font-mono text-[10.5px] text-text-muted">{approval.subject}</span>
              <span className="ml-auto text-[10.5px] text-text-muted">{k.requestedBy(approval.requestedBy)}</span>
            </div>
            {guardian === null ? <p className="mt-[4px] text-text-secondary" dir="auto">{approval.note}</p> : null}
            {approval.state !== 'pending' ? null : (
              <div className="mt-[8px] flex flex-wrap gap-[6px]">
                {approval.kind === 'client' ? (
                  <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void pollApproval(approval.id)}>
                    {k.checkAnswer}
                  </button>
                ) : null}
                {guardian?.prompt === undefined ? null : (
                  <button
                    type="button"
                    className={BTN_SM + ' ' + BTN_PRIMARY}
                    onClick={() => {
                      useModelStore.getState().setDraft(guardian.prompt ?? '');
                      void decideApproval(approval.id, 'approved');
                      useKernelUi.getState().closeAgency();
                    }}
                  >
                    <Wand2 size={11} aria-hidden="true" />
                    {k.openFix}
                  </button>
                )}
                {guardian === null ? (
                  <>
                    <button type="button" className={BTN_SM + ' ' + BTN_PRIMARY} onClick={() => void decideApproval(approval.id, 'approved')}>
                      {k.approve}
                    </button>
                    <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => void decideApproval(approval.id, 'declined')}>
                      {k.decline}
                    </button>
                  </>
                ) : null}
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
}

function PlaybooksTab() {
  const [playbooks, setPlaybooks] = useState<Playbook[]>([]);
  const [sites, setSites] = useState<SiteRecord[]>([]);
  const [chosen, setChosen] = useState<string[]>([]);
  const [draft, setDraft] = useState<{ name: string; steps: string } | null>(null);
  const [result, setResult] = useState<string | null>(null);

  useEffect(() => {
    void listPlaybooks().then(setPlaybooks);
    void listSites().then(setSites);
  }, []);

  return (
    <div className="flex flex-col gap-[12px] text-[12px]">
      <p className="text-text-muted">{k.playbooksHelp}</p>
      <fieldset className="flex flex-wrap gap-[12px]">
        <legend className="mb-[4px] text-[10px] font-semibold uppercase tracking-[.1em] text-text-muted">{k.onSites}</legend>
        {sites.length === 0 ? <span className="text-text-muted">{k.noSites}</span> : null}
        {sites.map((site) => (
          <label key={site.id} className="flex items-center gap-[6px]">
            <input
              type="checkbox"
              className="accent-[var(--accent)]"
              checked={chosen.includes(site.id)}
              onChange={(event) => setChosen(event.target.checked ? [...chosen, site.id] : chosen.filter((id) => id !== site.id))}
            />
            {site.name}
          </label>
        ))}
      </fieldset>
      <ul className="flex flex-col gap-[6px]">
        {playbooks.map((playbook) => (
          <li key={playbook.id} className="flex flex-wrap items-center gap-[8px] rounded-md border border-border-subtle bg-bg-raised px-[10px] py-[8px]">
            <span className="font-semibold">{playbook.name}</span>
            <span className="text-text-muted">{k.stepCount(playbook.steps.length)}</span>
            <span className="ml-auto flex gap-[6px]">
              <button
                type="button"
                className={BTN_SM + ' ' + BTN_PRIMARY}
                disabled={chosen.length === 0 && playbook.steps.some((step) => step.kind === 'command')}
                onClick={() =>
                  void runPlaybook(playbook.id, chosen).then((run) => {
                    if (run === null) {
                      return;
                    }

                    setResult(k.playbookStarted(run.deploys.length));

                    if (run.prompts[0] !== undefined) {
                      useModelStore.getState().setDraft(run.prompts[0]);
                    }
                  })
                }
              >
                <Play size={11} aria-hidden="true" />
                {k.run}
              </button>
              <button type="button" className={BTN_SM + ' ' + BTN_GHOST} onClick={() => void removePlaybook(playbook.id).then((list) => (list === null ? undefined : setPlaybooks(list)))}>
                <Trash2 size={11} aria-hidden="true" />
              </button>
            </span>
          </li>
        ))}
      </ul>
      {result === null ? null : <p className="text-state-success">{result}</p>}
      {draft === null ? (
        <button type="button" className={BTN_SM + ' ' + BTN_SECONDARY + ' self-start'} onClick={() => setDraft({ name: '', steps: '' })}>
          <Plus size={11} aria-hidden="true" />
          {k.newPlaybook}
        </button>
      ) : (
        <div className="flex flex-col gap-[8px] rounded-md border border-border-default p-[10px]">
          <input className={inputClass} placeholder={k.form.name} value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.target.value })} />
          <textarea rows={4} className={inputClass + ' font-mono'} dir="ltr" placeholder={k.playbookStepsHint} value={draft.steps} onChange={(event) => setDraft({ ...draft, steps: event.target.value })} />
          <div className="flex gap-[8px]">
            <button
              type="button"
              className={BTN_SM + ' ' + BTN_PRIMARY}
              disabled={draft.name.trim() === ''}
              onClick={() =>
                void savePlaybook({
                  name: draft.name,
                  steps: draft.steps
                    .split('\n')
                    .map((line) => line.trim())
                    .filter((line) => line !== '')
                    .map((line) => (line.toLowerCase().startsWith('ai:') ? { kind: 'prompt' as const, text: line.slice(3).trim() } : { kind: 'command' as const, text: line.replace(/^cmd:/i, '').trim() })),
                }).then((list) => {
                  if (list !== null) {
                    setPlaybooks(list);
                    setDraft(null);
                  }
                })
              }
            >
              {k.form.save}
            </button>
            <button type="button" className={BTN_SM + ' ' + BTN_GHOST} onClick={() => setDraft(null)}>
              {k.form.cancel}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

function XrayTab() {
  const allHosts = useAppStore((state) => state.hosts);
  const hosts = useMemo(() => allHosts.filter((host) => host.type === 'vps'), [allHosts]);
  const [hostId, setHostId] = useState<string>(hosts[0]?.id ?? '');
  const live = useAppStore((state) => state.kernel.xray[hostId]);
  const [stored, setStored] = useState<XrayMap | null>(null);
  const [scanning, setScanning] = useState(false);

  useEffect(() => {
    if (hostId !== '') {
      void loadXray(hostId).then(setStored);
    }
  }, [hostId]);

  useEffect(() => {
    if (live !== undefined) {
      setScanning(false);
    }
  }, [live]);

  const map = live?.map ?? stored;

  if (hosts.length === 0) {
    return <p className="rounded-md border border-dashed border-border-default p-[18px] text-center text-[12.5px] text-text-muted">{k.xray.noHosts}</p>;
  }

  return (
    <div className="flex flex-col gap-[12px] text-[12px]">
      <p className="text-text-muted">{k.xray.help}</p>
      <div className="flex flex-wrap items-center gap-[8px]">
        <select className={inputClass} value={hostId} onChange={(event) => setHostId(event.target.value)}>
          {hosts.map((host) => (
            <option key={host.id} value={host.id}>
              {host.name}
            </option>
          ))}
        </select>
        <button
          type="button"
          className={BTN_SM + ' ' + BTN_PRIMARY}
          disabled={scanning}
          onClick={() => {
            setScanning(true);
            void scanHost(hostId);
          }}
        >
          {scanning ? <LoaderCircle size={11} className="animate-spin" aria-hidden="true" /> : <ScanSearch size={11} aria-hidden="true" />}
          {scanning ? k.xray.scanning : k.xray.scan}
        </button>
        {live?.path === undefined || live.path === null ? null : (
          <button type="button" className="font-mono text-[10.5px] text-accent hover:underline" dir="ltr" onClick={() => void openOutside(`file:///${(live.path ?? '').replace(/\\/g, '/')}`)}>
            {live.path}
          </button>
        )}
      </div>
      {live?.error === null || live?.error === undefined ? null : <p className="text-state-error">{live.error}</p>}
      {map === null ? null : (
        <>
          <p className="text-text-secondary">
            {map.os} · {map.uptime} · {k.xray.scannedAt(map.scannedAt.slice(0, 16).replace('T', ' '))}
          </p>
          <section>
            <Title>{k.xray.risks(map.risks.length)}</Title>
            <ul className="flex flex-col gap-[6px]">
              {map.risks.map((risk) => (
                <li key={risk.title} className="rounded-md border border-border-subtle bg-bg-raised px-[10px] py-[6px]">
                  <div className="flex items-center gap-[6px]">
                    <Badge tone={risk.level === 'low' ? 'muted' : 'warning'}>{k.xray.level[risk.level]}</Badge>
                    <span className="font-semibold">{risk.title}</span>
                  </div>
                  <p className="text-text-secondary">{risk.why}</p>
                  <p className="text-text-muted">{k.xray.fix(risk.fix)}</p>
                </li>
              ))}
            </ul>
          </section>
          <div className="grid grid-cols-2 gap-[12px] max-700:grid-cols-1">
            {([
              ['sites', map.sites.map((site) => `${site.names.join(', ')} → ${site.root ?? '?'}${site.ssl ? ' · HTTPS' : ''}`)],
              ['wordpress', map.wordpress],
              ['databases', map.databases],
              ['services', map.services],
              ['cron', map.cron],
              ['docker', map.docker],
              ['ports', map.ports],
              ['runtimes', map.runtimes],
              ['certificates', map.certificates.map((cert) => `${cert.name} · ${cert.days ?? '?'}d`)],
              ['backups', map.backups],
            ] as const).map(([key, items]) => (
              <section key={key}>
                <Title>{k.xray.section[key]}</Title>
                {items.length === 0 ? <p className="text-text-muted">{k.xray.none}</p> : null}
                <ul className="max-h-[150px] overflow-y-auto font-mono text-[10.5px] text-text-secondary" dir="ltr">
                  {items.map((item) => (
                    <li key={item} className="truncate" title={item}>
                      {item}
                    </li>
                  ))}
                </ul>
              </section>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function GuardianTab() {
  const actions = useAppStore((state) => state.kernel.guardian);
  const alerts = useAppStore((state) => state.kernel.alerts);
  const [sites, setSites] = useState<SiteRecord[]>([]);

  useEffect(() => void listSites().then(setSites), []);

  return (
    <div className="flex flex-col gap-[14px] text-[12px]">
      <p className="text-text-muted">{k.guardian.help}</p>
      <ul className="flex flex-col gap-[6px]">
        {sites.map((site) => {
          const guardian = (site.config.guardian ?? {}) as { enabled?: boolean; autoRollback?: boolean };

          return (
            <li key={site.id} className="flex flex-wrap items-center gap-[12px] rounded-md border border-border-subtle bg-bg-raised px-[10px] py-[8px]">
              <Moon size={13} className="text-purple" aria-hidden="true" />
              <span className="font-semibold">{site.name}</span>
              <label className="flex items-center gap-[6px]">
                <input
                  type="checkbox"
                  className="accent-[var(--accent)]"
                  checked={guardian.enabled === true}
                  onChange={(event) => void setGuardian(site.id, event.target.checked, guardian.autoRollback === true).then(() => listSites().then(setSites))}
                />
                {k.guardian.watch}
              </label>
              <label className="flex items-center gap-[6px]">
                <input
                  type="checkbox"
                  className="accent-[var(--accent)]"
                  checked={guardian.autoRollback === true}
                  disabled={guardian.enabled !== true}
                  onChange={(event) => void setGuardian(site.id, true, event.target.checked).then(() => listSites().then(setSites))}
                />
                {k.guardian.autoRollback}
              </label>
            </li>
          );
        })}
      </ul>
      <section>
        <Title>{k.guardian.log}</Title>
        {actions.length + alerts.length === 0 ? <p className="text-text-muted">{k.guardian.quiet}</p> : null}
        <ul className="flex flex-col gap-[4px]">
          {[...actions.map((action) => ({ ts: action.ts, text: action.sentence, tone: action.action === 'rolled_back' ? 'text-state-waiting' : 'text-text-secondary' })), ...alerts.map((alert) => ({ ts: alert.ts, text: alert.sentence, tone: alert.level === 'critical' ? 'text-state-error' : 'text-text-secondary' }))]
            .sort((a, b) => b.ts.localeCompare(a.ts))
            .slice(0, 40)
            .map((row, index) => (
              <li key={index} className={'flex gap-[8px] ' + row.tone}>
                <span className="shrink-0 font-mono text-[10.5px] text-text-muted">{row.ts.slice(5, 16).replace('T', ' ')}</span>
                <span dir="auto">{row.text}</span>
              </li>
            ))}
        </ul>
      </section>
    </div>
  );
}

const TABS: readonly { id: AgencyTab; icon: typeof Globe }[] = [
  { id: 'sites', icon: Globe },
  { id: 'deploys', icon: Play },
  { id: 'approvals', icon: ShieldCheck },
  { id: 'playbooks', icon: Wand2 },
  { id: 'xray', icon: ScanSearch },
  { id: 'guardian', icon: Moon },
];

export function Agency() {
  const open = useKernelUi((state) => state.agencyOpen);
  const tab = useKernelUi((state) => state.agencyTab);
  const openTab = useKernelUi((state) => state.openAgency);
  const close = useKernelUi((state) => state.closeAgency);
  const pending = useAppStore((state) => Object.values(state.kernel.approvals).filter((approval) => approval.state === 'pending').length);

  return (
    <Modal open={open} label={k.title} onClose={close} center className="flex h-[88vh] w-[min(1040px,96vw)] overflow-hidden">
      <nav className="flex w-[190px] shrink-0 flex-col gap-[2px] border-r border-border-subtle bg-bg-raised p-[10px] max-700:w-[60px]" aria-label={k.title}>
        <div className="mb-[8px] px-[8px] text-[13px] font-semibold text-text-primary max-700:hidden">{k.title}</div>
        {TABS.map((item) => {
          const Icon = item.icon;

          return (
            <button
              key={item.id}
              type="button"
              aria-current={item.id === tab}
              className={
                'flex items-center gap-[10px] rounded-md px-[10px] py-[8px] text-left text-[12.5px] transition-colors duration-fast ' +
                (item.id === tab ? 'bg-accent-subtle text-accent' : 'text-text-secondary hover:bg-bg-hover hover:text-text-primary')
              }
              onClick={() => openTab(item.id)}
            >
              <Icon size={14} aria-hidden="true" />
              <span className="max-700:hidden">{k.tabs[item.id]}</span>
              {item.id === 'approvals' && pending > 0 ? <span className="ml-auto rounded-full bg-orange-subtle px-[6px] text-[10px] text-state-waiting">{pending}</span> : null}
            </button>
          );
        })}
      </nav>
      <div className="min-w-0 flex-1 overflow-y-auto px-[22px] py-[18px]">
        <h2 className="mb-[12px] text-[15px] font-semibold text-text-primary">{k.tabs[tab]}</h2>
        {tab === 'sites' ? <SitesTab onDeploys={() => openTab('deploys')} /> : null}
        {tab === 'deploys' ? <DeploysTab /> : null}
        {tab === 'approvals' ? <ApprovalsTab /> : null}
        {tab === 'playbooks' ? <PlaybooksTab /> : null}
        {tab === 'xray' ? <XrayTab /> : null}
        {tab === 'guardian' ? <GuardianTab /> : null}
      </div>
    </Modal>
  );
}
