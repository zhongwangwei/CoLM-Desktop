import TaskCard from '../../components/TaskCard';
import { engineStep, type BuildProgress } from './engine-progress';

const STEPS: [string, string][] = [
  ['核对引擎', 'Check'],
  ['上传源码', 'Upload'],
  ['安装工具链', 'Toolchain'],
  ['编译模块', 'Compile'],
  ['链接程序', 'Link'],
];

/**
 * 服务器端计算引擎的准备进度（U-06 之后的易用性跟进）。
 *
 * 引擎按源码快照（引擎源码内容哈希）缓存在服务器 `<根>/engine/<快照id>/`：同一快照编好一次就复用，
 * 只有换软件版本/改引擎源码才需要再编。原来卡片只说"正在准备与当前版本一致的计算引擎"，用户刚
 * 编过又看到它，很容易误以为每次运行都在重复编译（2026-10-28 实测反馈）。现在：
 * - 核对阶段说明"有就直接复用、只有快照变了才重编"；
 * - 编译阶段的说明写清"服务器上还没有这个源码快照的引擎，所以编这一次"；
 * - 知道快照 id 时显示前 8 位，和服务器上的 `engine/<id>` 目录对得上。
 */
export default function EngineProgress({
  host,
  phase,
  waited,
  progress,
  engineId,
  en,
  hint,
}: {
  host: string;
  phase: 'checking' | 'building';
  waited: string;
  progress?: BuildProgress | null;
  /** 正在准备/编译的源码快照 id（服务器上的引擎目录名）；未知时为空。 */
  engineId?: string | null;
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
  const snapshot = engineId?.slice(0, 8);
  const snapshotNote = snapshot
    ? t(`（源码快照 ${snapshot}）`, ` (source snapshot ${snapshot})`)
    : '';
  // 编译发生只有一个原因：服务器上还没有当前源码快照的引擎。把这句话和"编好后复用"写在一起，
  // 用户就不会把一次性的新快照编译当成每次都重编。
  const whyBuilding = t(
    `服务器上还没有当前源码快照的引擎，所以为它编这一次${snapshot ? `（快照 ${snapshot}）` : ''}；同一快照编好后会复用，不会每次运行都重编。`,
    `The server has no engine for this source snapshot yet, so it builds one now${snapshot ? ` (snapshot ${snapshot})` : ''}; later runs with the same snapshot reuse it instead of rebuilding.`,
  );
  const detail = [
    t(
      `正在核对 ${host} 上有没有与当前源码快照一致的引擎${snapshotNote}：有就直接复用，只有源码或依赖变了才需要重编一次。`,
      `Checking whether ${host} already has an engine for this source snapshot${snapshotNote}: it is reused when present; a changed source or dependency snapshot needs one new build.`,
    ),
    t(
      `正在把当前源码快照传到服务器…${snapshotNote}`,
      `Sending this source snapshot to the server…${snapshotNote}`,
    ),
    progress?.stage?.includes('CMake')
      ? t('正在安装 CMake…', 'Installing CMake…')
      : t('正在安装 Rust 编译器…', 'Installing the Rust compiler…'),
    counted
      ? t(
          `已编译 ${progress!.done} / ${progress!.total} 个模块${current ? ` · 正在编译 ${current}` : ''}。${whyBuilding}`,
          `${progress!.done} of ${progress!.total} units compiled${current ? ` · compiling ${current}` : ''}. ${whyBuilding}`,
        )
      : `${t('正在准备编译…', 'Preparing to compile…')}${whyBuilding}`,
    t(
      `正在编译并链接最后的程序${current ? `（${current}）` : ''}，这一步最慢，通常还要几分钟。${whyBuilding}`,
      `Compiling and linking the last programs${current ? ` (${current})` : ''}; the slowest step, usually a few more minutes. ${whyBuilding}`,
    ),
  ][step];
  return (
    <TaskCard
      title={
        phase === 'checking'
          ? t(`${host} 正在核对计算引擎`, `${host} is checking the compute engine`)
          : t(
              `${host} 正在为当前源码快照编译计算引擎`,
              `${host} is compiling an engine for the current source snapshot`,
            )
      }
      waited={t(`已用 ${waited}`, `${waited} so far`)}
      steps={STEPS.map(([zh, english]) => (en ? english : zh))}
      step={step}
      percent={percent}
      detail={detail}
      hint={
        hint ??
        t(
          '可以先继续填写，读取站点和创建算例会等它完成；同一源码快照编好后会复用。',
          'Keep filling in the form; reading sites and creating the case wait for it. A built snapshot is reused later.',
        )
      }
    />
  );
}
