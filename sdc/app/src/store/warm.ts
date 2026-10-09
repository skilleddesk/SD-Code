import { sdcpCall } from '../lib/sdcp';
import { chatChoice } from './chatModel';

/**
 * Opens the provider connection while the person types (0.21) - `provider.warm`.
 *
 * The first answer after a pause used to start with a TCP connect and a TLS handshake to the provider (two
 * or three round trips, several hundred milliseconds from Bangladesh to a US endpoint). The daemon now opens
 * that connection as soon as typing starts, so it is ready when Send is pressed. At most every 15 seconds
 * from here (the daemon keeps its own 20-second limit), only for API models - a CLI engine has its own
 * process, and a local Ollama model needs no network.
 */
let last = 0;

export function warmProvider(sessionId: string | null | undefined, now = Date.now()): boolean {
  if (now - last < 15_000) {
    return false;
  }

  const choice = chatChoice(sessionId);

  if (choice.engine !== 'native_api' || choice.model === '' || choice.providerId === 'ollama') {
    return false;
  }

  last = now;

  void sdcpCall('provider.warm', choice.providerId === null ? { model: choice.model } : { model: choice.model, provider: choice.providerId }).catch(() => {
    /* A warm-up that fails costs nothing: the turn opens its own connection, as before. */
  });

  return true;
}
