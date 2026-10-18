import { useEffect, useState } from 'react';
import { Button, Dialog, Input, Notice } from './ui';
import { invoke } from '../lib/ipc';
import type { Server } from '../lib/types';

type Listing = {
  path: string;
  parent: string | null;
  entries: { name: string; path: string; directory: boolean }[];
  truncated?: boolean;
  skipped?: number;
};
type Props = {
  host: string;
  server?: Server;
  initialPath?: string;
  directory: boolean;
  en: boolean;
  onSelect: (path: string) => void;
  onClose: () => void;
};

export default function RemotePathBrowser(props: Props) {
  return (
    <Browser
      key={`${props.host}\n${props.initialPath ?? ''}\n${JSON.stringify(props.server ?? null)}`}
      {...props}
    />
  );
}

function Browser({ host, server, initialPath, directory, en, onSelect, onClose }: Props) {
  const [address, setAddress] = useState(initialPath ?? '');
  const [request, setRequest] = useState<{ path: string | null; name?: string }>({
    path: initialPath || null,
  });
  const [newFolder, setNewFolder] = useState<string | null>(null);
  const [result, setResult] = useState<{ request: typeof request; listing: Listing } | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState('');
  const listing =
    result &&
    (result.request === request ||
      (request.name !== undefined && result.listing.path === request.path))
      ? result.listing
      : null;
  const creating = loading && request.name !== undefined;
  useEffect(() => {
    let alive = true;
    setLoading(true);
    setError('');
    const failed = (reason: unknown) => {
      if (!alive) return;
      setError(String(reason));
      setLoading(false);
    };
    const args = {
      host,
      path: request.path,
      ...(server ? { server } : {}),
    };
    if (request.name !== undefined) {
      void invoke<{ path: string }>('remote_mkdir', { ...args, name: request.name }).then(
        (value) => {
          if (!alive) return;
          setNewFolder(null);
          setAddress(value.path);
          setRequest({ path: value.path });
        },
        failed,
      );
    } else {
      void invoke<Listing>('remote_browse', args).then((value) => {
        if (!alive) return;
        setResult({ request, listing: value });
        setAddress(value.path);
        setLoading(false);
      }, failed);
    }
    return () => {
      alive = false;
    };
  }, [host, server, request]);
  const browse = (path: string | null) => {
    setError('');
    setNewFolder(null);
    setFilter('');
    setLoading(true);
    setRequest({ path });
  };
  const query = filter.trim().toLowerCase();
  const entries = (listing?.entries ?? [])
    .filter((entry) => !directory || entry.directory)
    .sort((a, b) => Number(b.directory) - Number(a.directory) || a.name.localeCompare(b.name));
  const shown = query
    ? entries.filter((entry) => entry.name.toLowerCase().includes(query))
    : entries;
  const folders = entries.filter((entry) => entry.directory).length;
  const crumbs = listing ? breadcrumbs(listing.path) : [];
  return (
    <Dialog
      open
      className="remote-browser"
      showClose={false}
      title={en ? 'Browse server files' : '浏览服务器文件'}
      description={`${host} · ${directory ? (en ? 'Select a folder' : '选择目录') : en ? 'Select a file' : '选择文件'}`}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <form
        className="remote-browser-bar"
        onSubmit={(event) => {
          event.preventDefault();
          browse(address.trim() || null);
        }}
      >
        <Input
          className="remote-browser-address"
          aria-label={en ? 'Server path' : '服务器路径'}
          disabled={creating}
          value={address}
          onChange={(event) => setAddress(event.target.value)}
          placeholder={en ? 'Home directory' : '用户主目录'}
        />
        <Button type="submit" disabled={creating}>
          {en ? 'Go' : '前往'}
        </Button>
        <Button disabled={!listing?.parent || loading} onClick={() => browse(listing!.parent)}>
          {en ? 'Up' : '上一级'}
        </Button>
        <Button disabled={creating} onClick={() => browse(null)}>
          {en ? 'Home' : '主目录'}
        </Button>
      </form>
      {crumbs.length > 0 && (
        <nav className="remote-browser-crumbs" aria-label={en ? 'Path' : '路径'}>
          {crumbs.map((crumb, index) =>
            index === crumbs.length - 1 ? (
              <span key={crumb.path} className="remote-browser-crumb current">
                {crumb.name}
              </span>
            ) : (
              <span key={crumb.path} className="remote-browser-crumb-item">
                <button
                  type="button"
                  className="remote-browser-crumb"
                  disabled={loading}
                  onClick={() => browse(crumb.path)}
                >
                  {crumb.name}
                </button>
                <span className="remote-browser-separator" aria-hidden="true">
                  ›
                </span>
              </span>
            ),
          )}
        </nav>
      )}
      {listing && !loading && entries.length > 12 && (
        <Input
          type="search"
          className="remote-browser-filter"
          aria-label={en ? 'Filter this folder' : '筛选当前目录'}
          placeholder={en ? 'Filter this folder' : '筛选当前目录'}
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
        />
      )}
      {error && <Notice tone="error">{error}</Notice>}
      <div className="remote-browser-list" aria-label={en ? 'Server entries' : '服务器文件列表'}>
        {loading && (
          <p role="status" className="remote-browser-empty">
            {en ? 'Loading…' : '正在加载…'}
          </p>
        )}
        {listing &&
          !loading &&
          shown.map((entry) => (
            <button
              type="button"
              key={entry.path}
              className={`remote-browser-entry ${entry.directory ? 'folder' : 'file'}`}
              title={entry.path}
              onClick={() => (entry.directory ? browse(entry.path) : onSelect(entry.path))}
            >
              <EntryIcon directory={entry.directory} />
              <span className="remote-browser-name">{entry.name}</span>
            </button>
          ))}
        {listing && !loading && shown.length === 0 && (
          <p className="remote-browser-empty muted">
            {entries.length
              ? en
                ? 'No name matches the filter'
                : '没有匹配的名称'
              : en
                ? 'No entries'
                : '目录为空'}
          </p>
        )}
      </div>
      {listing && !loading && !!listing.skipped && (
        <Notice tone="warning">
          {en
            ? `${listing.skipped} names could not be displayed.`
            : `${listing.skipped} 个文件名无法显示。`}
        </Notice>
      )}
      {listing && !loading && listing.truncated && (
        <Notice tone="warning">
          {en
            ? 'This directory has more entries. Enter a more specific path to browse them.'
            : '目录内容较多，未全部显示；请输入更具体的路径继续浏览。'}
        </Notice>
      )}
      {newFolder !== null && listing && (
        <form
          className="remote-browser-new-folder"
          onSubmit={(event) => {
            event.preventDefault();
            if (loading || !newFolder.trim()) return;
            setLoading(true);
            setRequest({ path: listing.path, name: newFolder.trim() });
          }}
        >
          <Input
            autoFocus
            aria-label={en ? 'New folder name' : '新文件夹名称'}
            disabled={creating}
            placeholder={en ? 'Folder name' : '文件夹名称'}
            value={newFolder}
            onChange={(event) => setNewFolder(event.target.value)}
          />
          <Button type="submit" variant="primary" disabled={loading || !newFolder.trim()}>
            {en ? 'Create folder' : '创建'}
          </Button>
          <Button
            disabled={creating}
            onClick={() => {
              setNewFolder(null);
              setError('');
            }}
          >
            {en ? 'Cancel' : '取消'}
          </Button>
        </form>
      )}
      <div className="remote-browser-footer">
        <Button
          disabled={!listing || loading || newFolder !== null}
          onClick={() => setNewFolder('')}
        >
          {en ? 'New folder' : '新建文件夹'}
        </Button>
        {listing && !loading && (
          <span className="remote-browser-count">
            {directory
              ? en
                ? `${folders} folders`
                : `${folders} 个文件夹`
              : en
                ? `${folders} folders · ${entries.length - folders} files`
                : `${folders} 个文件夹 · ${entries.length - folders} 个文件`}
          </span>
        )}
        <span className="remote-browser-spacer" />
        <Button variant="ghost" onClick={onClose}>
          {en ? 'Close' : '关闭'}
        </Button>
        {directory && listing && !loading && (
          <Button variant="primary" onClick={() => onSelect(listing.path)}>
            {en ? 'Select this folder' : '选择此目录'}
          </Button>
        )}
      </div>
    </Dialog>
  );
}

/** `/a/b` → `/`, `a`, `b`, each with the path it opens. */
function breadcrumbs(path: string) {
  if (!path.startsWith('/')) return [{ name: path, path }];
  const parts = path.split('/').filter(Boolean);
  return [
    { name: '/', path: '/' },
    ...parts.map((name, index) => ({ name, path: `/${parts.slice(0, index + 1).join('/')}` })),
  ];
}

function EntryIcon({ directory }: { directory: boolean }) {
  return (
    <svg className="remote-browser-icon" viewBox="0 0 20 20" aria-hidden="true">
      {directory ? (
        <path d="M2.5 5.5a1.5 1.5 0 0 1 1.5-1.5h3.6l1.8 2H16a1.5 1.5 0 0 1 1.5 1.5v7A1.5 1.5 0 0 1 16 16H4a1.5 1.5 0 0 1-1.5-1.5z" />
      ) : (
        <path d="M5.5 2.5h6l3 3v11a1 1 0 0 1-1 1h-8a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1zM11.5 2.5v3h3" />
      )}
    </svg>
  );
}
