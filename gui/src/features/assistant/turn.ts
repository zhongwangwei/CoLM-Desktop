/**
 * 一轮"发送"在前端的状态机（GUI-18）。
 *
 * 为什么不能把 `assistant_send` 的 IPC 成功当成"模型已接受"：IPC 只证明消息写进了助手进程的 stdin，
 * 而"会话到上限 / 上一条还在答 / 没有 Key"这类拒绝发生在同一时刻的前后两个方向 —— 拒绝事件可能比
 * IPC 确认先到，也可能后到。只有后端产出了这一轮自己的事件（增量、工具、用量、结束）才算被接受；
 * 没有产出的才允许撤回气泡、恢复草稿。
 *
 * 状态流转：
 *   preparing  --assistant_send 确认--> delivered --后端这一轮的事件--> accepted
 *       \                                \
 *        \--轮次前拒绝（事件）--> rejected（撤回气泡 + 恢复草稿）
 *         \--IPC 自身失败--> failed（撤回气泡，草稿本来就没清）
 *   accepted 之后再收到"轮次前拒绝"只说明后端报错，消息其实已被记下：不撤回、不恢复。
 */
export type SendPhase = 'preparing' | 'delivered' | 'accepted' | 'rejected' | 'failed';

export interface PendingTurn<A> {
  /** 前端轮次序号；迟到的确认/错误靠它识别是不是当前轮。 */
  id: number;
  /** 用户实际打的字（不含附件正文），恢复草稿用。 */
  typed: string;
  /** 实际发出的提示词：没有打字时是"请阅读附件"的兜底句，输入历史记这一句。 */
  prompt: string;
  attachments: A[];
  phase: SendPhase;
}

export function newTurn<A>(
  id: number,
  typed: string,
  prompt: string,
  attachments: A[],
): PendingTurn<A> {
  // 附件数组必须快照：发送后界面可能清空或替换它，恢复草稿要的还是这一轮发出去的内容。
  return { id, typed, prompt, attachments: [...attachments], phase: 'preparing' };
}

/** 这一轮自己的业务事件：后端已经开始/完成这一轮，消息确实被记下了。 */
export function turnOutput<A>(turn: PendingTurn<A>): PendingTurn<A> {
  return turn.phase === 'preparing' || turn.phase === 'delivered'
    ? { ...turn, phase: 'accepted' }
    : turn;
}

/** IPC 确认只把 preparing 推进到 delivered；已经 rejected/failed 的轮次不能被"确认"洗掉。 */
export function turnDelivered<A>(turn: PendingTurn<A>): PendingTurn<A> {
  return turn.phase === 'preparing' ? { ...turn, phase: 'delivered' } : turn;
}

/**
 * 轮次前拒绝：只有还没被模型接受的轮次才能撤回气泡并恢复草稿。
 * 已经被接受（后端已经产出）的轮次收到同样的错误文字，只能当运行错误显示。
 */
export function turnPreTurnRejected<A>(turn: PendingTurn<A>): {
  turn: PendingTurn<A>;
  restore: boolean;
} {
  if (turn.phase === 'preparing' || turn.phase === 'delivered')
    return { turn: { ...turn, phase: 'rejected' }, restore: true };
  return { turn: { ...turn, phase: 'failed' }, restore: false };
}

export function turnFailed<A>(turn: PendingTurn<A>): PendingTurn<A> {
  return { ...turn, phase: 'failed' };
}

/**
 * IPC 确认后是否该清空输入/附件：只有真正送达且没被拒绝的轮次才清。
 * 拒绝事件先到时草稿已经被恢复，这里必须返回 false，否则恢复的内容又被清掉（GUI-18）。
 */
export function clearsDraftAfterAck<A>(turn: PendingTurn<A>): boolean {
  return turn.phase === 'delivered' || turn.phase === 'accepted';
}
