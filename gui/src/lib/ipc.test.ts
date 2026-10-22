import test from 'node:test';
import assert from 'node:assert/strict';

// 运行中的算例不能被改：改运行位置、改输出目录这类单个 `dir`/`caseDir` 参数的命令原来不在清单里。
test('writes to a running case are refused whatever the directory argument is called', async () => {
  const calls: string[] = [];
  (globalThis as { window?: unknown }).window = {
    __TAURI__: {
      core: {
        invoke: async (command: string) => {
          calls.push(command);
          return null;
        },
      },
      event: { listen: async () => () => {} },
    },
  };
  const { invoke, bindRunningTargets } = await import('./ipc');
  bindRunningTargets(() => new Set(['/p/running']));
  await assert.rejects(invoke('set_case_run_host', { dir: '/p/running', host: null }));
  await assert.rejects(
    invoke('redirect_output_dir', { caseDir: '/p/running', targetDir: '/elsewhere' }),
  );
  await assert.rejects(invoke('mark_results_stale', { dirs: ['/p/running'] }));
  await invoke('set_case_run_host', { dir: '/p/idle', host: null });
  assert.deepEqual(calls, ['set_case_run_host']);
});
