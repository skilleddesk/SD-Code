import type { ProjectView, TurnView } from '../../store/types';

/**
 * Live preview (0.12.5): where the page is, and when it changed - both read from the chat's own log.
 *
 * A dev server says its address when it starts (`Local: http://localhost:5173/`); a Run card carries that
 * line, and so can the agent's words. A site on a VPS is its domain. Nothing here opens a connection: it
 * only reads what the turns already said.
 */

const LOCAL_URL = /\bhttps?:\/\/(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\])(?::\d{2,5})?(?:\/[^\s'"<>)\]`]*)?/gi;
const DOMAIN = /^(?:[a-z0-9-]+\.)+[a-z]{2,}$/i;

/** The addresses worth previewing for a chat, newest first: dev servers it started, then its site. */
export function previewCandidates(turns: readonly TurnView[], sessionId: string, project: ProjectView | undefined): string[] {
  const found: string[] = [];
  const add = (raw: string): void => {
    /* `0.0.0.0` is where a server listens, not an address a browser can open. */
    const url = raw.replace('://0.0.0.0', '://localhost').replace(/[.,;:*_~]+$/, '');

    if (!found.includes(url)) {
      found.push(url);
    }
  };

  for (const turn of [...turns].reverse()) {
    if (turn.sessionId !== sessionId) {
      continue;
    }

    for (const tool of [...turn.tools].reverse()) {
      for (const line of [...tool.output].reverse()) {
        for (const match of line.text.matchAll(LOCAL_URL)) {
          add(match[0]);
        }
      }
    }

    for (const match of turn.text.matchAll(LOCAL_URL)) {
      add(match[0]);
    }
  }

  /* A project named after a domain (a site on a VPS) previews at that domain. */
  if (project !== undefined && DOMAIN.test(project.name)) {
    add(`https://${project.name}/`);
  }

  return found.slice(0, 4);
}

/** The newest finished change in a chat - what a live preview reloads after: its stamp and its file. */
export function lastChange(turns: readonly TurnView[], sessionId: string): { key: string; target: string } | null {
  for (const turn of [...turns].reverse()) {
    if (turn.sessionId !== sessionId) {
      continue;
    }

    for (const tool of [...turn.tools].reverse()) {
      if (tool.tool === 'edit' && tool.status === 'done') {
        return { key: `${turn.id}/${tool.callId}`, target: tool.target };
      }
    }
  }

  return null;
}
