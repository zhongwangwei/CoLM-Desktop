import { hasBackend, invoke } from './ipc';
import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';
export const cn = (...inputs: ClassValue[]) => twMerge(clsx(inputs));
export const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);
export async function download(name: string, text: string, type = 'text/plain') {
  if (hasBackend()) {
    const path = await invoke<string | null>('export_text_file', {
      suggestedName: name,
      content: text,
      mimeType: type,
    });
    return {
      status: path === null ? ('cancelled' as const) : ('saved' as const),
      filename: name,
      mime_type: type,
      path,
    };
  }
  const url = URL.createObjectURL(new Blob([text], { type }));
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  try {
    link.click();
  } finally {
    setTimeout(() => URL.revokeObjectURL(url), 0);
  }
  return { status: 'download_requested' as const, filename: name, mime_type: type };
}

export async function waitForContract(
  ready: () => boolean,
  wait: () => Promise<void>,
  timeoutMs = 10000,
) {
  const until = Date.now() + timeoutMs;
  while (!ready()) {
    if (Date.now() >= until) throw new Error('The page did not become ready; read ui_state.');
    await wait();
  }
}

export function pageExecutionContext(
  contracts: { kernelDir?: string | null; preprocessors?: string }[],
) {
  const selected = contracts.find((contract) => contract.kernelDir !== undefined);
  return { kernelDir: selected?.kernelDir ?? null, preprocessors: selected?.preprocessors ?? null };
}

// The assistant receives public outcomes; the page retains the complete original for display/export.
export function publicUiResult(value: unknown, depth = 0): unknown {
  if (depth > 12) return { omitted: 'nested output exceeds the UI context limit' };
  if (typeof value === 'string') {
    try {
      const parsed: unknown = JSON.parse(value);
      if (parsed && typeof parsed === 'object')
        return JSON.stringify(publicUiResult(parsed, depth + 1));
    } catch {
      /* Ordinary logs are not JSON. */
    }
    const text = value.replace(
      /\b(password|api[_-]?key|access[_-]?token|authorization)\s*[:=]\s*(?:Bearer\s+\S+|"[^"]*"|'[^']*'|\S+)/gi,
      '$1=[redacted]',
    );
    return text.length > 16000
      ? `${text.slice(0, 16000)}\n[truncated ${text.length - 16000} characters]`
      : text;
  }
  if (Array.isArray(value)) {
    const items = value.slice(0, 1000).map((item) => publicUiResult(item, depth + 1));
    return value.length > 1000 ? { items, omitted_items: value.length - 1000 } : items;
  }
  if (value && typeof value === 'object')
    return Object.fromEntries(
      Object.entries(value)
        .filter(
          ([key]) =>
            ![
              'password',
              'passwd',
              'apikey',
              'token',
              'accesstoken',
              'refreshtoken',
              'authorization',
              'cookie',
              'credentials',
              'privatekey',
              'sshauth',
            ].includes(key.toLowerCase().replace(/[^a-z0-9]/g, '')),
        )
        .map(([key, item]) => [key, publicUiResult(item, depth + 1)]),
    );
  return value;
}

export function assistantPanelWidth(requested: number, viewport: number) {
  const sidebar = viewport <= 1240 ? 208 : 232;
  const max = Math.max(280, Math.min(600, viewport - sidebar - 320));
  return Math.round(Math.max(280, Math.min(max, Number.isFinite(requested) ? requested : 320)));
}
