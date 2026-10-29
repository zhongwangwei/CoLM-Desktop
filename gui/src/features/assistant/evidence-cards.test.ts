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

function renderBanner(review: PendingReconciliation, error = '') {
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
      language: 'en',
      t: en,
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
          identity: { state: 'verified', root: '/repos/desktop' },
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
