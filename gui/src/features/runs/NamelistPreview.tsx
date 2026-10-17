import { useEffect, useState } from 'react';
import { Advanced, Field, Notice, Select } from '../../components/ui';
import { invoke } from '../../lib/ipc';

type Preview = { files: { name: string; text: string }[]; warnings: string[] };

export default function NamelistPreview({
  casePaths,
  disabled,
  en,
}: {
  casePaths: string[];
  disabled: boolean;
  en: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState(casePaths[0] ?? '');
  const dir = casePaths.includes(selected) ? selected : (casePaths[0] ?? '');
  const [result, setResult] = useState<{ dir: string; data?: Preview; error?: string } | null>(
    null,
  );
  useEffect(() => {
    setResult(null);
    if (!open || disabled || !dir) return;
    let cancelled = false;
    void invoke<Preview>('case_namelist_preview', { dir }).then(
      (data) => {
        if (!cancelled) setResult({ dir, data });
      },
      (error: unknown) => {
        if (!cancelled) setResult({ dir, error: String(error) });
      },
    );
    return () => {
      cancelled = true;
    };
  }, [open, disabled, dir]);
  const current = result?.dir === dir ? result : null;
  return (
    <Advanced open={open} onOpenChange={setOpen} title={en ? 'NML preview' : 'NML 配置预览'}>
      {disabled ? (
        <Notice tone="warning">
          {en
            ? 'Save and load the case settings before previewing the generated files.'
            : '请先保存并成功读取算例设置，再预览生成的配置文件。'}
        </Notice>
      ) : (
        <>
          <p className="muted">
            {en
              ? 'Read-only preview of the saved case and its referenced forcing configuration.'
              : '只读预览已保存的算例配置及其引用的强迫场配置。'}
          </p>
          {casePaths.length > 1 && (
            <Field label={en ? 'Station' : '站点'}>
              <Select value={dir} onChange={(event) => setSelected(event.target.value)}>
                {casePaths.map((path) => (
                  <option key={path} value={path}>
                    {path.split(/[\\/]/).filter(Boolean).at(-1) ?? path}
                  </option>
                ))}
              </Select>
            </Field>
          )}
          {!current && <p role="status">{en ? 'Loading…' : '正在读取…'}</p>}
          {current?.error && <Notice tone="error">{current.error}</Notice>}
          {current?.data?.warnings.map((warning, index) => (
            <Notice key={index} tone="warning">
              {warning}
            </Notice>
          ))}
          {current?.data?.files.map((file) => (
            <section key={file.name} aria-label={file.name}>
              <h4>{file.name}</h4>
              <pre className="run-log" tabIndex={0} aria-label={file.name}>
                {file.text}
              </pre>
            </section>
          ))}
        </>
      )}
    </Advanced>
  );
}
