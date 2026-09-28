/**
 * The words of 0.13's surfaces, in English - `strings.agent`: the composer's `/` commands and `@` files,
 * attached images, the context meter, the agent's questions, the memory editor, background processes,
 * and the erase-everything switch. The language packs in `locales/` translate them like any other part.
 */
export const agentStrings = {
  commands: {
    title: 'Commands',
    empty: 'No command matches',
    hint: '↑↓ choose · ↵ run · Esc close',
    fromProject: 'project',
    help: 'Commands: /compact frees the model’s context · /init writes the project’s rules · /review checks the changes · /remember <fact> · /memory · /clear starts a new chat',
    remembered: (path: string): string => `Remembered · ${path}`,
    rememberEmpty: 'Write the fact after /remember',
    rememberFailed: 'Could not save it to memory',
    noFolder: 'This chat has no folder yet - open one first',
    compacting: 'Compacting: the model is summarising this chat…',
    initPrompt:
      'Study this project and write a file .sdc/rules.md for any coding agent that works on it next. Read the manifests, the configs, the README and the main source folders first. The file must say, concisely: what the project is; how to install, build, test, lint and run it (the exact commands); the layout of the code and where the important parts live; the conventions to follow (style, naming, patterns, libraries used for what); and anything that is easy to get wrong here. Keep it under 120 lines. If .sdc/rules.md, CLAUDE.md or AGENTS.md already exist, improve on them instead of starting over.',
    reviewPrompt:
      'Review the uncommitted changes in this project (git diff, and new files) as a careful senior engineer. Look for bugs, broken edge cases, security problems (injection, secrets, unsafe input), performance traps, and code that does not match the project’s style. For each finding give the file and line, why it is a problem, and the fix. Say plainly when the change looks good. Do not change any file.',
    agentNeeded: 'Writing the rules file needs Agent mode for an API model - switch Chat → Agent, or use a CLI engine',
  },
  mention: {
    title: 'Files',
    empty: 'No file matches',
    searching: 'Searching…',
  },
  images: {
    attached: (count: number): string => `${count} ${count === 1 ? 'image' : 'images'}`,
    remove: 'Remove image',
    tooBig: 'That image is larger than 10 MB',
    pasted: 'Image attached',
  },
  context: {
    chip: (percent: number): string => `ctx ${percent}%`,
    title: (used: string, window: string, compacted: boolean, resumed: boolean): string =>
      `The next turn sends about ${used} of the model's ${window} tokens.` +
      (compacted ? ' Older turns are folded into a summary.' : '') +
      (resumed ? ' The CLI continues its own conversation, so it remembers more than this.' : '') +
      ' /compact frees room.',
    full: 'The context is getting full - /compact summarises the chat and frees room',
  },
  question: {
    label: 'The agent asks',
    placeholder: 'Or write your own answer…',
    send: 'Answer',
    skip: 'Let it decide',
    failed: 'The answer did not reach the agent',
  },
  memory: {
    title: 'Memory',
    subtitle: 'What SDC keeps in mind in every chat. The agent adds to it with “remember”; you can edit it here.',
    project: 'This project (.sdc/memory.md)',
    global: 'Everywhere (all projects)',
    noProject: 'This chat has no folder, so only the global memory can be edited.',
    save: 'Save',
    saved: 'Memory saved',
    loading: 'Reading…',
    placeholder: '- Use pnpm, never npm\n- The live site is example.com; staging is staging.example.com',
  },
  processes: {
    chip: (count: number): string => `${count} running`,
    title: 'Background processes',
    stop: 'Stop',
    stopped: 'Stopped',
    none: 'Nothing is running in the background.',
  },
  alerts: {
    done: (chat: string): string => `Done · ${chat}`,
    doneBody: 'The turn finished.',
    needsYou: (chat: string): string => `SDC needs you · ${chat}`,
    problem: (chat: string): string => `Stopped · ${chat}`,
    stuck: 'The turn looks stuck.',
    budget: 'The turn hit its budget and was stopped.',
  },
  settings: {
    title: 'Agent',
    desc: 'How the agent finishes its work, and what SDC remembers.',
    autoCheck: 'Check the work before it finishes',
    autoCheckHelp: 'When an agent says it is done and has changed files, SDC runs the project’s own checks (typecheck, lint, tests, build). If one fails, the agent reads why and fixes it first.',
    memory: 'Memory',
    memoryHelp: 'Facts every chat is given: this project’s, and your own for all projects.',
    openMemory: 'Edit memory',
    erase: 'Erase all SDC data',
    eraseHelp: 'Deletes every chat, host, project, setting, checkpoint and memory SDC keeps on this computer, and the API keys it stored. Your project files are not touched. SDC restarts empty.',
    eraseConfirm: 'Type ERASE to delete everything SDC keeps on this computer. This cannot be undone.',
    eraseButton: 'Erase everything',
    erasing: 'Erasing… SDC restarts in a moment',
    eraseFailed: 'SDC could not erase its data',
  },
};
