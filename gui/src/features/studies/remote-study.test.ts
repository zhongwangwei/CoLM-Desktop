import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { remoteStudyPermissions, remoteStudyReply, studyModelRevision } from './remote-study';

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
