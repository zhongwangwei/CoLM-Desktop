import test from 'node:test';
import assert from 'node:assert/strict';
import {
  EMPTY_PREP_FORM,
  clearPrepDraft,
  prepDraftHasInput,
  readPrepDraft,
  writePrepDraft,
} from './preparation-draft';

function fakeStorage() {
  const data = new Map<string, string>();
  return {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => void data.set(key, value),
    removeItem: (key: string) => void data.delete(key),
    data,
  };
}

test('an unfinished preparation form survives a reload and abandons cleanly', () => {
  const storage = fakeStorage();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: storage,
  });
  try {
    assert.equal(readPrepDraft(), null);
    const form = { ...EMPTY_PREP_FORM, src: '/in/observations.csv', lon: '113.5' };
    writePrepDraft({ mode: 'observation', form });
    const restored = readPrepDraft();
    assert.equal(restored?.mode, 'observation');
    assert.equal(restored?.form.src, '/in/observations.csv');
    assert.equal(restored?.form.lon, '113.5');
    assert.equal(restored?.form.short_gap, '3', 'defaults that were not saved stay at defaults');
    assert.equal(prepDraftHasInput(restored!), true);
    // 纯默认表单不落盘：刷新后不应显示"有未生成输入"。
    writePrepDraft({ mode: 'netcdf', form: { ...EMPTY_PREP_FORM } });
    assert.equal(storage.data.size, 0);
    writePrepDraft({ mode: 'observation', form });
    clearPrepDraft();
    assert.equal(readPrepDraft(), null);
  } finally {
    delete (globalThis as { localStorage?: unknown }).localStorage;
  }
});

test('corrupt or foreign drafts are ignored instead of restored', () => {
  const storage = fakeStorage();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: storage,
  });
  try {
    for (const raw of [
      '{',
      'null',
      '[]',
      JSON.stringify({ mode: 'site' }),
      JSON.stringify({ mode: 'site', form: 'bad' }),
    ]) {
      storage.data.set('colm-prep-draft-v1', raw);
      assert.equal(readPrepDraft(), null, raw);
    }
    storage.data.set(
      'colm-prep-draft-v1',
      JSON.stringify({ mode: 'invented', form: { src: '/x', extra: 'drop' } }),
    );
    const restored = readPrepDraft();
    assert.equal(restored?.mode, 'netcdf', 'unknown modes fall back to the default task');
    assert.equal(restored?.form.src, '/x');
    assert.equal('extra' in (restored?.form ?? {}), false, 'unknown fields are dropped');
  } finally {
    delete (globalThis as { localStorage?: unknown }).localStorage;
  }
});
