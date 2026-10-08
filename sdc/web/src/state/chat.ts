// Starting and continuing a chat from the browser.
//
// A message is text plus *paths* (ids of files picked in the Files tab). The file contents never travel to this
// page and back: the daemon puts the paths in the prompt and the AI reads the files itself, on the machine that has
// them. That is why attaching a 2 MB file costs the phone nothing.

import { RequestFailed, type Link } from '../transport/link';
import type { Workspace } from './workspace';

export interface Choice {
  label: string;
  engine: string;
  provider: string | null;
  model: string;
  group: 'subscription' | 'api' | 'local';
}

export interface ChatState {
  choices: Choice[];
  choice: number;
  loaded: boolean;
  text: string;
  sending: boolean;
  sessionId: string | null;
  error: string | null;
}

export const emptyChat = (): ChatState => ({ choices: [], choice: 0, loaded: false, text: '', sending: false, sessionId: null, error: null });

export class Chat {
  state: ChatState = emptyChat();

  constructor(
    private readonly link: () => Link | null,
    private readonly workspace: () => Workspace,
    private readonly changed: () => void,
  ) {}

  private set(patch: Partial<ChatState>): void {
    this.state = { ...this.state, ...patch };
    this.changed();
  }

  reset(): void {
    this.state = emptyChat();
    this.changed();
  }

  async loadOptions(): Promise<void> {
    const link = this.link();

    if (!link) return;

    try {
      const options = (await link.rpc('chat.options')) as { choices: Choice[] };

      this.set({ choices: options.choices, loaded: true, choice: Math.min(this.state.choice, Math.max(0, options.choices.length - 1)) });
    } catch (error) {
      this.set({ loaded: true, error: (error as Error).message });
    }
  }

  setChoice(index: number): void {
    this.set({ choice: index });
  }

  setText(text: string): void {
    this.set({ text });
  }

  newChat(): void {
    this.set({ sessionId: null, error: null });
  }

  async send(): Promise<boolean> {
    const link = this.link();
    const workspace = this.workspace().state;
    const text = this.state.text.trim();

    if (!link || !text || this.state.sending) return false;

    const choice = this.state.choices[this.state.choice];

    this.set({ sending: true, error: null });

    try {
      const sent = await link.rpc('chat.send', {
        text,
        attachments: workspace.picked.map((item) => item.path_id),
        root_id: workspace.rootId,
        session_id: this.state.sessionId ?? undefined,
        engine: choice?.engine,
        provider: choice?.provider ?? undefined,
        model: choice?.model,
        agent: true,
        autonomy: 'ask',
      });

      this.set({ sending: false, text: '', sessionId: sent.session_id });
      this.workspace().clearPicked();

      return true;
    } catch (error) {
      this.set({ sending: false, error: error instanceof RequestFailed ? error.message : (error as Error).message });

      return false;
    }
  }
}
