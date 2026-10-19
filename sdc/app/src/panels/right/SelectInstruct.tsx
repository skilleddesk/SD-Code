import { MessageSquarePlus } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode } from 'react';

import { selectionDraft } from '../../lib/diffReview';
import { useModelStore } from '../../store/model';
import { strings } from '../../strings';
import { BTN_SECONDARY, BTN_SM } from '../ui/button';

/**
 * Select & Instruct (P3-11): select text in a file, press "Instruct", and the prompt opens with the selection quoted and its file named
 * as `@path`. Nothing is sent - the person finishes the sentence and presses Send themselves.
 *
 * The browser's own selection is read (the editor draws real text), and only a selection that is inside this wrapper counts.
 */
export function SelectInstruct({ path, children }: { path: string; children: ReactNode }) {
  const box = useRef<HTMLDivElement | null>(null);
  const [selected, setSelected] = useState('');

  useEffect(() => {
    const read = (): void => {
      const selection = window.getSelection();
      const inside = selection !== null && box.current !== null && selection.anchorNode !== null && box.current.contains(selection.anchorNode);

      setSelected(inside ? selection.toString() : '');
    };

    document.addEventListener('selectionchange', read);

    return () => document.removeEventListener('selectionchange', read);
  }, []);

  return (
    <div ref={box} className="relative flex min-h-0 flex-1 flex-col">
      {children}

      {selected.trim() === '' ? null : (
        <button
          type="button"
          className={BTN_SM + ' ' + BTN_SECONDARY + ' absolute bottom-[10px] right-[14px] z-10 shadow-md'}
          title={strings.review.instructHint}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            useModelStore.getState().setDraft(selectionDraft(path, selected, strings.review.instructDefault));
            setSelected('');
          }}
        >
          <MessageSquarePlus size={11} aria-hidden="true" />
          {strings.review.instruct}
        </button>
      )}
    </div>
  );
}
