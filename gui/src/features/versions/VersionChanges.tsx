import { useEffect, useState } from 'react';
import { invokeJson } from '../../lib/ipc';
import { useApp } from '../../lib/app-state';
import type { FrozenRevision, VersionRef } from '../../lib/types';
import { Card, Field, Notice, Select } from '../../components/ui';

type Change = { key: string; before: string | null; after: string | null };
type Study = {
  study: string;
  purpose: string;
  sites: string[];
  best_member: string | null;
  best_objective: number | null;
  targets: string[];
};
export type VersionChangeReport = {
  version: VersionRef;
  against: VersionRef | null;
  against_is_parent: boolean;
  code: {
    kind: string;
    workspace: string | null;
    from_official: { path: string; added: number; removed: number }[];
    touches: { engine: boolean; kernel: boolean };
    differs: string[];
    note: 'official_revisions_differ' | 'different_official_source' | null;
  };
  parameters: Change[];
  physics: Change[];
  process_files: Change[];
  network: Change | null;
  studies: Study[];
  origin: string | null;
};

const refKey = (v: VersionRef) => `${v.id}#${v.revision}`;

/**
 * 一个冻结版本改了什么：代码相对官方源码改了哪些文件，参数、物理方案、过程文件、AI 网络相对父版本
 * （或另选的一个版本）有什么不同，以及它由哪些研究得来。只读注册表，冻结后工作区删了也能看。
 */
export default function VersionChanges({
  version,
  versions,
}: {
  version: FrozenRevision;
  versions: FrozenRevision[];
}) {
  const { state } = useApp();
  const t = (zh: string, en: string) => (state.language === 'en' ? en : zh);
  const [against, setAgainst] = useState('');
  const [report, setReport] = useState<VersionChangeReport | null>(null);
  const [error, setError] = useState('');
  const mine = refKey(version.version);
  useEffect(() => {
    setAgainst('');
  }, [mine]);
  useEffect(() => {
    let live = true;
    const other = versions.find((v) => refKey(v.version) === against);
    setError('');
    void invokeJson<VersionChangeReport>('model_changes', {
      id: version.version.id,
      revision: version.version.revision,
      againstId: other?.version.id ?? null,
      againstRevision: other?.version.revision ?? null,
    })
      .then((value) => live && setReport(value))
      .catch((e) => {
        if (!live) return;
        setReport(null);
        setError(String(e));
      });
    return () => {
      live = false;
    };
  }, [mine, against, versions, version.version.id, version.version.revision]);
  return (
    <Card title={t('这个版本改了什么', 'What this version changes')}>
      <Field label={t('比较对象', 'Compare with')}>
        <Select value={against} onChange={(e) => setAgainst(e.target.value)}>
          <option value="">{t('父版本', 'Its parent')}</option>
          {versions
            .filter((v) => refKey(v.version) !== mine)
            .map((v) => (
              <option key={refKey(v.version)} value={refKey(v.version)}>
                {v.name} · r{v.version.revision}
              </option>
            ))}
        </Select>
      </Field>
      {error && <Notice tone="error">{error}</Notice>}
      {report && <ChangeReport report={report} versions={versions} en={state.language === 'en'} />}
    </Card>
  );
}

/** 改动报告本身（不取数据，便于测试）。 */
export function ChangeReport({
  report,
  versions,
  en,
}: {
  report: VersionChangeReport;
  versions: FrozenRevision[];
  en: boolean;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const label = (ref: VersionRef | null) => {
    const found = ref && versions.find((v) => refKey(v.version) === refKey(ref));
    return found ? `${found.name} · r${found.version.revision}` : ref ? refKey(ref) : '';
  };
  const base = report.against
    ? report.against_is_parent
      ? t(`父版本 ${label(report.against)}`, `the parent ${label(report.against)}`)
      : label(report.against)
    : '';
  const table = (title: string, rows: Change[]) =>
    rows.length > 0 && (
      <div className="version-changes-block">
        <h4>{title}</h4>
        <ul>
          {rows.map((row) => (
            <li key={row.key}>
              <code>{row.key}</code>
              <span>
                {row.before ?? '—'} → {row.after ?? t('（去掉）', '(removed)')}
              </span>
            </li>
          ))}
        </ul>
      </div>
    );
  const overlay = [
    ...report.parameters,
    ...report.physics,
    ...report.process_files,
    ...(report.network ? [report.network] : []),
  ];
  const where = [
    report.code.touches.engine && t('Rust 引擎', 'Rust engine'),
    report.code.touches.kernel && t('Fortran 内核', 'Fortran kernel'),
  ].filter(Boolean);
  return (
    <div className="version-changes">
      <div className="version-changes-block">
        <h4>{t('代码', 'Code')}</h4>
        {report.code.workspace == null ? (
          <p className="muted">{t('官方源码，没有改动。', 'The official source, unchanged.')}</p>
        ) : report.code.from_official.length === 0 ? (
          <p className="muted">
            {t('代码与官方源码相同。', 'The code is the same as the official source.')}
          </p>
        ) : (
          <>
            <p>
              {t(
                `相对官方源码改了 ${report.code.from_official.length} 个文件`,
                `${report.code.from_official.length} files changed from the official source`,
              )}
              {where.length > 0 && t(`（${where.join('、')}）`, ` (${where.join(', ')})`)}
            </p>
            <ul className="version-changes-files">
              {report.code.from_official.map((file) => (
                <li key={file.path}>
                  <code>{file.path}</code>
                  <small>
                    +{file.added} −{file.removed}
                  </small>
                </li>
              ))}
            </ul>
          </>
        )}
        {report.against && (
          <p>
            {report.code.differs.length === 0
              ? t(`代码与${base}相同。`, `Same code as ${base}.`)
              : t(
                  `与${base}内容不同的文件：${report.code.differs.join('、')}`,
                  `Files that differ from ${base}: ${report.code.differs.join(', ')}`,
                )}
          </p>
        )}
        {report.code.note === 'official_revisions_differ' && (
          <p className="muted">
            {t(
              '两个官方修订的源码不同；官方修订之间的上游改动不在这里逐个列出。',
              'The two official revisions have different sources; upstream changes between them are not listed file by file.',
            )}
          </p>
        )}
        {report.code.note === 'different_official_source' && (
          <p className="muted">
            {t(
              '两个版本基于不同的官方源码：这里只比了两边各自改过的文件，官方修订之间的上游改动没有列出。',
              'The two versions are based on different official sources: only the files either one changed are compared; upstream changes between the official revisions are not listed.',
            )}
          </p>
        )}
      </div>
      {report.against ? (
        overlay.length === 0 ? (
          <p className="muted">
            {t(
              `参数、物理方案、过程文件与 AI 网络都与${base}相同。`,
              `Parameters, physics, process files and the AI network are the same as ${base}.`,
            )}
          </p>
        ) : (
          <>
            <p className="muted">{t(`以下相对${base}：`, `Compared with ${base}:`)}</p>
            {table(t('参数', 'Parameters'), report.parameters)}
            {table(t('物理方案', 'Physics'), report.physics)}
            {table(t('过程文件', 'Process files'), report.process_files)}
            {table(t('AI 网络', 'AI network'), report.network ? [report.network] : [])}
          </>
        )
      ) : (
        <>
          {table(t('参数改值', 'Parameter overrides'), report.parameters)}
          {table(t('物理方案', 'Physics'), report.physics)}
          {table(t('过程文件', 'Process files'), report.process_files)}
          {table(t('AI 网络', 'AI network'), report.network ? [report.network] : [])}
        </>
      )}
      {(report.studies.length > 0 || report.origin) && (
        <div className="version-changes-block">
          <h4>{t('来自哪些研究', 'Derived from')}</h4>
          <ul>
            {report.studies.map((study) => (
              <li key={`${study.purpose}:${study.study}`}>
                <span>
                  {study.purpose === 'parameters'
                    ? t('导入率定参数', 'Calibrated parameters')
                    : t('训练 AI 网络', 'Trained AI network')}
                  {study.sites.length > 0 && ` · ${study.sites.join(t('、', ', '))}`}
                  {study.targets.length > 0 && ` · ${study.targets.join(t('、', ', '))}`}
                  {study.best_member &&
                    ` · ${t('最优', 'best')} ${study.best_member}${
                      study.best_objective != null
                        ? ` (${study.best_objective.toPrecision(4)})`
                        : ''
                    }`}
                </span>
                <code>{study.study}</code>
              </li>
            ))}
            {report.origin && (
              <li>
                <code>{report.origin}</code>
              </li>
            )}
          </ul>
        </div>
      )}
    </div>
  );
}
