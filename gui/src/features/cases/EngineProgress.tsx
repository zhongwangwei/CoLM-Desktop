import './engine-progress.css';
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
}: {
  host: string;
  phase: 'checking' | 'building';
  waited: string;
  progress?: BuildProgress | null;
  en: boolean;
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
    <section className="engine-progress" role="status" aria-live="polite">
      <header className="engine-progress-head">
        <strong>
          {t(
            `${host} 正在准备与当前版本一致的计算引擎`,
            `${host} is preparing an engine for this version`,
          )}
        </strong>
        <span className="engine-progress-time">{t(`已用 ${waited}`, `${waited} so far`)}</span>
      </header>
      <ol className="engine-progress-steps">
        {STEPS.map(([zh, english], index) => (
          <li
            key={zh}
            className={index < step ? 'is-done' : index === step ? 'is-active' : undefined}
            aria-current={index === step ? 'step' : undefined}
          >
            {en ? english : zh}
          </li>
        ))}
      </ol>
      <div
        className={`engine-progress-bar${percent === null ? ' is-indeterminate' : ''}`}
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
      >
        <span style={percent === null ? undefined : { width: `${Math.max(percent, 2)}%` }} />
      </div>
      <div className="engine-progress-meta">
        <span className="engine-progress-detail">{detail}</span>
        {percent !== null && <span className="engine-progress-percent">{percent}%</span>}
      </div>
      <p className="engine-progress-hint">
        {t(
          '可以先继续填写，读取站点和创建算例会等它完成。',
          'Keep filling in the form; reading sites and creating the case wait for it.',
        )}
      </p>
    </section>
  );
}
