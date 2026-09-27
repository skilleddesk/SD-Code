import type {
  ActiveWork,
  ApprovalRecord,
  AuditEntry,
  CliSelfCheck,
  CostSummary,
  CrashReport,
  DeployRecord,
  GlossaryTerm,
  HealthReport,
  LedgerCheck,
  Playbook,
  Policy,
  SiteRecord,
  TaskSpec,
  TeamMember,
  TeamState,
  TimelineBranch,
  UpdateInfo,
  XrayMap,
} from '../../../protocol/types';
import { sdcpCall } from '../lib/sdcp';
import { isSdcpError } from '../lib/transport';
import { strings } from '../strings';
import { dispatch, useAppStore } from './store';
import { findSession } from './sessions';

/**
 * The Trust Kernel's, the Intent Engine's and the agency's verbs (0.12) - the same three-step shape as
 * `intents.ts`: call the daemon, let its events land in the log, and say a failure out loud (P4). Kept in
 * their own file because `intents.ts` is already the app's longest.
 */

function fail(error: unknown, fallback: string): void {
  dispatch({ type: 'Toast', message: isSdcpError(error) ? error.message : fallback });
}

async function call<T>(work: () => Promise<T>, fallback: string): Promise<T | null> {
  try {
    return await work();
  } catch (error) {
    fail(error, fallback);

    return null;
  }
}

/** The host a chat lives on, for methods that act on its folder. */
export function hostOfSession(sessionId: string | null): string | undefined {
  if (sessionId === null) {
    return undefined;
  }

  const found = findSession(useAppStore.getState().hosts, sessionId);

  return found?.host.id;
}

/* ---- The kill switch --------------------------------------------------------------------------- */

export async function killAll(): Promise<{ stopped: ActiveWork[]; checkpoints: { sessionId: string; checkpointId: string }[] } | null> {
  const result = await call(() => sdcpCall('kill.all', {}), strings.kernel.kill.failed);

  if (result !== null) {
    dispatch({ type: 'Toast', message: strings.kernel.kill.done(result.stopped.length, result.checkpoints.length) });
  }

  return result;
}

export async function activeWork(): Promise<ActiveWork[]> {
  return (await call(() => sdcpCall('kill.list', {}), strings.kernel.kill.failed))?.active ?? [];
}

/* ---- Policy and the ledger ---------------------------------------------------------------------- */

export async function loadPolicy(sessionId: string | null): Promise<{ policy: Policy; text: string; exists: boolean; path: string | null; root: string | null; defaults: { protected: string[]; alwaysAsk: string[] } } | null> {
  return call(
    () => sdcpCall('policy.get', { ...(sessionId === null ? {} : { sessionId }), ...(hostOfSession(sessionId) === undefined ? {} : { hostId: hostOfSession(sessionId) }) }),
    strings.kernel.policy.loadFailed,
  );
}

export async function savePolicy(sessionId: string, input: { text?: string; policy?: Partial<Policy> }): Promise<Policy | null> {
  const result = await call(
    () => sdcpCall('policy.set', { sessionId, ...(hostOfSession(sessionId) === undefined ? {} : { hostId: hostOfSession(sessionId) }), ...input }),
    strings.kernel.policy.saveFailed,
  );

  return result?.policy ?? null;
}

export async function loadAudit(input: { sessionId?: string; turnId?: string; limit?: number }): Promise<{ entries: AuditEntry[]; chain: LedgerCheck } | null> {
  return call(() => sdcpCall('audit.list', input), strings.kernel.audit.failed);
}

export async function verifyLedger(): Promise<LedgerCheck | null> {
  return call(() => sdcpCall('audit.verify', {}), strings.kernel.audit.failed);
}

/* ---- Cost ----------------------------------------------------------------------------------------- */

export async function costSummary(): Promise<CostSummary | null> {
  return call(() => sdcpCall('cost.summary', {}), strings.kernel.cost.failed);
}

export async function estimateCost(input: { prompt: string; engine: string; model: string; provider?: string; sessionId?: string; agent?: boolean }) {
  try {
    return await sdcpCall('cost.estimate', input);
  } catch {
    /* An estimate is a hint; a daemon that cannot give one is not an error worth a toast. */
    return null;
  }
}

export async function setBudgets(input: { turn?: number | null; chat?: number | null; day?: number | null; month?: number | null; baseline?: string }): Promise<CostSummary | null> {
  const result = await call(() => sdcpCall('cost.budget.set', input), strings.kernel.cost.failed);

  if (result !== null) {
    dispatch({ type: 'Toast', message: strings.kernel.cost.savedToast });
  }

  return result;
}

/* ---- The Time Machine ---------------------------------------------------------------------------- */

export async function labelCheckpoint(checkpointId: string, label: string): Promise<void> {
  await call(() => sdcpCall('checkpoint.label', { checkpointId, label }), strings.kernel.timeline.labelFailed);
}

export async function checkpointFiles(checkpointId: string): Promise<{ status: string; path: string }[] | null> {
  return (await call(() => sdcpCall('checkpoint.files', { checkpointId }), strings.kernel.timeline.filesFailed))?.files ?? null;
}

export async function checkpointFileDiff(checkpointId: string, path: string): Promise<string | null> {
  return (await call(() => sdcpCall('checkpoint.fileDiff', { checkpointId, path }), strings.kernel.timeline.filesFailed))?.diff ?? null;
}

export async function restoreFile(checkpointId: string, path: string): Promise<boolean> {
  const result = await call(() => sdcpCall('checkpoint.restoreFile', { checkpointId, path }), strings.kernel.timeline.restoreFailed);

  if (result !== null) {
    dispatch({ type: 'Toast', message: strings.kernel.timeline.restored(result.path, result.outcome) });
  }

  return result !== null;
}

export async function timelineBranches(sessionId: string): Promise<TimelineBranch[]> {
  return (await call(() => sdcpCall('timeline.branches', { sessionId }), strings.kernel.timeline.branchesFailed))?.branches ?? [];
}

export async function switchBranch(sessionId: string, frameId: number): Promise<boolean> {
  const result = await call(
    () => sdcpCall('timeline.switch', { sessionId, frameId, ...(hostOfSession(sessionId) === undefined ? {} : { hostId: hostOfSession(sessionId) }) }),
    strings.kernel.timeline.branchesFailed,
  );

  return result?.switched ?? false;
}

/* ---- Proof ------------------------------------------------------------------------------------------ */

export async function exportProof(sessionId: string, turnId: string | undefined, lang: string): Promise<{ htmlPath: string; jsonPath: string; html: string } | null> {
  return call(() => sdcpCall('proof.export', { sessionId, lang, ...(turnId === undefined ? {} : { turnId }) }), strings.kernel.proof.failed);
}

export async function trustScore(turnId: string) {
  return call(() => sdcpCall('trust.score', { turnId }), strings.kernel.proof.failed);
}

/* ---- The Intent Engine --------------------------------------------------------------------------------- */

export async function parseIntent(input: { text: string; sessionId: string; engine: string; model: string; provider?: string }): Promise<string | null> {
  const result = await call(
    () => sdcpCall('intent.parse', { ...input, ...(hostOfSession(input.sessionId) === undefined ? {} : { hostId: hostOfSession(input.sessionId) }) }),
    strings.kernel.intent.failed,
  );

  return result?.intentId ?? null;
}

export async function confirmIntent(intentId: string, sessionId: string, spec: TaskSpec, glossary: { term: string; meaning: string }[]): Promise<boolean> {
  return (await call(() => sdcpCall('intent.confirm', { intentId, sessionId, spec, glossary }), strings.kernel.intent.failed)) !== null;
}

export async function compileIntent(intentId: string, engine: string, sessionId: string): Promise<string | null> {
  return (await call(() => sdcpCall('intent.compile', { intentId, engine, sessionId }), strings.kernel.intent.failed))?.prompt ?? null;
}

export async function cancelIntent(intentId: string): Promise<void> {
  await call(() => sdcpCall('intent.cancel', { intentId }), strings.kernel.intent.failed);
}

export async function loadGlossary(sessionId: string | null): Promise<{ scope: string; terms: GlossaryTerm[] } | null> {
  return call(() => sdcpCall('glossary.list', sessionId === null ? {} : { sessionId }), strings.kernel.language.glossaryFailed);
}

export async function saveGlossaryTerm(scope: string, term: string, meaning: string): Promise<GlossaryTerm[] | null> {
  return (await call(() => sdcpCall('glossary.set', { scope, term, meaning }), strings.kernel.language.glossaryFailed))?.terms ?? null;
}

export async function voiceStatus() {
  try {
    return await sdcpCall('voice.status', {});
  } catch {
    return null;
  }
}

export async function transcribe(audio: string, mime: string, language: string | undefined, sessionId?: string): Promise<string | null> {
  return (
    await call(
      () => sdcpCall('voice.transcribe', { audio, mime, ...(language === undefined ? {} : { language }), ...(sessionId === undefined ? {} : { sessionId }) }),
      strings.kernel.voice.failed,
    )
  )?.requestId ?? null;
}

/* ---- Sites, deploys, health ------------------------------------------------------------------------------ */

export async function listSites(): Promise<SiteRecord[]> {
  return (await call(() => sdcpCall('site.list', {}), strings.kernel.agency.failed))?.sites ?? [];
}

export async function detectSite(hostId: string, root: string): Promise<Record<string, unknown> | null> {
  return (await call(() => sdcpCall('site.detect', { hostId, root }), strings.kernel.agency.failed))?.config ?? null;
}

export async function saveSite(input: { siteId?: string; name: string; hostId: string; root: string; url: string; config: Record<string, unknown> }): Promise<string | null> {
  return (await call(() => sdcpCall('site.save', input), strings.kernel.agency.failed))?.siteId ?? null;
}

export async function removeSite(siteId: string): Promise<void> {
  await call(() => sdcpCall('site.remove', { siteId }), strings.kernel.agency.failed);
}

export async function deploySite(siteId: string): Promise<{ deployId?: string; state: string; approvalId?: string } | null> {
  const result = await call(() => sdcpCall('deploy.run', { siteId }), strings.kernel.agency.failed);

  if (result?.state === 'awaiting_approval') {
    dispatch({ type: 'Toast', message: strings.kernel.agency.awaitingApproval });
  }

  return result;
}

export async function listDeploys(siteId?: string): Promise<DeployRecord[]> {
  return (await call(() => sdcpCall('deploy.list', siteId === undefined ? {} : { siteId }), strings.kernel.agency.failed))?.deploys ?? [];
}

export async function previewRollback(deployId: string) {
  return call(() => sdcpCall('deploy.preview', { deployId }), strings.kernel.agency.failed);
}

export async function rollbackDeploy(deployId: string): Promise<void> {
  await call(() => sdcpCall('deploy.rollback', { deployId }), strings.kernel.agency.failed);
}

export async function restoreDatabase(deployId: string): Promise<boolean> {
  const result = await call(() => sdcpCall('deploy.restoreDb', { deployId, confirm: 'RESTORE' }), strings.kernel.agency.failed);

  return result?.restored ?? false;
}

export async function checkHealth(siteId: string): Promise<void> {
  await call(() => sdcpCall('health.check', { siteId }), strings.kernel.agency.failed);
}

export async function healthHistory(siteId: string): Promise<HealthReport[]> {
  return (await call(() => sdcpCall('health.history', { siteId, limit: 60 }), strings.kernel.agency.failed))?.reports ?? [];
}

export async function setGuardian(siteId: string, enabled: boolean, autoRollback: boolean): Promise<void> {
  await call(() => sdcpCall('guardian.set', { siteId, enabled, autoRollback }), strings.kernel.agency.failed);
}

export async function listApprovals(): Promise<ApprovalRecord[]> {
  return (await call(() => sdcpCall('approval.list', {}), strings.kernel.agency.failed))?.approvals ?? [];
}

export async function decideApproval(approvalId: string, decision: 'approved' | 'declined', note?: string): Promise<void> {
  await call(() => sdcpCall('approval.decide', { approvalId, decision, ...(note === undefined ? {} : { note }) }), strings.kernel.agency.failed);
}

export async function pollApproval(approvalId: string): Promise<boolean> {
  return (await call(() => sdcpCall('approval.poll', { approvalId }), strings.kernel.agency.failed))?.answered ?? false;
}

export async function createStaging(input: { siteId: string; summary: string; changes: string[]; before?: string; after?: string; lang: string }): Promise<string | null> {
  return (await call(() => sdcpCall('staging.create', input), strings.kernel.agency.failed))?.approvalId ?? null;
}

export async function stopStaging(siteId: string, remove: boolean): Promise<void> {
  await call(() => sdcpCall('staging.stop', { siteId, remove }), strings.kernel.agency.failed);
}

export async function rehearseMigration(siteId: string, command: string): Promise<string | null> {
  return (await call(() => sdcpCall('shadowdb.run', { siteId, command }), strings.kernel.agency.failed))?.runId ?? null;
}

export async function scanHost(hostId: string): Promise<void> {
  await call(() => sdcpCall('xray.scan', { hostId }), strings.kernel.agency.failed);
}

export async function loadXray(hostId: string): Promise<XrayMap | null> {
  return (await call(() => sdcpCall('xray.get', { hostId }), strings.kernel.agency.failed))?.map ?? null;
}

export async function listPlaybooks(): Promise<Playbook[]> {
  return (await call(() => sdcpCall('playbook.list', {}), strings.kernel.agency.failed))?.playbooks ?? [];
}

export async function savePlaybook(playbook: Partial<Playbook>): Promise<Playbook[] | null> {
  return (await call(() => sdcpCall('playbook.save', { playbook }), strings.kernel.agency.failed))?.playbooks ?? null;
}

export async function removePlaybook(playbookId: string): Promise<Playbook[] | null> {
  return (await call(() => sdcpCall('playbook.remove', { playbookId }), strings.kernel.agency.failed))?.playbooks ?? null;
}

export async function runPlaybook(playbookId: string, siteIds: string[]) {
  return call(() => sdcpCall('playbook.run', { playbookId, siteIds }), strings.kernel.agency.failed);
}

/* ---- Team, settings, release safety --------------------------------------------------------------------- */

export async function loadTeam(): Promise<TeamState | null> {
  return call(() => sdcpCall('team.get', {}), strings.kernel.team.failed);
}

export async function saveTeam(input: { members?: TeamMember[]; current?: string }): Promise<TeamState | null> {
  return call(() => sdcpCall('team.set', input), strings.kernel.team.failed);
}

export async function daemonSetting(key: string, value: string | boolean): Promise<void> {
  try {
    await sdcpCall('settings.set', { key, value });
  } catch {
    /* A daemon that is not there yet learns the value the next time it is set. */
  }
}

export async function checkUpdates(channel: 'stable' | 'beta'): Promise<UpdateInfo | null> {
  return call(() => sdcpCall('update.check', { channel }), strings.kernel.updates.failed);
}

export async function crashReports(): Promise<CrashReport[]> {
  return (await call(() => sdcpCall('crash.list', {}), strings.kernel.updates.failed))?.reports ?? [];
}

export async function clearCrashReports(): Promise<void> {
  await call(() => sdcpCall('crash.clear', {}), strings.kernel.updates.failed);
}

export async function cliSelfCheck(): Promise<CliSelfCheck[]> {
  return (await call(() => sdcpCall('cli.selfcheck', {}), strings.kernel.updates.failed))?.clis ?? [];
}

export async function shareStatus(enabled: boolean): Promise<{ enabled: boolean; url?: string; error?: string } | null> {
  return call(() => sdcpCall('status.share', { enabled }), strings.kernel.updates.failed);
}
