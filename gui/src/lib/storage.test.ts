import test from 'node:test';
import assert from 'node:assert/strict';
import { readStored, writeStored } from './storage';

test('a refusing storage reads as empty and never throws', () => {
  const refuse = () => {
    throw new Error('SecurityError');
  };
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: { getItem: refuse, setItem: refuse, removeItem: refuse },
  });
  assert.equal(readStored('colm-ui-language'), '');
  assert.doesNotThrow(() => writeStored('colm-ui-language', 'en'));
  assert.doesNotThrow(() => writeStored('colm-ui-language', ''));
});

test('a working storage keeps and removes values', () => {
  const data = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => data.get(key) ?? null,
      setItem: (key: string, value: string) => data.set(key, value),
      removeItem: (key: string) => data.delete(key),
    },
  });
  writeStored('k', 'v');
  assert.equal(readStored('k'), 'v');
  writeStored('k', '');
  assert.equal(readStored('k'), '');
});
