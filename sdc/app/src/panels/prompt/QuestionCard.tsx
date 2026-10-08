import { MessageCircleQuestion } from 'lucide-react';
import { useState } from 'react';

import { strings } from '../../strings';
import { answerQuestion } from '../../store/agentIntents';
import { useAppStore } from '../../store/store';
import { BTN_PRIMARY, BTN_SECONDARY, BTN_SM } from '../ui/button';

/**
 * The agent's question (0.13, `ask_user`), above the composer of the chat it belongs to. The turn waits
 * for it: a choice, the person's own words, or "let it decide". Answering closes the card everywhere
 * (`QuestionAnswered`), and a turn that ends takes its question with it.
 */
export function QuestionCard({ sessionId }: { sessionId: string }) {
  const all = useAppStore((state) => state.questions);
  const questions = all.filter((question) => question.sessionId === sessionId);

  if (questions.length === 0) {
    return null;
  }

  return (
    <div className="mb-[8px] flex flex-col gap-[8px]">
      {questions.map((question) => (
        <Question key={question.questionId} {...question} />
      ))}
    </div>
  );
}

function Question({ questionId, turnId, sessionId, question, options }: { questionId: string; turnId: string; sessionId: string; question: string; options: string[] }) {
  const [text, setText] = useState('');
  const [sending, setSending] = useState(false);
  const words = strings.agent.question;

  const answer = (reply: string): void => {
    setSending(true);
    void answerQuestion(questionId, reply, turnId, sessionId).finally(() => setSending(false));
  };

  return (
    <div className="question-card rounded-xl border border-border-default border-l-[3px] border-l-accent bg-accent-subtle p-[12px] shadow-sm" role="group" aria-label={words.label}>
      <div className="flex items-center gap-[6px] font-mono text-[10.5px] uppercase tracking-wide text-accent">
        <MessageCircleQuestion size={13} aria-hidden="true" />
        {words.label}
      </div>
      <p className="mt-[6px] whitespace-pre-wrap text-[13.5px] leading-[1.55] text-text-primary">{question}</p>

      {options.length === 0 ? null : (
        <div className="mt-[10px] flex flex-wrap gap-[6px]">
          {options.map((option) => (
            <button key={option} type="button" disabled={sending} className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => answer(option)}>
              {option}
            </button>
          ))}
        </div>
      )}

      <form
        className="mt-[10px] flex items-center gap-[6px]"
        onSubmit={(event) => {
          event.preventDefault();

          if (text.trim() !== '') {
            answer(text.trim());
          }
        }}
      >
        <input
          className="h-[30px] min-w-0 flex-1 rounded-md border border-border-default bg-bg-input px-[10px] text-[12.5px] text-text-primary placeholder:text-text-muted"
          value={text}
          placeholder={words.placeholder}
          aria-label={words.placeholder}
          onChange={(event) => setText(event.target.value)}
        />
        <button type="submit" disabled={sending || text.trim() === ''} className={BTN_SM + ' ' + BTN_PRIMARY}>
          {words.send}
        </button>
        <button type="button" disabled={sending} className={BTN_SM + ' ' + BTN_SECONDARY} onClick={() => answer('')}>
          {words.skip}
        </button>
      </form>
    </div>
  );
}
