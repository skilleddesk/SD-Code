import type {
  ApprovalRecord,
  CostUpdatedEvent,
  DeployUpdatedEvent,
  GuardianActionEvent,
  HealthAlertEvent,
  HealthUpdatedEvent,
  IntentParsedEvent,
  PolicyViolationEvent,
  SdcpEvent,
  ShadowDbUpdatedEvent,
  StagingUpdatedEvent,
  TrustReason,
  TrustLevel,
  VoiceTranscribedEvent,
  XrayReadyEvent,
  FileRestoredEvent,
} from '../../../protocol/types';

/**
 * The Trust Kernel's slice of the state (0.12) - folded from its events like everything else the window
 * draws (spec section 3.3). One pure function, `applyKernel`, so a replayed log draws the same costs,
 * scores, deploys and health as the window that watched them happen.
 */

type Without<T> = Omit<T, 'type'>;

export interface KernelState {
  /** Each turn's cost, by turn id. */
  costs: Record<string, Without<CostUpdatedEvent>>;
  /** Each turn's Trust score, by turn id - the newest (Verify re-scores a turn). */
  scores: Record<string, { score: number; level: TrustLevel; reasons: TrustReason[] }>;
  /** The kernel stopped these turns, and why. */
  stops: Record<string, { kind: 'budget' | 'runaway'; sentence: string }>;
  /** Policy rules that were hit, newest last (the last hundred). */
  violations: Without<PolicyViolationEvent>[];
  /** The last time the kill switch was pressed. */
  killSwitch: { at: string; stopped: number; checkpoints: number } | null;
  /** Readings of the Intent Engine, by intent id. */
  intents: Record<string, Without<IntentParsedEvent> & { confirmed: boolean; corrections: number }>;
  voice: Record<string, Without<VoiceTranscribedEvent>>;
  deploys: Record<string, Without<DeployUpdatedEvent> & { ts: string }>;
  health: Record<string, Without<HealthUpdatedEvent> & { ts: string }>;
  alerts: (Without<HealthAlertEvent> & { ts: string })[];
  guardian: (Without<GuardianActionEvent> & { ts: string })[];
  approvals: Record<string, ApprovalRecord>;
  xray: Record<string, Without<XrayReadyEvent>>;
  shadow: Record<string, Without<ShadowDbUpdatedEvent>>;
  staging: Record<string, Without<StagingUpdatedEvent>>;
  restored: (Without<FileRestoredEvent> & { ts: string })[];
}

export const EMPTY_KERNEL: KernelState = {
  costs: {},
  scores: {},
  stops: {},
  violations: [],
  killSwitch: null,
  intents: {},
  voice: {},
  deploys: {},
  health: {},
  alerts: [],
  guardian: [],
  approvals: {},
  xray: {},
  shadow: {},
  staging: {},
  restored: [],
};

/** The events this slice folds. */
export const KERNEL_EVENTS = new Set<SdcpEvent['type']>([
  'CostUpdated',
  'PolicyViolation',
  'BudgetStop',
  'KillSwitch',
  'TrustScored',
  'IntentParsed',
  'IntentConfirmed',
  'VoiceTranscribed',
  'DeployUpdated',
  'HealthUpdated',
  'HealthAlert',
  'GuardianAction',
  'ApprovalRecorded',
  'XrayReady',
  'ShadowDbUpdated',
  'StagingUpdated',
  'FileRestored',
]);

function strip<T extends { type: string }>(event: T): Omit<T, 'type'> {
  const copy: Record<string, unknown> = { ...event };

  delete copy.type;

  return copy as Omit<T, 'type'>;
}

function last<T>(list: readonly T[], item: T, keep: number): T[] {
  return [...list, item].slice(-keep);
}

/** Folds one kernel event. Pure: the clock is the log's own `ts`. */
export function applyKernel(state: KernelState, event: SdcpEvent, ts: string): KernelState {
  switch (event.type) {
    case 'CostUpdated':
      return { ...state, costs: { ...state.costs, [event.turnId]: strip(event) } };

    case 'TrustScored':
      return { ...state, scores: { ...state.scores, [event.turnId]: { score: event.score, level: event.level, reasons: event.reasons } } };

    case 'BudgetStop':
      return { ...state, stops: { ...state.stops, [event.turnId]: { kind: event.kind, sentence: event.sentence } } };

    case 'PolicyViolation':
      return { ...state, violations: last(state.violations, strip(event), 100) };

    case 'KillSwitch':
      return { ...state, killSwitch: { at: ts, stopped: event.stopped.length, checkpoints: event.checkpoints.length } };

    case 'IntentParsed':
      return { ...state, intents: { ...state.intents, [event.intentId]: { ...strip(event), confirmed: false, corrections: 0 } } };

    case 'IntentConfirmed': {
      const intent = state.intents[event.intentId];

      return intent === undefined
        ? state
        : { ...state, intents: { ...state.intents, [event.intentId]: { ...intent, confirmed: true, corrections: event.corrections } } };
    }

    case 'VoiceTranscribed':
      return { ...state, voice: { ...state.voice, [event.requestId]: strip(event) } };

    case 'DeployUpdated':
      return { ...state, deploys: { ...state.deploys, [event.deployId]: { ...strip(event), ts } } };

    case 'HealthUpdated':
      return { ...state, health: { ...state.health, [event.siteId]: { ...strip(event), ts } } };

    case 'HealthAlert':
      return { ...state, alerts: last(state.alerts, { ...strip(event), ts }, 50) };

    case 'GuardianAction':
      return { ...state, guardian: last(state.guardian, { ...strip(event), ts }, 50) };

    case 'ApprovalRecorded':
      return { ...state, approvals: { ...state.approvals, [event.approval.id]: event.approval } };

    case 'XrayReady':
      return { ...state, xray: { ...state.xray, [event.hostId]: strip(event) } };

    case 'ShadowDbUpdated':
      return { ...state, shadow: { ...state.shadow, [event.runId]: strip(event) } };

    case 'StagingUpdated':
      return { ...state, staging: { ...state.staging, [event.siteId]: strip(event) } };

    case 'FileRestored':
      return { ...state, restored: last(state.restored, { ...strip(event), ts }, 20) };

    default:
      return state;
  }
}

/* ------------------------------------------------------------------------------------------------
 * Derivations the panels share
 * ---------------------------------------------------------------------------------------------- */

/** A dollar amount as a person reads it: `$0.0031` below a cent, `$1.24` above. */
export function usd(amount: number | null | undefined): string {
  if (amount === null || amount === undefined || !Number.isFinite(amount)) {
    return '—';
  }

  if (amount === 0) {
    return '$0';
  }

  return amount < 0.01 ? `$${amount.toFixed(4)}` : `$${amount.toFixed(2)}`;
}

/** What a chat has cost, summed from its turns' measured or priced costs. */
export function chatCost(state: KernelState, sessionId: string): number {
  return Object.values(state.costs)
    .filter((cost) => cost.sessionId === sessionId)
    .reduce((sum, cost) => sum + cost.costUsd, 0);
}

/** Whether a turn's cost is a measured number (true) or not measured (false) - P4's label. */
export function measured(source: string | undefined): boolean {
  return source === 'measured' || source === 'priced' || source === 'local';
}
