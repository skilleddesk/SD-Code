import { strings } from '../../strings';
import { landIn, newChatOnHost } from '../../store/intents';
import { findSession } from '../../store/reducer';
import { listCommands, rememberFact, useAgentUi } from '../../store/agentIntents';
import { useAppStore } from '../../store/store';
import { toast } from '../../store/toast';

/**
 * The composer's `/` commands (0.13) - Claude Code's, on SDC's pipeline:
 *
 * | command            | what happens                                                                  |
 * | ------------------ | ----------------------------------------------------------------------------- |
 * | `/compact`         | a turn whose answer becomes the chat's summary; later turns start from it      |
 * | `/init`            | the agent studies the project and writes `.sdc/rules.md`                       |
 * | `/review`          | a review of the uncommitted changes, nothing changed                           |
 * | `/remember <fact>` | one line into the project's memory (the global one without a folder)           |
 * | `/memory`          | the memory editor                                                              |
 * | `/clear`           | a new chat in the same folder                                                  |
 * | `/help`            | what these do                                                                  |
 * | `/<project cmd>`   | the project's `.sdc/commands/<name>.md` (or `.claude/commands`), `$ARGUMENTS` filled |
 *
 * Anything else that starts with `/` goes to the engine as typed - Claude Code has commands of its own.
 */
export type SlashOutcome =
  | { kind: 'handled' }
  | { kind: 'send'; prompt: string; compact?: boolean };

export async function runSlash(text: string, sessionId: string | null): Promise<SlashOutcome> {
  const trimmed = text.trim();
  const [head = '', ...rest] = trimmed.slice(1).split(/\s+/);
  const name = head.toLowerCase();
  const args = rest.join(' ').trim();
  const words = strings.agent.commands;

  switch (name) {
    case 'compact':
      toast(words.compacting);

      return { kind: 'send', prompt: '/compact', compact: true };

    case 'init': {
      return { kind: 'send', prompt: args === '' ? words.initPrompt : `${words.initPrompt}\n\n${args}` };
    }

    case 'review':
      return { kind: 'send', prompt: args === '' ? words.reviewPrompt : `${words.reviewPrompt}\n\nFocus: ${args}` };

    case 'remember':
      if (args === '') {
        toast(words.rememberEmpty);
      } else {
        await rememberFact(sessionId, args);
      }

      return { kind: 'handled' };

    case 'memory':
      useAgentUi.getState().openMemory();

      return { kind: 'handled' };

    case 'clear': {
      const found = sessionId === null ? null : findSession(useAppStore.getState().hosts, sessionId);

      if (found !== null) {
        const fresh = await newChatOnHost(found.host.id, found.session.projectId ?? null);

        if (fresh !== null) {
          landIn(fresh);
        }
      }

      return { kind: 'handled' };
    }

    case 'help':
      toast(words.help);

      return { kind: 'handled' };

    default: {
      const project = (await listCommands(sessionId)).find((command) => command.name.toLowerCase() === name && command.body !== undefined);

      if (project?.body !== undefined) {
        const body = project.body.replace(/^---[\s\S]*?---\s*/, '');

        return { kind: 'send', prompt: body.includes('$ARGUMENTS') ? body.replace(/\$ARGUMENTS/g, args) : args === '' ? body : `${body}\n\n${args}` };
      }

      return { kind: 'send', prompt: text };
    }
  }
}
