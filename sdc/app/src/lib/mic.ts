import { strings } from '../strings';

/** Why the microphone could not be opened, in words the person can act on. */
export function micProblem(error: unknown): string {
  const name = error instanceof DOMException || error instanceof Error ? error.name : '';

  switch (name) {
    case 'NotAllowedError':
    case 'PermissionDeniedError':
    case 'SecurityError':
      return strings.kernel.voice.micDenied;
    case 'NotFoundError':
    case 'OverconstrainedError':
    case 'DevicesNotFoundError':
      return strings.kernel.voice.micNotFound;
    case 'NotReadableError':
    case 'AbortError':
      return strings.kernel.voice.micBusy;
    default:
      return strings.kernel.voice.micOther(name === '' ? 'unknown error' : name);
  }
}
