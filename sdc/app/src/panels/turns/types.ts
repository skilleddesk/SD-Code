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

/**
 * The engine's answer - the one thing a turn is for (0.7.3).
 *
 * **Its absence was the report.** `Turn` had `user`, `meta`, `thinking`, `tools`, `error` and `footer`,
 * and no answer, so `toTurns` could not pass the text the reducer had been accumulating in
 * `TurnView.text` and `TurnStream` had nothing to draw. Every turn on every engine therefore read
 * `You · Reply with exactly: OK` / `Balanced · claude_code · sonnet` / `Done · $0.0295 · 1.6s · 2 in ·
 * 4 out` with the answer missing - which is exactly what *"sudu done lakha aslo, kono response pelam
 * nah"* describes. The turn had run and the tokens had been counted; the text was thrown away between
 * the log and the screen.
 */
export interface AnswerData {
  /** The deltas so far, in arrival order. Empty string until the first one lands. */
  text: string;
  /** Still streaming: the block draws a caret and the footer says the totals are pending. */
  streaming: boolean;
}

export interface Turn {
  id: string;
  user: UserMessageData;
  meta: TurnMetaData;
  /** Absent when the engine did not think out loud for this turn. */
  thinking?: ThinkingData;
  /** Absent until the first `TurnDelta` - a turn shows no empty answer block before it answers. */
  answer?: AnswerData;
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
 * The spec's collapsing rule (section 7.5): the last five turns stay open, anything older folds into
 * one line. The constant lives here so the rule has a name and a home rather than a magic 5 in a
 * component - `live.ts` is what applies it to a real session.
 */
export const OPEN_TURN_WINDOW = 5;
