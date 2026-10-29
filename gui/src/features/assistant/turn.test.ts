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

describe('U05: events must belong to the pending turn', () => {
  it('accepts turn-scoped event types but not checkpoints or app-level events', () => {
    const sent = turn();
    for (const type of [
      'assistant_delta',
      'reasoning_delta',
      'tool_call',
      'tool_result',
      'approval_request',
      'ui_request',
      'usage',
      'context',
      'turn_done',
    ])
      assert.equal(eventBelongsToTurn(sent, { type }), true, type);
    // task_state 在 turn_done 之后才发；docs_index_status 是应用级后台事件；ready/exited/error
    // 都不是"这一轮被模型接受"的证据。
    for (const type of ['task_state', 'docs_index_status', 'ready', 'exited', 'error'])
      assert.equal(eventBelongsToTurn(sent, { type }), false, type);
  });

  it('prefers an explicit turn/request id and rejects stale ids', () => {
    const sent = turn(); // id = 1
    assert.equal(eventBelongsToTurn(sent, { type: 'usage', turn: 1 }), true);
    assert.equal(eventBelongsToTurn(sent, { type: 'usage', turn: '1' }), true);
    assert.equal(eventBelongsToTurn(sent, { type: 'task_state', turn: 1 }), true);
    assert.equal(eventBelongsToTurn(sent, { type: 'tool_result', turn_id: 1 }), true);
    assert.equal(eventBelongsToTurn(sent, { type: 'usage', request_id: 1 }), true);
    // 迟到旧轮事件即使类型属于轮内事件，也因 ID 不匹配而不算本轮接受。
    assert.equal(eventBelongsToTurn(sent, { type: 'assistant_delta', turn: 2 }), false);
    assert.equal(eventBelongsToTurn(sent, { type: 'tool_result', turn_id: 'old-turn' }), false);
    assert.equal(eventBelongsToTurn(sent, { type: 'usage', request_id: 99 }), false);
    // 空字符串不算提供了 ID：退回类型白名单。
    assert.equal(eventBelongsToTurn(sent, { type: 'usage', turn: '' }), true);
    assert.equal(eventBelongsToTurn(sent, { type: 'task_state', turn: '' }), false);
  });
});
