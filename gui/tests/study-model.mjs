import { cp, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import assert from 'node:assert/strict';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-study-model-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
const moduleUrl = name => pathToFileURL(join(temp, 'app', name)).href;

const {
  STUDY_STAGES,
  MAX_STUDY_CANDIDATES,
  canonicalStatus,
  aggregateMember,
  aggregateStudy,
  aggregateStudyStatuses,
  bestTuningSummary,
  aggregateStages,
  paginate,
  percentageWindow,
  replaceScopedStudyDirs,
  scopedStudyDirs,
  studyBudget,
  studyActionState,
  studySiteId,
  studyWarnings,
} = await import(moduleUrl('study-model.js'));

assert.deepEqual(STUDY_STAGES, ['mksrfdata', 'mkinidata', 'colm']);
assert.equal(MAX_STUDY_CANDIDATES, 1000);
assert.equal(canonicalStatus('completed_with_failures'), 'CompletedWithFailures');
assert.equal(canonicalStatus('CompletedWithFailures'), 'CompletedWithFailures');
assert.equal(studyBudget({ method: 'oat', paramCount: 3, siteCount: 4 }).candidateCount, 6);
assert.equal(studyBudget({ method: 'lhs', paramCount: 3, siteCount: 4 }).suggestedCandidateCount, 40);
assert.equal(studyBudget({ method: 'lhs', paramCount: 8, siteCount: 4 }).suggestedCandidateCount, 80);
const de = studyBudget({ method: 'de', paramCount: 5, siteCount: 2, population: 12, generations: 4, jobs: 4, baselineSeconds: 10 });
assert.equal(de.deCandidateLimit, 60);
assert.equal(de.memberSiteTasks, 122); // S × (C + 1)
assert.equal(de.totalStageRuns, 366);
assert.deepEqual(de.stageRuns, { mksrfdata: 122, mkinidata: 122, colm: 122 });
assert.equal(de.estimatedSeconds, 1220);
assert.equal(de.candidateLimitExceeded, false);
assert.equal(studyBudget({ method: 'de', population: 1000, generations: 1 }).candidateLimitExceeded, true);
assert.equal(studyBudget({ method: 'oat', paramCount: 2, siteCount: 3, candidates: 7 }).memberSiteTasks, 24);

assert.equal(aggregateStages([{ status: 'Succeeded' }, { status: 'Running' }, { status: 'Pending' }]).status, 'Running');
assert.equal(aggregateStages([{ status: 'Succeeded' }, { status: 'Failed' }]).status, 'Failed');
assert.equal(aggregateStages([{ status: 'Cancelled' }]).status, 'Cancelled');
const member = aggregateMember({
  id: 'm1',
  sites: [
    { name: 'A', stages: [{ status: 'Succeeded' }, { status: 'Succeeded' }, { status: 'Succeeded' }] },
    { name: 'B', stages: [{ status: 'Succeeded' }, { status: 'Failed' }, { status: 'Pending' }] },
  ],
});
assert.equal(member.status, 'Failed');
assert.equal(member.sites[0].progress, 1);
assert.equal(member.sites[1].failed, 1);
const study = aggregateStudy({ members: [member, { sites: [{ stages: [{ status: 'Running' }] }] }] });
assert.equal(study.status, 'Running');
assert.equal(study.total, 2);
const backendStudy = aggregateStudy({
  status: 'completed_with_failures',
  tasks: {
    'm1/A': { member: 'm1', site: 'A', status: 'succeeded' },
    'm2/A': { member: 'm2', site: 'A', status: 'failed' },
  },
});
assert.equal(backendStudy.total, 2);
assert.equal(backendStudy.done, 2);
assert.equal(backendStudy.status, 'CompletedWithFailures');
const readyStudy = aggregateStudy({
  status: 'ready',
  tasks: {
    'm1/A': { member: 'm1', site: 'A', status: 'materialized' },
    'm2/A': { member: 'm2', site: 'A', status: 'queued' },
  },
});
assert.equal(readyStudy.running, 0);
assert.equal(readyStudy.status, 'Ready');
const cancelledStudy = aggregateStudy({
  status: 'cancelled',
  tasks: { 'm1/A': { member: 'm1', site: 'A', status: 'cancelled' } },
});
assert.equal(cancelledStudy.done, 1);
assert.equal(cancelledStudy.status, 'Cancelled');
assert.equal(aggregateStudy({
  status: 'paused',
  tasks: {
    'm1/A': { member: 'm1', site: 'A', status: 'failed' },
    'm2/A': { member: 'm2', site: 'A', status: 'queued' },
  },
}).status, 'Paused');
const deManifest = { spec: { kind: 'tuning', method: 'differential-evolution', budget: { population: 4, generations: 2 } } };
const deTasks = Object.fromEntries(['m000000', 'm000001', 'm000002', 'm000003', 'm000004']
  .map(member => [`${member}/A`, { member, site: 'A', status: 'succeeded' }]));
const runningDe = aggregateStudy({
  manifest: deManifest,
  state: {
    status: 'running',
    completed_candidates: 4,
    candidates: { m000000: { feasible: true, calibration: 1 } },
    tasks: deTasks,
  },
});
assert.equal(runningDe.status, 'Running');
assert.equal(runningDe.done, 5);
assert.equal(runningDe.total, 13);
assert.ok(runningDe.progress < 1, 'DE progress must not reach 100% before future generations are done');
const endedEarlyDe = aggregateStudy({ manifest: deManifest, state: { status: 'completed', completed_candidates: 4, tasks: deTasks } });
assert.deepEqual({ done: endedEarlyDe.done, total: endedEarlyDe.total, progress: endedEarlyDe.progress }, { done: 5, total: 13, progress: 1 });
assert.deepEqual(percentageWindow(0, 8 * 86400, 0, 75, 86400), { from: 0, to: 6 * 86400 });
assert.deepEqual(percentageWindow(0, 8 * 86400, 75, 100, 86400), { from: 6 * 86400, to: 8 * 86400 });
assert.throws(() => percentageWindow(0, 86400, 0, 1, 86400), /shorter/);
const best = bestTuningSummary({
  state: {
    best_member: 'm000001',
    candidates: {
      m000000: { generation: 0, feasible: true, calibration: 2, validation: 3 },
      m000001: { generation: 1, feasible: true, calibration: 1, validation: 2.5 },
    },
  },
});
assert.deepEqual({ member: best.member, generation: best.generation, calibration: best.calibration }, { member: 'm000001', generation: 1, calibration: 1 });
assert.equal(aggregateStudy({
  status: 'cancelled',
  tasks: {
    'm1/A': { member: 'm1', site: 'A', status: 'failed' },
    'm2/A': { member: 'm2', site: 'A', status: 'cancelled' },
  },
}).status, 'Cancelled');
assert.deepEqual(studyActionState('draft', false), {
  run: false, refresh: false, retry: false, pause: false, resume: false,
  cancel: false, export: false, apply: false, results: false,
});
assert.deepEqual(studyActionState('ready', true), {
  run: true, refresh: true, retry: false, pause: false, resume: false,
  cancel: false, export: true, apply: false, results: false,
});
assert.deepEqual(studyActionState('running', true), {
  run: false, refresh: true, retry: false, pause: true, resume: false,
  cancel: true, export: false, apply: false, results: false,
});
assert.deepEqual(studyActionState('paused', true), {
  run: false, refresh: true, retry: false, pause: false, resume: true,
  cancel: true, export: true, apply: false, results: false,
});
assert.equal(studyActionState('needs_review', true).run, true);
assert.equal(studyActionState('needs_review', true).cancel, true);
assert.deepEqual(studyActionState('completed_with_failures', true), {
  run: false, refresh: true, retry: true, pause: false, resume: false,
  cancel: false, export: true, apply: true, results: true,
});
assert.equal(studyActionState('completed', true).results, true);
assert.equal(studyActionState('cancelled', true).results, false);
assert.equal(aggregateStudyStatuses(['ready', 'ready']), 'Ready');
assert.equal(aggregateStudyStatuses(['completed', 'ready']), 'Ready');
assert.equal(aggregateStudyStatuses(['ready', 'running']), 'Running');
assert.equal(aggregateStudyStatuses(['completed', 'completed_with_failures']), 'CompletedWithFailures');
assert.equal(aggregateStudyStatuses(['completed_with_failures', 'cancelled']), 'Cancelled');
assert.equal(aggregateStudyStatuses(['completed_with_failures', 'paused']), 'Paused');
const multiStudy = aggregateStudy({
  tasks: {
    's1/m000000/A': { study_dir: '/cases/.colm/studies/s1', member: 'm000000', site: 'A', status: 'succeeded' },
    's2/m000000/B': { study_dir: '/cases/.colm/studies/s2', member: 'm000000', site: 'B', status: 'failed' },
  },
});
assert.equal(multiStudy.total, 2);
assert.equal(multiStudy.members.map(m => m.id).sort().join('|'), 's1/m000000|s2/m000000');

const page = paginate(['a', 'b', 'c', 'd', 'e'], 3, 2);
assert.deepEqual(page.items, ['e']);
assert.equal(page.pages, 3);
assert.equal(paginate(['a'], 99, 10).page, 1);

const savedStudies = [
  '/cases-a/.colm/studies/s-a',
  '/cases-b/.colm/studies/s-b',
  'C:\\Cases C\\.colm\\studies\\s-c',
];
assert.deepEqual(scopedStudyDirs(savedStudies, ['/cases-a/site']), [savedStudies[0]]);
assert.deepEqual(scopedStudyDirs(savedStudies, ['c:\\cases c\\site']), [savedStudies[2]]);
assert.deepEqual(
  replaceScopedStudyDirs(savedStudies, ['/cases-a/site'], ['/cases-a/.colm/studies/s-new']),
  [savedStudies[1], savedStudies[2], '/cases-a/.colm/studies/s-new'],
);
assert.equal(studySiteId({ dir: '/cases/case-a', name: 'same alias' }), 'case-a');
assert.equal(studySiteId({ dir: 'C:\\Cases\\case-b\\', name: 'same alias' }), 'case-b');

assert.deepEqual(studyWarnings({ total: 10, failed: 3, nEff: 8, minNEff: 20 }).map(w => w.type), ['failure-rate', 'n-eff']);
assert.deepEqual(studyWarnings({ total: 10, failed: 2, nEff: 20 }), []);

console.log('study-model: budget, backend-state aggregation, pagination, and warnings are stable');
