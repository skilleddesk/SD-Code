/**
 * The prompt module (spec sections 7.6 and 9.3). `PromptArea` is the whole bottom block of a pane;
 * the pieces are exported because the model selector is also reachable from the status bar
 * (spec section 7.15).
 *
 * There is no keyboard hook here: Alt+M and Alt+E are registered in `commands/registry.ts` like
 * every other shortcut, so the palette, the F1 reference and the Settings → Keymap tab all list
 * them (spec section 9.1, principle P7).
 */
export { PromptArea } from './PromptArea';
export { ModelSelector } from './ModelSelector';
export { ModelDropdown } from './ModelDropdown';
export { tierIcon, engineIcon } from './modelIcons';
export { QueuedChips } from './QueuedChips';
