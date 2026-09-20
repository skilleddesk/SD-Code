import { strings } from '../../strings';

/**
 * The turn stream's data model - spec section 7.5.
 *
 * One `Turn` is one of everything the stream can draw: the user's message and its attachments, the
 * meta line that says which engine answered, an optional thinking block, zero or more tool cards,
 * an optional error card, and a footer with the run's totals and the feedback buttons.
 *
 * The types are discriminated unions rather than one loose shape, because the three tool cards are
 * genuinely different objects: a Read has no body, an Edit has diff lines, a Run has output and a
 * spinner while it is going. `ToolCard` switches on `kind` and the compiler will not let a variant
 * forget its body.
 *
 * The seed at the bottom of this file is the prototype's worked example, verbatim - the same
 * prompt, the same four blocks, the same numbers - so that the first thing anyone sees is the thing
 * the spec was measured against. It is demo data for this step; the structured stream of spec
 * section 3.2 replaces it, and nothing else about the components changes when it does.
 */

/** A tool card's outcome. `running` keeps a spinner instead of a chevron. */
export type ToolStatus = 'done' | 'running' | 'failed';

/** One line of a diff: the number in the old/new file, the text, and which side it belongs to. */
export interface DiffLine {
  /** Line number as a string - `14`, `18` - printed in the gutter. */
  lineNumber: string;
  text: string;
  change: 'add' | 'rem';
}

/** One line of command output. `dim` is the optional third tone the prototype styles. */
export interface RunLine {
  level: 'ok' | 'fail' | 'dim';
  text: string;
}

/** `Read` - file-text icon, a target path, and a `done · 42 ln` pill. No body. */
export interface ReadToolData {
  kind: 'read';
  name: string;
  target: string;
  status: ToolStatus;
  /** The right-hand status pill's text, already formatted by the producer. */
  meta: string;
}

/** `Edit` - file-pen icon, a target path, `done · +18 −2`, and a diff body. */
export interface EditToolData {
  kind: 'edit';
  name: string;
  target: string;
  status: ToolStatus;
  meta: string;
  diff: DiffLine[];
}

/** `Run` - play icon, a command, `running`, and a live 5-line output window. */
export interface RunToolData {
  kind: 'run';
  name: string;
  target: string;
  status: ToolStatus;
  meta: string;
  output: RunLine[];
}

export type ToolCardData = ReadToolData | EditToolData | RunToolData;

/** The reasoning block: collapsed by default, and only its duration is shown until you open it. */
export interface ThinkingData {
  /** The duration text, parentheses included - `(4s)`. */
  duration: string;
  text: string;
}

/**
 * An attachment chip. `thumb` is the gradient preview with no label; `image` and `file` carry one.
 * The discriminator is what decides the icon and whether the chip is accent-tinted.
 */
export type AttachmentData =
  | { kind: 'thumb' }
  | { kind: 'image'; label: string }
  | { kind: 'file'; label: string };

export interface UserMessageData {
  /** `You · 14:02`. Rendered uppercase, with a hairline divider filling the rest of the row. */
  who: string;
  body: string;
  attachments: AttachmentData[];
}

/** The line under the user's message: `Balanced · claude_code · sonnet`, plus the forecast. */
export interface TurnMetaData {
  tier: string;
  engine: string;
  model: string;
  /** The estimate, e.g. `~$0.10 – $0.28 forecast`. */
  forecast: string;
}

export interface ErrorCardData {
  title: string;
  explanation: string;
}

/** `Done · 1m 12s · 12,400 tokens · $0.16` - two pieces so the verb can be weighted. */
export interface TurnFooterData {
  summary: string;
  detail: string;
}

export interface Turn {
  id: string;
  user: UserMessageData;
  meta: TurnMetaData;
  /** Absent when the engine did not think out loud for this turn. */
  thinking?: ThinkingData;
  tools: ToolCardData[];
  error?: ErrorCardData;
  footer: TurnFooterData;
}

/** The one-line block at the top of a stream longer than five turns (spec section 7.5). */
export interface CollapsedSummaryData {
  label: string;
  meta: string;
}

/**
 * The demo turn: exactly the prototype's first turn (design/ui-prototype.html, `paneHTML`), so the
 * visual regression of this step is a comparison against the reference rather than against a memory.
 *
 * The three tool cards are one of each variant on purpose - a Read with no body, an Edit with a
 * collapsed diff, a Run that is still going with two lines of output - and the error card that
 * follows them is the reason the Run has not finished.
 */
export const DEMO_TURNS: readonly Turn[] = [
  {
    id: 'turn-7',
    user: {
      who: strings.turns.who,
      body: strings.turns.prompt,
      attachments: [
        { kind: 'thumb' },
        { kind: 'image', label: strings.turns.attachments.image },
        { kind: 'file', label: strings.turns.attachments.file },
      ],
    },
    meta: {
      tier: 'Balanced',
      engine: 'claude_code',
      model: 'sonnet',
      forecast: '~$0.10 – $0.28 forecast',
    },
    thinking: {
      duration: strings.turns.thinking.duration,
      text: strings.turns.thinking.body,
    },
    tools: [
      {
        kind: 'read',
        name: strings.turns.tools.read,
        target: 'src/auth.ts',
        status: 'done',
        meta: strings.turns.tools.readStatus,
      },
      {
        kind: 'edit',
        name: strings.turns.tools.edit,
        target: 'src/auth.ts',
        status: 'done',
        meta: strings.turns.tools.editStatus,
        diff: [
          { lineNumber: '14', change: 'add', text: "+ import rateLimit from 'express-rate-limit';" },
          { lineNumber: '15', change: 'add', text: '+ const loginLimiter = rateLimit({' },
          { lineNumber: '16', change: 'add', text: '+   windowMs: 15 * 60 * 1000,' },
          { lineNumber: '17', change: 'add', text: '+   max: 5,' },
          { lineNumber: '18', change: 'rem', text: "− app.post('/login', handler);" },
          { lineNumber: '18', change: 'add', text: "+ app.post('/login', loginLimiter, handler);" },
        ],
      },
      {
        kind: 'run',
        name: strings.turns.tools.run,
        target: 'npm test',
        status: 'running',
        meta: strings.turns.tools.runStatus,
        output: [
          { level: 'ok', text: 'auth.test.ts' },
          { level: 'fail', text: 'rate.test.ts > limits after 5 tries' },
        ],
      },
    ],
    error: {
      title: strings.turns.error.title,
      explanation: strings.turns.error.explanation,
    },
    footer: {
      summary: strings.turns.footer.summary,
      detail: strings.turns.footer.detail,
    },
  },
];

/** "Turns 1–6 collapsed · 8,420 tokens · $0.31" - the six turns above the demo turn. */
export const DEMO_COLLAPSED: CollapsedSummaryData = {
  label: strings.turns.collapsed.label,
  meta: strings.turns.collapsed.meta,
};

/** How many turns came before the window `DEMO_TURNS` is: the "Turns 1–6" of that label. */
export const DEMO_TURNS_BEFORE = 6;

/**
 * The spec's collapsing rule (section 7.5): the last five turns stay open, anything older folds
 * into one line - and at thirty or more they fold into a single summary block. Only the first
 * threshold can be reached with the seed, so it is the one this step implements; the constant is
 * here so the rule has a name and a home rather than a magic 5 in a component.
 */
export const OPEN_TURN_WINDOW = 5;
