import TaskCard from '../../components/TaskCard';
import { engineStep, type BuildProgress } from './engine-progress';

const STEPS: [string, string][] = [
  ['核对引擎', 'Check'],
  ['上传源码', 'Upload'],
  ['安装工具链', 'Toolchain'],
  ['编译模块', 'Compile'],
  ['链接程序', 'Link'],
];

export default function EngineProgress({
  host,
  phase,
  waited,
  progress,
  en,
  hint,
}: {
  host: string;
  phase: 'checking' | 'building';
  waited: string;
  progress?: BuildProgress | null;
  en: boolean;
  /** What the user can do meanwhile; the wizard's wording by default. */
  hint?: string;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const step = engineStep(phase, progress);
  const counted = step >= 3 && progress?.done != null && !!progress.total;
  const percent = counted
    ? Math.min(100, Math.floor((progress!.done! / progress!.total!) * 100))
    : null;
  const current = progress?.current?.replace(/\(.*?\)/g, '').trim();
  const detail = [
    t(`正在核对 ${host} 上的计算引擎…`, `Checking the engine on ${host}…`),
    t('正在把当前版本的源码传到服务器…', 'Sending this version’s source to the server…'),
    progress?.stage?.includes('CMake')
      ? t('正在安装 CMake…', 'Installing CMake…')
      : t('正在安装 Rust 编译器…', 'Installing the Rust compiler…'),
    counted
      ? t(
          `已编译 ${progress!.done} / ${progress!.total} 个模块${current ? ` · 正在编译 ${current}` : ''}`,
          `${progress!.done} of ${progress!.total} units compiled${current ? ` · compiling ${current}` : ''}`,
        )
      : t('正在准备编译…', 'Preparing to compile…'),
    t(
      `正在编译并链接最后的程序${current ? `（${current}）` : ''}，这一步最慢，通常还要几分钟`,
      `Compiling and linking the last programs${current ? ` (${current})` : ''}; the slowest step, usually a few more minutes`,
    ),
  ][step];
  return (
    <TaskCard
      title={t(
        `${host} 正在准备与当前版本一致的计算引擎`,
        `${host} is preparing an engine for this version`,
      )}
      waited={t(`已用 ${waited}`, `${waited} so far`)}
      steps={STEPS.map(([zh, english]) => (en ? english : zh))}
      step={step}
      percent={percent}
      detail={detail}
      hint={
        hint ??
        t(
          '可以先继续填写，读取站点和创建算例会等它完成。',
          'Keep filling in the form; reading sites and creating the case wait for it.',
        )
      }
    />
  );
}
