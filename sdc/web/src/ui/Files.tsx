import { lazy, Suspense, useState } from 'react';
import { useModel, useSnapshot, useT } from './hooks';
import type { Entry } from '../state/workspace';

const Code = lazy(() => import('./Code'));

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(bytes < 10 * 1024 ? 1 : 0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;

  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
}

export function Files() {
  const t = useT();
  const { workspace: ws, link } = useSnapshot();
  const model = useModel();

  if (link.kind === 'locked') return <p className="muted empty">{t('unlockHelp')}</p>;
  if (!ws.loaded) return <p className="muted empty">{t('loading')}</p>;
  if (ws.file) return <FileView />;

  const host = ws.hosts.find((entry) => entry.host === ws.host);

  if (!host || host.roots.length === 0) {
    return (
      <div>
        <HostPicker />
        <p className="muted empty">{t('filesNoProjects')}</p>
        {ws.error && <p role="alert" className="error">{ws.error}</p>}
      </div>
    );
  }

  return (
    <div className="files">
      <HostPicker />
      <label htmlFor="file-search" className="sr-only">
        {t('filesSearch')}
      </label>
      <input id="file-search" type="search" placeholder={t('filesSearch')} value={ws.search?.query ?? ''} onChange={(event) => model.workspace.setQuery(event.target.value)} autoComplete="off" />
      {ws.search ? <SearchResults /> : <Folder />}
      {ws.picked.length > 0 && (
        <div className="picked" role="status">
          <span>{t('picked', { n: ws.picked.length })}</span>
          <button onClick={() => model.workspace.clearPicked()}>{t('pickedClear')}</button>
        </div>
      )}
    </div>
  );
}

function HostPicker() {
  const t = useT();
  const { workspace: ws } = useSnapshot();
  const model = useModel();
  const host = ws.hosts.find((entry) => entry.host === ws.host);

  return (
    <div className="pickers">
      {ws.hosts.length > 1 && (
        <label>
          <span className="small muted">{t('filesHost')}</span>
          <select value={ws.host ?? ''} onChange={(event) => model.workspace.chooseHost(event.target.value)}>
            {ws.hosts.map((entry) => (
              <option key={entry.host} value={entry.host}>
                {entry.type === 'vps' ? '🖥 ' : '💻 '}
                {entry.name} ({entry.status})
              </option>
            ))}
          </select>
        </label>
      )}
      {host && host.roots.length > 1 && (
        <label>
          <span className="small muted">{t('filesProject')}</span>
          <select value={ws.rootId ?? ''} onChange={(event) => void model.workspace.openRoot(event.target.value)}>
            {host.roots.map((root) => (
              <option key={root.path_id} value={root.path_id}>
                {root.name}
              </option>
            ))}
          </select>
        </label>
      )}
    </div>
  );
}

function Folder() {
  const t = useT();
  const { workspace: ws } = useSnapshot();
  const model = useModel();
  const folder = ws.folder;

  if (!folder) return <p className="muted empty">{ws.error ?? t('loading')}</p>;

  const parent = folder.breadcrumbs.length > 1 ? folder.breadcrumbs[folder.breadcrumbs.length - 2] : null;

  return (
    <div>
      <nav aria-label="Path" className="crumbs">
        {folder.breadcrumbs.map((crumb, index) => (
          <span key={crumb.path_id}>
            {index > 0 && <span aria-hidden="true"> / </span>}
            <button className="link" onClick={() => void model.workspace.openFolder(crumb.path_id)} aria-current={index === folder.breadcrumbs.length - 1 ? 'location' : undefined}>
              {crumb.name}
            </button>
          </span>
        ))}
      </nav>
      {ws.git && ws.git.dirty > 0 && (
        <p className="small muted">
          {ws.git.branch ? `⎇ ${ws.git.branch} · ` : ''}
          {t('gitChanged', { n: ws.git.dirty })}
        </p>
      )}
      {parent && (
        <button className="link" onClick={() => void model.workspace.openFolder(parent.path_id)}>
          ↩ {t('filesUp')}
        </button>
      )}
      {folder.entries.length === 0 && <p className="muted empty">{t('filesEmpty')}</p>}
      <ul className="entries">
        {folder.entries.map((entry) => (
          <Row key={entry.path_id ?? entry.name} entry={entry} />
        ))}
      </ul>
      {folder.hidden > 0 && <p className="small muted">{t('filesHidden', { n: folder.hidden })}</p>}
      {folder.next_cursor && (
        <button disabled={folder.loading} onClick={() => void model.workspace.more()}>
          {folder.loading ? t('loading') : t('filesMore')}
        </button>
      )}
      {ws.error && <p role="alert" className="error">{ws.error}</p>}
    </div>
  );
}

function Row({ entry }: { entry: Entry }) {
  const t = useT();
  const { workspace: ws } = useSnapshot();
  const model = useModel();
  const picked = ws.picked.some((item) => item.path_id === entry.path_id);
  const glyph = entry.outside ? '⤴' : entry.protected ? '🔒' : entry.dir ? '📁' : entry.link ? '🔗' : '📄';

  return (
    <li className="entry">
      {!entry.dir && entry.path_id && !entry.protected && (
        <input type="checkbox" checked={picked} aria-label={`${t('pick')}: ${entry.name}`} onChange={() => model.workspace.toggle(entry.path_id!, entry.name)} />
      )}
      <button
        className="entry-main"
        disabled={!entry.path_id}
        onClick={() => (entry.dir ? void model.workspace.openFolder(entry.path_id!) : void model.workspace.openFile(entry.path_id!, entry.name))}
      >
        <span aria-hidden="true">{glyph}</span> <span className="name">{entry.name}</span>
        {entry.protected && <span className="small muted"> · {t('filesProtected')}</span>}
        {entry.outside && <span className="small muted"> · {t('filesOutside')}</span>}
        {!entry.dir && <span className="small muted size">{formatSize(entry.size)}</span>}
      </button>
    </li>
  );
}

function SearchResults() {
  const t = useT();
  const { workspace: ws } = useSnapshot();
  const model = useModel();
  const search = ws.search!;

  return (
    <div aria-live="polite">
      {search.busy && <p className="muted">{t('filesSearching')}</p>}
      {!search.busy && search.names.length === 0 && search.hits.length === 0 && search.query.trim().length >= 2 && <p className="muted">{t('filesNoResults')}</p>}
      {search.error && <p role="alert" className="error">{search.error}</p>}
      {search.names.length > 0 && (
        <>
          <h3>{t('filesNames')}</h3>
          <ul className="entries">
            {search.names.map((item) => (
              <li key={item.path_id} className="entry">
                <button className="entry-main" onClick={() => (item.dir ? void model.workspace.openFolder(item.path_id) : void model.workspace.openFile(item.path_id, item.name))}>
                  <span aria-hidden="true">{item.dir ? '📁' : '📄'}</span> <span className="name">{item.rel}</span>
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
      {search.hits.length > 0 && (
        <>
          <h3>{t('filesLines')}</h3>
          <ul className="entries">
            {search.hits.map((hit, index) => (
              <li key={`${hit.path_id}:${hit.line}:${index}`} className="entry">
                <button className="entry-main" onClick={() => void model.workspace.openFile(hit.path_id, hit.rel.split('/').pop() ?? hit.rel)}>
                  <span className="name">{hit.rel}:{hit.line}</span>
                  {hit.text !== null ? <span className="mono small block">{hit.text}</span> : <span className="small muted"> · {t('filesProtected')}</span>}
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

function FileView() {
  const t = useT();
  const { workspace: ws } = useSnapshot();
  const model = useModel();
  const file = ws.file!;
  const [copied] = useState(false);

  void copied;

  return (
    <div className="file">
      <button className="link" onClick={() => model.workspace.closeFile()}>
        ← {t('filesBack')}
      </button>
      <h2 className="mono">{file.name}</h2>
      <p className="small muted">{formatSize(file.size)}</p>
      {file.loading && file.text === '' && <p className="muted">{t('loading')}</p>}
      {file.error && <p role="alert" className="error">{file.error}</p>}
      {file.image && <img className="preview" alt={file.name} src={`data:${file.image.mime};base64,${file.image.data}`} />}
      {file.binary && !file.image && <p className="muted">{t('fileBinary', { size: formatSize(file.size) })}</p>}
      {!file.binary && file.text !== '' && (
        <Suspense fallback={<p className="muted">{t('loading')}</p>}>
          <Code text={file.text} name={file.name} />
        </Suspense>
      )}
      {file.next_offset !== null && (
        <button disabled={file.loading} onClick={() => void model.workspace.moreOfFile()}>
          {file.loading ? t('loading') : t('fileLoadMore')}
        </button>
      )}
      {!file.binary && (
        <button
          onClick={() => {
            if (!ws.picked.some((item) => item.path_id === file.path_id)) model.workspace.toggle(file.path_id, file.name);
          }}
          disabled={ws.picked.some((item) => item.path_id === file.path_id)}
        >
          {t('pick')}
        </button>
      )}
    </div>
  );
}
