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

  /** The frame every dialog shares (`src/modals/Modal.tsx`). */
  modal: {
    close: 'Close',
  },

  /** A labelled input and its own actions (`src/panels/ui/Field.tsx`). */
  field: {
    show: 'Show',
    hide: 'Hide',
    paste: 'Paste',
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
    /** `+ New chat` when the host already had an empty chat to land on (0.7.5). */
    reusedChatOn: (host: string): string => `Using the empty chat on ${host}`,
    actions: {
      rename: 'Rename',
      delete: 'Delete',
      fork: 'Fork this chat',
      newChatOnHost: 'New chat on this host',
      removeHost: 'Remove this host',
    },
    /** The fork lands on screen with its transcript, and its name says where it came from (0.7.8). */
    forked: (title: string, turns: number): string =>
      turns === 0
        ? `Forked “${title}”`
        : `Forked “${title}” · ${turns} turn${turns === 1 ? '' : 's'} came with it`,
    /** The confirmation before a host goes - it names what goes with it (spec section 9.12's other half). */
    removeHostConfirm: (name: string, sessions: number): string =>
      sessions === 0
        ? `Remove "${name}" from the host list?`
        : `Remove "${name}" and its ${sessions} chat${sessions === 1 ? '' : 's'}?`,
    hostRemoved: (name: string): string => `Removed ${name}`,
    hostRemovedWith: (name: string, sessions: number): string =>
      `Removed ${name} · ${sessions} chat${sessions === 1 ? '' : 's'} went with it`,
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
    /** Spec section 7.13's `No project` state: "Open a folder to get started" (v0.7.6). */
    noProject: {
      title: 'Open a folder to get started',
      description:
        'A chat works inside a folder: the engine runs there and the files it touches are the ones you see.',
      action: 'Open folder',
    },
    /** A session with no turns yet: what the pane says instead of drawing someone else's chat. */
    emptyPane: {
      title: 'Nothing here yet',
      body:
        'Type below and SDC sends it to the engine. What comes back - and anything that goes wrong - is written to this chat’s log.',
    },
    /** Separator between the host name and the title in `.pane-header` (spec section 9.15). */
    paneHeaderSeparator: '·',
  },

  timeline: {
    /** `Y` in the Time Machine: there is nothing to copy until a turn has finished. */
    nothingToCopy: 'No finished turn to summarise yet',
  },

  /** Turn stream - spec section 7.5. */
  turns: {
    /** The label above a user's message. The prototype's `You · 14:02` fixed the clock this app does
     *  not have on the turn: the log carries the prompt, not the minute it was typed. */
    who: 'You',
    collapsed: {
      /** `Turns 1-6 collapsed` - the count is the turns the window hides, not a demo number. */
      labelFor: (hidden: number): string => `Turns 1-${hidden} collapsed`,
      /** The line's right-hand side, which names the rule rather than a made-up cost. */
      windowMeta: (window: number): string => `last ${window} shown`,
      toast: 'Expanded the earlier turns',
    },
    thinking: {
      title: 'Thinking',
      duration: '',
    },
    /** The engine's answer block (0.7.3). The stream had every part of a turn except this one. */
    answer: {
      title: 'Answer',
      streaming: 'streaming…',
    },
    tools: {
      read: 'Read',
      edit: 'Edit',
      run: 'Run',
      /** The two words a run's output lines are prefixed with (spec section 7.5). */
      runPass: 'PASS',
      runFail: 'FAIL',
    },
    error: {
      fix: 'Fix this',
      showCode: 'Show code',
      explainMore: 'Explain more',
      showCodeToast: 'Opened the file',
      explainMoreToast: 'Expanded',
    },
    footer: {
      /** Before `TurnCompleted` arrives: the run has not reported what it used. */
      pending: 'Running · totals arrive with the last event',
      /** A turn that ended in `ErrorRaised` never reports totals, so the line says what happened. */
      failed: 'Failed',
      up: 'Good response',
      down: 'Bad response',
      upToast: 'Thanks',
      downToast: 'Noted',
    },
  },

  /** The file tree in the sidebar and the file the Preview shows (0.7.7). */
  files: {
    /** The section's own heading, and the `title` of its root row. */
    title: 'Files',
    /** The root row's tooltip: the whole path, since the label is only the folder's name. */
    rootTitle: (path: string): string => path,
    /** A chat with no folder has no tree: the way in is the empty state's `Open folder`. */
    noFolder: 'Open a folder to see its files',
    /** `Reading…` while `fs.list` is out. */
    loading: 'Reading…',
    /** An empty directory is a fact, not a failure. */
    empty: 'Nothing in this folder',
    /**
     * The guard's count, said out loud: `3 names hidden`. A tree that is quietly three rows short is
     * exactly the kind of half-truth this build keeps removing (principle P4), and the daemon already
     * sends the number.
     */
    hidden: (count: number): string => `${count} ${count === 1 ? 'name' : 'names'} hidden`,
    /** `2.4 KB · 128 lines` - what the Preview's header says about the open file. */
    fileMeta: (bytes: number, lines: number): string => `${sizeOf(bytes)} · ${lines} ${lines === 1 ? 'line' : 'lines'}`,
    /** The honest note on a file the daemon had to cut short. */
    truncated: (bytes: number): string => `First 1 MB of ${sizeOf(bytes)}`,
    /** The × on the Preview's file header. */
    close: 'Close this file',
    /** The tree's refresh, for a file an engine just wrote. */
    refresh: 'Refresh files',
    /** Edit / Save / Cancel in the file view (0.7.9). Saving takes a checkpoint first (P5). */
    edit: 'Edit this file',
    save: 'Save',
    saving: 'Saving…',
    cancel: 'Cancel',
    saved: (name: string): string => `Saved ${name} · a checkpoint was taken first`,
    saveFailed: 'Could not save that file',
    /** Shown beside the meta line while the draft differs from what is on disk. */
    unsaved: 'edited',
    /** The git header, and the diff behind its button (0.7.9). */
    gitClean: (branch: string): string => `${branch} · clean`,
    gitDirty: (branch: string, count: number): string =>
      `${branch} · ${count} changed`,
    diff: 'Diff',
    diffTitle: (branch: string): string => `Working tree · ${branch}`,
    diffEmpty: 'Nothing has changed in this folder.',
    diffClose: 'Close the diff',
    diffFailed: 'Could not read the diff',
    /** Failed reads, in the daemon's own words when it has them. */
    failed: 'Could not read that folder',
    openFailed: 'Could not open that file',
  },

  /** Folders - the directory a chat works in (spec section 7.13's `No project`, v0.7.6). */
  folder: {
    /** The chip in the prompt area: `No folder` when the chat has none. */
    none: 'No folder',
    /** Tooltip on that chip, and the menu item that re-points a chat. */
    change: 'Change folder',
    /** `Working in SDC` - the chip's label. The full path is the chip's tooltip. */
    workingIn: (name: string): string => `Working in ${name}`,
    /** `Opened SDC` - what the daemon answered, said back to the person. */
    opened: (name: string): string => `Opened ${name}`,
    /** The chat already existed and was pointed at the folder instead of a second chat being made. */
    pointed: (name: string, chat: string): string => `Pointed “${chat}” at ${name}`,
    changed: (name: string): string => `Now working in ${name}`,
    /** `Closed SDC · 2 chats kept their conversation` - a folder going away is not a chat going away. */
    closed: (name: string, chats: number): string =>
      chats === 0
        ? `Closed ${name}`
        : `Closed ${name} · ${chats} chat${chats === 1 ? '' : 's'} kept their conversation`,
    couldNotOpen: 'Could not open that folder',
    couldNotChange: 'Could not change this chat’s folder',
  },

  /** Prompt area and model dropdown - spec sections 7.6 and 9.3. */
  prompt: {
    placeholder: 'Ask or describe what you want to build…',
    /** `1 file` / `3 files` - drawn only when this prompt really has that many attachments. */
    filesChip: (count: number): string => `${count} ${count === 1 ? 'file' : 'files'}`,
    /** `12.4k ctx` - drawn only from a context count the daemon reported. */
    contextChip: (tokens: number): string => `${tokens >= 1000 ? `${(tokens / 1000).toFixed(1)}k` : String(tokens)} ctx`,
    queued: {
      remove: 'Remove queued prompt',
      /** Seeded so the queue is visible; the spec caps it at three (section 9.7). */
      seed: ['also add a test for this'],
    },
    toolbar: {
      attach: 'Attach a file',
      /** It opens a dialog now (`lib/picker.ts`), so it says so: a button that promises a paste and
          opens a file browser is the kind of small lie this build keeps removing. */
      image: 'Pick an image',
      file: '@ file',
      command: '/ command',
    },
    send: 'Send',
    /** The picker itself failed to open (a missing capability, a broken plugin) - not a cancel. */
    pickFailed: 'Could not open the file picker',
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
      /** The count beside the MODEL title: how many providers can actually run right now. */
      verifiedCount: (connected: number, total: number): string =>
        connected === 0 ? 'nothing connected yet' : `${connected} of ${total} connected`,
      verified: 'connected',
      notConnected: 'not connected',
      catalogEmpty: 'the daemon has not listed any yet',
      connect: (mode: 'login' | 'api', label: string): string =>
        mode === 'login' ? `Sign in to ${label}` : `Add a key for ${label}`,
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
      /** The empty frame: no URL is attached, so the panel says so rather than drawing a mock page. */
      noUrl: 'no URL attached',
      emptyTitle: 'Nothing is being previewed',
      emptyBody:
        'Attach a running dev server’s URL and this frame shows it. Until then there is no page to draw.',
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
      /** Shown instead of `empty` while the chat has no turn to race: a duel needs a prompt. */
      needsTurn: 'A duel races a prompt you have already sent. Send one in this chat first.',
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
    /** The build this window is running, and the daemon behind it. */
    version: (app: string, daemon: string): string =>
      daemon === '' ? `v${app}` : `v${app} · sdcd ${daemon}`,
    versionTitle: (app: string, daemon: string): string =>
      daemon === '' ? `SDC ${app} · the daemon has not reported its version` : `SDC ${app} · sdcd ${daemon}`,
    versionCopied: (app: string, daemon: string): string =>
      `Copied: SDC ${app}${daemon === '' ? '' : ` · sdcd ${daemon}`}`,
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
    /** Raised once, when the heartbeat stops answering. */
    lost: 'Lost the daemon · sdcd is not answering; the app is retrying every 5s',
    /** Raised once, when it answers again. */
    backOnline: 'sdcd is answering again',
    /** The banner above the chat while the heartbeat is failing. */
    banner: 'Not connected to the daemon (sdcd). Actions will fail until it answers.',
    /** The banner after several consecutive misses. */
    bannerStale: 'sdcd has not answered for 15 seconds. Restart the app to start it again.',
    retry: 'Retry now',
    /** The in-process stand-in a browser tab gets, and what it can honestly do (see `lib/standin.ts`). */
    browserHostName: 'Browser tab',
    browserSessionTitle: 'New chat',
    browserNote:
      'This is a browser tab: it has no daemon behind it, so it cannot run an engine, read the filesystem or hold a credential. Start the desktop app (which starts `sdcd` itself) and this becomes the real thing.',
    /** Beside a `bundled` model row: where the row came from, and what would make it live. */
    bundledNote: 'the catalogue bundled with this build · a provider’s own list needs sdcd',
    bundledSnapshot: 'bundled',
    /** The startup toast on a window that has not reached a daemon yet. */
    waiting: 'Waiting for the daemon · this window has no engine until sdcd answers',
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
    /**
     * The CLI recipe, shown *before* a sign-in starts (0.7.8).
     *
     * `claude`, `codex` and `gemini` are three separate programs, and Connect on a provider whose program is
     * missing used to launch the login and then report the failure - so the sentence a person needed ("install
     * `claude`") arrived as the explanation of something that had already gone wrong. The row says it first,
     * with the daemon's own install words (`cli.recipes`' `note`) and a copy button, and the Sign in button
     * stays where it is for a machine the doctor cannot see into.
     */
    recipe: {
      missing: (program: string): string => `\`${program}\` is not installed`,
      missingBody: 'Install it first - these are the words to run:',
      present: (program: string): string => `\`${program}\` is installed`,
      copy: 'Copy',
      recheck: 'Check again',
      recheckToast: 'Checked again',
    },
    loginTitle: 'Sign in with the CLI',
    loginBody:
      'This starts the CLI’s own sign-in. Approve the page in your browser, then paste the code it shows back here — the credential is written by the CLI, never by SDC.',
    signIn: 'Sign in',
    tryAgain: 'Try again',
    signedInToast: (label: string) => `${label} is signed in · the card now says connected`,
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
    verifiedBadge: 'connected',
    notConnectedBadge: 'not connected',
    close: 'Close',
    pageTitle: 'The page to open',
    note: 'SDC never sees the credential: the CLI writes it, in the CLI’s own store.',
    outputHint: 'last 12 lines',
    copyOutput: 'Copy output',
    keyTitle: 'Credential',
    keyLabel: 'API key',
    keyPlaceholder: 'sk-… (the key the provider gave you)',
    keySaved: 'saved',
    keyNotSaved: 'not saved yet',
    keyEditing: 'unsaved change',
    keyHint: (provider: string): string =>
      `Pasted here and sent straight to the OS keychain. ${provider} is called by the daemon with it; SDC never shows it again after saving.`,
    keyNote:
      'A key that is rejected is reported in the provider’s own words — press Refresh under MODELS after saving to see what it listed.',
    saveKey: 'Save key',
    refresh: 'Refresh',
    modelsHint: (shown: number, total: number, snapshot: string): string =>
      shown === total ? `${total} rows · bundled ${snapshot}` : `${shown} of ${total} rows`,
    modelFilter: 'Filter models…',
    modelNoMatch: 'No model matches that filter.',
    footer: {
      login: 'The credential is written by the CLI, never by SDC.',
      api: 'Save the key, then pick a model above.',
    },
    signedInNote:
      'The card now says connected, and a chat on this provider works from here. The credential is in the CLI’s own store — SDC never sees it.',
    pageLabel: 'The page the CLI opened',
    codeHint:
      'Copy the code that page shows — the code itself, not the link in the box above. Pasting the link is the one mistake that looks right and fails.',
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
    context: (ctx: number) => (ctx <= 0 ? 'context not listed' : `${Math.round(ctx / 1000)}K context`),
  },

  /** Toast stack - spec section 9.14. */
  toast: {
    dismiss: 'Dismiss',
    /** The × on every toast (0.7.5): a message with no action chip had no way to be closed by hand. */
    close: 'Close',
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
    /** Shown in the Local flow before any `host.doctor` run: the tab has nothing real to print yet. */
    localEmpty:
      'Nothing has been probed on this machine yet. Run the doctor (Settings → Environment) and the checks it returns appear here.',
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
    sshTargetHelp:
      'Paste what you would type in your own terminal — `ssh -p 8443 user@host` works, and so does `user@host`.',
    password: 'Password (optional)',
    passwordPlaceholder: 'Only used once, to copy SDC’s key',
    passwordHelp:
      'SDC copies its own key (`~/.ssh/sdc_ed25519`) to that machine with this password, then drops the password. Every connection after that is passwordless — and you can revoke the key by deleting one line from `authorized_keys`. A host that only accepts a verification code cannot be set up this way, and SDC will say so.',
    sshTargetPlaceholder: 'user@vps.example.com',
    labelField: 'Label (optional)',
    labelPlaceholder: 'prod-1',
    cancel: 'Cancel',
    connect: 'Connect',
    connecting: (label: string): string => `Connecting to ${label}…`,
    connected: (label: string): string => `Connected: ${label}`,
    localAlready: 'Local host already connected',
    needTarget: 'Enter an SSH target like user@host',
    /**
     * The same `user@host` is one host, so a second Connect says what happened rather than adding a
     * row that cannot be told apart from the first (see `host.add` in the daemon).
     */
    alreadyThere: (label: string): string => `${label} is already in the host list`,
    /** `host.add` answered: the row exists. Whether it can be *reached* is the daemon's next sentence. */
    added: (label: string): string => `Added ${label} · checking it can be reached…`,
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
      /**
       * The three version rows: **labels only**, because the values are read from the things they
       * describe. They used to be literals in this table, and on 0.7.5 they still said `v0.4.4` - a row
       * that cannot go stale is a row that is not typed by hand. `AboutTab` fills them from
       * `package.json` (the app), the event log's `HostStatus` (the daemon) and `protocol/types.ts`
       * (the protocol), which is where `StatusBar`'s version cell reads two of them as well.
       */
      rows: [
        { id: 'app', label: 'SDC App' },
        { id: 'daemon', label: 'sdcd daemon' },
        { id: 'protocol', label: 'SDCP protocol' },
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

  overlays: {
    none: '',
  },
} as const;

export type Strings = typeof strings;

/**
 * `2.4 KB` - a file size a person can read.
 *
 * Used by the file tree's file rows and the Preview's header, and it rounds to one decimal because
 * the question it answers is "is this big?" rather than "how many bytes exactly" - `truncated` in the
 * Preview says the exact size when the answer matters.
 */
export function sizeOf(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }

  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }

  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
