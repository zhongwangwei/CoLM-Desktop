import { useEffect, useState } from 'react';
import { Button, Dialog, Field, Input, Notice } from './ui';
import { invoke } from '../lib/ipc';

type Listing = {
  path: string;
  parent: string | null;
  entries: { name: string; path: string; directory: boolean }[];
  truncated?: boolean;
  skipped?: number;
};
type Props = {
  host: string;
  initialPath?: string;
  directory: boolean;
  en: boolean;
  onSelect: (path: string) => void;
  onClose: () => void;
};

export default function RemotePathBrowser(props: Props) {
  return <Browser key={`${props.host}\n${props.initialPath ?? ''}`} {...props} />;
}

function Browser({ host, initialPath, directory, en, onSelect, onClose }: Props) {
  const [address, setAddress] = useState(initialPath ?? '');
  const [request, setRequest] = useState({ path: initialPath || null });
  const [result, setResult] = useState<{ request: typeof request; listing: Listing } | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const listing = result?.request === request ? result.listing : null;
  useEffect(() => {
    let alive = true;
    setLoading(true);
    setError('');
    void invoke<Listing>('remote_browse', { host, path: request.path }).then(
      (value) => {
        if (!alive) return;
        setResult({ request, listing: value });
        setAddress(value.path);
        setLoading(false);
      },
      (reason: unknown) => {
        if (!alive) return;
        setError(String(reason));
        setLoading(false);
      },
    );
    return () => {
      alive = false;
    };
  }, [host, request]);
  const browse = (path: string | null) => {
    setError('');
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
            value={address}
            onChange={(event) => setAddress(event.target.value)}
            placeholder={en ? 'Home directory' : '用户主目录'}
          />
        </Field>
        <div className="actions">
          <Button type="submit">{en ? 'Go' : '前往'}</Button>
          <Button disabled={!listing?.parent || loading} onClick={() => browse(listing!.parent)}>
            {en ? 'Up' : '上一级'}
          </Button>
          <Button onClick={() => browse(null)}>{en ? 'Home' : '主目录'}</Button>
        </div>
      </form>
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
