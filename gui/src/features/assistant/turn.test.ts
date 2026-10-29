import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  clearsDraftAfterAck,
  eventBelongsToTurn,
  newTurn,
  turnDelivered,
  turnFailed,
  turnOutput,
  turnPreTurnRejected,
  type PendingTurn,
} from './turn';

const turn = (): PendingTurn<{ name: string }> =>
  newTurn(1, 'hello', 'hello', [{ name: 'paper.pdf' }]);

describe('assistant send turn state machine', () => {
  it('keeps a private attachment snapshot so clearing the composer cannot empty the restore', () => {
    const attachments = [{ name: 'paper.pdf' }];
    const sent = newTurn(1, '', 'summarize', attachments);
    attachments.length = 0;
    assert.deepEqual(sent.attachments, [{ name: 'paper.pdf' }]);
    assert.equal(sent.phase, 'preparing');
  });

  it('treats backend output, not the IPC ack, as acceptance', () => {
    const acked = turnDelivered(turn());
    assert.equal(acked.phase, 'delivered');
    assert.equal(clearsDraftAfterAck(acked), true);
    const accepted = turnOutput(acked);
    assert.equal(accepted.phase, 'accepted');
    assert.equal(turnOutput(accepted), accepted, 'repeated output keeps the accepted phase');
  });

  it('a pre-turn rejection restores only while the message was never accepted', () => {
    // 拒绝先到（此时可能还没收到 IPC 确认）、或 IPC 确认先到，两种顺序都允许恢复。
    for (const phase of ['preparing', 'delivered'] as const) {
      const sent = { ...turn(), phase };
      const outcome = turnPreTurnRejected(sent);
      assert.deepEqual(outcome, { turn: { ...sent, phase: 'rejected' }, restore: true });
      assert.equal(
        clearsDraftAfterAck(outcome.turn),
        false,
        'a late ack must not clear the restore',
      );
    }
    // 已经产出过这一轮的输出：同样的文字只是运行错误，不能再把用户消息当成没发出去。
    const accepted = { ...turn(), phase: 'accepted' as const };
    const late = turnPreTurnRejected(accepted);
    assert.equal(late.restore, false);
    assert.equal(late.turn.phase, 'failed');
  });

  it('an IPC failure keeps the bubble out of the transcript and never clears the draft', () => {
    const failed = turnFailed(turnDelivered(turn()));
    assert.equal(failed.phase, 'failed');
    assert.equal(clearsDraftAfterAck(failed), false);
  });
});

describe('A3: only a matching turn_started accepts the pending turn', () => {
  it('accepts only a turn_started carrying the pending request id', () => {
    const sent = newTurn(1, 'hello', 'hello', [], 'turn-1-abc');
    assert.equal(
      eventBelongsToTurn(sent, { type: 'turn_started', request_id: 'turn-1-abc' }),
      true,
    );
    assert.equal(eventBelongsToTurn(sent, { type: 'turn_started', requestId: 'turn-1-abc' }), true);
    // 迟到旧轮的 turn_started 不匹配：不能把新消息标成已接受。
    assert.equal(
      eventBelongsToTurn(sent, { type: 'turn_started', request_id: 'turn-0-old' }),
      false,
    );
    assert.equal(eventBelongsToTurn(sent, { type: 'turn_started' }), false, '没有 request id 不算');
    assert.equal(eventBelongsToTurn(sent, { type: 'turn_started', request_id: 1 }), false);
    // 其它事件即使带匹配 id 也不再标记接受（严格模式，旧白名单已删除）。
    for (const type of [
      'usage',
      'context',
      'tool_result',
      'assistant_delta',
      'task_state',
      'turn_done',
      'ready',
      'exited',
    ])
      assert.equal(eventBelongsToTurn(sent, { type, request_id: 'turn-1-abc' }), false, type);
  });

  it('never accepts when the pending turn has no request id (old agent / malformed send)', () => {
    const legacy = turn();
    assert.equal(legacy.requestId, '');
    assert.equal(eventBelongsToTurn(legacy, { type: 'turn_started', request_id: 'turn-1' }), false);
    assert.equal(eventBelongsToTurn(legacy, { type: 'turn_started', request_id: '' }), false);
    // 空字符串 request id 的 pending 也不能被任何事件“匹配”成接受。
    const empty = newTurn(1, 'hello', 'hello', [], '  ');
    assert.equal(eventBelongsToTurn(empty, { type: 'turn_started', request_id: '  ' }), false);
  });
});
