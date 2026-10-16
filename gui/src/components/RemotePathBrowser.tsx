import { useEffect, useState } from 'react';
import { Button, Dialog, Field, Input, Notice } from './ui';
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
    setLoading(true);
    setRequest({ path });
  };
  return (
    <Dialog
      open
      title={en ? 'Browse server files' : '浏览服务器文件'}
      description={`${host} · ${directory ? (en ? 'Select a folder' : '选择目录') : en ? 'Select a file' : '选择文件'}`}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          browse(address.trim() || null);
        }}
      >
        <Field label={en ? 'Server path' : '服务器路径'}>
          <Input
            disabled={creating}
            value={address}
            onChange={(event) => setAddress(event.target.value)}
            placeholder={en ? 'Home directory' : '用户主目录'}
          />
        </Field>
        <div className="actions">
          <Button type="submit" disabled={creating}>
            {en ? 'Go' : '前往'}
          </Button>
          <Button disabled={!listing?.parent || loading} onClick={() => browse(listing!.parent)}>
            {en ? 'Up' : '上一级'}
          </Button>
          <Button disabled={creating} onClick={() => browse(null)}>
            {en ? 'Home' : '主目录'}
          </Button>
          <Button
            disabled={!listing || loading || newFolder !== null}
            onClick={() => setNewFolder('')}
          >
            {en ? 'New folder' : '新建文件夹'}
          </Button>
        </div>
      </form>
      {newFolder !== null && listing && (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (loading || !newFolder.trim()) return;
            setLoading(true);
            setRequest({ path: listing.path, name: newFolder.trim() });
          }}
        >
          <Field label={en ? 'New folder name' : '新文件夹名称'}>
            <Input
              autoFocus
              disabled={creating}
              placeholder={en ? 'Folder name' : '文件夹名称'}
              value={newFolder}
              onChange={(event) => setNewFolder(event.target.value)}
            />
          </Field>
          <div className="actions">
            <Button type="submit" disabled={loading || !newFolder.trim()}>
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
          </div>
        </form>
      )}
      {error && <Notice tone="error">{error}</Notice>}
      {loading && <p role="status">{en ? 'Loading…' : '正在加载…'}</p>}
      {listing && !loading && (
        <>
          <p className="path">{listing.path}</p>
          <div style={{ maxHeight: '45vh', overflowY: 'auto' }}>
            <ul aria-label={en ? 'Server entries' : '服务器文件列表'}>
              {listing.entries
                .filter((entry) => !directory || entry.directory)
                .map((entry) => (
                  <li key={entry.path}>
                    <Button
                      variant="ghost"
                      onClick={() => (entry.directory ? browse(entry.path) : onSelect(entry.path))}
                    >
                      {entry.directory ? '▸ ' : ''}
                      {entry.name}
                      {entry.directory ? '/' : ''}
                    </Button>
                  </li>
                ))}
            </ul>
            {!listing.entries.some((entry) => !directory || entry.directory) && (
              <p className="muted">{en ? 'No entries' : '目录为空'}</p>
            )}
          </div>
          {!!listing.skipped && (
            <Notice tone="warning">
              {en
                ? `${listing.skipped} names could not be displayed.`
                : `${listing.skipped} 个文件名无法显示。`}
            </Notice>
          )}
          {listing.truncated && (
            <Notice tone="warning">
              {en
                ? 'This directory has more entries. Enter a more specific path to browse them.'
                : '目录内容较多，未全部显示；请输入更具体的路径继续浏览。'}
            </Notice>
          )}
          {directory && (
            <Button variant="primary" onClick={() => onSelect(listing.path)}>
              {en ? 'Select this folder' : '选择此目录'}
            </Button>
          )}
        </>
      )}
    </Dialog>
  );
}
