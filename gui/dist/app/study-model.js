//! 两个 Study 工作流共用的纯数据层：预算、状态聚合、评分和分页。
//! 不碰 DOM/Tauri，便于 Node 直接锁住不确定性分析与参数调优的核心规则。

export const STUDY_STAGES = ['mksrfdata', 'mkinidata', 'colm'];
export const MAX_STUDY_CANDIDATES = 1000;
export const TERMINAL_STATUSES = new Set(['Succeeded', 'Failed', 'Interrupted', 'NeedsReview', 'Cancelled', 'Completed', 'CompletedWithFailures']);
export const SUCCESS_STATUSES = new Set(['Succeeded', 'Completed']);
export const FAILURE_STATUSES = new Set(['Failed', 'Interrupted', 'NeedsReview']);
export const ACTIVE_STATUSES = new Set(['Running', 'Evaluating', 'Reconcile']);

export function canonicalStatus(value = 'Pending') {
  return String(value || 'Pending').replace(/([a-z0-9])([A-Z])/g, '$1_$2').split('_')
    .map(part => part ? part[0].toUpperCase() + part.slice(1).toLowerCase() : '').join('');
}

export function studyBudget({
  method = 'lhs', paramCount = 0, siteCount = 1, candidates = null,
  population = 0, generations = 0, jobs = 1, baselineSeconds = null,
} = {}) {
  const k = Math.max(0, Math.trunc(Number(paramCount) || 0));
  const sites = Math.max(0, Math.trunc(Number(siteCount) || 0));
  const algo = String(method || '').toLowerCase();
  const deCandidates = Math.max(0, Math.trunc(Number(population) || 0)) * (Math.max(0, Math.trunc(Number(generations) || 0)) + 1);
  const suggested = algo === 'oat' ? 2 * k
    : algo === 'de' ? deCandidates
    : Math.max(40, 10 * k);
  const candidateCount = candidates == null ? suggested : Math.max(0, Math.trunc(Number(candidates) || 0));
  const memberSiteTasks = sites * (candidateCount + 1);
  const stageRuns = Object.fromEntries(STUDY_STAGES.map(stage => [stage, memberSiteTasks]));
  const seconds = Number(baselineSeconds);
  return {
    method: algo || 'lhs',
    paramCount: k,
    siteCount: sites,
    candidateCount,
    suggestedCandidateCount: suggested,
    deCandidateLimit: deCandidates,
    candidateLimitExceeded: !Number.isSafeInteger(candidateCount) || candidateCount > MAX_STUDY_CANDIDATES,
    baselineTasks: sites,
    memberSiteTasks,
    stageRuns,
    totalStageRuns: memberSiteTasks * STUDY_STAGES.length,
    jobs: Math.max(1, Math.trunc(Number(jobs) || 1)),
    estimatedSeconds: Number.isFinite(seconds) ? memberSiteTasks * seconds : null,
  };
}

export function countStatuses(items = []) {
  return items.reduce((counts, item) => {
    const status = canonicalStatus(item?.status);
    counts[status] = (counts[status] || 0) + 1;
    return counts;
  }, {});
}

export function aggregateStages(stages = []) {
  const list = (Array.isArray(stages) ? stages : Object.entries(stages).map(([name, value]) => ({ name, ...value })))
    .map(item => ({ ...item, status: canonicalStatus(item?.status) }));
  const counts = countStatuses(list);
  const total = list.length;
  const succeeded = list.filter(x => SUCCESS_STATUSES.has(x.status)).length;
  const failed = list.filter(x => FAILURE_STATUSES.has(x.status)).length;
  const cancelled = list.filter(x => x.status === 'Cancelled').length;
  const running = list.filter(x => ACTIVE_STATUSES.has(x.status)).length;
  const done = succeeded + failed + cancelled;
  const status = failed ? 'Failed' : running ? 'Running' : cancelled ? 'Cancelled' : done === total && total ? 'Succeeded' : 'Pending';
  return { status, counts, total, succeeded, failed, cancelled, running, done, progress: total ? done / total : 0 };
}

export function aggregateSite(site = {}) {
  if (!(Array.isArray(site.stages) ? site.stages.length : Object.keys(site.stages || {}).length) && site.status) {
    const status = canonicalStatus(site.status);
    const succeeded = SUCCESS_STATUSES.has(status) ? 1 : 0;
    const failed = FAILURE_STATUSES.has(status) ? 1 : 0;
    const cancelled = status === 'Cancelled' ? 1 : 0;
    const running = ACTIVE_STATUSES.has(status) ? 1 : 0;
    const done = succeeded + failed + cancelled;
    return { ...site, status, total: 1, succeeded, failed, cancelled, running, done, progress: done };
  }
  const stages = aggregateStages(site.stages ?? []);
  return { ...site, ...stages };
}

export function aggregateMember(member = {}) {
  const sites = (member.sites ?? []).map(aggregateSite);
  const counts = countStatuses(sites);
  const total = sites.length;
  const succeeded = sites.filter(x => x.status === 'Succeeded').length;
  const failed = sites.filter(x => FAILURE_STATUSES.has(x.status)).length;
  const cancelled = sites.filter(x => x.status === 'Cancelled').length;
  const running = sites.filter(x => ACTIVE_STATUSES.has(x.status) || x.status === 'Running').length;
  const done = succeeded + failed + cancelled;
  const review = sites.some(x => x.status === 'NeedsReview');
  const status = review ? 'NeedsReview' : failed ? 'Failed' : running ? 'Running' : cancelled ? 'Cancelled' : done === total && total ? 'Succeeded' : 'Pending';
  return { ...member, sites, status, counts, total, succeeded, failed, cancelled, running, done, progress: total ? done / total : 0 };
}

export function aggregateStudy(study = {}) {
  const grouped = new Map();
  if (!Array.isArray(study.members) && study.tasks) {
    const tasks = Array.isArray(study.tasks) ? study.tasks : Object.values(study.tasks);
    for (const task of tasks) {
      const studyKey = task.study_key || task.study_dir || task.studyDir || '';
      const member = task.member || String(task.id || '').split('/')[0] || 'unknown';
      const key = studyKey ? `${studyKey}\u001f${member}` : member;
      if (!grouped.has(key)) grouped.set(key, { id: studyKey ? `${String(studyKey).split(/[\\/]/).pop()}/${member}` : member, member, study_key: studyKey, sites: [] });
      grouped.get(key).sites.push({ name: task.site, ...task });
    }
  }
  const members = (Array.isArray(study.members) ? study.members : [...grouped.values()]).map(aggregateMember);
  const counts = countStatuses(members);
  const total = members.length;
  const succeeded = members.filter(x => x.status === 'Succeeded').length;
  const failed = members.filter(x => FAILURE_STATUSES.has(x.status)).length;
  const cancelled = members.filter(x => x.status === 'Cancelled').length;
  const running = members.filter(x => x.status === 'Running').length;
  const done = succeeded + failed + cancelled;
  const review = members.some(x => x.status === 'NeedsReview');
  const status = running ? 'Running'
    : review ? 'NeedsReview'
    : failed ? 'CompletedWithFailures'
      : cancelled || canonicalStatus(study.status ?? 'Draft') === 'Cancelled' ? 'Cancelled'
      : done === total && total ? 'Completed' : canonicalStatus(study.status ?? 'Draft');
  return { ...study, members, status, counts, total, succeeded, failed, cancelled, running, done, progress: total ? done / total : 0 };
}

export function studyActionState(status = 'Draft', hasTask = false, localRunning = false) {
  const current = canonicalStatus(localRunning ? 'Running' : status);
  const running = hasTask && current === 'Running';
  const results = hasTask && ['Completed', 'CompletedWithFailures'].includes(current);
  return {
    run: hasTask && current === 'Ready',
    refresh: hasTask,
    retry: hasTask && ['CompletedWithFailures', 'NeedsReview', 'Failed'].includes(current),
    pause: running,
    resume: hasTask && current === 'Paused',
    cancel: hasTask && ['Running', 'Paused', 'NeedsReview'].includes(current),
    export: hasTask && !running,
    apply: results,
    results,
  };
}

export function aggregateStudyStatuses(values = []) {
  const statuses = values.filter(Boolean).map(canonicalStatus);
  return ['Running', 'NeedsReview', 'Paused', 'CompletedWithFailures', 'Ready', 'Cancelled', 'Completed']
    .find(status => statuses.includes(status)) || 'Draft';
}

export function paginate(items = [], page = 1, pageSize = 50) {
  const size = Math.max(1, Math.trunc(Number(pageSize) || 50));
  const pages = Math.max(1, Math.ceil(items.length / size));
  const current = Math.min(Math.max(1, Math.trunc(Number(page) || 1)), pages);
  const start = (current - 1) * size;
  return { page: current, pageSize: size, pages, total: items.length, start, end: Math.min(items.length, start + size), items: items.slice(start, start + size) };
}

const normalizedPath = value => {
  const path = String(value || '').replace(/\\/g, '/').replace(/\/+$/, '');
  return /^[A-Za-z]:\//.test(path) ? path.toLowerCase() : path;
};
const dirname = value => normalizedPath(value).replace(/\/[^/]*$/, '');
const studyCaseRoot = value => {
  const path = normalizedPath(value);
  const marker = '/.colm/studies/';
  const index = path.indexOf(marker);
  return index < 0 ? '' : path.slice(0, index);
};

export const studySiteId = value => normalizedPath(value?.dir).split('/').pop() || '';

export function scopedStudyDirs(dirs = [], caseDirs = []) {
  const roots = new Set(caseDirs.map(dirname).filter(Boolean));
  return roots.size ? dirs.filter(dir => roots.has(studyCaseRoot(dir))) : [];
}

export function replaceScopedStudyDirs(dirs = [], caseDirs = [], next = []) {
  const roots = new Set(caseDirs.map(dirname).filter(Boolean));
  return [...dirs.filter(dir => !roots.has(studyCaseRoot(dir))), ...next];
}

export function studyWarnings({ total = 0, failed = 0, nEff = null, minNEff = 20, failureRateWarn = 0.2 } = {}) {
  const n = Math.max(0, Number(total) || 0);
  const failures = Math.max(0, Number(failed) || 0);
  const failureRate = n ? failures / n : 0;
  const warnings = [];
  if (n && failureRate > failureRateWarn) warnings.push({ type: 'failure-rate', level: 'warning', failureRate, failed: failures, total: n });
  if (nEff != null && Number(nEff) < minNEff) warnings.push({ type: 'n-eff', level: 'warning', nEff: Number(nEff), minNEff });
  return warnings;
}
