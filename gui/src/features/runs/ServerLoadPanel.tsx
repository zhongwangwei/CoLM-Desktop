import { loadPercent, myJobsText, type ServerLoad } from './server-load';
import { plainError } from '../../lib/bilingual';

function Meter({ value, label }: { value: number; label: string }) {
  const level = value >= 85 ? ' is-high' : value >= 60 ? ' is-mid' : '';
  return (
    <div
      className={`server-meter${level}`}
      role="meter"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value}
    >
      <span style={{ width: `${Math.min(100, value)}%` }} />
    </div>
  );
}

/** How busy the server is: login node, partitions or hosts, everyone's queue and the own jobs. */
export default function ServerLoadPanel({
  host,
  load,
  error,
  at,
  busy,
  partition,
  en,
  onRefresh,
}: {
  host: string;
  load?: ServerLoad;
  error?: string;
  at?: number;
  busy: boolean;
  partition?: string;
  en: boolean;
  onRefresh: () => void;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const queueOf = (name: string) => load?.queues.find((queue) => queue.name === name);
  const loadShare = load?.load && load.cpus ? loadPercent(load.load[0], load.cpus) : null;
  const memoryShare =
    load?.memory_total_gb && load.memory_available_gb != null
      ? loadPercent(load.memory_total_gb - load.memory_available_gb, load.memory_total_gb)
      : null;
  return (
    <section className="server-load" aria-label={t('服务器状况', 'Server status')}>
      <header className="server-load-head">
        <strong>{t('服务器状况', 'Server status')}</strong>
        <span className="muted">
          {load?.hostname || host}
          {load?.scheduler ? ` · ${load.scheduler.toUpperCase()}` : ''}
          {at
            ? ` · ${t('更新于', 'updated')} ${new Date(at).toLocaleTimeString(en ? 'en-US' : 'zh-CN')}`
            : ''}
        </span>
        {/* 原生按钮：运行面板的测试用真实 require 加载本组件，ui 组件库会牵出 localStorage。 */}
        <button type="button" className="button button-ghost" disabled={busy} onClick={onRefresh}>
          {busy ? t('读取中…', 'Reading…') : t('刷新', 'Refresh')}
        </button>
      </header>
      {error && <p className="server-load-error">{plainError(error)}</p>}
      {!load && !error && (
        <p className="muted">
          {busy
            ? t('正在读取服务器的负载和排队情况…', 'Reading the server load and queue…')
            : t(
                '点“刷新”读取服务器的负载和排队情况。',
                'Press Refresh to read the server load and queue.',
              )}
        </p>
      )}
      {load && (
        <>
          <div className="server-load-tiles">
            {loadShare != null && (
              <div className="server-tile">
                <span className="server-tile-label">{t('登录节点负载', 'Login node load')}</span>
                <strong>
                  {load.load![0].toFixed(1)}
                  <small>{t(` / ${load.cpus} 核`, ` / ${load.cpus} cores`)}</small>
                </strong>
                <Meter value={loadShare} label={t('登录节点负载', 'Login node load')} />
              </div>
            )}
            {memoryShare != null && (
              <div className="server-tile">
                <span className="server-tile-label">{t('登录节点内存', 'Login node memory')}</span>
                <strong>
                  {t(`可用 ${load.memory_available_gb}`, `${load.memory_available_gb} free`)}
                  <small>{` / ${load.memory_total_gb} GB`}</small>
                </strong>
                <Meter value={memoryShare} label={t('登录节点内存', 'Login node memory')} />
              </div>
            )}
            {load.hosts && (
              <div className="server-tile">
                <span className="server-tile-label">
                  {t('计算主机作业槽', 'Compute job slots')}
                </span>
                <strong>
                  {load.hosts.slots_used}
                  <small>{` / ${load.hosts.slots_total}`}</small>
                </strong>
                <Meter
                  value={loadPercent(load.hosts.slots_used, load.hosts.slots_total)}
                  label={t('计算主机作业槽', 'Compute job slots')}
                />
                <span className="server-tile-note">
                  {t(
                    `可用 ${load.hosts.ok} 台 · 已满 ${load.hosts.full} 台 · 关闭 ${load.hosts.closed} 台`,
                    `${load.hosts.ok} available · ${load.hosts.full} full · ${load.hosts.closed} closed`,
                  )}
                </span>
              </div>
            )}
            <div className="server-tile server-tile-mine">
              <span className="server-tile-label">{t('你的作业', 'Your jobs')}</span>
              <span>{myJobsText(load, en)}</span>
            </div>
          </div>
          {!!load.partitions.length && (
            <div className="table-scroll">
              <table className="server-load-table">
                <thead>
                  <tr>
                    <th>{t('分区', 'Partition')}</th>
                    <th>{t('节点：忙 / 空闲 / 下线', 'Nodes: busy / idle / down')}</th>
                    <th>{t('CPU 占用', 'CPUs in use')}</th>
                    <th>{t('全体排队 / 运行', 'All users: queued / running')}</th>
                  </tr>
                </thead>
                <tbody>
                  {load.partitions.map((entry) => {
                    const [allocated, , , total] = entry.cpus;
                    const share = loadPercent(allocated, total);
                    const queue = queueOf(entry.name);
                    return (
                      <tr
                        key={entry.name}
                        className={entry.name === partition ? 'is-current' : undefined}
                      >
                        <td>
                          {entry.name}
                          {entry.name === partition ? t('（当前）', ' (current)') : ''}
                        </td>
                        <td>{`${entry.nodes[0]} / ${entry.nodes[1]} / ${entry.nodes[2]}`}</td>
                        <td>
                          <div className="server-load-cpu">
                            <Meter value={share} label={`${entry.name} CPU`} />
                            <span>{`${share}% · ${allocated}/${total}`}</span>
                          </div>
                        </td>
                        <td>{queue ? `${queue.pending} / ${queue.running}` : '—'}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
          {!load.partitions.length && !!load.queues.length && (
            <div className="table-scroll">
              <table className="server-load-table">
                <thead>
                  <tr>
                    <th>{t('队列', 'Queue')}</th>
                    <th>{t('全体排队', 'All users queued')}</th>
                    <th>{t('全体运行', 'All users running')}</th>
                  </tr>
                </thead>
                <tbody>
                  {load.queues.map((queue) => (
                    <tr
                      key={queue.name}
                      className={queue.name === partition ? 'is-current' : undefined}
                    >
                      <td>{queue.name}</td>
                      <td>{queue.pending}</td>
                      <td>{queue.running}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </>
      )}
    </section>
  );
}
