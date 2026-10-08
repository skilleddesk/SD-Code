// The Files tab's state: which computer, which project, which folder, which file, what is picked for a chat.
//
// Everything here is ids the daemon gave us. This code never builds, joins or normalises a path: that is how a
// page cannot be talked into asking for `../../etc/passwd`, and why the same code works for Windows, macOS and Linux.

import { RequestFailed, type Link } from '../transport/link';

export interface Entry {
  name: string;
  path_id: string | null;
  dir: boolean;
  size: number;
  modified: number | null;
  protected: boolean;
  link: boolean;
  outside: boolean;
}

export interface Crumb {
  name: string;
  path_id: string;
}

export interface Folder {
  path_id: string;
  path: string;
  breadcrumbs: Crumb[];
  entries: Entry[];
  total: number;
  hidden: number;
  next_cursor: string | null;
  loading: boolean;
}

export interface HostInfo {
  host: string;
  name: string;
  type: string;
  status: string;
  roots: Array<{ name: string; path: string; path_id: string }>;
}

export interface OpenFile {
  path_id: string;
  name: string;
  size: number;
  text: string;
  binary: boolean;
  image: { mime: string; data: string } | null;
  next_offset: number | null;
  sha256: string | null;
  loading: boolean;
  error: string | null;
}

export interface SearchState {
  query: string;
  busy: boolean;
  names: Array<{ name: string; path_id: string; rel: string; dir: boolean }>;
  hits: Array<{ path_id: string; rel: string; line: number; text: string | null; protected: boolean }>;
  error: string | null;
}

export interface GitState {
  branch: string;
  dirty: number;
  files: Array<{ status: string; rel: string; path_id: string | null; hidden: boolean }>;
}

export interface WorkspaceState {
  hosts: HostInfo[];
  loaded: boolean;
  host: string | null;
  rootId: string | null;
  folder: Folder | null;
  file: OpenFile | null;
  search: SearchState | null;
  git: GitState | null;
  /** Picked for the next chat message: id and the name to show. */
  picked: Array<{ path_id: string; name: string }>;
  error: string | null;
}

export const emptyWorkspace = (): WorkspaceState => ({
  hosts: [],
  loaded: false,
  host: null,
  rootId: null,
  folder: null,
  file: null,
  search: null,
  git: null,
  picked: [],
  error: null,
});

const message = (error: unknown): string => (error instanceof RequestFailed ? error.message : (error as Error).message);

export class Workspace {
  state: WorkspaceState = emptyWorkspace();
  private searchTimer: ReturnType<typeof setTimeout> | null = null;
  private searchToken = 0;

  constructor(
    private readonly link: () => Link | null,
    private readonly changed: () => void,
  ) {}

  private set(patch: Partial<WorkspaceState>): void {
    this.state = { ...this.state, ...patch };
    this.changed();
  }

  reset(): void {
    this.state = emptyWorkspace();
    this.changed();
  }

  private requireLink(): Link {
    const link = this.link();

    if (!link) throw new Error('not connected');

    return link;
  }

  async loadHosts(): Promise<void> {
    try {
      const { hosts } = (await this.requireLink().rpc('hosts.list')) as { hosts: HostInfo[] };
      const current = hosts.find((host) => host.host === this.state.host) ?? hosts.find((host) => host.roots.length > 0) ?? hosts[0] ?? null;

      this.set({ hosts, loaded: true, host: current?.host ?? null, error: null });

      // The ids of a new session are new ones: an open folder has to be asked for again.
      if (current && !hosts.some((host) => host.roots.some((root) => root.path_id === this.state.rootId))) {
        this.set({ rootId: null, folder: null, file: null, search: null, git: null });
      }

      if (current && current.roots.length > 0 && !this.state.rootId) await this.openRoot(current.roots[0]!.path_id);
    } catch (error) {
      this.set({ error: message(error), loaded: true });
    }
  }

  chooseHost(host: string): void {
    const info = this.state.hosts.find((entry) => entry.host === host);

    this.set({ host, rootId: null, folder: null, file: null, search: null, git: null });

    if (info && info.roots.length > 0) void this.openRoot(info.roots[0]!.path_id);
  }

  async openRoot(pathId: string): Promise<void> {
    this.set({ rootId: pathId, file: null, search: null });
    await this.openFolder(pathId);
    void this.loadGit(pathId);
  }

  async openFolder(pathId: string): Promise<void> {
    this.set({ file: null, search: null, folder: { ...(this.state.folder ?? blankFolder(pathId)), path_id: pathId, loading: true } });

    try {
      const page = await this.requireLink().rpc('fs.list', { path_id: pathId });

      this.set({ folder: { ...page, loading: false }, error: null });
    } catch (error) {
      this.set({ folder: null, error: message(error) });
    }
  }

  async more(): Promise<void> {
    const folder = this.state.folder;

    if (!folder?.next_cursor || folder.loading) return;

    this.set({ folder: { ...folder, loading: true } });

    try {
      const page = await this.requireLink().rpc('fs.list', { path_id: folder.path_id, cursor: folder.next_cursor });

      this.set({ folder: { ...page, entries: [...folder.entries, ...page.entries], loading: false } });
    } catch (error) {
      this.set({ folder: { ...folder, loading: false }, error: message(error) });
    }
  }

  async loadGit(pathId: string): Promise<void> {
    try {
      this.set({ git: await this.requireLink().rpc('fs.git', { path_id: pathId }) });
    } catch {
      this.set({ git: null });
    }
  }

  // --- a file ---------------------------------------------------------------------------------------------------

  async openFile(pathId: string, name: string): Promise<void> {
    this.set({ file: { path_id: pathId, name, size: 0, text: '', binary: false, image: null, next_offset: null, sha256: null, loading: true, error: null } });

    try {
      const read = await this.requireLink().readFile(pathId, 0);

      this.set({ file: { path_id: pathId, name, size: read.size, text: read.text ?? '', binary: !!read.binary, image: read.image ?? null, next_offset: read.next_offset ?? null, sha256: read.sha256 ?? null, loading: false, error: null } });
    } catch (error) {
      this.set({ file: { path_id: pathId, name, size: 0, text: '', binary: false, image: null, next_offset: null, sha256: null, loading: false, error: message(error) } });
    }
  }

  async moreOfFile(): Promise<void> {
    const file = this.state.file;

    if (!file || file.next_offset === null || file.loading) return;

    this.set({ file: { ...file, loading: true } });

    try {
      const read = await this.requireLink().readFile(file.path_id, file.next_offset);

      this.set({ file: { ...file, text: file.text + (read.text ?? ''), next_offset: read.next_offset ?? null, loading: false } });
    } catch (error) {
      this.set({ file: { ...file, loading: false, error: message(error) } });
    }
  }

  closeFile(): void {
    this.set({ file: null });
  }

  // --- search ----------------------------------------------------------------------------------------------------------

  setQuery(query: string): void {
    if (this.searchTimer) clearTimeout(this.searchTimer);

    if (query.trim().length < 2) {
      this.searchToken++;
      this.set({ search: query.length ? { query, busy: false, names: [], hits: [], error: null } : null });

      return;
    }

    this.set({ search: { query, busy: true, names: [], hits: [], error: null } });
    this.searchTimer = setTimeout(() => void this.runSearch(query), 350);
  }

  private async runSearch(query: string): Promise<void> {
    const token = ++this.searchToken;
    const start = this.state.folder?.path_id ?? this.state.rootId;

    if (!start) return;

    try {
      // Names come back first (cheap), then lines: the page shows what it has as soon as it has it.
      const names = await this.requireLink().rpc('fs.search', { path_id: start, query, mode: 'name' });

      if (token !== this.searchToken) return;

      this.set({ search: { query, busy: true, names: names.names, hits: [], error: null } });

      const content = await this.requireLink().rpc('fs.search', { path_id: start, query, mode: 'content' });

      if (token !== this.searchToken) return;

      this.set({ search: { query, busy: false, names: names.names, hits: content.hits, error: null } });
    } catch (error) {
      if (token === this.searchToken) this.set({ search: { query, busy: false, names: [], hits: [], error: message(error) } });
    }
  }

  // --- picking files for a chat ---------------------------------------------------------------------------------------------

  toggle(pathId: string, name: string): void {
    const picked = this.state.picked.some((item) => item.path_id === pathId) ? this.state.picked.filter((item) => item.path_id !== pathId) : [...this.state.picked, { path_id: pathId, name }];

    this.set({ picked: picked.slice(0, 25) });
  }

  clearPicked(): void {
    this.set({ picked: [] });
  }
}

function blankFolder(pathId: string): Folder {
  return { path_id: pathId, path: '', breadcrumbs: [], entries: [], total: 0, hidden: 0, next_cursor: null, loading: true };
}
