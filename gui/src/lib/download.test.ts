import test from 'node:test';
import assert from 'node:assert/strict';
import { download } from './utils';
import { download as resultDownload } from '../features/results/series';
import { download as historyDownload } from '../features/history/history';

function globals(windowValue: unknown, documentValue: unknown) {
  const previous = new Map(
    ['window', 'document'].map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)]),
  );
  Object.defineProperty(globalThis, 'window', { value: windowValue, configurable: true });
  Object.defineProperty(globalThis, 'document', { value: documentValue, configurable: true });
  return () => {
    for (const [key, descriptor] of previous) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
  };
}
test('all desktop text exports await the native chooser and never enter WebKit downloads', async () => {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  let finish!: (path: string | null) => void;
  const restore = globals(
    {
      __TAURI__: {
        core: {
          invoke: (command: string, args: Record<string, unknown>) => {
            calls.push({ command, args });
            return new Promise<string | null>((resolve) => {
              finish = resolve;
            });
          },
        },
      },
    },
    { createElement: () => assert.fail('desktop export must not create an anchor') },
  );
  const objectUrl = URL.createObjectURL;
  URL.createObjectURL = () => assert.fail('desktop export must not create a Blob URL');
  try {
    for (const save of [
      () => download('full.csv', 'a,b\n1,2\n', 'text/csv;charset=utf-8'),
      () => resultDownload('full.csv', 'a,b\n1,2\n', 'text/csv;charset=utf-8'),
      () => historyDownload('a,b\n1,2\n', 'full.csv', 'text/csv;charset=utf-8'),
    ]) {
      let completed = false;
      const pending = save().then((result) => {
        completed = true;
        return result;
      });
      await Promise.resolve();
      assert.equal(completed, false);
      assert.deepEqual(calls.at(-1), {
        command: 'export_text_file',
        args: {
          suggestedName: 'full.csv',
          content: 'a,b\n1,2\n',
          mimeType: 'text/csv;charset=utf-8',
        },
      });
      finish('/chosen/full.csv');
      assert.deepEqual(await pending, {
        status: 'saved',
        filename: 'full.csv',
        mime_type: 'text/csv;charset=utf-8',
        path: '/chosen/full.csv',
      });
    }
    const cancelled = resultDownload('sample.json', '{}');
    finish(null);
    assert.equal((await cancelled).status, 'cancelled');
  } finally {
    URL.createObjectURL = objectUrl;
    restore();
  }
});
test('native write errors propagate without silently falling back to browser download', async () => {
  const restore = globals(
    {
      __TAURI__: {
        core: {
          invoke: async () => {
            throw new Error('write failed');
          },
        },
      },
    },
    { createElement: () => assert.fail('no browser fallback') },
  );
  try {
    await assert.rejects(download('report.json', '{}'), /write failed/);
  } finally {
    restore();
  }
});
test('browser export still downloads the full UTF-8 content and releases its object URL', async () => {
  let blob: Blob | undefined,
    clicks = 0;
  const revoked: string[] = [];
  const anchor = {
    href: '',
    download: '',
    click: () => {
      clicks++;
    },
  };
  const restore = globals(
    {},
    {
      createElement: (tag: string) => {
        assert.equal(tag, 'a');
        return anchor;
      },
    },
  );
  const make = URL.createObjectURL,
    revoke = URL.revokeObjectURL;
  URL.createObjectURL = (value) => {
    assert.ok(value instanceof Blob);
    blob = value;
    return 'blob:full';
  };
  URL.revokeObjectURL = (url) => {
    revoked.push(url);
  };
  const content =
    'time,value\n' + Array.from({ length: 10000 }, (_, i) => `${i},观测${i}`).join('\n');
  try {
    assert.equal(
      (await resultDownload('complete.csv', content, 'text/csv;charset=utf-8')).status,
      'download_requested',
    );
    assert.equal(await blob!.text(), content);
    assert.equal(clicks, 1);
    assert.equal(anchor.download, 'complete.csv');
    await new Promise((resolve) => setTimeout(resolve, 5));
    assert.deepEqual(revoked, ['blob:full']);
  } finally {
    URL.createObjectURL = make;
    URL.revokeObjectURL = revoke;
    restore();
  }
});
