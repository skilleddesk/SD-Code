/**
 * Every user-visible string in the app lives in this file (spec section 2.7), grouped by
 * surface, so that localisation has a door to open later. Components must not inline copy.
 *
 * The file also carries the demo data the UI is seeded with - session titles, prompt bodies, the
 * console entries, the Time Machine checkpoints, the verify rows - because those are user-visible
 * copy too, and because seeding them anywhere else would put prose in a component. Each seed lives
 * next to the surface it belongs to, so `strings.sidebar.sessions` is what the session store starts
 * from and `strings.rightPanel.console.entries` is what the Console tab renders.
 *
 * A handful of entries are functions of one or two values (`strings.prompt.sent(engine, model)`).
 * They are still strings, still here, still the only place that knows the wording.
 */
export const strings = {
  /** Window/document metadata. The OS window title is set in src-tauri/tauri.conf.json. */
  app: {
    name: 'SDC',
    tagline: 'Tauri 2 · React 18 · TypeScript · Tailwind',
  },

  /** Topbar - spec section 7.1, in the order the buttons appear (left to right). */
  topbar: {
    brandInitial: 'S',
    brandText: 'SDC',
    brandTitle: 'About SDC',
    activeHostTitle: 'Active host',
    palette: {
      label: 'Search or jump to…',
      shortcut: '⌘K',
      title: 'Command palette',
    },
    providers: {
      title: 'Providers & Models',
      dotTitle: 'A provider needs authentication',
    },
    theme: {
      title: 'Toggle theme',
      dark: 'Theme: dark',
      light: 'Theme: light',
    },
    sidebar: { title: 'Toggle sidebar' },
    right: { title: 'Toggle right panel' },
    settings: { title: 'Settings' },
    modeSwitchTitle: 'Interface mode',
    /** Simple / Pro / Auto - Pro is the default (spec section 7.1, row 5). */
    modes: {
      simple: 'Simple',
      pro: 'Pro',
      auto: 'Auto',
    },
    modeChanged: (mode: string): string => `Mode: ${mode}`,
  },

  /** Host switcher popover and the New chat popover (spec sections 7.1 row 3 and 9.5). */
  popover: {
    hostSwitcherTitle: 'Active host',
    newChatTitle: 'New chat on…',
    connected: 'connected',
    degraded: 'degraded',
    offline: 'offline',
    connecting: 'connecting',
    /** `3 chats · connected` - the second line of a New chat row. */
    chatsAndStatus: (chats: number, status: string): string =>
      `${chats} chat${chats === 1 ? '' : 's'} · ${status}`,
  },

  /** Sidebar - spec section 7.3. */
  sidebar: {
    newChat: 'New chat',
    newChatShortcut: '⌘N',
    filterPlaceholder: 'Filter chats…',
    addHost: 'Add host (VPS or local)',
    startChat: 'Start a chat',
    renamePrompt: 'Rename chat:',
    deleteConfirm: (title: string): string => `Delete "${title}"?`,
    deleted: 'Chat deleted',
    newChatOn: (host: string): string => `New chat on ${host}`,
    actions: {
      rename: 'Rename',
      delete: 'Delete',
      newChatOnHost: 'New chat on this host',
    },
    /** The badge is one character; its colour and meaning come from the class (spec section 7.3). */
    attentionBadge: '!',
    /** Seed host names and session titles - the prototype's demo data, verbatim. */
    hosts: {
      local: 'Local',
      prod1: 'prod-1',
      staging2: 'staging-2',
    },
    sessions: {
      rateLimiting: {
        title: 'Add rate limiting',
        prompt: 'Add rate limiting to the login route',
      },
      loginBug: { title: 'Fix login bug', prompt: 'Fix the login bug' },
      refactorAuth: { title: 'Refactor auth', prompt: 'Refactor src/auth.ts' },
      deployScript: { title: 'Deploy script', prompt: 'Fix the deploy script' },
      logAggregation: { title: 'Log aggregation', prompt: 'Set up log aggregation' },
      updateReadme: { title: 'Update README', prompt: 'Update README' },
      newChat: { title: 'New chat', prompt: 'Describe what you want to build…' },
    },
  },

  /** Tab strip - spec section 7.4. */
  tabs: {
    split: {
      title: 'Split view (⌘\\)',
      on: 'Split view on',
      off: 'Split view off',
    },
    newTab: { title: 'New chat (⌘N)' },
    close: 'Close tab',
  },

  /** Main column - the degraded banner of spec section 7.4 and the empty state of 7.13. */
  main: {
    degraded: {
      message: (host: string): string =>
        `Not connected to ${host}. Files and history are still available.`,
      reconnect: 'Reconnect',
      reconnected: 'Reconnected',
    },
    empty: {
      title: 'No chat open',
      description:
        'Start a new chat, or pick one from the sidebar. Work across multiple chats and hosts.',
      chips: {
        newChat: 'New chat',
        connectModel: 'Connect a model',
        addVps: 'Add a VPS',
      },
    },
    /** Separator between the host name and the title in `.pane-header` (spec section 9.15). */
    paneHeaderSeparator: '·',
  },

  /** Turn stream - spec section 7.5. The seed is the prototype's worked example, verbatim. */
  turns: {
    who: 'You · 14:02',
    prompt: 'Add rate limiting to the login route',
    attachments: {
      /** The gradient preview chip carries no label; the other two are labelled. */
      image: 'screenshot.png',
      file: '@src/auth.ts',
    },
    collapsed: {
      label: 'Turns 1–6 collapsed',
      meta: '8,420 tokens · $0.31',
      toast: 'Expanded 6 earlier turns',
    },
    thinking: {
      title: 'Thinking',
      duration: '(4s)',
      body:
        'I will add express-rate-limit to the login route. First I need to read the current ' +
        'auth.ts, then install the dependency and wire up the middleware.',
    },
    tools: {
      read: 'Read',
      edit: 'Edit',
      run: 'Run',
      /** Status pills: `done · 42 ln`, `done · +18 −2`, `running`. */
      readStatus: 'done · 42 ln',
      editStatus: 'done · +18 −2',
      runStatus: 'running',
      /** The two words a run's output lines are prefixed with (spec section 7.5). */
      runPass: 'PASS',
      runFail: 'FAIL',
    },
    error: {
      title: 'Test failed: rate.test.ts line 42',
      explanation:
        'The limiter allows a 6th request. The window may be resetting too early, or the ' +
        'middleware is not applied to the correct route.',
      fix: 'Fix this',
      showCode: 'Show code',
      explainMore: 'Explain more',
      showCodeToast: 'Opened rate.test.ts:42',
      explainMoreToast: 'Expanded',
    },
    footer: {
      summary: 'Done',
      detail: '1m 12s · 12,400 tokens · $0.16',
      up: 'Good response',
      down: 'Bad response',
      upToast: 'Thanks',
      downToast: 'Noted',
    },
    /** The live turn's meta line - the forecast the prompt area shows before sending. */
    metaForecast: '~$0.10 – $0.28 forecast',
    /**
     * What a live turn streams as its answer, word by word, so `TurnDelta` has something real to
     * concatenate. The seeded demo turn of `panels/turns/types.ts` is still what the stream draws
     * for the prototype's example; this is what the daemon's timings push into the event log.
     */
    answer:
      'Added express-rate-limit to the login route and wired the limiter as middleware. Five ' +
      'attempts per fifteen minutes per IP, with the store shared across workers.',
  },

  /** Prompt area and model dropdown - spec sections 7.6 and 9.3. */
  prompt: {
    placeholder: 'Ask or describe what you want to build…',
    filesChip: '1 file',
    contextChip: '12.4k ctx',
    queued: {
      remove: 'Remove queued prompt',
      /** Seeded so the queue is visible; the spec caps it at three (section 9.7). */
      seed: ['also add a test for this'],
    },
    toolbar: {
      attach: 'Attach a file',
      image: 'Paste image',
      file: '@ file',
      command: '/ command',
    },
    send: 'Send',
    tip: {
      file: '@',
      fileLabel: 'reference file',
      command: '/',
      commandLabel: 'commands',
      palette: '⌘K',
      paletteLabel: 'palette',
    },
    sent: (engine: string, model: string): string => `Sent to ${engine} · ${model}`,
    empty: 'Type a prompt first',
    interrupt: 'Interrupted',
    killed: 'Force killed',
    /** Spec section 16.5: the Session Bridge toast when a mid-turn engine switch succeeds. */
    bridged: (from: string, to: string): string => `Switched ${from} → ${to}, context replayed`,
    model: {
      groupTitles: {
        tier: 'Tier',
        engine: 'Engine',
        model: 'Model',
      },
      tiers: {
        fast: { label: 'Fast', description: 'quick edits' },
        balanced: { label: 'Balanced', description: 'everyday' },
        deep: { label: 'Deep', description: 'architecture' },
      },
      engines: {
        claude_code: { name: 'Claude Code', description: 'Claude Max subscription' },
        codex: { name: 'Codex CLI', description: 'ChatGPT Plus' },
        gemini: { name: 'Gemini CLI', description: 'Google AI' },
        native_api: { name: 'Native API', description: 'API key · any provider' },
      },
      models: {
        haiku: { name: 'Haiku 4', description: 'fast' },
        sonnet: { name: 'Sonnet 4.5', description: 'balanced' },
        opus: { name: 'Opus 4', description: 'deep' },
        codexDefault: { name: 'Default', description: 'gpt-5-mini' },
        gpt5: { name: 'GPT-5', description: 'flagship' },
        flash: { name: 'Gemini Flash', description: 'fast' },
        pro: { name: 'Gemini Pro', description: 'long ctx' },
        claudeSonnet: { name: 'claude-sonnet-4-5', description: 'Anthropic' },
        deepseekChat: { name: 'deepseek-chat', description: 'DeepSeek' },
        llama: { name: 'llama3.2', description: 'Ollama local' },
      },
      connectMore: 'Connect more providers or models…',
      footerNote: 'Every change visible',
      footerCost: '~$0.10 – $0.28',
      tierChanged: (tier: string): string => `Tier: ${tier}`,
      engineChanged: (engine: string): string => `Engine: ${engine}`,
      modelChanged: (model: string): string => `Model: ${model}`,
    },
  },

  /** Right panel - spec sections 7.7 to 7.12. */
  rightPanel: {
    tabs: {
      preview: 'Preview',
      console: 'Console',
      timemachine: 'Time Machine',
      duel: 'Duel',
      verify: 'Verify',
      analytics: 'Analytics',
    },
    resize: 'Resize panel',
    preview: {
      back: 'Back',
      forward: 'Forward',
      reload: 'Reload',
      popOut: 'Pop out',
      url: 'http://localhost:5173/login',
      devices: {
        mobile: 'Mobile 390',
        tablet: 'Tablet 768',
        desktop: 'Desktop',
      },
      /** The toast names the width the frame was clamped to (spec section 7.7). */
      deviceToast: (width: number | null): string =>
        width === null ? 'Preview: full width' : `Preview: ${width}px`,
      page: { name: 'Login', path: 'src/routes/login.tsx' },
      attach: 'Attach screenshot',
      attached: 'Screenshot attached',
    },
    console: {
      fixWithAgent: 'Fix with agent',
      /** What `Fix with agent` seeds the new turn with (spec section 15.5). */
      fixPrompt: 'Fix this error so the preview runs again.',
      empty: 'Nothing logged yet.',
      jumped: (file: string, line: number): string => `Jumped to ${file}:${line}`,
      /** Deduplicated entries: `count` is how often the same line was logged (spec section 7.8). */
      entries: [
        {
          level: 'error',
          message: 'Uncaught ReferenceError: handleSubmit is not defined',
          source: 'at LoginForm.tsx:42:11',
          file: 'LoginForm.tsx',
          line: 42,
          count: 3,
        },
        {
          level: 'warn',
          message: 'Warning: Each child in a list should have a unique "key" prop.',
          source: 'at UserList.tsx:18:5',
          file: 'UserList.tsx',
          line: 18,
          count: 1,
        },
      ],
    },
    timeMachine: {
      current: 'CURRENT',
      compare: 'Compare two points',
      compareToast: 'Select two points',
      rewound: (turn: number): string => `Rewound to turn ${turn}`,
      undo: 'Undo this',
      empty: 'Checkpoints appear here after your first change.',
      entries: [
        { turn: 14, when: 'now', title: 'Added validation' },
        { turn: 13, when: '2 min ago', title: 'Fixed the test' },
        { turn: 12, when: '8 min ago', title: 'Broke the login' },
      ],
    },
    duel: {
      diff: 'Diff',
      keep: 'Keep',
      kept: (engine: string): string => `${engine} kept`,
      empty: 'No duel yet. Run the same prompt on two engines and compare.',
      run: 'Run a duel',
      runAgain: 'Run again',
      keepNeither: 'Keep neither',
      keptLabel: 'KEPT',
      /** Two runs of the same prompt, as the prototype's Duel tab shows them (spec section 7.10). */
      panes: [
        {
          engine: 'claude_code',
          model: 'sonnet',
          time: '48s',
          cost: '$0.12',
          pass: true,
          headline: '3 files changed',
          files: ['+ rate.ts (new)', '+ auth.ts modified', '+ auth.test.ts'],
        },
        {
          engine: 'codex',
          model: 'default',
          time: '71s',
          cost: '$0.09',
          pass: false,
          headline: '2 files changed',
          files: ['+ auth.ts modified', '+ package.json'],
        },
      ],
    },
    verify: {
      run: 'Run verify (⌘⏎)',
      result: 'Verify — 3 pass, 1 fail',
      empty: 'Nothing to verify yet.',
      rows: [
        { name: 'typecheck', pass: true, time: '2.1s' },
        { name: 'build', pass: true, time: '4.8s' },
        { name: 'test · rate.test.ts', pass: false, time: '1.2s' },
        { name: 'lint', pass: true, time: '0.9s' },
      ],
    },
    analytics: {
      spending: 'Spending',
      range: '7d',
      totalTitle: 'Total this week',
      total: '$4.12',
      byEngineTitle: 'By engine',
      limitsTitle: 'Limits',
      /** Seven bars, each a percentage of the chart's height (spec section 7.12). */
      spendBars: [33, 48, 40, 58, 45, 32, 25],
      byEngine: [
        { label: 'claude_code', percent: 68 },
        { label: 'codex', percent: 22 },
        { label: 'gemini', percent: 10 },
      ],
      limits: [
        { name: 'Claude Max', value: '~60%', estimate: '(est)' },
        { name: 'resets in', value: '2h 14m', estimate: null },
      ],
    },
  },

  /** Status bar - spec section 7.15, left to right. */
  statusBar: {
    providers: 'providers',
    chats: 'chats',
    hosts: 'hosts',
    connection: {
      ready: 'ready',
      degraded: 'degraded',
      offline: 'offline',
    },
  },

  /**
   * The daemon handshake at startup (spec sections 3.1 and 5.4).
   *
   * The window asks `host.status` as soon as it mounts, which is what starts the daemon on a fresh
   * install (the Tauri bridge spawns `sdcd` when nothing is listening). If that fails, the app says
   * so once, in plain words, instead of looking connected while every action fails.
   */
  daemon: {
    offline: 'sdcd did not answer · the daemon is not running',
  },

  /**
   * The Connect modal - the two flows that get a user working (spec section 9.10).
   *
   * `login` is the CLI's own sign-in, driven from here: the daemon starts the CLI, shows the URL it
   * prints, and hands back the code that gets pasted in. `api` is an API key plus the model to use it
   * with, and the model list says where each row came from so "up to date" is visible.
   */
  connect: {
    title: 'Connect',
    loginTitle: 'Sign in with the CLI',
    loginBody:
      'This starts the CLI’s own sign-in. Approve the page in your browser, then paste the code it shows back here — the credential is written by the CLI, never by SDC.',
    signIn: 'Sign in',
    cancel: 'Cancel',
    copyLink: 'Copy link',
    copied: 'Link copied',
    openLink: 'Open in browser',
    waiting: 'Waiting for the CLI…',
    waitingForCode: 'Approve the page, then paste the code below.',
    codeLabel: 'Code from the page',
    codePlaceholder: 'Paste the code or the whole redirect URL',
    submitCode: 'Submit code',
    authenticated: 'Signed in',
    authenticatedBody: 'The CLI is signed in. The card now says connected.',
    finished: 'The CLI finished — read its own output below to see how it went.',
    failed: 'The sign-in stopped before it finished.',
    outputTitle: 'The CLI’s own output',
    outputEmpty: 'Nothing yet.',
    installFirst: 'That CLI is not installed yet. Settings → Environment has the row with the install step.',
    loginFailed: 'Could not start the sign-in',
    codeFailed: 'The CLI did not take that code',
    apiTitle: 'API key and model',
    apiBody: 'Paste the key, then load the models the provider lists and pick the one to use.',
    loadModels: 'Load models',
    refreshModels: 'Refresh',
    modelsTitle: 'Models',
    modelsEmpty: 'No models yet — press Load models.',
    modelsSnapshot: (date: string) => `Bundled list · snapshot ${date}`,
    source: {
      live: 'live',
      cache: 'cached',
      bundled: 'bundled',
    },
    sourceHelp: {
      live: 'the provider answered just now',
      cache: 'the last answer from the provider',
      bundled: 'shipped with this build',
    },
    selected: 'In use',
    use: 'Use',
    modelsFailed: 'Could not load the models',
    modelFailed: 'Could not record that model',
    context: (ctx: number) => `${Math.round(ctx / 1000)}K context`,
  },

  /** Toast stack - spec section 9.14. */
  toast: {
    dismiss: 'Dismiss',
  },

  /**
   * Provider Hub - spec section 9.10. Seven nav items, the provider card, the six connect flows,
   * the model registry and the environment doctor. Every word here is the prototype's, because the
   * prototype is the UI source of truth (spec section 7).
   */
  hub: {
    navTitle: 'Providers',
    navConfigTitle: 'Configuration',
    /** `[id, label, icon]`, in the prototype's order. The icon is a Lucide component name. */
    nav: [
      { id: 'all', label: 'All providers', icon: 'grid' },
      { id: 'subscriptions', label: 'Subscriptions', icon: 'crown' },
      { id: 'api-keys', label: 'API keys', icon: 'key' },
      { id: 'local', label: 'Local (Ollama)', icon: 'hardDrive' },
      { id: 'custom', label: 'Custom endpoint', icon: 'plug' },
      { id: 'registry', label: 'Model registry', icon: 'list' },
      { id: 'doctor', label: 'Environment doctor', icon: 'stethoscope' },
    ],
    titles: {
      all: ['All providers', 'Connect subscriptions and API keys. SDC never stores what it did not create.'],
      subscriptions: [
        'Subscriptions',
        'Use your existing Claude Pro / Max, ChatGPT Plus, or Google AI subscription.',
      ],
      'api-keys': ['API keys', 'Direct API access. Keys are stored in the OS keychain, never on disk.'],
      local: ['Local models', 'Ollama, LM Studio, vLLM — running on this machine.'],
      custom: ['Custom endpoint', 'Any OpenAI-compatible endpoint.'],
      registry: [
        'Model registry',
        'All models across your connected providers. Enable the ones you want.',
      ],
      doctor: ['Environment doctor', 'Checks runtime versions, missing dependencies, ports, and disk.'],
    },
    close: 'Close',
    connectedCount: (n: number): string => `${n} connected`,
    availableTitle: 'Available to connect',
    noneYet: 'None yet',
    status: {
      connected: 'Connected',
      'needs-auth': 'Needs setup',
      available: 'Available',
      error: 'Error',
    },
    kind: {
      subscription: 'Subscription',
      'api-key': 'API key',
      local: 'Local',
      custom: 'Custom',
    },
    manage: 'Manage',
    connect: 'Connect',
    back: 'Back',
    save: 'Save',
    test: 'Test connection',
    testShort: 'Test',
    testing: 'Testing…',
    testEmpty: 'Enter a key first',
    testOk: (models: number): string => `OK · key valid · ${models} models available`,
    testFail: (detail: string): string => `Failed · ${detail}`,
    keyLabel: 'API key',
    keyRequired: '*',
    keyPlaceholder: 'sk-…',
    keyHint: (name: string): string => `Get one from ${name} dashboard`,
    labelField: 'Label (optional)',
    labelPlaceholder: 'e.g. personal',
    keychainNote: 'Your key is stored in the OS keychain — never on disk.',
    subscriptionNote: (name: string): string =>
      `A browser window will open. ${name} handles the login; SDC only receives a token.`,
    openBrowser: 'Open browser login',
    oauthWaiting: 'Waiting for browser authorization…',
    oauthOk: 'Authorized · token received',
    localNote: 'Auto-detected local servers.',
    daemonRow: 'Ollama daemon',
    installedRow: 'Installed models',
    daemonDetail: 'running · http://localhost:11434',
    daemonDown: 'not running · start it with `ollama serve`',
    installedDetail: (models: readonly string[]): string =>
      `${models.length} present · ${models.join(', ')}`,
    noModels: 'none installed · pull one from the registry',
    customUrl: 'Endpoint URL',
    customKey: 'API key',
    customProtocol: 'Protocol',
    protocols: ['OpenAI-compatible', 'Anthropic-compatible'],
    registryTitle: (n: number): string => `${n} models across your connected providers`,
    registryCtx: (k: number): string => `${k}k ctx`,
    doctorTitle: 'Environment checks',
    manageToast: (account: string): string => `${account} — configured`,
    connectToast: (name: string): string => `${name} connected`,
    alreadyConnected: (name: string, account: string): string =>
      `${name} — already connected (${account})`,
    keyToast: 'Enter an API key',
    endpointToast: 'Custom endpoint saved',
    modelToggled: (id: string, on: boolean): string => `${id} ${on ? 'enabled' : 'disabled'}`,
    doctorFixed: (fix: string): string => `${fix}: done`,
  },
  /** Add host - spec section 9.12. */
  addHost: {
    title: 'Add a host',
    sub: 'Connect to a local machine or a VPS over SSH',
    types: {
      local: { label: 'Local', desc: 'This computer' },
      ssh: { label: 'SSH / VPS', desc: 'user@host' },
    },
    sshTarget: 'SSH target',
    sshTargetPlaceholder: 'user@vps.example.com',
    labelField: 'Label (optional)',
    labelPlaceholder: 'prod-1',
    cancel: 'Cancel',
    connect: 'Connect',
    connecting: (label: string): string => `Connecting to ${label}…`,
    connected: (label: string): string => `Connected: ${label}`,
    localAlready: 'Local host already connected',
    needTarget: 'Enter an SSH target like user@host',
    welcomeTitle: (label: string): string => `Welcome to ${label}`,
    welcomePrompt: 'Try the sample project',
  },

  /** Permission dialog - spec section 9.13. */
  permission: {
    title: 'Delete a file',
    sub: 'Claude wants to perform a mutating action',
    target: 'src/database.js',
    explainStrong: 'What this file does:',
    explain:
      'Your database connection settings. If this is deleted, your app will stop loading data.',
    note: 'A checkpoint was saved before this turn, so you can undo it either way.',
    deny: 'Esc · Deny',
    allowOnce: 'Allow once · Enter',
    allowAlways: 'Always allow',
    showMe: 'Show me the file',
    granted: 'Permission granted',
    denied: 'Denied',
    alwaysToast: (target: string): string => `Always allow: ${target}`,
    /** The three risk levels; the risk decides which button is the default (spec section 9.13). */
    risk: {
      SAFE: 'Safe',
      MUTATING: 'Mutating',
      DANGEROUS: 'Dangerous',
    },
  },

  /** Search overlay - spec section 9.4. */
  search: {
    placeholder: 'Search sessions, files, prompts…',
    groups: {
      sessions: (n: number): string => `Sessions (${n})`,
      files: (n: number): string => `Files (${n})`,
      prompts: (n: number): string => `Prompt library (${n})`,
    },
    empty: 'No matches',
    /** The five files the prototype's index carries (design/ui-prototype.html). */
    files: ['src/auth.ts', 'src/login.tsx', 'src/rate.ts', 'tests/auth.test.ts', 'package.json'],
    prompts: [
      {
        title: 'Fix the failing test',
        body:
          'The test <name> is failing. Read the test, read the code it tests, find the cause, and ' +
          'fix the code (not the test) unless the test is wrong.',
      },
      {
        title: 'Explain this file',
        body:
          'Explain @<file> in plain language. What is it for, what calls it, and what would break ' +
          'if it were deleted?',
      },
      {
        title: 'Add tests',
        body: 'Write tests for @<file>. Cover the happy path and at least two edge cases.',
      },
    ],
    openedFile: (file: string): string => `Opened ${file}`,
    insertedPrompt: (title: string): string => `Inserted prompt: ${title}`,
  },

  /** Command palette - spec section 9.2. */
  palette: {
    placeholder: 'Type a command or search…',
    recent: 'Recent',
    actions: 'Actions',
    empty: 'No matching commands',
  },

  /** Keyboard reference overlay - spec section 9.1, F1. */
  keymap: {
    title: 'Keyboard reference',
    sub: 'Every shortcut the app listens for. One registry, so this list cannot drift.',
    close: 'Close',
    groups: {
      global: 'Global',
      session: 'Session',
      model: 'Model',
      approval: 'Approval dialog',
      timeline: 'Timeline',
      actions: 'Actions',
    },
    /** The four documented exceptions: Ctrl+letter that DOES fire inside a text field. */
    firesInInputs: 'Works while typing',
  },
  /**
   * Settings - spec section 9.11. Seven tabs, and the rows are *data* rather than markup: the tab
   * bodies differ only in which rows they carry, so the table lives here (where the copy rule of
   * spec section 2.7 wants it) and `modals/Settings.tsx` renders it. The Keymap tab is the one
   * exception - it reads the command registry, so the two can never disagree (spec section 9.1).
   */
  settings: {
    nav: [
      { id: 'general', label: 'General', icon: 'sliders' },
      { id: 'appearance', label: 'Appearance', icon: 'palette' },
      { id: 'keymap', label: 'Keymap', icon: 'keyboard' },
      { id: 'safety', label: 'Safety', icon: 'shield' },
      { id: 'notifications', label: 'Notifications', icon: 'bell' },
      { id: 'backup', label: 'Backup', icon: 'databaseBackup' },
      { id: 'about', label: 'About', icon: 'info' },
    ],
    general: {
      title: 'General',
      desc: 'Startup, behavior, and language.',
      groups: [
        {
          title: 'Behavior',
          rows: [
            {
              kind: 'toggle',
              id: 'restore-session',
              label: 'Restore last session on launch',
              help: 'Reopen the chats you had open',
              value: true,
            },
            {
              kind: 'toggle',
              id: 'confirm-close',
              label: 'Confirm before closing',
              help: 'Warn if a turn is running',
              value: true,
            },
            {
              kind: 'toggle',
              id: 'auto-verify',
              label: 'Auto-verify after turn',
              help: 'Run tests, build, lint automatically',
              value: true,
            },
            {
              kind: 'select',
              id: 'permission-mode',
              label: 'Default permission mode',
              options: ['Ask', 'Auto-edit', 'Full auto'],
              value: 'Auto-edit',
            },
          ],
        },
      ],
    },
    appearance: {
      title: 'Appearance',
      desc: 'Theme, density, typography.',
      groups: [
        {
          title: 'Theme',
          rows: [
            {
              kind: 'theme',
              id: 'theme',
              label: 'Color theme',
              options: ['Dark', 'Light'],
              value: 'Dark',
            },
            {
              kind: 'select',
              id: 'font-size',
              label: 'Font size',
              options: ['90%', '100%', '110%'],
              value: '100%',
            },
          ],
        },
        {
          title: 'Layout',
          rows: [
            {
              kind: 'toggle',
              id: 'compact-rows',
              label: 'Compact rows',
              help: '28px row height (developer density)',
              value: true,
            },
          ],
        },
      ],
    },
    safety: {
      title: 'Safety',
      desc: 'Permission, sandbox, and audit.',
      groups: [
        {
          title: 'Permissions',
          rows: [
            {
              kind: 'toggle',
              id: 'learn-allowlist',
              label: 'Learn allowlist per project',
              help: 'Remember "always allow" choices',
              value: true,
            },
            {
              kind: 'toggle',
              id: 'dry-run-dangerous',
              label: 'Show dry-run preview for DANGEROUS',
              help: 'Preview what a destructive command touches before approving',
              value: true,
            },
          ],
        },
        {
          title: 'Sandbox',
          rows: [
            {
              kind: 'toggle',
              id: 'bubblewrap',
              label: 'bubblewrap sandbox',
              help: "Linux only · some commands won't work",
              value: false,
            },
          ],
        },
        {
          title: 'Secrets',
          rows: [
            {
              kind: 'toggle',
              id: 'block-secrets',
              label: 'Block .env, *.pem, id_rsa',
              help: 'Never send these to providers',
              value: true,
            },
            {
              kind: 'toggle',
              id: 'redact-logs',
              label: 'Redact secrets in logs and prompts',
              value: true,
            },
          ],
        },
      ],
    },
    notifications: {
      title: 'Notifications',
      desc: 'Desktop alerts and sound cues.',
      groups: [
        {
          title: 'Desktop',
          rows: [
            {
              kind: 'toggle',
              id: 'notify-approval',
              label: 'Notify when approval is needed',
              value: true,
            },
            {
              kind: 'toggle',
              id: 'notify-complete',
              label: 'Notify when turn completes (if unfocused)',
              value: true,
            },
            {
              kind: 'toggle',
              id: 'notify-stuck',
              label: 'Notify on stuck / budget stop',
              value: true,
            },
          ],
        },
        {
          title: 'Sound',
          rows: [
            {
              kind: 'toggle',
              id: 'sound-cues',
              label: 'Sound cues',
              help: 'Distinct sounds for done, approval, stuck',
              value: false,
            },
            { kind: 'sound-test', id: 'sound-test', label: 'Test sound', button: 'Play' },
          ],
        },
      ],
    },
    backup: {
      title: 'Backup & restore',
      desc: 'Encrypted local archives. API keys never leave the keychain.',
      autoBackup: {
        kind: 'toggle',
        id: 'auto-backup',
        label: 'Auto-backup',
        help: 'Weekly, keep last 4',
        value: true,
      },
      recentTitle: 'Recent backups',
      create: 'Create backup now',
      restoreFromFile: 'Restore from file',
      restore: 'Restore',
      createToast: 'Creating encrypted backup…',
      restoreFromFileToast: 'Select an archive to restore',
      restoreToast: 'Restore would start here',
      /** The two archives the prototype lists. */
      archives: [
        {
          name: 'sdc-backup-2026-09-19.tar.age',
          meta: '2.4 MB · encrypted · 12 minutes ago',
          fresh: true,
        },
        {
          name: 'sdc-backup-2026-09-12.tar.age',
          meta: '2.3 MB · encrypted · 1 week ago',
          fresh: false,
        },
      ],
    },
    about: {
      title: 'About',
      desc: 'Version and diagnostic information.',
      rows: [
        { label: 'SDC App', value: 'v0.4.4' },
        { label: 'sdcd daemon', value: 'v0.4.4' },
        { label: 'SDCP protocol', value: '0.1' },
      ],
      /** `This host` is filled in from the event log's `HostStatus`, not hardcoded. */
      hostRow: 'This host',
      diagnostics: 'Diagnostics',
      runDoctor: 'Run doctor',
      createBundle: 'Create bundle',
      checkUpdates: 'Check updates',
      bundleToast: 'Diagnostic bundle created',
      updatesToast: 'Checking for updates…',
      telemetry: 'Telemetry',
      telemetryLabel: 'Send anonymous usage data',
      telemetryHelp: 'No prompts, no code, no file paths. Default off.',
    },
    /** The Keymap tab's Editor group - the only group that is not read from the registry. */
    keymapEditor: {
      title: 'Editor',
      rows: [
        {
          kind: 'select',
          id: 'editor-mode',
          label: 'Editor mode',
          options: ['Default', 'Vim', 'Emacs'],
          value: 'Default',
        },
      ],
    },
    close: 'Close',
  },
  /**
   * The seed data of the event log (spec section 3.3: "Seed the reducer with the same demo data
   * used in the HTML prototype"). These are literals rather than events because they are what the
   * log's first fold produces, not something that happened at runtime.
   */
  seed: {
    providers: [
      { id: 'claude', name: 'Claude', kind: 'subscription', logo: 'claude', initial: 'C', detail: 'Claude Pro / Max subscription · uses your own login', status: 'connected', account: 'Max — user@mehedi.dev' },
      { id: 'openai', name: 'OpenAI', kind: 'subscription', logo: 'openai', initial: 'O', detail: 'ChatGPT Plus · Codex CLI subscription', status: 'connected', account: 'Plus' },
      { id: 'gemini', name: 'Gemini', kind: 'subscription', logo: 'gemini', initial: 'G', detail: 'Google AI · Gemini CLI', status: 'needs-auth', account: null },
      { id: 'anthropic-api', name: 'Anthropic API', kind: 'api-key', logo: 'claude', initial: 'A', detail: 'Direct API key · pay per token', status: 'connected', account: 'sk-ant-…4f8a' },
      { id: 'openai-api', name: 'OpenAI API', kind: 'api-key', logo: 'openai', initial: 'O', detail: 'Direct API key · $12.40 / $50.00 this month', status: 'connected', account: 'sk-…8b2c' },
      { id: 'deepseek', name: 'DeepSeek', kind: 'api-key', logo: 'deepseek', initial: 'D', detail: 'Direct API · cheap, fast', status: 'available', account: null },
      { id: 'groq', name: 'Groq', kind: 'api-key', logo: 'groq', initial: 'G', detail: 'Ultra-fast inference', status: 'available', account: null },
      { id: 'openrouter', name: 'OpenRouter', kind: 'api-key', logo: 'openrouter', initial: 'O', detail: 'One key · 200+ models', status: 'available', account: null },
      { id: 'ollama', name: 'Ollama', kind: 'local', logo: 'ollama', initial: 'O', detail: 'Local models · auto-detected on this machine', status: 'needs-auth', account: null },
    ],
    /** The registry of spec section 9.10: the prototype's twelve models, verbatim. */
    models: [
      { id: 'anthropic/claude-sonnet-4-5', provider: 'anthropic-api', tier: 'balanced', ctx: 200000, cost: '$3 / $15', enabled: true },
      { id: 'anthropic/claude-opus-4', provider: 'anthropic-api', tier: 'deep', ctx: 200000, cost: '$15 / $75', enabled: true },
      { id: 'anthropic/claude-haiku-4', provider: 'anthropic-api', tier: 'fast', ctx: 200000, cost: '$0.80 / $4', enabled: true },
      { id: 'openai/gpt-5', provider: 'openai-api', tier: 'deep', ctx: 400000, cost: '$10 / $30', enabled: true },
      { id: 'openai/gpt-5-mini', provider: 'openai-api', tier: 'fast', ctx: 400000, cost: '$0.25 / $2', enabled: true },
      { id: 'google/gemini-2.5-pro', provider: 'gemini', tier: 'deep', ctx: 2000000, cost: '$1.25 / $5', enabled: true },
      { id: 'google/gemini-2.5-flash', provider: 'gemini', tier: 'fast', ctx: 1000000, cost: '$0.075 / $0.30', enabled: true },
      { id: 'deepseek/deepseek-chat', provider: 'deepseek', tier: 'balanced', ctx: 64000, cost: '$0.14 / $0.28', enabled: false },
      { id: 'groq/llama-3.3-70b', provider: 'groq', tier: 'balanced', ctx: 128000, cost: '$0.59 / $0.79', enabled: false },
      { id: 'openrouter/anthropic/claude-sonnet-4-5', provider: 'openrouter', tier: 'balanced', ctx: 200000, cost: '$3 / $15', enabled: true },
      { id: 'ollama/deepseek-coder:6.7b', provider: 'ollama', tier: 'balanced', ctx: 16000, cost: 'free', enabled: false, size: '3.8 GB' },
      { id: 'ollama/llama3.2:3b', provider: 'ollama', tier: 'fast', ctx: 128000, cost: 'free', enabled: false, size: '2.0 GB' },
    ],
    /** The ten environment checks of spec section 9.10, in the prototype's order. */
    doctor: [
      { id: 'node', label: 'Node.js', state: 'ok', detail: 'v20.11.0 · at /usr/local/bin/node', fix: null },
      { id: 'claude', label: 'Claude Code CLI', state: 'ok', detail: 'v2.1.3 · logged in as user@mehedi.dev', fix: null },
      { id: 'codex', label: 'Codex CLI', state: 'ok', detail: 'v1.4.0 · logged in', fix: null },
      { id: 'gemini', label: 'Gemini CLI', state: 'fail', detail: 'not installed', fix: 'Install' },
      { id: 'ollama', label: 'Ollama', state: 'ok', detail: 'running · 2 models', fix: null },
      { id: 'ripgrep', label: 'ripgrep', state: 'ok', detail: 'v14.1.0 · bundled', fix: null },
      { id: 'port3000', label: 'Port 3000', state: 'fail', detail: 'already in use by pid 8412', fix: 'Kill process' },
      { id: 'disk', label: 'Disk space', state: 'ok', detail: '28.4 GB free', fix: null },
      { id: 'git', label: 'Git', state: 'ok', detail: 'v2.43.0', fix: null },
      { id: 'ssh', label: 'SSH to prod-1', state: 'warn', detail: 'host key changed — needs re-pin', fix: 'Re-pin' },
    ],
    /** `ollama list`, as the Local flow reports it. */
    ollamaModels: ['llama3.2:3b', 'mistral:7b'],
    hostPlatform: 'macOS 15.1 · arm64',
    sdcdVersion: '0.4.4',
    /** Spec section 9.10's `Test connection` answer: twelve models behind a valid key. */
    testModelCount: 12,
    /** The day the bundled catalogue was last curated; the daemon reports the same thing. */
    modelSnapshot: '2026-09-20',
    /** The tip toast the prototype raises 1.4s after load. */
    tip: 'Tip: click the plug icon to connect Claude, OpenAI, Gemini, or Ollama',
    tipAction: 'Got it',
  },

  /**
   * Surfaces reachable from the chrome. Kept as a namespace so a future surface has a home; the
   * placeholder wording of the earlier steps is gone now that every one of the six is built.
   */
  overlays: {
    none: '',
  },
} as const;

export type Strings = typeof strings;
