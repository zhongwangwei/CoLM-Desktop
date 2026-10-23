import { bilingual } from './bilingual';
import { commandContract, type Command } from './command-contract';
export type IpcCommand = Command;
type Tauri = {
  core: { invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T> };
  event: {
    listen: <T>(event: string, callback: (event: { payload: T }) => void) => Promise<() => void>;
  };
};
declare global {
  interface Window {
    __TAURI__?: Tauri;
  }
}
export const hasBackend = () => !!window.__TAURI__?.core?.invoke;
/**
 * Bound one startup/background call so a backend that never answers becomes a reported failure
 * instead of leaving the window busy forever (GUI 审查 2026-10-22 P1-3).
 */
export function withTimeout<T>(promise: Promise<T>, timeoutMs: number, what: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(
      () =>
        reject(
          new Error(
            bilingual(
              `${what}超过 ${Math.round(timeoutMs / 1000)} 秒仍未完成`,
              `Timed out after ${Math.round(timeoutMs / 1000)}s waiting for ${what}`,
            ),
          ),
        ),
      timeoutMs,
    );
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}
let runningTargets: () => Set<string> = () => new Set();
export const bindRunningTargets = (get: () => Set<string>) => {
  runningTargets = get;
};
const caseWrites = new Set([
  'case_data_save',
  'set_field_batch',
  'set_fields_batch',
  'reset_field_batch',
  'set_spinup',
  'set_process_parameter_field_batch',
  'reset_process_parameter_field_batch',
  'set_pft_parameter_batch',
  'set_pft_parameters_batch',
  'apply_import_parameter_overrides',
  'configure_ozone_batch',
  'configure_cbl_batch',
  'hybrid_install',
  'hybrid_remove',
  'hybrid_climate',
  // 也改算例目录（GUI 审查 2026-10-22 发现漏在清单外）。参数是 `dirs`、单个 `dir` 或 `caseDir`。
  'set_case_run_host',
  'mark_results_stale',
  'redirect_output_dir',
]);
export async function invoke<T = unknown>(
  command: IpcCommand,
  args: Record<string, unknown> = {},
): Promise<T> {
  if (!hasBackend())
    throw new Error(
      bilingual('请在桌面软件中执行此操作', 'This operation requires the desktop backend.'),
    );
  if (caseWrites.has(command)) {
    // `set_pft_parameters_batch` nests its case directories in `changes[].dirs`.
    const nested = Array.isArray(args.changes)
      ? args.changes.flatMap((change) =>
          change && typeof change === 'object' && Array.isArray((change as { dirs?: unknown }).dirs)
            ? (change as { dirs: unknown[] }).dirs
            : [],
        )
      : [];
    const dirs = [
      ...(Array.isArray(args.dirs) ? args.dirs : []),
      ...(typeof args.dir === 'string' ? [args.dir] : []),
      ...(typeof args.caseDir === 'string' ? [args.caseDir] : []),
      ...nested,
    ];
    const busy = dirs.filter((dir) => typeof dir === 'string' && runningTargets().has(dir));
    if (busy.length)
      throw new Error(
        bilingual(
          '算例正在运行，暂时不能修改其配置',
          'The case is running; its configuration cannot be changed.',
        ),
      );
  }
  const signature: Record<string, { required: boolean }> | undefined =
    Object.prototype.hasOwnProperty.call(commandContract, command)
      ? commandContract[command as Command]
      : undefined;
  if (!signature) throw new Error(`${command}: unregistered command`);
  for (const key of Object.keys(args))
    if (!Object.prototype.hasOwnProperty.call(signature, key))
      throw new Error(`${command}: unexpected argument ${key}`);
  for (const [key, value] of Object.entries(signature))
    if (value.required && (args[key] === undefined || args[key] === null))
      throw new Error(`${command}: missing argument ${key}`);
  return window.__TAURI__!.core.invoke<T>(command, args);
}
export async function invokeJson<T = unknown>(
  command: IpcCommand,
  args: Record<string, unknown> = {},
): Promise<T> {
  const result = await invoke<unknown>(command, args);
  return (typeof result === 'string' ? JSON.parse(result) : result) as T;
}
export type AppEvent =
  | 'run://started'
  | 'run://stage'
  | 'run://progress'
  | 'run://lines'
  | 'run://done'
  | 'bench://log'
  | 'assistant://event'
  | 'remote-create://progress'
  | 'colm-about'
  | 'app://close-requested';
export async function listen<T = unknown>(
  event: AppEvent,
  callback: (payload: T) => void,
): Promise<() => void> {
  if (!window.__TAURI__?.event) return () => {};
  return window.__TAURI__.event.listen<unknown>(event, ({ payload }) => {
    let value = payload;
    if (typeof payload === 'string') {
      try {
        value = JSON.parse(payload);
      } catch {
        /* Plain log strings are valid payloads. */
      }
    }
    callback(value as T);
  });
}
