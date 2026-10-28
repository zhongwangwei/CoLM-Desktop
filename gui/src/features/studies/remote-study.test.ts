import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  remoteStudyPermissions,
  remoteStudyRecovery,
  remoteStudyReply,
  studyModelRevision,
} from './remote-study';

describe('server study lifecycle', () => {
  it('blocks duplicate submission after uncertain transport and after restart', () => {
    assert.equal(remoteStudyPermissions(null, null, null).submit, true);
    assert.equal(
      remoteStudyPermissions(null, null, { status: { state: 'unknown' }, warnings: [] }).submit,
      false,
    );
    assert.equal(
      remoteStudyPermissions(null, { host: '7920l', job: 'saved-intent' }, null).submit,
      false,
    );
    assert.equal(
      remoteStudyPermissions({ host: '7920l', job: 'saved-job' }, null, null).submit,
      false,
    );
    assert.equal(
      remoteStudyPermissions(
        null,
        { host: '7920l', job: 'intent' },
        remoteStudyReply({ status: { state: 'running' } }),
      ).cancel,
      false,
    );
    assert.equal(
      remoteStudyPermissions(
        null,
        { host: '7920l', job: 'intent' },
        remoteStudyReply({ status: { state: 'finished', exit_code: 0 } }),
      ).fetch,
      false,
    );
  });
  it('requires observed exit evidence before fetching and never calls accepted cancellation complete', () => {
    const accepted = remoteStudyReply({
      outcome: 'cancel_requested',
      request_accepted: true,
      terminal: false,
      status: { state: 'running' },
    });
    assert.equal(remoteStudyPermissions({ job: 'one' }, null, accepted).terminal, false);
    assert.equal(remoteStudyPermissions({ job: 'one' }, null, accepted).fetch, false);
    assert.equal(
      remoteStudyPermissions(
        { job: 'one' },
        null,
        remoteStudyReply({ status: { state: 'finished' } }),
      ).terminal,
      false,
    );
    assert.equal(
      remoteStudyPermissions(
        { job: 'one' },
        null,
        remoteStudyReply({ status: { state: 'finished', exit_code: 1 } }),
      ).fetch,
      true,
    );
    assert.equal(
      remoteStudyPermissions(
        { job: 'one', fetched_at: 123 },
        null,
        remoteStudyReply({ status: { state: 'finished', exit_code: 0 } }),
      ).fetch,
      false,
    );
    const lost = remoteStudyPermissions(
      { job: 'one' },
      null,
      remoteStudyReply({ status: { state: 'lost' } }),
    );
    assert.equal(lost.submit, false);
    assert.equal(lost.fetch, false);
    assert.equal(lost.cancel, false);
    // 丢失的作业可以由用户确认后关闭记录；运行中/正常结束的不行（R25）。
    assert.equal(lost.close, true);
    assert.equal(
      remoteStudyPermissions(
        null,
        { host: '7920l', job: 'intent' },
        remoteStudyReply({ status: { state: 'unknown' } }),
      ).close,
      true,
    );
    assert.equal(
      remoteStudyPermissions(
        { job: 'one' },
        null,
        remoteStudyReply({ status: { state: 'running' } }),
      ).close,
      false,
    );
    assert.equal(
      remoteStudyPermissions(
        { job: 'one' },
        null,
        remoteStudyReply({ status: { state: 'finished', exit_code: 1 } }),
      ).close,
      false,
    );
    assert.equal(
      remoteStudyPermissions(
        { job: 'one', fetched_at: 123 },
        null,
        remoteStudyReply({ status: { state: 'lost' } }),
      ).close,
      false,
    );
  });
  it('keeps credentials and unrelated configuration out of the assistant snapshot', () => {
    const parsed = remoteStudyReply({
      host: '7920l',
      record: {
        host: '7920l',
        job: 'one',
        ssh_auth: { password: 'secret', identity_file: '/private/key' },
      },
      password: 'secret',
      status: { state: 'queued' },
      ledger_warning: 'Ledger needs review',
    });
    assert.deepEqual(parsed.record, { host: '7920l', job: 'one', fetched_at: undefined });
    assert.equal(JSON.stringify(parsed).includes('secret'), false);
    assert.equal(JSON.stringify(parsed).includes('/private/key'), false);
    assert.deepEqual(parsed.warnings, ['Ledger needs review']);
  });
  it('uses the exact pinned revision for resume rather than newer selection or draft', () => {
    const selected = { id: 'modified', revision: 3 };
    assert.deepEqual(
      studyModelRevision(selected, {
        model_revision: { version: { id: 'modified', revision: 1 } },
      }),
      { id: 'modified', revision: 1 },
    );
    assert.deepEqual(studyModelRevision(selected), selected);
    assert.throws(() => studyModelRevision({ id: 'modified', revision: 0 }));
  });
  it('keeps diagnostic fetch separate from verified scientific evidence, including after restart', () => {
    assert.equal(
      remoteStudyPermissions(
        { job: 'one', fetched_at: 123 },
        null,
        remoteStudyReply({ fetched: true, scientific_verified: false }),
      ).scientificVerified,
      false,
    );
    const record = remoteStudyReply({
      record: {
        host: '7920l',
        job: 'one',
        fetched_at: 123,
        provenance: { scientific_verified: true, password: 'private' },
      },
    }).record;
    assert.equal(remoteStudyPermissions(record, null, null).scientificVerified, true);
    assert.equal(JSON.stringify(record).includes('private'), false);
    assert.equal(
      remoteStudyPermissions({ job: 'legacy', fetched_at: 123 }, null, null).scientificVerified,
      null,
    );
  });
});

describe('server study failure stages and recovery actions', () => {
  const record = (fetched = false) => ({
    host: '7920l',
    job: 'study-1',
    fetched_at: fetched ? 1_700_000_000 : undefined,
  });
  const recoveryReply = (recovery: Record<string, unknown>, status: Record<string, unknown>) =>
    remoteStudyReply({
      record: record(),
      status,
      recovery: {
        outcome: 'unknown',
        terminal: false,
        finished: false,
        study_created: false,
        fetched: false,
        permissions: {
          submit: false,
          resubmit: false,
          cancel: false,
          fetch: false,
          close: false,
          poll: true,
        },
        next_zh: '下一步',
        next_en: 'next step',
        ...recovery,
      },
    });

  it('offers resubmit, archive and a log hint for a build failure that never created the study', () => {
    const reply = recoveryReply(
      {
        outcome: 'build_failed',
        terminal: true,
        finished: true,
        study_created: false,
        exit_code: 1,
        permissions: {
          submit: false,
          resubmit: true,
          cancel: false,
          fetch: true,
          close: true,
          poll: false,
        },
        next_zh: '服务器在构建引擎阶段失败；看日志，归档关闭后重提。',
        next_en: 'The server failed while building the engine; check the log, close, resubmit.',
        fetch_error_zh: '服务器上没有研究目录（构建引擎阶段失败），没有结果可取回。',
        fetch_error_en:
          'The server has no Study directory (engine build failed); nothing to fetch.',
      },
      { state: 'finished', exit_code: 1, phase: 'preparing the engine' },
    );
    const permissions = remoteStudyPermissions(record(), null, reply);
    assert.equal(permissions.outcome, 'build_failed');
    assert.equal(permissions.submit, false);
    assert.equal(permissions.resubmit, true);
    assert.equal(permissions.close, true);
    assert.equal(permissions.fetch, true);
    assert.equal(permissions.fetchNote?.zh.includes('没有研究目录'), true);
    assert.equal(permissions.fetchNote?.en.includes('no Study directory'), true);
    assert.equal(permissions.nextStep?.zh.includes('构建'), true);
    assert.match(permissions.closeLabel?.zh ?? '', /归档失败的服务器记录/);
  });

  it('requires fetching a successful run before the record can be archived', () => {
    const succeeded = recoveryReply(
      {
        outcome: 'succeeded',
        terminal: true,
        finished: true,
        study_created: true,
        exit_code: 0,
        permissions: {
          submit: false,
          resubmit: false,
          cancel: false,
          fetch: true,
          close: false,
          poll: false,
        },
      },
      { state: 'finished', exit_code: 0, phase: 'running the Study' },
    );
    const before = remoteStudyPermissions(record(), null, succeeded);
    assert.equal(before.fetch, true);
    assert.equal(before.close, false);

    const fetched = recoveryReply(
      {
        outcome: 'succeeded',
        terminal: true,
        finished: true,
        study_created: true,
        fetched: true,
        permissions: {
          submit: false,
          resubmit: false,
          cancel: false,
          fetch: false,
          close: true,
          poll: false,
        },
      },
      { state: 'finished', exit_code: 0, phase: 'running the Study' },
    );
    const after = remoteStudyPermissions(record(true), null, fetched);
    assert.equal(after.close, true);
    assert.equal(after.fetch, false);
    assert.match(after.closeLabel?.zh ?? '', /归档已取回/);
  });

  it('never archives a live job', () => {
    const running = recoveryReply(
      {
        outcome: 'running',
        permissions: {
          submit: false,
          resubmit: false,
          cancel: true,
          fetch: false,
          close: false,
          poll: true,
        },
        next_zh: '等它结束再取回或先取消。',
        next_en: 'Wait for it to finish, or cancel first.',
      },
      { state: 'running' },
    );
    const permissions = remoteStudyPermissions(record(), null, running);
    assert.equal(permissions.close, false);
    assert.equal(permissions.resubmit, false);
    assert.equal(permissions.cancel, true);
  });

  it('falls back to the old matrix when the backend did not classify the reply', () => {
    const running = remoteStudyReply({ status: { state: 'running' } });
    const permissions = remoteStudyPermissions({ job: 'one' }, null, running);
    assert.equal(permissions.outcome, undefined);
    assert.equal(permissions.close, false);
    assert.equal(permissions.cancel, true);
    const lost = remoteStudyReply({ status: { state: 'lost' } });
    assert.equal(remoteStudyPermissions({ job: 'one' }, null, lost).close, true);
  });

  it('drops a malformed recovery instead of guessing', () => {
    assert.equal(remoteStudyRecovery({ outcome: 'exploded' }), undefined);
    assert.equal(remoteStudyRecovery('build_failed'), undefined);
    const parsed = remoteStudyRecovery({
      outcome: 'cancelled',
      permissions: { close: true },
      next_zh: '归档关闭',
      next_en: 'close',
    });
    assert.equal(parsed?.outcome, 'cancelled');
    assert.equal(parsed?.permissions.close, true);
    assert.equal(parsed?.permissions.fetch, false);
  });
});
