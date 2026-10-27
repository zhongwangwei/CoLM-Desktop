import { it } from 'node:test';
import assert from 'node:assert/strict';
import {
  clearParameterContextMarker,
  parameterContextKey,
  sameParameterContextIdentity,
  shouldStartParameterContextLoad,
  type ParameterContextIdentity,
} from './parameter-context-cache';

const identity = (overrides: Partial<ParameterContextIdentity> = {}): ParameterContextIdentity => ({
  version: { id: 'official', revision: 1 },
  kernel: '/kernels/default',
  sites: ['/p/site1'],
  spatial: false,
  types: [],
  ...overrides,
});

const start = (
  value: ParameterContextIdentity,
  extra: {
    loadedFor?: string;
    failedFor?: string;
    typeChoicesReady?: boolean;
    busy?: boolean;
    step?: number;
  } = {},
) =>
  shouldStartParameterContextLoad({
    step: 2,
    identity: value,
    loadedFor: '',
    failedFor: '',
    typeChoicesReady: true,
    busy: false,
    ...extra,
  });

it('the parameter cache identity carries the model id and revision', () => {
  const base = identity();
  assert.equal(parameterContextKey(identity()), parameterContextKey(base));
  // 同一算例、同一编译配置换了模型修订：身份必须不同，否则仍会命中旧参数（GUI-14）。
  assert.notEqual(
    parameterContextKey(identity({ version: { id: 'official', revision: 2 } })),
    parameterContextKey(base),
  );
  assert.notEqual(
    parameterContextKey(identity({ version: { id: 'modified', revision: 1 } })),
    parameterContextKey(base),
  );
});

it('the parameter cache identity covers every dimension that changes the read result', () => {
  const base = identity();
  const changed = [
    identity({ kernel: '/kernels/grid' }),
    identity({ sites: ['/p/site2'] }),
    identity({ sites: ['/p/site1', '/p/site2'] }),
    // 站点顺序影响后端参数行顺序（已勾参数按行号对应），所以顺序也算身份。
    identity({ sites: ['/p/site2', '/p/site1'] }),
    identity({ sites: ['/p/b', '/p/a'] }),
    identity({ spatial: true, types: [1] }),
  ];
  for (const other of changed)
    assert.notEqual(parameterContextKey(other), parameterContextKey(base));
  // 格点研究里类型影响参数行；单点按算例自己的地类定，残留的类型选择不应换键。
  assert.notEqual(
    parameterContextKey(identity({ spatial: true, types: [1] })),
    parameterContextKey(identity({ spatial: true, types: [2] })),
  );
  assert.equal(
    parameterContextKey(identity({ types: [1] })),
    parameterContextKey(identity({ types: [2] })),
  );
  assert.notEqual(
    parameterContextKey(identity({ sites: ['/p/a', '/p/b'] })),
    parameterContextKey(identity({ sites: ['/p/b', '/p/a'] })),
  );
  // 路径里带分隔符也不能把两个身份拼成同一个键。
  assert.notEqual(
    parameterContextKey(identity({ sites: ['/p/a|b'] })),
    parameterContextKey(identity({ sites: ['/p/a', 'b'] })),
  );
});

it('the request identity is compared by value, not by object reference', () => {
  assert.equal(sameParameterContextIdentity(identity(), identity()), true);
  const changed: Partial<ParameterContextIdentity>[] = [
    { version: { id: 'official', revision: 2 } },
    { version: { id: 'modified', revision: 1 } },
    { kernel: '/kernels/grid' },
    { sites: ['/p/site2'] },
  ];
  for (const patch of changed)
    assert.equal(sameParameterContextIdentity(identity(), identity(patch)), false);
});

it('step 2 reads once per identity and never in a busy or unready state', () => {
  const value = identity();
  const key = parameterContextKey(value);
  assert.equal(start(value), true);
  // 同一身份成功读过（标记还在）：普通渲染不再重复请求。
  assert.equal(start(value, { loadedFor: key }), false);
  // 刚失败过的身份不自动重试（否则 busy 翻回会无限重试）；换身份才放开。
  assert.equal(start(value, { failedFor: key }), false);
  assert.equal(
    start(identity({ version: { id: 'official', revision: 2 } }), { failedFor: key }),
    true,
  );
  // 版本变了，旧标记自然不再命中。
  assert.equal(
    start(identity({ version: { id: 'official', revision: 2 } }), { loadedFor: key }),
    true,
  );
  assert.equal(start(value, { busy: true }), false);
  assert.equal(start(value, { step: 1 }), false);
  assert.equal(start(value, { step: 4 }), false);
  assert.equal(start(identity({ kernel: '' })), false);
  assert.equal(start(identity({ sites: [] })), false);
  // 格点研究要等类型读出来（否则会读成缺类型行的一套）。
  assert.equal(start(identity({ spatial: true, types: [] }), { typeChoicesReady: false }), false);
  assert.equal(start(identity({ spatial: true, types: [1] }), { typeChoicesReady: true }), true);
  // 单点研究不依赖类型选择。
  assert.equal(start(identity(), { typeChoicesReady: false }), true);
});

it('a late read only releases its own load marker', () => {
  const oldKey = parameterContextKey(identity());
  // 新身份的请求已经写下自己的标记：旧请求收尾不能把它清掉。
  const shared = { current: 'newer-key' };
  assert.equal(clearParameterContextMarker(shared, oldKey), false);
  assert.equal(shared.current, 'newer-key');
  // 标记还是旧请求自己写的：清掉，让当前身份能重读。
  const own = { current: oldKey };
  assert.equal(clearParameterContextMarker(own, oldKey), true);
  assert.equal(own.current, '');
});
