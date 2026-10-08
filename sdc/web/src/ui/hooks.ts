import { createContext, useContext, useEffect, useState, useSyncExternalStore } from 'react';
import { isRtl, pickLanguage, translator, type Key, type Lang } from '../i18n';
import type { Model, Snapshot } from '../state/model';

export const ModelContext = createContext<Model | null>(null);

export function useModel(): Model {
  const model = useContext(ModelContext);

  if (!model) throw new Error('no model');

  return model;
}

export function useSnapshot(): Snapshot {
  const model = useModel();

  return useSyncExternalStore(model.subscribe, model.getSnapshot);
}

/** The translator for the person's language, and the page direction kept in step with it. */
export function useT(): (key: Key, vars?: Record<string, string | number>) => string {
  const [lang] = useState<Lang>(() => pickLanguage());

  useEffect(() => {
    const tag = navigator.languages?.[0] ?? lang;

    document.documentElement.lang = lang;
    document.documentElement.dir = isRtl(tag) ? 'rtl' : 'ltr';
  }, [lang]);

  return translator(lang);
}

/** The current time in ms, ticking each second, for countdowns. */
export function useNow(): number {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);

    return () => clearInterval(timer);
  }, []);

  return now;
}
