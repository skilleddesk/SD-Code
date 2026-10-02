/**
 * The words of `/research` and the local model (0.16.1), in English - `strings.research`. The language
 * packs in `locales/` translate them like any other part.
 */
export const researchStrings = {
  usage: 'Write the question after /research - for example: /research what changed in Ollama this year',
  stopped: 'Research stopped',
  nothingToStop: 'No research is running in this chat',
  planFailed: 'Could not prepare the research',
  card: {
    title: 'Research',
    question: 'Question',
    model: 'Model',
    place: { local: 'on this machine', api: 'API', cli: 'subscription CLI' },
    search: 'Search',
    limits: (searches: number, pages: number, minutes: number): string =>
      `up to ${searches} searches · ${pages} pages · ${minutes} min`,
    cost: 'Cost',
    free: 'free - runs on this machine',
    subscription: 'included in the subscription',
    estimate: (usd: string): string => `about $${usd}`,
    unknown: 'not known for this model',
    synthesis: (model: string, usd: string | null): string =>
      usd === null ? `final answer by ${model}` : `final answer by ${model} (about $${usd})`,
    context: (tokens: number): string => `context ${tokens.toLocaleString()} tokens`,
    keyMissing: (service: string): string => `${service} has no API key yet - DuckDuckGo will answer instead. Add the key in Settings → Research.`,
    ollamaDown: 'Ollama is not running - start it with `ollama serve`, or pick another model.',
    start: 'Start research',
    cancel: 'Cancel',
  },
  sources: {
    title: (count: number): string => `Sources · ${count}`,
    seen: 'seen in results',
    open: (url: string): string => `Open ${url}`,
  },
  model: {
    usage: 'Pick a model with /model <name> - or /model alone to open the list',
    notFound: (query: string): string => `No connected model matches “${query}” - open the list with /model`,
  },
  settings: {
    title: 'Research',
    intro:
      '/research answers a question from the web, with numbered sources. The search service, the limits and an optional model for the final answer are set here.',
    provider: 'Search service',
    providers: {
      duckduckgo: 'DuckDuckGo - free, no key (no dates, can be rate-limited)',
      searxng: 'SearXNG - your own server, free',
      tavily: 'Tavily - 1,000 free searches a month, key needed',
      brave: 'Brave Search - free monthly credit, key needed',
      serper: 'Serper (Google) - 2,500 free searches, key needed',
    },
    searxngUrl: 'SearXNG address',
    searxngHint: 'The server’s settings.yml must list json under search → formats.',
    key: 'API key',
    keySaved: (masked: string): string => `Key saved · ${masked}`,
    keyNone: 'No key saved',
    saveKey: 'Save key',
    removeKey: 'Remove key',
    keySavedToast: 'Key saved in the system keychain',
    keyRemovedToast: 'Key removed',
    limits: 'Limits for one question',
    maxSearches: 'Searches',
    maxPages: 'Pages',
    maxMinutes: 'Minutes',
    localWebOnly: 'Keep a local model off the web except through /research',
    localWebOnlyHint: 'Off (the default): every model searches the web when a question needs it, with the search service above. An API model keeps its web tools either way.',
    synthesis: 'Final answer model',
    synthesisNone: 'The same model that did the research',
    synthesisHint: 'Optional: a local model gathers and summarises the pages, and this model writes the final answer - it is paid for.',
    localContext: 'Local model context (tokens)',
    localContextHint: 'What SDC asks Ollama to load for every local turn. More holds more pages but needs more graphics memory; 16384 suits most 8 GB cards.',
    ollama: (running: boolean, models: number): string =>
      running ? `Ollama is running · ${models} model${models === 1 ? '' : 's'} downloaded` : 'Ollama is not running on this machine',
    pull: (model: string): string => `Not downloaded: run  ollama pull ${model}`,
    saved: 'Saved',
    failed: 'Could not save it',
  },
} as const;
