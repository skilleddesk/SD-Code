import { Brain, Scale, Sparkles, SquareTerminal, Zap, type LucideIcon } from 'lucide-react';

import { TIERS, type EngineId, type Tier } from '../../store/model';

/**
 * Name-to-component for the model dropdown's chips.
 *
 * The store keeps the icons as strings (`'zap' | 'scale' | 'brain'`, `'sparkles' | 'terminal'`)
 * because it is a data module and should not import a component library. These two functions are
 * where those names become components, and they live in their own file so that both the dropdown's
 * rows and the trigger's chip can use them without one module's exports becoming the other's
 * business - a module that exports a component *and* a helper cannot be hot-reloaded, which is what
 * the `react-refresh/only-export-components` warning is about.
 */

export function tierIcon(tier: Tier): LucideIcon {
  const name = TIERS.find((candidate) => candidate.id === tier)?.icon ?? 'scale';

  if (name === 'zap') {
    return Zap;
  }

  return name === 'brain' ? Brain : Scale;
}

/** `native_api` is the one engine that is not a CLI, so it gets the terminal. */
export function engineIcon(engine: EngineId): LucideIcon {
  return engine === 'native_api' ? SquareTerminal : Sparkles;
}
