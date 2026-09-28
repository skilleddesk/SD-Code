import { eventLog } from '../store/events';
import { useAppStore } from '../store/store';
import { findSession } from '../store/reducer';
import { setting } from './settings';
import { strings } from '../strings';

/**
 * Desktop alerts and sound cues (0.13) - Settings → Notifications, which until now drew three switches
 * and a "Test sound" button that did nothing but toast "played". Codex and Claude Code both tell you when a
 * long turn is done or needs you; SDC now does too:
 *
 * | event                               | switch             | cue       |
 * | ----------------------------------- | ------------------ | --------- |
 * | `TurnCompleted`                     | notify-complete    | done      |
 * | `PermissionRequested`, `QuestionAsked` | notify-approval | attention |
 * | `StuckDetected`, `BudgetStop`       | notify-stuck       | problem   |
 *
 * A desktop alert only when the window is not in front (you are looking at it otherwise); a sound either
 * way when the sound switch is on. Events replayed when the window starts are history, not news: only
 * events the daemon stamped after this window opened are announced.
 */
type Cue = 'done' | 'attention' | 'problem';

const OPENED = Date.now();

/** Two short tones, different per cue - made here, so no sound file ships. */
export function playCue(cue: Cue): void {
  try {
    const Context = (globalThis as { AudioContext?: typeof AudioContext }).AudioContext;

    if (Context === undefined) {
      return;
    }

    const audio = new Context();
    const notes: Record<Cue, number[]> = { done: [660, 880], attention: [880, 660, 880], problem: [440, 330] };
    let at = audio.currentTime;

    for (const frequency of notes[cue]) {
      const oscillator = audio.createOscillator();
      const gain = audio.createGain();

      oscillator.type = 'sine';
      oscillator.frequency.value = frequency;
      gain.gain.setValueAtTime(0.0001, at);
      gain.gain.exponentialRampToValueAtTime(0.18, at + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + 0.16);
      oscillator.connect(gain).connect(audio.destination);
      oscillator.start(at);
      oscillator.stop(at + 0.18);
      at += 0.19;
    }

    setTimeout(() => void audio.close(), 1200);
  } catch {
    /* No audio device: a silent cue is not an error. */
  }
}

/** A desktop notification, through the Tauri plugin; outside Tauri (a browser, a test) nothing. */
async function notify(title: string, body: string): Promise<void> {
  try {
    const plugin = await import('@tauri-apps/plugin-notification');
    let allowed = await plugin.isPermissionGranted();

    if (!allowed) {
      allowed = (await plugin.requestPermission()) === 'granted';
    }

    if (allowed) {
      plugin.sendNotification({ title, body });
    }
  } catch {
    /* Not in the desktop app. */
  }
}

function chatTitle(sessionId: string | null | undefined): string {
  const found = sessionId === null || sessionId === undefined ? null : findSession(useAppStore.getState().hosts, sessionId);

  return found?.session.title ?? 'SDC';
}

function announce(cue: Cue, toggle: string, title: string, body: string): void {
  if (!setting(toggle, true)) {
    return;
  }

  if (setting('sound-cues', false)) {
    playCue(cue);
  }

  const inFront = typeof document !== 'undefined' && document.visibilityState === 'visible' && document.hasFocus();

  if (!inFront) {
    void notify(title, body);
  }
}

let started = false;

/** Starts listening; called once when the window boots. */
export function startAlerts(): void {
  if (started) {
    return;
  }

  started = true;

  eventLog.subscribe((entry) => {
    const stamped = Date.parse(entry.ts);

    if (Number.isFinite(stamped) && stamped < OPENED - 2_000) {
      return;
    }

    const event = entry.event;
    const words = strings.agent.alerts;

    switch (event.type) {
      case 'TurnCompleted': {
        const turn = useAppStore.getState().turns.find((candidate) => candidate.id === event.turnId);

        announce('done', 'notify-complete', words.done(chatTitle(turn?.sessionId ?? entry.sessionId)), event.summary || words.doneBody);
        break;
      }
      case 'PermissionRequested':
        announce('attention', 'notify-approval', words.needsYou(chatTitle(entry.sessionId)), event.title);
        break;
      case 'QuestionAsked':
        announce('attention', 'notify-approval', words.needsYou(chatTitle(event.sessionId)), event.question.slice(0, 180));
        break;
      case 'StuckDetected':
      case 'BudgetStop':
        announce('problem', 'notify-stuck', words.problem(chatTitle(entry.sessionId)), event.type === 'BudgetStop' ? words.budget : words.stuck);
        break;
      default:
        break;
    }
  });
}
