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
  const value = { tasks, begin, update, finish, clear };
  return <BenchTaskContext.Provider value={value}>{children}</BenchTaskContext.Provider>;
}

export function useBenchTasks(): { tasks: BenchTask[] } & BenchTaskSink {
  return useContext(BenchTaskContext);
}
