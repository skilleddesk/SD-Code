import type { ProjectView, TurnView } from '../../store/types';

/**
 * Live preview (0.12.5): where the page is, and when it changed - both read from the chat's own log.
 *
 * A dev server says its address when it starts (`Local: http://localhost:5173/`); a Run card carries that
 * line, and so can the agent's words. A site on a VPS is its domain. Nothing here opens a connection: it
 * only reads what the turns already said.
 *
 * 0.14.4 - **the page, not just the site.** The report: the preview sat on `localhost:3000` while the
 * agent built a new page on the live VPS site, and nothing followed it. So every finished edit is mapped
 * to the page it serves (`src/app/(public)/pricing/page.tsx` → `/pricing`, `about.php` → `/about.php`,
 * `pages/blog/index.vue` → `/blog`), the site's domain is read from the edited file's own path as well
 * as the project's (`/var/www/example.com/…` → `example.com`), and the preview follows the newest page.
 */

const LOCAL_URL = /\bhttps?:\/\/(?:localhost|127\.0\.0\.1|0\.0\.0\.0|\[::1\])(?::\d{2,5})?(?:\/[^\s'"<>)\]`]*)?/gi;
const DOMAIN = /^(?:[a-z0-9-]+\.)+[a-z]{2,}$/i;
/** A path segment that looks like a domain but is a file: `index.html`, `package.json`. */
const FILE_LIKE = /\.(?:html?|php|jsx?|tsx?|mjs|cjs|json|md|mdx|txt|css|scss|vue|svelte|astro|py|rb|go|rs|lock|ya?ml|xml|env|log|bak|conf|ini|sh|sql|zip|gz|map|ico|png|jpe?g|svg|webp|gif)$/i;

/** A web root inside a project folder: what comes after it is what the web server serves. */
const WEB_ROOTS = new Set(['public_html', 'public', 'htdocs', 'www', 'html', 'web', 'dist', 'build', 'out', '_site']);
/** Folders whose PHP/HTML files are pieces of a page, not pages. */
const NOT_PAGES = new Set([
  'includes', 'include', 'inc', 'partials', 'partial', 'components', 'component', 'layouts', 'layout',
  'templates', 'template-parts', 'lib', 'config', 'vendor', 'node_modules', 'wp-admin', 'wp-includes',
  'plugins', 'api', 'assets', 'css', 'js', 'scripts', 'styles', 'tests', 'test', '.next', 'storage', 'cache',
]);

/** The page a finished edit is about, and the site it is on. */
export interface PageChange {
  /** `turnId/callId` - changes when there is a newer edit. */
  key: string;
  /** The edited file, as the tool named it. */
  file: string;
  /** The path it serves (`/pricing`), or `null` for a file that is not a page (a stylesheet, a component). */
  page: string | null;
  /** The domain its path names (`/var/www/example.com/…`), when it names one. */
  domain: string | null;
}

/** The first path segment that is a domain (`/var/www/example.com/public_html/x` → `example.com`). */
export function domainIn(path: string): string | null {
  for (const segment of path.replace(/\\/g, '/').split('/')) {
    const candidate = segment.toLowerCase();

    if (DOMAIN.test(candidate) && !FILE_LIKE.test(candidate) && !candidate.startsWith('.')) {
      return candidate.replace(/^www\./, '');
    }
  }

  return null;
}

/** Route segments up to the first dynamic one (`[slug]`), without Next's `(group)` and `@slot` folders. */
function routeOf(segments: readonly string[]): string {
  const kept: string[] = [];

  for (const segment of segments) {
    if (/^\(.*\)$/.test(segment) || segment.startsWith('@')) {
      continue;
    }

    if (segment.startsWith('[') || segment.startsWith(':')) {
      break;
    }

    kept.push(segment);
  }

  return `/${kept.join('/')}`;
}

/**
 * The URL path a file serves, or `null` when it is not a page by itself.
 *
 * The rules are the frameworks' own: Next's `app/…/page.tsx` and `pages/…`, SvelteKit's `+page.svelte`,
 * Nuxt / Astro `pages/`, WordPress's `page-{slug}.php`, and a plain site's `.html` / `.php` files under its
 * web root. A component, a stylesheet or a config file answers `null`: the page on screen stays, and it
 * reloads.
 */
export function pageForFile(file: string, root = ''): string | null {
  const path = file.replace(/\\/g, '/');
  const base = root.replace(/\\/g, '/').replace(/\/+$/, '');
  /* Inside the project, the path from its root; outside it (an absolute path elsewhere), the whole path. */
  const relative = base !== '' && path.toLowerCase().startsWith(`${base.toLowerCase()}/`);
  const parts = (relative ? path.slice(base.length + 1) : path).split('/').filter((part) => part !== '' && part !== '.');
  const name = parts[parts.length - 1] ?? '';
  const dirs = parts.slice(0, -1);
  const lower = dirs.map((part) => part.toLowerCase());

  /* Next.js App Router: `app/…/page.tsx`. */
  if (/^page\.(?:tsx|jsx|ts|js|mdx)$/.test(name)) {
    const app = lower.lastIndexOf('app');

    if (app !== -1) {
      return routeOf(dirs.slice(app + 1));
    }
  }

  /* SvelteKit: `src/routes/…/+page.svelte`. */
  if (/^\+page(?:\.server)?\.(?:svelte|ts|js)$/.test(name)) {
    const routes = lower.lastIndexOf('routes');

    if (routes !== -1) {
      return routeOf(dirs.slice(routes + 1));
    }
  }

  /* Next Pages Router, Nuxt, Astro: `pages/…`. */
  const pages = lower.lastIndexOf('pages');

  if (pages !== -1 && /\.(?:tsx|jsx|ts|js|vue|astro|md|mdx|svelte)$/.test(name)) {
    const inside = dirs.slice(pages + 1);

    if (inside[0]?.toLowerCase() === 'api' || name.startsWith('_')) {
      return null;
    }

    const stem = name.replace(/\.[^.]+$/, '');

    return routeOf(stem === 'index' ? inside : [...inside, stem]);
  }

  /* WordPress theme templates that name their page. */
  const themes = lower.lastIndexOf('themes');

  if (themes !== -1 && name.endsWith('.php')) {
    const slug = /^page-([a-z0-9-]+)\.php$/i.exec(name)?.[1];

    if (slug !== undefined) {
      return `/${slug}/`;
    }

    return /^(?:front-page|home|index)\.php$/.test(name) ? '/' : null;
  }

  /* A plain site: an .html / .php file, served from the web root it sits under. */
  if (/\.(?:html?|php)$/i.test(name) && !name.startsWith('_') && !name.includes('.blade.')) {
    let webRoot = -1;

    lower.forEach((part, index) => {
      if (WEB_ROOTS.has(part)) {
        webRoot = index;
      }
    });

    const site = dirs.findIndex((part) => DOMAIN.test(part) && !FILE_LIKE.test(part));
    /* Served from: the web root it is under, else the domain folder, else the project root. A file
       outside all three is not a page anyone can name. */
    const inside = webRoot !== -1 ? dirs.slice(webRoot + 1) : site !== -1 ? dirs.slice(site + 1) : relative ? dirs : null;

    if (inside === null || inside.some((part) => NOT_PAGES.has(part.toLowerCase()))) {
      return null;
    }

    const folder = inside.length === 0 ? '/' : `/${inside.join('/')}/`;

    return /^index\.(?:html?|php)$/i.test(name) ? folder : `${folder}${name}`;
  }

  return null;
}

/** The newest finished edits in a chat, newest first, each with the page it serves. */
export function pageChanges(turns: readonly TurnView[], sessionId: string, root = ''): PageChange[] {
  const changes: PageChange[] = [];

  for (const turn of [...turns].reverse()) {
    if (turn.sessionId !== sessionId) {
      continue;
    }

    for (const tool of [...turn.tools].reverse()) {
      if (tool.tool === 'edit' && tool.status === 'done' && tool.target !== '') {
        changes.push({
          key: `${turn.id}/${tool.callId}`,
          file: tool.target,
          page: pageForFile(tool.target, root),
          domain: domainIn(tool.target),
        });
      }
    }
  }

  return changes;
}

/** The addresses worth previewing for a chat, newest first: dev servers it started, then its site. */
export function previewCandidates(turns: readonly TurnView[], sessionId: string, project: ProjectView | undefined): string[] {
  const found: string[] = [];
  const add = (raw: string): void => {
    /* `0.0.0.0` is where a server listens, and `127.0.0.1` is `localhost`: one chip per server. */
    const url = raw.replace(/:\/\/(?:0\.0\.0\.0|127\.0\.0\.1|\[::1\])/, '://localhost').replace(/[.,;:*_~]+$/, '');

    if (!found.includes(url)) {
      found.push(url);
    }
  };

  for (const turn of [...turns].reverse()) {
    if (turn.sessionId !== sessionId) {
      continue;
    }

    for (const tool of [...turn.tools].reverse()) {
      for (const line of [...tool.output].reverse()) {
        for (const match of line.text.matchAll(LOCAL_URL)) {
          add(match[0]);
        }
      }
    }

    for (const match of turn.text.matchAll(LOCAL_URL)) {
      add(match[0]);
    }
  }

  /* The site the chat's edits are on (0.14.4), then the project's own domain. */
  for (const change of pageChanges(turns, sessionId, project?.root)) {
    if (change.domain !== null) {
      add(`https://${change.domain}/`);
    }
  }

  const projectDomain = project === undefined ? null : (DOMAIN.test(project.name) ? project.name : domainIn(project.root));

  if (projectDomain !== null) {
    add(`https://${projectDomain.replace(/^www\./, '')}/`);
  }

  return found.slice(0, 5);
}

/** A local address's port (`http://localhost:3000/x` → 3000), or `null` for anything else. */
export function localPort(url: string): number | null {
  const match = /^https?:\/\/(?:localhost|127\.0\.0\.1|\[::1\])(?::(\d{2,5}))?/i.exec(url);

  if (match === null) {
    return null;
  }

  return match[1] === undefined ? 80 : Number(match[1]);
}

/** `base` + `page`, with one slash between them. */
export function joinPage(base: string, page: string): string {
  try {
    return new URL(page, base.endsWith('/') ? base : `${base}/`).toString();
  } catch {
    return base;
  }
}

/** The newest finished change in a chat - what a live preview reloads after: its stamp and its file. */
export function lastChange(turns: readonly TurnView[], sessionId: string): { key: string; target: string } | null {
  const newest = pageChanges(turns, sessionId)[0];

  return newest === undefined ? null : { key: newest.key, target: newest.file };
}
