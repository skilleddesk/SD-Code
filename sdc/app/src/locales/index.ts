import type { Locale, Pack } from '../i18n';
import type { Strings } from '../strings';
import { ar } from './ar';
import { bn } from './bn';
import { es } from './es';
import { fr } from './fr';
import { hi } from './hi';
import { id } from './id';
import { pt } from './pt';
import { ur } from './ur';
import { zh } from './zh';

/**
 * The language packs (0.12). Each is any part of `strings`, in one language; what a pack leaves out is
 * shown in English. English itself is `strings.ts`.
 */
export type StringsPack = Pack<Strings>;

export const PACKS: Record<Locale, StringsPack | undefined> = { en: undefined, bn, hi, ar, ur, es, pt, fr, id, zh };
