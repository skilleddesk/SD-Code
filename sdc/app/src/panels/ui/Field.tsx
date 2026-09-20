import { Eye, EyeOff, ClipboardPaste } from 'lucide-react';
import { useState, type ReactNode, type Ref } from 'react';

import { strings } from '../../strings';
import { BTN_GHOST, BTN_SM } from './button';

/**
 * A labelled input with its own actions (0.7.1).
 *
 * The API-key field in the screenshot was a bare `<input type="password">` with a placeholder and no
 * label at all: nothing said what it wanted, nothing said whether the key was already saved, and there
 * was no way to see what had been pasted or to clear it. Three defects in one control, and all three are
 * things a field can carry:
 *
 *   label     what this is (`API key`), with a status slot on the right (`Saved`, `Not saved yet`)
 *   actions   `show` / `hide` on a secret, `Paste` on anything long
 *   hint      the sentence under it that says where the value goes
 *
 * Monospace is the default because every secret, id and path in this app is monospace; prose fields pass
 * `mono={false}`.
 */
export interface FieldProps {
  id: string;
  label: string;
  /** The input itself, so a caller can focus it (the Connect dialog focuses the code field once). */
  inputRef?: Ref<HTMLInputElement>;
  /** The right side of the label row: a `Badge` that says whether the value is stored. */
  status?: ReactNode;
  placeholder?: string;
  value: string;
  onChange: (value: string) => void;
  /** A secret: rendered as dots, with a show/hide button. */
  secret?: boolean;
  /** Offer a `Paste` button (clipboard read; degrades to focus-and-hint where it is denied). */
  pausable?: boolean;
  mono?: boolean;
  hint?: ReactNode;
  onEnter?: () => void;
}

export function Field({
  id,
  label,
  inputRef,
  status,
  placeholder,
  value,
  onChange,
  secret = false,
  pausable = true,
  mono = true,
  hint,
  onEnter,
}: FieldProps) {
  const [revealed, setRevealed] = useState(false);

  const paste = (): void => {
    void navigator.clipboard
      ?.readText()
      .then((text) => {
        if (text.trim() !== '') {
          onChange(text.trim());
        }
      })
      .catch(() => {
        /* A denied clipboard is not an error the user needs: the field is right there to type into. */
      });
  };

  return (
    <div className="field flex flex-col gap-[5px]">
      <div className="flex items-center gap-[8px]">
        <label htmlFor={id} className="text-[11.5px] font-medium text-text-secondary">
          {label}
        </label>

        {status === undefined ? null : <div className="ml-auto">{status}</div>}
      </div>

      <div className="flex items-center gap-[6px]">
        <input
          id={id}
          ref={inputRef}
          type={secret && !revealed ? 'password' : 'text'}
          autoComplete="off"
          spellCheck={false}
          placeholder={placeholder}
          value={value}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && onEnter !== undefined) {
              onEnter();
            }
          }}
          className={
            'min-w-0 flex-1 rounded-md border border-border-default bg-bg-input px-[10px] py-[7px] text-[12.5px] ' +
            'text-text-primary placeholder:text-text-muted focus:border-border-strong ' +
            (mono ? 'font-mono' : '')
          }
        />

        {secret ? (
          <button
            type="button"
            className={BTN_SM + ' ' + BTN_GHOST}
            aria-pressed={revealed}
            title={revealed ? strings.field.hide : strings.field.show}
            onClick={() => setRevealed((current) => !current)}
          >
            {revealed ? <EyeOff size={12} aria-hidden="true" /> : <Eye size={12} aria-hidden="true" />}
            {revealed ? strings.field.hide : strings.field.show}
          </button>
        ) : null}

        {pausable ? (
          <button type="button" className={BTN_SM + ' ' + BTN_GHOST} title={strings.field.paste} onClick={paste}>
            <ClipboardPaste size={12} aria-hidden="true" />
            {strings.field.paste}
          </button>
        ) : null}
      </div>

      {hint === undefined ? null : <p className="text-[11px] leading-[1.5] text-text-muted">{hint}</p>}
    </div>
  );
}
