import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as compareConfig from './config';
import {
  benchErrorText,
  buildOpenbenchConfig,
  comparisonName,
  designFingerprint,
  jobContacted,
  jobEnded,
  jobUncertain,
  parseRemoteJob,
  remoteJobTerminal,
  type RemoteJobIdentity,
} from './compare';

/**
 * 这些用例执行 ComparePage.tsx 里的**真实函数**（TypeScript AST 抽出函数体，`new Function` 在受控上下文里跑），
 * 用受控 promise 控制 IPC 返回顺序，证明 GUI-05（A 慢 B 快、A 快 B 慢、A 失败 B 成功）、GUI-07（查询超时保留
 * 作业身份、终态才结束跟踪、重开可恢复核对、未核对不重复提交）、GUI-08（取回失败提示旧报告仍在、并发取回去重）
 * 在真实页面逻辑下的行为，而不是对源码文本做断言。
 */
const pagePath = new URL('./ComparePage.tsx', import.meta.url);
const pageSource = readFileSync(pagePath, 'utf8');
const source = ts.createSourceFile(
  pagePath.pathname,
  pageSource,
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);

/** 取页面里一个 `const name = ...` 声明（保持真实源码）。 */
function declaration(name: string): string {
  let found = '';
  const visit = (node: ts.Node) => {
    if (ts.isVariableStatement(node))
      for (const declarationNode of node.declarationList.declarations)
        if (ts.isIdentifier(declarationNode.name) && declarationNode.name.text === name)
          found = node.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(found, `ComparePage.tsx defines ${name}`);
  return found;
}

function execute<T>(name: string, context: Record<string, unknown>): T {
  // watch() 现在还会镜像跨路由任务条；这些用例不关心状态条，给 no-op 默认值。
  const withDefaults = {
    benchTasks: { begin: () => '', update: () => {}, finish: () => {}, clear: () => {} },
    benchTaskId: { current: '' },
    LEFT: 'compare page left',
    ...context,
  };
  const code = ts.transpileModule(`${declaration(name)}; return ${name};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(withDefaults), code)(...Object.values(withDefaults)) as T;
}

const t = (_zh: string, english: string) => english;
const LEFT = 'compare page left';
const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

type Deferred = {
  resolve: (value: unknown) => void;
  reject: (error: unknown) => void;
};
type Request = { command: string; args: Record<string, unknown> | undefined } & Deferred;

/** 受控后端：请求都挂在队列里，测试按任意顺序 resolve/reject（证明迟到的响应不写当前页面）。 */
function controlledIpc() {
  const requests: Request[] = [];
  const invokeJson = (command: string, args?: Record<string, unknown>) =>
    new Promise<unknown>((resolve, reject) => {
      requests.push({ command, args, resolve, reject });
    });
  const wait = async (index: number) => {
    // 有上限的等待：请求号写错时让用例快速失败，而不是永远等下去把测试进程挂住。
    for (let attempt = 0; requests.length <= index; attempt++) {
      assert.ok(attempt < 2000, `expected backend request #${index} was never made`);
      await tick();
    }
    return requests[index]!;
  };
  const answer = async (index: number, value: unknown) => {
    (await wait(index)).resolve(value);
    await tick();
  };
  const fail = async (index: number, error: unknown) => {
    (await wait(index)).reject(error);
    await tick();
  };
  return { requests, invokeJson, wait, answer, fail };
}

function memoryStorage(seed: Record<string, string> = {}) {
  const values: Record<string, string> = { ...seed };
  const stored = (key: string) => values[key] ?? '';
  const store = (key: string, value: string) => {
    if (value) values[key] = value;
    else delete values[key];
  };
  return { values, stored, store };
}

const status = (root: string) => ({ root, openbench: null, projects: [] });

// ---- GUI-05：机器上下文的请求代次 ----

function refreshHarness() {
  const ipc = controlledIpc();
  const hostRef = { current: 'A' };
  const hostEpoch = { current: 1 };
  const statuses: unknown[] = [];
  const references: unknown[] = [];
  const errors: string[] = [];
  const stillCurrent = execute<(machine: string, epoch: number) => boolean>('stillCurrent', {
    hostRef,
    hostEpoch,
  });
  const refresh = execute<(machine?: string, epoch?: number, quiet?: boolean) => Promise<unknown>>(
    'refresh',
    {
      host: 'A',
      hostEpoch,
      stillCurrent,
      invokeJson: ipc.invokeJson,
      setStatus: (value: unknown) => statuses.push(value),
      setReferences: (value: unknown) => references.push(value),
      setError: (value: string) => errors.push(value),
      parseReferences: (value: unknown) => value,
      shown: (error: unknown) => String(error),
    },
  );
  const switchTo = (machine: string) => {
    hostRef.current = machine;
    hostEpoch.current += 1;
  };
  return { ipc, hostRef, hostEpoch, statuses, references, errors, refresh, switchTo };
}

test('GUI-05: A slow, B fast — the late answer from A never writes the B page', async () => {
  const h = refreshHarness();
  const slowA = h.refresh();
  h.switchTo('B');
  const fastB = h.refresh('B', h.hostEpoch.current);
  await h.ipc.answer(1, status('/b'));
  await fastB;
  assert.deepEqual(h.statuses, [status('/b')], 'B is what the page shows');
  // A 的响应这时才回来：状态、登记表、错误都不能写。
  await h.ipc.answer(0, status('/a'));
  assert.equal(await slowA, null);
  assert.deepEqual(h.statuses, [status('/b')]);
  assert.deepEqual(h.references, [[]]);
  assert.deepEqual(h.errors, []);
});

test('GUI-05: A fast, B slow — the page first shows A and then exactly B', async () => {
  const h = refreshHarness();
  const fastA = h.refresh();
  await h.ipc.answer(0, status('/a'));
  await fastA;
  h.switchTo('B');
  const slowB = h.refresh('B', h.hostEpoch.current);
  await h.ipc.answer(1, status('/b'));
  await slowB;
  assert.deepEqual(h.statuses, [status('/a'), status('/b')]);
  assert.deepEqual(h.errors, []);
});

test('GUI-05: A fails, B succeeds — A’s late failure does not clear or replace B’s page', async () => {
  const h = refreshHarness();
  const failingA = h.refresh();
  h.switchTo('B');
  const workingB = h.refresh('B', h.hostEpoch.current);
  await h.ipc.answer(1, status('/b'));
  await workingB;
  await h.ipc.fail(0, new Error('ssh: connect to host A: Connection timed out'));
  await failingA;
  assert.deepEqual(h.statuses, [status('/b')]);
  assert.deepEqual(h.errors, [], 'the old machine’s error is not shown on the new page');
});

test('GUI-05: an old machine’s reference-list failure and result are both dropped', async () => {
  const h = refreshHarness();
  const a = h.refresh();
  await h.ipc.answer(0, { ...status('/a'), openbench: '3.0.8' });
  // A 的状态回来了，登记表请求已经发出；这时用户切到 B。
  const referencesRequest = await h.ipc.wait(1);
  assert.equal(referencesRequest.command, 'bench_remote_references');
  h.switchTo('B');
  await h.ipc.fail(1, new Error('cannot read the registry'));
  await a;
  assert.deepEqual(h.statuses, [{ ...status('/a'), openbench: '3.0.8' }]);
  assert.deepEqual(h.references, [], 'A’s registry never lands on B’s page');
  assert.deepEqual(h.errors, [], 'A’s registry error is not shown on B’s page');
});

test('GUI-05: a background refresh for a stale machine sends no request and writes nothing', async () => {
  const h = refreshHarness();
  h.switchTo('B');
  // 任务结束时旧代码会拿旧机器的闭包调 refresh；现在连请求都不发。
  assert.equal(await h.refresh('A', 1, true), null);
  assert.equal(h.ipc.requests.length, 0);
  assert.deepEqual(h.statuses, []);
  assert.deepEqual(h.errors, []);
});

test('GUI-05: quiet background refresh never overwrites the message the operation just showed', async () => {
  const h = refreshHarness();
  const pending = h.refresh('A', 1, true);
  await h.ipc.fail(0, new Error('ssh: timeout'));
  await pending;
  assert.deepEqual(
    h.errors,
    [],
    'a quiet failure is swallowed instead of clobbering the visible error',
  );
});

// ---- GUI-08：取回失败保留已显示报告，并发取回去重 ----

function fetchHarness() {
  const ipc = controlledIpc();
  const hostRef = { current: 'A' };
  const hostEpoch = { current: 1 };
  const fetched: string[] = [];
  const stillCurrent = execute<(machine: string, epoch: number) => boolean>('stillCurrent', {
    hostRef,
    hostEpoch,
  });
  const runs: unknown[] = [];
  const fetchReport = execute<(machine: string, project: string, runId?: string) => Promise<void>>(
    'fetchReport',
    {
      fetching: { current: new Set<string>() },
      hostEpoch,
      stillCurrent,
      t,
      invokeJson: ipc.invokeJson,
      // 页面的 setLastRun 支持函数式更新（取回后按服务器返回的 run 身份补记）。
      setLastRun: (value: unknown) => {
        const previous = runs.at(-1) ?? null;
        runs.push(
          typeof value === 'function' ? (value as (old: unknown) => unknown)(previous) : value,
        );
      },
      view: async (path: string) => {
        fetched.push(path);
      },
    },
  );
  const switchTo = (machine: string) => {
    hostRef.current = machine;
    hostEpoch.current += 1;
  };
  return { ipc, fetched, fetchReport, switchTo, runs };
}

test('GUI-08: fetching the same comparison twice at once is refused with one backend call', async () => {
  const h = fetchHarness();
  const first = h.fetchReport('A', 'cmp');
  await h.ipc.wait(0);
  await assert.rejects(h.fetchReport('A', 'cmp'), /being fetched already/);
  assert.equal(h.ipc.requests.length, 1, 'the duplicate never reaches the backend');
  await h.ipc.answer(0, { report: '/local/report.html' });
  await first;
  assert.deepEqual(h.fetched, ['/local/report.html']);
  // 取回结束后可以再取：同一比较不会永久锁死。
  const again = h.fetchReport('A', 'cmp');
  await h.ipc.answer(1, { report: '/local/report.html' });
  await again;
  assert.deepEqual(h.fetched, ['/local/report.html', '/local/report.html']);
});

test('GUI-08: fetching a specific run passes runId and records the returned run identity', async () => {
  const h = fetchHarness();
  const call = h.fetchReport('A', 'cmp', 'bench-run-77');
  await h.ipc.wait(0);
  assert.deepEqual(h.ipc.requests[0]!.args, {
    host: 'A',
    name: 'cmp',
    runId: 'bench-run-77',
  });
  await h.ipc.answer(0, {
    report: '/local/report.html',
    run_id: 'bench-run-77',
    config_sha: 'a'.repeat(64),
    state: 'succeeded',
    legacy: false,
  });
  await call;
  assert.deepEqual(h.fetched, ['/local/report.html']);
  const recorded = h.runs.at(-1) as { runId: string; configSha: string; report: string | null };
  assert.equal(recorded.runId, 'bench-run-77');
  assert.equal(recorded.configSha, 'a'.repeat(64));
  assert.equal(recorded.report, '/local/report.html');
});

test('GUI-08: a failed fetch replaces no report and the old-version message is bilingual', async () => {
  const h = fetchHarness();
  const call = h.fetchReport('A', 'cmp');
  const request = await h.ipc.wait(0);
  assert.equal(request.command, 'bench_remote_fetch');
  // 先挂上拒绝断言再让后端失败：页面里 rejection 立刻被 task 的 catch 接住，测试里不能留成未处理拒绝。
  const rejected = assert.rejects(call, /旧报告未改动/);
  await h.ipc.fail(
    0,
    new Error(
      '取回失败：ssh: connect to host 7920ts port 22: Connection timed out。本机旧报告未改动，仍是上次成功版本。',
    ),
  );
  await rejected;
  assert.deepEqual(h.fetched, [], 'the report already on screen is kept');
  // 页面 task 会把后端错误交给 benchErrorText：英文界面看到的是"旧报告仍是上次成功版本"。
  const [, english] = benchErrorText(
    '取回失败：ssh: connect to host 7920ts port 22: Connection timed out。本机旧报告未改动，仍是上次成功版本。',
  );
  assert.match(english, /previous report on this computer is unchanged/);
});

test('GUI-08: what the task shows after a failed fetch is the bilingual old-version hint', async () => {
  const errors: string[] = [];
  const notices: string[] = [];
  const task = execute<(fn: () => Promise<unknown>, done?: string) => Promise<void>>('task', {
    lock: { current: false },
    setBusy: () => {},
    setError: (value: string) => errors.push(value),
    setNotice: (value: string) => notices.push(value),
    benchErrorText,
    shown: (error: unknown) => {
      const [zh, english] = benchErrorText(String(error).replace(/^Error: /, ''));
      return english === zh ? english : english;
    },
    t,
    LEFT,
    host: 'A',
    hostEpoch: { current: 1 },
    refresh: async () => null,
  });
  await task(async () => {
    throw new Error(
      '取回失败：ssh: connect to host 7920ts port 22: Connection timed out。本机旧报告未改动，仍是上次成功版本。',
    );
  });
  assert.match(errors.at(-1) ?? '', /previous report on this computer is unchanged/);
  // task 开始时把提示清空（setNotice('')），之后失败走错误通道，不应再出现"完成"提示。
  assert.deepEqual(notices.filter(Boolean), []);
});

test('GUI-05/08: a report fetched for the old machine is not shown after switching machines', async () => {
  const h = fetchHarness();
  const call = h.fetchReport('A', 'cmp');
  await h.ipc.wait(0);
  h.switchTo('B');
  await h.ipc.answer(0, { report: '/local/report.html' });
  await call;
  assert.deepEqual(h.fetched, [], 'the old machine’s report is dropped');
});

// ---- GUI-07：查询超时保留作业身份、终态才结束跟踪 ----

type WatchHarness = ReturnType<typeof watchHarness>;

function watchHarness(seed: Record<string, string> = {}) {
  const ipc = controlledIpc();
  const storage = memoryStorage(seed);
  const hostRef = { current: 'serverA' };
  const hostEpoch = { current: 1 };
  const jobRef = { current: null as unknown };
  const stopping = { current: false };
  const alive = { current: true };
  const jobs: unknown[] = [];
  const logs: string[][] = [];
  const notices: string[] = [];
  const unsettled: unknown[] = [];
  const lastJobs: unknown[] = [];
  const jobKey = execute<(machine: string) => string>('jobKey', {});
  const jobHistoryKey = execute<(machine: string) => string>('jobHistoryKey', {});
  const stillCurrent = execute<(machine: string, epoch: number) => boolean>('stillCurrent', {
    hostRef,
    hostEpoch,
  });
  const rememberJob = execute<(machine: string, identity: unknown) => void>('rememberJob', {
    store: storage.store,
    jobKey,
  });
  // 页面里的 state setter 支持函数式更新（settleJob 用它只在作业匹配时清空）；这里照同样语义记录。
  let unsettledValue: unknown = null;
  const setUnsettled = (value: unknown) => {
    unsettledValue =
      typeof value === 'function' ? (value as (old: unknown) => unknown)(unsettledValue) : value;
    unsettled.push(unsettledValue);
  };
  const setLastJob = (value: unknown) => lastJobs.push(value);
  const setActiveJob = (value: unknown) => jobs.push(value);
  const settleJob = execute<
    (machine: string, identity: unknown, end: string, reason?: string) => void
  >('settleJob', {
    store: storage.store,
    jobKey,
    jobHistoryKey,
    hostRef,
    setUnsettled,
    setLastJob,
    setActiveJob,
    jobEnded,
  });
  const markUncertain = execute<(machine: string, identity: unknown, error: string) => unknown>(
    'markUncertain',
    { rememberJob, hostRef, setUnsettled, jobUncertain },
  );
  const jobFailureText = execute<(reply: unknown) => string>('jobFailureText', { t });
  const watch = execute<
    (
      machine: string,
      active: unknown,
    ) => Promise<
      | { state: 'succeeded' | 'cancelled'; job: RemoteJobIdentity }
      | { state: 'failed'; job: RemoteJobIdentity; error: string }
      | { state: 'unknown'; job: RemoteJobIdentity; reason: string }
      | { state: 'uncertain'; job: RemoteJobIdentity; reason: string }
      | { state: 'detached' }
    >
  >('watch', {
    hostEpoch,
    hostRef,
    stillCurrent,
    job: jobRef,
    setActiveJob,
    alive,
    stopping,
    rememberJob,
    settleJob,
    markUncertain,
    jobContacted,
    jobFailureText,
    remoteJobTerminal,
    setLog: (value: string[]) => logs.push(value),
    setNotice: (value: string) => notices.push(value),
    setUnsettled,
    invokeJson: ipc.invokeJson,
    LEFT,
    sleep: async () => {},
    t,
  });
  return {
    ipc,
    storage,
    hostRef,
    hostEpoch,
    stopping,
    alive,
    jobs,
    logs,
    notices,
    unsettled,
    lastJobs,
    watch,
  };
}

const jobReply = (state: string, extra: Record<string, unknown> = {}) => ({
  job: 'bench-run-1',
  status: { state, exit_code: null, log_tail: '', detail: null, ...extra },
  ended: null as string | null,
});

test('GUI-07: one timed-out query keeps the whole job identity instead of deleting it', async () => {
  const h: WatchHarness = watchHarness();
  const pending = h.watch('serverA', { job: 'bench-run-1', kind: 'run', name: 'qfo_era5' });
  await h.ipc.wait(0);
  // 第一次查询还没回，身份已经写进本机记录（重开应用也能恢复核对）。
  const before = parseRemoteJob(h.storage.values['colm.benchJob@serverA']);
  assert.equal(before?.job, 'bench-run-1');
  assert.equal(before?.host, 'serverA');
  await h.ipc.fail(0, new Error('ssh: connect to host serverA port 22: Connection timed out'));
  const outcome = await pending;
  assert.equal(outcome.state, 'uncertain');
  // 一次超时不能删掉活动作业记录，也不能误标成功/失败：身份、主机、比较项目、提交时间、最后状态都在。
  const kept = parseRemoteJob(h.storage.values['colm.benchJob@serverA']);
  assert.equal(kept?.job, 'bench-run-1');
  assert.equal(kept?.name, 'qfo_era5');
  assert.equal(kept?.host, 'serverA');
  assert.equal(typeof kept?.submittedAt, 'string');
  assert.equal(kept?.uncertain, true);
  assert.match(kept?.lastError ?? '', /Connection timed out/);
  assert.equal(
    h.storage.values['colm.benchLastJob@serverA'],
    undefined,
    'not recorded as finished',
  );
  assert.equal(h.lastJobs.length, 0, 'no terminal history was written');
  assert.equal(h.unsettled.length, 1, 'the page shows an awaiting-check entry');
});

test('GUI-07: only a confirmed terminal success ends tracking and keeps the identity in history', async () => {
  const h: WatchHarness = watchHarness();
  const pending = h.watch('serverA', { job: 'bench-run-2', kind: 'run', name: 'qfo_era5' });
  await h.ipc.answer(0, jobReply('finished', { exit_code: 0, log_tail: 'all good\n' }));
  const outcome = await pending;
  assert.equal(outcome.state, 'succeeded');
  assert.equal(h.storage.values['colm.benchJob@serverA'], undefined, 'active tracking ended');
  const history = parseRemoteJob(h.storage.values['colm.benchLastJob@serverA']);
  assert.equal(history?.job, 'bench-run-2');
  assert.equal(history?.name, 'qfo_era5');
  assert.equal(history?.end, 'succeeded');
  assert.equal(h.unsettled.at(-1), null, 'no awaiting-check entry remains');
  assert.equal(h.lastJobs.length, 1, 'the page keeps the last job identity');
});

test('GUI-07: scheduler-recorded cancel and failure are different terminal states', async () => {
  const cancelledHarness = watchHarness();
  const cancelledJob = cancelledHarness.watch('serverA', {
    job: 'bench-install-1',
    kind: 'install',
  });
  await cancelledHarness.ipc.answer(0, { ...jobReply('lost'), ended: 'cancelled' });
  assert.equal((await cancelledJob).state, 'cancelled');
  assert.equal(
    parseRemoteJob(cancelledHarness.storage.values['colm.benchLastJob@serverA'])?.end,
    'cancelled',
  );

  const failedHarness = watchHarness();
  const failedJob = failedHarness.watch('serverA', { job: 'bench-run-3', kind: 'run', name: 'x' });
  await failedHarness.ipc.answer(0, { ...jobReply('lost'), ended: 'failed' });
  const failed = await failedJob;
  assert.equal(failed.state, 'failed');
  assert.equal(
    parseRemoteJob(failedHarness.storage.values['colm.benchLastJob@serverA'])?.end,
    'failed',
  );
});

test('GUI-07: lost/unknown replies never become a terminal state by polling count', async () => {
  const h: WatchHarness = watchHarness();
  const pending = h.watch('serverA', { job: 'bench-run-4', kind: 'run', name: 'x' });
  // 查不到记录既不是查询失败、也不是确认终态：原身份保留、继续核对（旧实现 3 次后写 failed 是错的）。
  await h.ipc.answer(0, jobReply('lost'));
  assert.equal(parseRemoteJob(h.storage.values['colm.benchJob@serverA'])?.lastState, 'lost');
  await h.ipc.answer(1, jobReply('unknown'));
  assert.equal(parseRemoteJob(h.storage.values['colm.benchJob@serverA'])?.lastState, 'unknown');
  await h.ipc.answer(2, jobReply('lost'));
  assert.equal(
    parseRemoteJob(h.storage.values['colm.benchJob@serverA'])?.job,
    'bench-run-4',
    'the identity is still tracked after repeated lost/unknown replies',
  );
  assert.equal(h.storage.values['colm.benchLastJob@serverA'], undefined, 'no terminal history yet');
  // 只有调度系统明确记下的结束才结束跟踪：这里回一条 ended=failed 收尾。
  await h.ipc.answer(3, { ...jobReply('lost'), ended: 'failed' });
  const outcome = await pending;
  assert.equal(outcome.state, 'failed');
  assert.equal(parseRemoteJob(h.storage.values['colm.benchJob@serverA']), null);
  assert.equal(parseRemoteJob(h.storage.values['colm.benchLastJob@serverA'])?.end, 'failed');
});

test('GUI-07: a cancellation in flight is not reported as cancelled until the scheduler confirms', async () => {
  const h: WatchHarness = watchHarness();
  const pending = h.watch('serverA', { job: 'bench-run-5', kind: 'run', name: 'x' });
  await h.ipc.wait(0);
  h.stopping.current = true;
  // 取消请求已被调度器收下，但作业还 queued：只提示"正在取消"，身份与跟踪都保留（旧实现这一步就写 cancelled）。
  await h.ipc.answer(0, { ...jobReply('queued'), cancel_requested: true });
  assert.equal(
    parseRemoteJob(h.storage.values['colm.benchJob@serverA'])?.job,
    'bench-run-5',
    'the job identity is still tracked while cancellation is in flight',
  );
  assert.equal(h.storage.values['colm.benchLastJob@serverA'], undefined);
  assert.match(h.notices.at(-1) ?? '', /Cancellation requested/);
  // 调度系统确认取消之后才结束跟踪。
  await h.ipc.answer(1, { ...jobReply('lost'), ended: 'cancelled' });
  const outcome = await pending;
  assert.equal(outcome.state, 'cancelled');
  assert.equal(parseRemoteJob(h.storage.values['colm.benchLastJob@serverA'])?.end, 'cancelled');
});

test('GUI-07: a scheduler-confirmed end without exit code keeps polling instead of failing', async () => {
  const h: WatchHarness = watchHarness();
  const pending = h.watch('serverA', { job: 'bench-run-11', kind: 'run', name: 'x' });
  // terminal=true 只说明"作业不会再写"，退出码还没经 NFS 可见：不能当失败，继续轮询。
  await h.ipc.answer(0, { ...jobReply('lost'), terminal: true });
  assert.equal(
    parseRemoteJob(h.storage.values['colm.benchJob@serverA'])?.job,
    'bench-run-11',
    'tracking continues until the result is visible',
  );
  assert.equal(h.storage.values['colm.benchLastJob@serverA'], undefined);
  // 退出码可见后按正常终态处理。
  await h.ipc.answer(1, jobReply('finished', { exit_code: 0 }));
  assert.equal((await pending).state, 'succeeded');
});

test('GUI-05/07: switching to another machine detaches the wait but keeps the job identity', async () => {
  const h: WatchHarness = watchHarness();
  const pending = h.watch('serverA', { job: 'bench-run-6', kind: 'run', name: 'x' });
  await h.ipc.wait(0);
  h.hostRef.current = 'serverB';
  h.hostEpoch.current += 1;
  await h.ipc.answer(0, jobReply('running'));
  const outcome = await pending;
  assert.equal(outcome.state, 'detached');
  assert.equal(h.logs.length, 0, 'the old machine’s log is not written into the new page');
  assert.equal(parseRemoteJob(h.storage.values['colm.benchJob@serverA'])?.job, 'bench-run-6');
});

test('GUI-07: an unresolved recorded job blocks silent resubmission; other hosts are unaffected', () => {
  const storage = memoryStorage({
    'colm.benchJob@serverA': JSON.stringify({
      job: 'bench-run-9',
      kind: 'run',
      name: 'qfo_era5',
      host: 'serverA',
    }),
  });
  const jobKey = execute<(machine: string) => string>('jobKey', {});
  const guard = execute<(machine: string) => void>('assertNoUnresolvedJob', {
    stored: storage.stored,
    jobKey,
    parseRemoteJob,
    t,
  });
  assert.throws(() => guard('serverA'), /unconfirmed/);
  assert.throws(() => guard('serverA'), /never written twice/);
  assert.doesNotThrow(() => guard('serverB'));
  // 确认并清掉活动记录之后才允许提交。
  storage.store('colm.benchJob@serverA', '');
  assert.doesNotThrow(() => guard('serverA'));
});

test('GUI-07: running a comparison refuses to resubmit while an unconfirmed job exists', async () => {
  const ipc = controlledIpc();
  const storage = memoryStorage({
    'colm.benchJob@serverA': JSON.stringify({
      job: 'bench-run-9',
      kind: 'run',
      name: 'qfo_era5',
      host: 'serverA',
    }),
  });
  const jobKey = execute<(machine: string) => string>('jobKey', {});
  const assertNoUnresolvedJob = execute<(machine: string) => void>('assertNoUnresolvedJob', {
    stored: storage.stored,
    jobKey,
    parseRemoteJob,
    t,
  });
  const simulation = {
    label: 'hb',
    info: {
      history_dir: '/p/hb/out/hb/history',
      prefix: 'hb_hist_',
      tim_res: 'Day',
      syear: 2002,
      eyear: 2005,
      spatial: true,
      lon: null,
      lat: null,
      files: 48,
      extent: [111, 115.5, 21.5, 24.5] as [number, number, number, number],
      resolution: 0.1,
      data_groupby: 'Month',
    },
  };
  const baseDesign = {
    name: 'qfo era5',
    years: [2002, 2005] as [number, number],
    timRes: 'Day',
    variables: { Latent_Heat: 'ERA5LAND_LowRes' },
    simulations: [simulation],
    projectDir: '/ob/projects/qfo_era5',
  };
  const yaml = buildOpenbenchConfig(baseDesign).config;
  const formDesign = {
    machine: { host: 'serverA', openbenchRoot: '/ob', refRoot: '/refs' },
    ...baseDesign,
  };
  // 运行必须绑定生成时冻结的快照（R01）：这里模拟"刚生成、未改动"的状态。
  const generated = {
    current: compareConfig.freezeCompareSnapshot(formDesign, { config: yaml, sites: [] }),
  };
  const run = execute<() => Promise<unknown>>('run', {
    task: (fn: () => Promise<unknown>) => fn(),
    generated,
    formDesign,
    designFingerprint,
    snapshotStale: compareConfig.snapshotStale,
    parseOpenbenchConfig: compareConfig.parseOpenbenchConfig,
    structuralProblems: compareConfig.structuralProblems,
    configFingerprint: compareConfig.configFingerprint,
    name: 'qfo era5',
    comparisonName,
    remote: true,
    host: 'serverA',
    assertNoUnresolvedJob,
    invokeJson: ipc.invokeJson,
    yaml,
    sites: [],
    scanDir: '/refs',
    watch: async () => ({ state: 'succeeded' }),
    afterWatch: async () => ({ state: 'succeeded' }),
    fetchReport: async () => {},
    setNotice: () => {},
    setConfigSaving: () => {},
    setLastRun: () => {},
    showRunNotice: () => {},
    t,
  });
  await assert.rejects(run(), /unconfirmed/);
  assert.equal(ipc.requests.length, 0, 'nothing is submitted while the old job is unconfirmed');
});

test('GUI-07: watch outcomes become errors/notices without throwing away an unconfirmed identity', async () => {
  const notices: string[] = [];
  const uncertainText = execute<(machine: string, identity: unknown) => string>('uncertainText', {
    t,
  });
  const afterWatch = execute<
    (
      machine: string,
      outcome: unknown,
      onSuccess?: () => Promise<void>,
    ) => Promise<{ state: string }>
  >('afterWatch', { t, setNotice: (value: string) => notices.push(value), uncertainText });
  const job = { job: 'bench-run-7', kind: 'run', name: 'x', host: 'serverA' };
  // 暂时联系不上：不抛错（task 不会显示"评估完成"，也不会报失败），保留身份并提示重连/核对/停止。
  const uncertain = await afterWatch('serverA', {
    state: 'uncertain',
    job,
    reason: 'ssh: timeout',
  });
  assert.equal(uncertain.state, 'uncertain');
  assert.equal(notices.length, 1);
  assert.match(notices[0]!, /still tracked|may still be running/);
  // 确认过的失败/取消才进入 task 的错误路径。
  await assert.rejects(afterWatch('serverA', { state: 'failed', job, error: 'boom' }), /boom/);
  await assert.rejects(afterWatch('serverA', { state: 'cancelled', job }), /已停止/);
  // 成功时执行后续动作（取回报告）。
  let fetched = 0;
  await afterWatch('serverA', { state: 'succeeded', job }, async () => {
    fetched++;
  });
  assert.equal(fetched, 1);
  // 换看了别的机器：什么也不写。
  assert.equal((await afterWatch('serverA', { state: 'detached' })).state, 'detached');
  assert.equal(notices.length, 1);
});

test('GUI-07: checking an unreachable job confirms the terminal state, keeps identity, then can fetch', async () => {
  const ipc = controlledIpc();
  const storage = memoryStorage({
    'colm.benchJob@serverA': JSON.stringify({
      job: 'bench-run-8',
      kind: 'run',
      name: 'qfo_era5',
      host: 'serverA',
      lastState: 'running',
      uncertain: true,
    }),
  });
  const hostRef = { current: 'serverA' };
  const hostEpoch = { current: 1 };
  const jobKey = execute<(machine: string) => string>('jobKey', {});
  const jobHistoryKey = execute<(machine: string) => string>('jobHistoryKey', {});
  const stillCurrent = execute<(machine: string, epoch: number) => boolean>('stillCurrent', {
    hostRef,
    hostEpoch,
  });
  const rememberJob = execute<(machine: string, identity: unknown) => void>('rememberJob', {
    store: storage.store,
    jobKey,
  });
  const updateUnsettled: unknown[] = [];
  const notices: string[] = [];
  const fetched: string[] = [];
  const settleJob = execute<
    (machine: string, identity: unknown, end: string, reason?: string) => void
  >('settleJob', {
    store: storage.store,
    jobKey,
    jobHistoryKey,
    hostRef,
    setUnsettled: (value: unknown) => updateUnsettled.push(value),
    setLastJob: () => {},
    setActiveJob: () => {},
    jobEnded,
  });
  const markUncertain = execute<(machine: string, identity: unknown, error: string) => unknown>(
    'markUncertain',
    {
      rememberJob,
      hostRef,
      setUnsettled: (value: unknown) => updateUnsettled.push(value),
      jobUncertain,
    },
  );
  const jobFailureText = execute<(reply: unknown) => string>('jobFailureText', { t });
  const current = parseRemoteJob(storage.values['colm.benchJob@serverA'])!;
  const check = execute<() => Promise<void>>('checkUnsettled', {
    unsettled: { job: current, reason: 'ssh: timeout' },
    host: 'serverA',
    hostRef,
    hostEpoch,
    stillCurrent,
    task: (fn: () => Promise<unknown>) => fn(),
    invokeJson: ipc.invokeJson,
    setLog: () => {},
    remoteJobTerminal,
    jobContacted,
    rememberJob,
    settleJob,
    markUncertain,
    jobFailureText,
    setUnsettled: (value: unknown) => updateUnsettled.push(value),
    setNotice: (value: string) => notices.push(value),
    fetchReport: async (machine: string, name: string) => {
      fetched.push(`${machine}/${name}`);
    },
    t,
  });
  const checking = check();
  await ipc.answer(0, jobReply('running'));
  await checking;
  // 仍在跑：最后确认状态更新，但身份与"待核对"保留。
  const kept = parseRemoteJob(storage.values['colm.benchJob@serverA']);
  assert.equal(kept?.lastState, 'running');
  assert.equal(kept?.uncertain, true);
  assert.match(notices.at(-1) ?? '', /still running/);
  assert.equal(fetched.length, 0, 'nothing is fetched before the terminal state is confirmed');
  // 下一次核对确认成功：活动记录结束、写历史、继续取回报告。
  const confirmed = execute<() => Promise<void>>('checkUnsettled', {
    unsettled: { job: kept, reason: '' },
    host: 'serverA',
    hostRef,
    hostEpoch,
    stillCurrent,
    task: (fn: () => Promise<unknown>) => fn(),
    invokeJson: ipc.invokeJson,
    setLog: () => {},
    remoteJobTerminal,
    jobContacted,
    rememberJob,
    settleJob,
    markUncertain,
    jobFailureText,
    setUnsettled: (value: unknown) => updateUnsettled.push(value),
    setNotice: (value: string) => notices.push(value),
    fetchReport: async (machine: string, name: string) => {
      fetched.push(`${machine}/${name}`);
    },
    t,
  });
  // 先发出核对（固定第 2 个请求），再按顺序回包；反过来等请求编号会死锁。
  const confirming = confirmed();
  await ipc.answer(1, jobReply('finished', { exit_code: 0 }));
  await confirming;
  assert.equal(storage.values['colm.benchJob@serverA'], undefined);
  assert.equal(parseRemoteJob(storage.values['colm.benchLastJob@serverA'])?.end, 'succeeded');
  assert.deepEqual(fetched, ['serverA/qfo_era5']);
  assert.match(notices.at(-1) ?? '', /finished successfully/);
});

test('GUI-07: a failed check keeps the identity and surfaces the communication error', async () => {
  const ipc = controlledIpc();
  const storage = memoryStorage();
  const hostRef = { current: 'serverA' };
  const hostEpoch = { current: 1 };
  const jobKey = execute<(machine: string) => string>('jobKey', {});
  const stillCurrent = execute<(machine: string, epoch: number) => boolean>('stillCurrent', {
    hostRef,
    hostEpoch,
  });
  const rememberJob = execute<(machine: string, identity: unknown) => void>('rememberJob', {
    store: storage.store,
    jobKey,
  });
  const current: RemoteJobIdentity = {
    job: 'bench-run-10',
    kind: 'run',
    name: 'x',
    host: 'serverA',
    uncertain: true,
    lastState: 'running',
  };
  storage.store('colm.benchJob@serverA', JSON.stringify(current));
  const markUncertain = execute<(machine: string, identity: unknown, error: string) => unknown>(
    'markUncertain',
    {
      rememberJob,
      hostRef,
      setUnsettled: () => {},
      jobUncertain,
    },
  );
  const check = execute<() => Promise<void>>('checkUnsettled', {
    unsettled: { job: current, reason: '' },
    host: 'serverA',
    hostRef,
    hostEpoch,
    stillCurrent,
    task: (fn: () => Promise<unknown>) => fn(),
    invokeJson: ipc.invokeJson,
    setLog: () => {},
    remoteJobTerminal,
    rememberJob,
    settleJob: () => {},
    markUncertain,
    jobFailureText: () => '',
    setUnsettled: () => {},
    setNotice: () => {},
    fetchReport: async () => {},
    t,
  });
  const call = check();
  await ipc.wait(0);
  const rejected = assert.rejects(call, /still unreachable/);
  await ipc.fail(0, new Error('ssh: still unreachable'));
  await rejected;
  const kept = parseRemoteJob(storage.values['colm.benchJob@serverA']);
  assert.equal(kept?.job, 'bench-run-10');
  assert.equal(kept?.uncertain, true);
  assert.match(kept?.lastError ?? '', /still unreachable/);
});

test('GUI-07: stopping an unconfirmed job cancels exactly that job, then waits for the confirmation', async () => {
  const cancels: { command: string; args: Record<string, unknown> | undefined }[] = [];
  const watched: unknown[] = [];
  const stopped: boolean[] = [];
  const notices: string[] = [];
  // 记录停止标记的每次变化（watch 的 finally 会复位它）。
  let stopFlag = false;
  const stopping = {
    get current() {
      return stopFlag;
    },
    set current(value: boolean) {
      stopFlag = value;
      stopped.push(value);
    },
  };
  const job: RemoteJobIdentity = {
    job: 'bench-run-11',
    kind: 'run',
    name: 'x',
    host: 'serverA',
    uncertain: true,
    lastState: 'running',
  };
  const uncertainText = execute<(machine: string, identity: unknown) => string>('uncertainText', {
    t,
  });
  const afterWatch = execute<
    (
      machine: string,
      outcome: unknown,
      onSuccess?: () => Promise<void>,
    ) => Promise<{ state: string }>
  >('afterWatch', { t, setNotice: (value: string) => notices.push(value), uncertainText });
  const stopUnsettled = execute<() => Promise<void>>('stopUnsettled', {
    unsettled: { job, reason: 'ssh: timeout' },
    host: 'serverA',
    task: (fn: () => Promise<unknown>) => fn(),
    stopping,
    invoke: async (command: string, args: Record<string, unknown>) => {
      cancels.push({ command, args });
      return null;
    },
    markUncertain: () => {},
    watch: async (machine: string, identity: unknown) => {
      watched.push({ machine, identity });
      // 真实 watch 的 finally 会把停止标记复位；这里照做，验证停止流程不会永远卡在"正在取消"。
      stopping.current = false;
      return { state: 'cancelled', job: identity };
    },
    afterWatch,
  });
  await assert.rejects(stopUnsettled(), /已停止/);
  assert.deepEqual(cancels, [
    { command: 'bench_remote_cancel', args: { host: 'serverA', job: 'bench-run-11' } },
  ]);
  // 取消之后仍按同一个作业号确认"已取消"，不是把身份一丢了事。
  assert.equal((watched[0] as { machine: string }).machine, 'serverA');
  assert.equal((watched[0] as { identity: RemoteJobIdentity }).identity.job, 'bench-run-11');
  assert.equal(stopped.at(0), true, 'the stop flag is set while cancelling');
  assert.equal(stopped.at(-1), false, 'the stop flag is released when the wait ends');
});

test('GUI-07: reconnecting waits for the recorded job and fetches its report on success', async () => {
  const fetched: string[] = [];
  const notices: string[] = [];
  const job: RemoteJobIdentity = {
    job: 'bench-run-12',
    kind: 'run',
    name: 'qfo_era5',
    host: 'serverA',
    uncertain: true,
    lastState: 'running',
  };
  const uncertainText = execute<(machine: string, identity: unknown) => string>('uncertainText', {
    t,
  });
  const afterWatch = execute<
    (
      machine: string,
      outcome: unknown,
      onSuccess?: () => Promise<void>,
    ) => Promise<{ state: string }>
  >('afterWatch', { t, setNotice: (value: string) => notices.push(value), uncertainText });
  const reconnect = execute<() => Promise<void>>('reconnect', {
    unsettled: { job, reason: 'ssh: timeout' },
    host: 'serverA',
    task: (fn: () => Promise<unknown>) => fn(),
    watch: async () => ({ state: 'succeeded', job }),
    afterWatch,
    fetchReport: async (machine: string, name: string) => {
      fetched.push(`${machine}/${name}`);
    },
    setNotice: (value: string) => notices.push(value),
    t,
  });
  await reconnect();
  assert.deepEqual(fetched, ['serverA/qfo_era5']);
  assert.match(notices.at(-1) ?? '', /server job has finished/);
});

// ---- GUI-05：目录探测结果的上下文核对 ----

test('GUI-05: a folder probe from the old machine is not written after switching machines', async () => {
  const ipc = controlledIpc();
  const hostRef = { current: 'A' };
  const hostEpoch = { current: 1 };
  const stillCurrent = execute<(machine: string, epoch: number) => boolean>('stillCurrent', {
    hostRef,
    hostEpoch,
  });
  let picks: unknown[] = [];
  const addPick = execute<(dir: string, label: string) => Promise<void>>('addPick', {
    picks: [],
    host: 'A',
    remote: true,
    hostEpoch,
    stillCurrent,
    invokeJson: ipc.invokeJson,
    setPicks: (update: unknown) => {
      picks =
        typeof update === 'function'
          ? (update as (old: unknown[]) => unknown[])(picks)
          : (update as unknown[]);
    },
    shown: (error: unknown) => String(error),
  });
  const call = addPick('/p/old/history', 'old');
  const request = await ipc.wait(0);
  assert.equal(request.command, 'bench_remote_inspect');
  hostRef.current = 'B';
  hostEpoch.current += 1;
  await ipc.answer(0, {
    history_dir: '/p/old/history',
    prefix: 'old_hist_',
    tim_res: 'Day',
    syear: 2000,
    eyear: 2001,
    spatial: false,
    lon: 1,
    lat: 2,
    files: 12,
  });
  await call;
  assert.deepEqual(picks, [{ dir: '/p/old/history', label: 'old', info: null }]);
});

test('R09: a bare job that vanished without an exit code settles as result-unknown and releases the host', async () => {
  const h = watchHarness();
  const pending = h.watch('serverA', { job: 'bench-run-1', kind: 'run', name: 'qfo_era5' });
  const request = await h.ipc.wait(0);
  request.resolve({
    job: 'bench-run-1',
    status: { state: 'lost', exit_code: null, log_tail: '' },
    ended: 'unknown',
    terminal: true,
    result_unknown: true,
  });
  const outcome = await pending;
  assert.equal(outcome.state, 'unknown');
  // 终态身份被清掉（历史里留一份），否则这台服务器会被一条已终止的记录永远挡住。
  assert.equal(h.storage.values['colm.benchJob@serverA'], undefined);
  const history = parseRemoteJob(h.storage.values['colm.benchLastJob@serverA']);
  assert.equal(history?.end, 'unknown');
  assert.equal(history?.uncertain, false);
  assert.equal(outcome.state, 'unknown');
  assert.match(outcome.state === 'unknown' ? outcome.reason : '', /result unknown/);
  // 清掉之后重提不再被"未确认终态"的作业挡住。
  const jobKey = execute<(machine: string) => string>('jobKey', {});
  const guard = execute<(machine: string) => void>('assertNoUnresolvedJob', {
    stored: h.storage.stored,
    jobKey,
    parseRemoteJob,
    t,
  });
  assert.doesNotThrow(() => guard('serverA'));
});

test('R09: result-unknown is a distinct terminal state, never success/failure', () => {
  assert.equal(
    remoteJobTerminal({
      status: { state: 'lost', exit_code: null },
      ended: 'unknown',
      terminal: true,
    }),
    'unknown',
  );
  assert.equal(
    remoteJobTerminal({
      status: { state: 'lost', exit_code: null },
      ended: null,
      terminal: true,
      result_unknown: true,
    }),
    'unknown',
  );
  // 旧契约：调度器说结束、退出码还在 NFS 上 → 仍然不是终态（不能当 unknown）。
  assert.equal(
    remoteJobTerminal({
      status: { state: 'lost', exit_code: null },
      ended: null,
      terminal: true,
    }),
    null,
  );
  const ended = jobEnded(
    { job: 'bench-run-2', kind: 'run', name: 'x' },
    'unknown',
    '2026-01-01T00:00:00.000Z',
    'gone',
  );
  assert.equal(ended.end, 'unknown');
  assert.equal(ended.uncertain, false);
  assert.equal(parseRemoteJob(JSON.stringify(ended))?.end, 'unknown');
});
