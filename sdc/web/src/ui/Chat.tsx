import { useEffect, useRef } from 'react';
import { useModel, useSnapshot, useT } from './hooks';

/**
 * The conversation with the AI on the person's own computer. The messages shown are what the daemon streams; the box
 * sends text and the *names* of picked files, never their contents (the AI reads them where they are).
 */
export function Chat() {
  const t = useT();
  const model = useModel();
  const { chat, live, workspace, link } = useSnapshot();
  const end = useRef<HTMLDivElement>(null);
  const items = chat.sessionId ? live.filter((item) => item.sessionId === chat.sessionId) : live;
  const locked = link.kind === 'locked';
  const operate = link.kind === 'operate';

  useEffect(() => {
    end.current?.scrollIntoView({ block: 'end' });
  }, [items]);

  if (locked) return <p className="muted empty">{t('unlockHelp')}</p>;

  const names = workspace.picked.map((item) => item.name).join(', ');

  return (
    <div className="chat">
      <div className="chat-head">
        {chat.choices.length > 0 ? (
          <label>
            <span className="small muted">{t('chatModel')}</span>
            <select value={chat.choice} onChange={(event) => model.chat.setChoice(Number(event.target.value))}>
              {chat.choices.map((choice, index) => (
                <option key={`${choice.engine}-${choice.model}-${index}`} value={index}>
                  {choice.label}
                </option>
              ))}
            </select>
          </label>
        ) : chat.loaded ? (
          <p className="muted small">{t('chatNoChoices')}</p>
        ) : null}
        <button onClick={() => model.chat.newChat()} disabled={!chat.sessionId}>
          {t('chatNew')}
        </button>
      </div>

      <div className="live" aria-live="polite">
        {items.length === 0 && <p className="muted empty">{t('chatEmpty')}</p>}
        {items.map((item) => (
          <p key={item.id} className={`live-${item.kind}`}>
            {item.text}
          </p>
        ))}
        <div ref={end} />
      </div>

      {chat.error && (
        <p role="alert" className="error">
          {chat.error}
        </p>
      )}

      <form
        className="composer"
        onSubmit={(event) => {
          event.preventDefault();

          // Starting work is a change: it needs the Operate window, which a passkey opens.
          void (async () => {
            if (!operate) await model.unlockOperate();

            await model.chat.send();
          })();
        }}
      >
        {names && (
          <p className="small muted" role="status">
            {t('chatAttached', { names })}
            <br />
            {t('chatFilesStay')}
          </p>
        )}
        <label htmlFor="chat-text" className="sr-only">
          {t('chatPlaceholder')}
        </label>
        <textarea id="chat-text" rows={3} placeholder={t('chatPlaceholder')} value={chat.text} onChange={(event) => model.chat.setText(event.target.value)} maxLength={20000} disabled={chat.choices.length === 0} />
        <p className="small muted">{t('chatAsksYou')}</p>
        <button type="submit" className="primary" disabled={chat.sending || !chat.text.trim() || chat.choices.length === 0}>
          {chat.sending ? t('chatSending') : t('chatSend')}
        </button>
      </form>
    </div>
  );
}
