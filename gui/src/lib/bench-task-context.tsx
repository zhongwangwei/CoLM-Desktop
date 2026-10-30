import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import {
  BENCH_TASKS_KEY,
  beginBenchTask,
  benchTaskId,
  clearBenchTask,
  finishBenchTask,
  markInterruptedBenchTasks,
  parseBenchTasks,
  serializeBenchTasks,
  updateBenchTask,
  type BenchTask,
  type BenchTaskFocus,
  type BenchTaskInput,
} from './bench-task';
import { readStored, writeStored } from './storage';

export interface BenchTaskSink {
  begin: (input: BenchTaskInput) => string;
  update: (
    id: string,
    patch: Partial<Pick<BenchTask, 'phase' | 'job' | 'report' | 'error'>>,
  ) => void;
  finish: (
    id: string,
    outcome: {
      state: 'done' | 'failed' | 'interrupted';
      report?: string | null;
      error?: string | null;
    },
  ) => void;
  clear: (id: string) => void;
  /** 记下"回到比较页后打开这次结果"的意图；由 App 的任务条设置，比较页挂载后消费一次。 */
  focus: (focus: BenchTaskFocus | null) => void;
  peekFocus: () => BenchTaskFocus | null;
  consumeFocus: () => BenchTaskFocus | null;
  /** 页面还活着时把它自己的"停止"逻辑借给壳；页面卸载/收尾后清掉，壳退回最小后端取消。 */
  registerStop: (id: string, handler: (() => void) | null) => void;
  /** 有页面注册的处理器就调用并返回 true；否则返回 false 让调用方走后端回退。 */
  stopTask: (id: string) => boolean;
}

/**
 * 默认值是全套 no-op：页面组件在测试/无壳渲染时也能工作，真正的任务条由 main.tsx 的 provider 提供。
 * 页面仍然自己掌握提交锁和等待逻辑，这里只做跨路由可见的镜像。
 */
const noop: BenchTaskSink = {
  begin: () => '',
  update: () => {},
  finish: () => {},
  clear: () => {},
  focus: () => {},
  peekFocus: () => null,
  consumeFocus: () => null,
  registerStop: () => {},
  stopTask: () => false,
};

const BenchTaskContext = createContext<{ tasks: BenchTask[] } & BenchTaskSink>({
  tasks: [],
  ...noop,
});

export function BenchTasksProvider({ children }: { children: ReactNode }) {
  // 启动时把上次留下的 running 标成 interrupted：本机任务不可能跨进程继续，远程作业的真实状态另行核对。
  const [tasks, setTasks] = useState<BenchTask[]>(() =>
    markInterruptedBenchTasks(parseBenchTasks(readStored(BENCH_TASKS_KEY))),
  );
  const tasksRef = useRef(tasks);
  tasksRef.current = tasks;
  useEffect(() => {
    writeStored(BENCH_TASKS_KEY, serializeBenchTasks(tasks));
  }, [tasks]);
  const begin = useCallback((input: BenchTaskInput) => {
    const id = benchTaskId();
    setTasks((old) => beginBenchTask(old, { ...input, id }).tasks);
    return id;
  }, []);
  const update = useCallback<BenchTaskSink['update']>((id, patch) => {
    setTasks((old) => updateBenchTask(old, id, patch));
  }, []);
  const finish = useCallback<BenchTaskSink['finish']>((id, outcome) => {
    setTasks((old) => finishBenchTask(old, id, outcome));
  }, []);
  const clear = useCallback((id: string) => {
    setTasks((old) => clearBenchTask(old, id));
  }, []);
  // focus 是"下一次进比较页要打开哪次结果"，只活在一次导航里；用 ref 保持它不触发多余渲染。
  const pendingFocus = useRef<BenchTaskFocus | null>(null);
  const focus = useCallback((next: BenchTaskFocus | null) => {
    pendingFocus.current = next;
  }, []);
  const peekFocus = useCallback(() => pendingFocus.current, []);
  const consumeFocus = useCallback(() => {
    const next = pendingFocus.current;
    pendingFocus.current = null;
    return next;
  }, []);
  const stopHandlers = useRef(new Map<string, () => void>());
  const registerStop = useCallback((id: string, handler: (() => void) | null) => {
    if (handler) stopHandlers.current.set(id, handler);
    else stopHandlers.current.delete(id);
  }, []);
  const stopTask = useCallback((id: string) => {
    const handler = stopHandlers.current.get(id);
    if (!handler) return false;
    handler();
    return true;
  }, []);
  const value = {
    tasks,
    begin,
    update,
    finish,
    clear,
    focus,
    peekFocus,
    consumeFocus,
    registerStop,
    stopTask,
  };
  return <BenchTaskContext.Provider value={value}>{children}</BenchTaskContext.Provider>;
}

export function useBenchTasks(): { tasks: BenchTask[] } & BenchTaskSink {
  return useContext(BenchTaskContext);
}
