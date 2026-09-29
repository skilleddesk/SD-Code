import { create } from 'zustand';

/**
 * How the live stream is drawn (0.15): the classic cards, Flow (a timed rail) or Console (a mission
 * log). A browser preference like the theme - it changes nothing the daemon knows.
 */
export type StreamStyle = 'classic' | 'flow' | 'console';

const KEY = 'sdc.streamStyle';

function load(): StreamStyle {
  try {
    const stored = window.localStorage.getItem(KEY);

    return stored === 'flow' || stored === 'console' || stored === 'classic' ? stored : 'flow';
  } catch {
    return 'flow';
  }
}

export const useStreamStyle = create<{ style: StreamStyle; setStyle: (style: StreamStyle) => void }>()((set) => ({
  style: load(),
  setStyle: (style) => {
    set({ style });

    try {
      window.localStorage.setItem(KEY, style);
    } catch {
      /* A lost preference is not worth an error. */
    }
  },
}));
