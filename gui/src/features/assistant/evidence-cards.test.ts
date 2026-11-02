import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement as h } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { EvidenceCardView, TaskReviewBanner } from './EvidenceCards';
import {
  historyEvidence,
  pendingReconciliation,
  taskSnapshotOf,
  type HistoryEvidence,
  type PendingReconciliation,
} from './model';

const en = (_zh: string, b: string) => b;
const noop = () => {};

function renderEvidence(evidence: HistoryEvidence) {
  return renderToStaticMarkup(
    h(EvidenceCardView, { evidence, t: en, onOpenPath: noop, onOpenUrl: noop }),
  );
}

function renderBanner(review: PendingReconciliation, error = '', language = 'en') {
  return renderToStaticMarkup(
    h(TaskReviewBanner, {
      review,
      note: '',
      onNoteChange: noop,
      reconciling: false,
      notice: '',
      error,
      onReconcile: noop,
      onOpenPath: noop,
      language,
      t: language === 'en' ? en : (zh: string) => zh,
    }),
  );
}

test('U03 evidence cards render structured fields and clickable targets', () => {
  const evidence = historyEvidence(
    'source_history',
    JSON.stringify({
      query: {
        symbol: 'f_fevg',
        repository: 'desktop',
        endpoints: { desktop: { ref: 'main', sha: 'b'.repeat(40) } },
      },
      repos: {
        desktop: {
          key: 'desktop',
          identity: { state: 'verified', root: '/repos/desktop' },
          retention: {
            state: 'retained',
            checked_ref: 'main',
            checked_sha: 'c'.repeat(40),
            note: 'line-level check only',
          },
          locations: [{ path: 'src/a.F90', line: 42 }],
          commits: [
            {
              sha: 'b'.repeat(40),
              author: 'A U Thor',
              author_time_unix: 1_790_000_000,
              committer_time_unix: 1_730_000_000,
              summary: 'change it',
              reason: { status: 'unchecked', excerpt: 'because' },
            },
          ],
          uncommitted: [],
          notes: [],
          errors: [],
        },
      },
      limits: [],
      truncated: false,
    }),
    'en',
  );
  assert.ok(evidence);
  const html = renderEvidence(evidence!);
  assert.match(html, /f_fevg/);
  assert.match(html, /main @ bbbbbbb/, 'endpoint ref + short SHA');
  assert.match(html, /author time/i);
  assert.match(html, /committer time/i);
  assert.match(html, /unchecked/, 'reason status stays raw');
  assert.match(html, /Open file/, 'location is clickable through open_path');
  // A1：保留状态三态之一按后端字段渲染，并带检查终点与 note 原文。
  assert.match(html, /current source retention/);
  assert.match(html, /still contains lines added/);
  assert.match(html, /line-level check only/);
  assert.match(html, /main @ ccccccc/);
  // A2：规范仓库 + 完整 SHA 的 commit 显示“打开 commit”。
  assert.match(html, /Open commit/);
});

test('U04 review banner lists old actions and offers the review controls', () => {
  const review = pendingReconciliation(
    taskSnapshotOf({
      requires_reconciliation: true,
      effect_unknown_actions: [
        {
          id: 'call-1',
          name: 'run_case',
          arguments: '{"case_dir":"/cases/a"}',
          effect: 'EffectUnknown',
          state: 'effect_unknown',
        },
      ],
      next_step: 'inspect existing outputs',
    }),
  );
  const html = renderBanner(review);
  assert.match(html, /Action result needs review/);
  assert.match(html, /run_case/);
  assert.match(html, /call-1/);
  assert.match(html, /View run\/output/);
  assert.match(html, /arguments \/ result/, '原始参数/结果可展开');
  assert.match(html, /case_dir/, '原始参数内容在展开区里');
  assert.match(html, /Mark reviewed/);
  assert.match(html, /Review note/);
  // 失败保留横幅：错误直接显示在同一个横幅里。
  assert.match(renderBanner(review, 'reconcile failed'), /reconcile failed/);
});

test('knowledge save review explains the target and validation failure without asking for scientific review', () => {
  const review = pendingReconciliation(
    taskSnapshotOf({
      requires_reconciliation: true,
      effect_unknown_actions: [
        {
          id: 'call-knowledge',
          name: 'knowledge_update',
          state: 'effect_unknown',
          effect: 'effect_unknown',
          arguments: JSON.stringify({
            title: 'Vcmax units',
            scope: { name: 'official@131' },
            markdown: 'RAW_CARD_MARKDOWN',
          }),
          result: 'error: draft must cite source vendor/CoLM202X/main/MOD_Const_LC.F90',
        },
      ],
    }),
  );
  const html = renderBanner(review, '', 'zh');
  const visible = html.replace(/<details>[\s\S]*?<\/details>/g, '');
  assert.match(visible, /知识卡保存结果待核实/);
  assert.match(visible, /Vcmax units · official@131/);
  assert.match(visible, /保存前校验未通过：缺少来源引用/);
  assert.doesNotMatch(visible, /vendor\/CoLM202X/);
  assert.match(visible, /不代表科学内容已经审核/);
  assert.match(visible, /自动检查/);
  assert.doesNotMatch(
    visible,
    /effect_unknown|call-knowledge|RAW_CARD_MARKDOWN|标记已核对|查看运行/,
  );
  assert.match(html, /<details>/);
  assert.match(html, /call-knowledge/);
  assert.match(html, /RAW_CARD_MARKDOWN/);
  assert.deepEqual(
    review.ids,
    ['call-knowledge'],
    'Presentation never resolves the pending action',
  );
});

test('truncated knowledge arguments retain the recorded action summary without inventing a version', () => {
  const review = pendingReconciliation(
    taskSnapshotOf({
      requires_reconciliation: true,
      effect_unknown_actions: [
        {
          id: 'truncated',
          name: 'knowledge_update',
          arguments: '{"markdown":"' + 'x'.repeat(1987),
          summary: '保存待核对知识草稿：Vcmax 参数说明',
          effect: 'effect_unknown',
        },
      ],
    }),
  );
  const visible = renderBanner(review, '', 'zh').replace(/<details>[\s\S]*?<\/details>/g, '');
  assert.match(visible, /保存待核对知识草稿：Vcmax 参数说明/);
  assert.doesNotMatch(visible, /未提供卡片标题或版本|official@/);
  assert.match(visible, /尚未确认知识卡是否已保存/);
});

test('unknown knowledge saves remain unresolved, including malformed arguments and mixed external actions', () => {
  const review = pendingReconciliation(
    taskSnapshotOf({
      requires_reconciliation: true,
      effect_unknown_actions: [
        {
          id: 'unknown',
          name: 'mcp__colm__knowledge_update',
          arguments: '{bad',
          result: 'error: disk IO failed',
          effect: 'effect_unknown',
        },
      ],
    }),
  );
  const html = renderBanner(review);
  assert.match(html, /Card title or version not provided/);
  assert.match(html, /save is not yet confirmed/);
  assert.match(html, /unconfirmed results remain visible/);
  assert.doesNotMatch(html, /Validation before saving failed/);
  assert.match(html, /disk IO failed/);
  assert.match(html, /If automatic verification is inconclusive, record a manual check/);
  assert.match(html, /does not retry saving or approve the scientific content/);
  assert.match(html, /Mark reviewed/, 'Manual fallback remains available inside closed details');
  assert.equal(review.required, true);
  review.actions.push({ id: 'remote', name: 'run_remote', effect: 'effect_unknown' });
  review.ids.push('remote');
  const mixed = renderBanner(review);
  assert.match(mixed, /Action result needs review/);
  assert.match(mixed, /inspect the actual run\/output/);
  assert.match(mixed, /run_remote/);
  assert.match(mixed, /Mark reviewed/);
});

test('English evidence field labels use a colon and space', () => {
  const html = renderEvidence({
    summary: 'Evidence',
    sections: [{ title: 'Details', fields: [{ label: 'Commit', value: 'abc' }] }],
  });
  assert.match(html, /Commit: /);
  assert.doesNotMatch(html, /Commit：/);
});

test('document search evidence shows actual Xerj retrieval and source validity without inventing citations', () => {
  const result = JSON.stringify({
    query: 'vcmax',
    scope: { kind: 'model', name: 'official@130' },
    provenance: { docs_root: '/frozen/docs' },
    retrieval: {
      backend: 'xerj',
      state: 'ready',
      candidates: 4,
      candidate_hits: 2,
      rejected_candidates: 1,
    },
    matches: [
      {
        file: 'photosynthesis.md',
        line: 12,
        context: ['Vcmax controls maximum carboxylation.'],
        knowledge_status: 'current',
        may_support_current_answer: true,
        match: 'candidate',
      },
    ],
    truncated: true,
  });
  for (const name of [
    'search_docs',
    'mcp__colm__search_docs',
    'colm/search_docs',
    'colm.search_docs',
    'colm:search_docs',
  ]) {
    const evidence = historyEvidence(name, result, 'en');
    assert.ok(evidence);
    assert.match(evidence.summary, /Xerj · model · official@130 · 1 matches returned/);
    assert.match(evidence.summary, /Results truncated/);
    const html = renderEvidence(evidence);
    assert.match(html, /Xerj candidates retrieved: /);
    assert.match(html, /vcmax/);
    assert.match(html, /Index ready/);
    assert.match(html, /photosynthesis.md:12/);
    assert.match(html, /Vcmax controls maximum carboxylation/);
    assert.match(html, /May support this answer: /);
    assert.match(html, /\/frozen\/docs\/photosynthesis.md/);
    assert.match(html, /not citations in the answer/);
  }
});

test('document search fallback is visible in collapsed evidence and preserves its reason', () => {
  const result = JSON.stringify({
    query: 'energy closure',
    scope: { kind: 'workspace', name: 'experiment' },
    retrieval: {
      backend: 'keyword',
      state: 'not_connected',
      fallback_reason: 'connection refused',
    },
    matches: [],
  });
  const evidence = historyEvidence('search_docs', result, 'en')!;
  assert.match(evidence.summary, /Keyword search/);
  assert.match(evidence.summary, /Fallback used · Cannot reach Xerj/);
  assert.match(renderEvidence(evidence), /connection refused/);
  const zh = historyEvidence('search_docs', result, 'zh')!;
  assert.match(zh.summary, /关键词检索/);
  assert.match(zh.summary, /已降级 · 无法连接 Xerj/);
});

test('document evidence handles missing snapshots, legacy diagnostics, and unsafe file links', () => {
  const missing = historyEvidence(
    'search_docs',
    JSON.stringify({
      query: 'vcmax',
      matches: [],
      documentation_status: 'missing',
      reason: 'No preserved snapshot',
      retrieval: { backend: 'keyword', state: 'disabled' },
    }),
    'en',
  )!;
  assert.match(missing.summary, /Document snapshot missing/);
  assert.match(renderEvidence(missing), /No preserved snapshot/);
  const legacy = historyEvidence(
    'search_docs',
    JSON.stringify({
      query: 'vcmax',
      provenance: { docs_root: '/docs' },
      matches: [
        {
          file: '../private.txt',
          path: '/private.txt',
          line: 1,
          text: 'withheld',
          may_support_current_answer: false,
        },
      ],
    }),
    'en',
  )!;
  assert.match(legacy.summary, /Not provided/);
  const html = renderEvidence(legacy);
  assert.doesNotMatch(html, /Open file/);
  assert.doesNotMatch(html, /Xerj/);
  assert.match(html, /withheld/);
  assert.equal(historyEvidence('search_docs', 'not json', 'en'), null);
  assert.equal(historyEvidence('not_search_docs', '{}', 'en'), null);
});
