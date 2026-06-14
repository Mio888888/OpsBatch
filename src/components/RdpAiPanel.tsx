import { memo, useEffect, useMemo, useRef, useState, useCallback } from 'react';
import type { FC } from 'react';
import {
  useAiChatStore,
  selectMessages,
  selectInputText,
  selectStreamingMessageId,
  selectActiveConversationId,
  type ChatMessage,
  type ChatMessageImage,
  type PendingAction,
} from '../stores/aiChat';
import type { RdpOperation } from '../utils/aiActionParser';
import { useTranslation } from '../i18n';

interface RdpAiPanelProps {
  hostId: string;
  hostName?: string;
  rdpSessionId: string | null;
  desktopWidth: number;
  desktopHeight: number;
  executeRdpOperations: (ops: RdpOperation[]) => Promise<void>;
  getRdpScreenshot?: () => string | null;
  onClose?: () => void;
}

function describeOperation(op: RdpOperation): string {
  switch (op.type) {
    case 'click': {
      const btn = op.button === 2 ? '右键' : op.button === 1 ? '中键' : '左键';
      return op.doubleClick ? `双击 (${op.x}, ${op.y})` : `${btn}点击 (${op.x}, ${op.y})`;
    }
    case 'move':
      return `移动到 (${op.x}, ${op.y})`;
    case 'drag':
      return `拖拽 (${op.fromX}, ${op.fromY}) → (${op.toX}, ${op.toY})`;
    case 'scroll':
      return `${op.vertical ? '垂直' : '水平'}滚动 ${op.delta > 0 ? '↑' : '↓'} ${Math.abs(op.delta)} (${op.x}, ${op.y})`;
    case 'type':
      return `输入文本: ${op.text.length > 40 ? `${op.text.slice(0, 37)}...` : op.text}`;
    case 'key':
      return `按键: ${op.keys.join(' + ')}`;
    case 'wait':
      return `等待 ${Math.round(op.ms / 100) / 10} 秒`;
    default:
      return '未知操作';
  }
}

// 流式输出时实时剥离 <RDP_PLAN> 块，避免把原始 JSON 显示给用户。
// 剥离逻辑与 aiActionParser 的提取保持一致：匹配完整标签或未闭合的开标签到末尾。
function stripRdpPlanBlock(content: string): string {
  // 完整闭合标签
  let result = content.replace(/<RDP_PLAN\s*>?[\s\S]*?<\/RDP_PLAN?>?/gi, '');
  // 未闭合的开标签（流式截断）：从 <RDP_PLAN> 到字符串末尾
  result = result.replace(/<RDP_PLAN\s*>?[\s\S]*$/i, '');
  // 不完整闭合标签残留（例如 </RDP、</RDP_PLAN）
  result = result.replace(/<\/RDP(?:_PLAN?)?>?\s*$/i, '');
  return result.replace(/^[\s\n]+|[\s\n]+$/g, '');
}

function buildRdpAgentContext(width: number, height: number): string {
  return [
    '你是一个 Windows 远程桌面操作助手（agent-rdp）。',
    `当前远程桌面分辨率：${width}x${height}，坐标原点 (0,0) 在左上角。`,
    '当需要操作远程桌面时，必须输出一个完整的 <RDP_PLAN> JSON 块，不要输出空对象，不要省略 steps。',
    '格式如下：',
    '<RDP_PLAN>',
    '{',
    '  "version": 1,',
    '  "summary": "操作摘要",',
    '  "steps": [',
    '    {',
    '      "description": "步骤描述",',
    '      "intent": "click|type|key|scroll|navigate|launch|operate|interact|wait",',
    '      "operations": [',
    '        {"type": "click", "x": 100, "y": 200},',
    '        {"type": "type", "text": "notepad"},',
    '        {"type": "key", "keys": ["Enter"]},',
    '        {"type": "key", "keys": ["Control", "s"]},',
    '        {"type": "scroll", "x": 400, "y": 300, "delta": -3},',
    '        {"type": "drag", "fromX": 10, "fromY": 10, "toX": 500, "toY": 500},',
    '        {"type": "wait", "ms": 1500}',
    '      ],',
    '      "expected_outcome": "预期结果"',
    '    }',
    '  ]',
    '}',
    '</RDP_PLAN>',
    '操作类型说明：',
    '- click: 单击坐标(x,y)，可选 button(0左/1中/2右)、doubleClick',
    '- move: 移动鼠标到坐标',
    '- drag: 从(fromX,fromY)拖拽到(toX,toY)',
    '- scroll: 在(x,y)滚动，delta正数向上、负数向下',
    '- type: 输入文本',
    '- key: 按键，单键如 ["Enter"]，组合键如 ["Control","c"]',
    '- wait: 等待指定毫秒数，如 {"type":"wait","ms":1500}；不要把 Wait 写成 key',
    '支持按键：Enter, Tab, Escape, Backspace, Delete, Insert, Home, End, PageUp, PageDown, ArrowUp/Down/Left/Right, Space, Shift, Control, Alt, Meta, Win, F1-F12 等。',
    '启动或打开应用（例如 Microsoft Edge、cmd、notepad）时，优先使用键盘路径：按 Win/Meta 打开开始菜单，搜索或 type 输入应用名/命令，再按 Enter；也可用 Win+R 运行框。不要优先依赖桌面图标坐标双击，除非用户明确要求点击当前屏幕上的某个图标。',
    '普通字符直接用 type 输入。坐标必须基于实际分辨率，不要超出范围。',
    '如果当前目标需要多步完成，只给出下一步可执行操作；执行后会继续检查。',
    '每个 step 必须包含非空 operations，不能只描述不操作。',
  ].join('\n');
}

const RdpMessageContent: FC<{ msg: ChatMessage }> = memo(({ msg }) => {
  const { t } = useTranslation();
  if (msg.role === 'system') return null;
  const isUser = msg.role === 'user';
  const displayContent = isUser ? msg.content : stripRdpPlanBlock(msg.content);
  const isEmpty = !displayContent;
  const hasActions = (msg.pendingActions?.length ?? 0) > 0;
  // AI 输出了 RDP_PLAN 但解析失败（无操作卡片）：给用户明确反馈
  const hasRdpPlanBlock = !isUser && /<RDP_PLAN/i.test(msg.content);
  const parseError = !isUser && !msg.streaming && !hasActions
    ? (msg.rdpParseError ?? (hasRdpPlanBlock && isEmpty ? t('rdp.aiParseFailed') : null))
    : null;
  const parseFailed = Boolean(parseError);

  return (
    <div className={`rdp-ai-msg ${isUser ? 'rdp-ai-msg-user' : 'rdp-ai-msg-assistant'}`}>
      <div className="rdp-ai-msg-role">{isUser ? '我' : 'AI'}</div>
      {displayContent && <div className="rdp-ai-msg-text">{displayContent}</div>}
      {isEmpty && msg.streaming && (
        <div className="rdp-ai-msg-thinking">
          <span className="rdp-ai-msg-thinking-dots" />
          {t('rdp.aiThinking')}
        </div>
      )}
      {isEmpty && !msg.streaming && hasActions && (
        <div className="rdp-ai-msg-plan-ready">{t('rdp.aiPlanReady')}</div>
      )}
      {parseFailed && parseError && (
        <div className="rdp-ai-msg-error">
          ⚠ {t('rdp.aiParseFailed')}
          {msg.rdpParseError && (
            <div className="rdp-ai-msg-error-detail">{msg.rdpParseError}</div>
          )}
        </div>
      )}
      {msg.streaming && !isEmpty && <span className="rdp-ai-msg-cursor" />}
    </div>
  );
});
RdpMessageContent.displayName = 'RdpMessageContent';

const RdpActionCard: FC<{
  action: PendingAction;
  onApprove: (id: string) => Promise<void>;
  onReject: (id: string) => void;
  disabled: boolean;
  batchExecutingId?: string | null;
}> = memo(({ action, onApprove, onReject, disabled, batchExecutingId }) => {
  const { t } = useTranslation();
  const [executing, setExecuting] = useState(false);
  const inBatchQueue = batchExecutingId === action.id;
  const showRunning = executing || inBatchQueue;

  const handleApprove = useCallback(() => {
    if (disabled || executing || inBatchQueue) return;
    setExecuting(true);
    void onApprove(action.id).finally(() => setExecuting(false));
  }, [action.id, disabled, executing, inBatchQueue, onApprove]);

  const ops = action.rdpOperations ?? [];

  return (
    <div className={`rdp-ai-action ${action.executed ? 'rdp-ai-action-done' : ''} ${action.rejected ? 'rdp-ai-action-rejected' : ''}`}>
      <div className="rdp-ai-action-header">
        <span className="rdp-ai-action-badge">RDP</span>
        <span className="rdp-ai-action-desc">{action.description}</span>
      </div>
      {ops.length > 0 && (
        <ul className="rdp-ai-action-ops">
          {ops.map((op, idx) => (
            <li key={idx}>{describeOperation(op)}</li>
          ))}
        </ul>
      )}
      {action.intent && (
        <div className="rdp-ai-action-meta">
          <span className="rdp-ai-action-tag">{action.intent}</span>
          {action.expectedOutcome && <span className="rdp-ai-action-outcome">{action.expectedOutcome}</span>}
        </div>
      )}
      <div className="rdp-ai-action-footer">
        {action.executed && <span className="rdp-ai-action-status rdp-ai-action-status-done">✓ {t('ai.executed')}</span>}
        {action.rejected && <span className="rdp-ai-action-status rdp-ai-action-status-rejected">✗ {t('ai.rejectTitle')}</span>}
        {showRunning && !action.executed && (
          <span className="rdp-ai-action-status rdp-ai-action-status-running">
            <span className="rdp-ai-action-dots" /> {t('ai.executing')}
          </span>
        )}
        {!action.executed && !action.rejected && !showRunning && (
          <div className="rdp-ai-action-buttons">
            <button className="rdp-ai-btn rdp-ai-btn-approve" onClick={handleApprove}>
              ✓ {t('ai.approve')}
            </button>
            <button className="rdp-ai-btn rdp-ai-btn-reject" onClick={() => onReject(action.id)}>
              ✗ {t('ai.reject')}
            </button>
          </div>
        )}
      </div>
    </div>
  );
});
RdpActionCard.displayName = 'RdpActionCard';

const RdpAiPanel: FC<RdpAiPanelProps> = ({
  hostId,
  rdpSessionId,
  desktopWidth,
  desktopHeight,
  executeRdpOperations,
  getRdpScreenshot,
  onClose,
}) => {
  const { t, tText } = useTranslation();
  const sessionKey = `rdp-${hostId}`;
  const messages = useAiChatStore(selectMessages);
  const inputText = useAiChatStore(selectInputText);
  const streamingMessageId = useAiChatStore(selectStreamingMessageId);
  const activeConversationId = useAiChatStore(selectActiveConversationId);
  const conversations = useAiChatStore((s) => s.conversations);
  const streaming = Boolean(streamingMessageId);

  const activateSession = useAiChatStore((s) => s.activateSession);
  const setInputText = useAiChatStore((s) => s.setInputText);
  const sendMessage = useAiChatStore((s) => s.sendMessage);
  const sendDirectMessage = useAiChatStore((s) => s.sendDirectMessage);
  const rejectAction = useAiChatStore((s) => s.rejectAction);
  const approveAction = useAiChatStore((s) => s.approveAction);
  const initStreamListener = useAiChatStore((s) => s.initStreamListener);
  const loadConversations = useAiChatStore((s) => s.loadConversations);
  const openConversation = useAiChatStore((s) => s.openConversation);
  const newConversation = useAiChatStore((s) => s.newConversation);
  const deleteConversation = useAiChatStore((s) => s.deleteConversation);
  const setRdpContext = useAiChatStore((s) => s.setRdpContext);
  const setRdpExecutor = useAiChatStore((s) => s.setRdpExecutor);

  const [showHistory, setShowHistory] = useState(false);
  const [batchExecutingId, setBatchExecutingId] = useState<string | null>(null);
  const [goalMode, setGoalMode] = useState(false);
  const [goalText, setGoalText] = useState('');
  const [goalRunning, setGoalRunning] = useState(false);
  const [goalRound, setGoalRound] = useState(0);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const executorRef = useRef(executeRdpOperations);
  executorRef.current = executeRdpOperations;

  // 目标模式运行时状态（用 ref 避免 effect 闭包陈旧）
  const goalActiveRef = useRef(false);
  const goalTargetRef = useRef('');
  const goalRoundRef = useRef(0);
  const goalLoopBusyRef = useRef(false);

  const batchExecuting = batchExecutingId !== null;
  const GOAL_MAX_ROUNDS = 8;

  // 激活 RDP 会话
  useEffect(() => {
    activateSession(sessionKey, { scope: 'rdp_host', scopeId: hostId });
    void initStreamListener();
  }, [activateSession, sessionKey, hostId, initStreamListener]);

  // 注入 RDP 上下文与执行器
  useEffect(() => {
    setRdpContext({
      width: desktopWidth,
      height: desktopHeight,
      sessionId: rdpSessionId ?? undefined,
    });
  }, [desktopWidth, desktopHeight, rdpSessionId, setRdpContext]);

  useEffect(() => {
    const executor = async (operations: RdpOperation[]) => {
      await executorRef.current(operations);
    };
    setRdpExecutor(executor);
    return () => setRdpExecutor(null);
  }, [setRdpExecutor]);

  useEffect(() => {
    void loadConversations({ scope: 'rdp_host', scopeId: hostId });
  }, [hostId, loadConversations]);

  // 自动滚动到底部
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
  }, [messages]);

  const allActions = useMemo(
    () => messages.flatMap((m) => m.pendingActions ?? []).filter((a) => !a.rejected),
    [messages],
  );
  const pendingActions = useMemo(
    () => allActions.filter((a) => !a.executed),
    [allActions],
  );
  const rdpActions = useMemo(
    () => allActions.filter((a) => a.type === 'rdp_action'),
    [allActions],
  );

  const getScreenshotImages = useCallback((): ChatMessageImage[] | undefined => {
    const dataUrl = getRdpScreenshot?.();
    return dataUrl ? [{ dataUrl }] : undefined;
  }, [getRdpScreenshot]);

  const handleSend = useCallback(() => {
    if (streaming || !inputText.trim()) return;
    const context = buildRdpAgentContext(desktopWidth, desktopHeight);
    void sendMessage(context, getScreenshotImages());
  }, [desktopHeight, desktopWidth, getScreenshotImages, inputText, sendMessage, streaming]);

  const handleApproveAction = useCallback(
    (actionId: string) => {
      return approveAction(actionId, rdpSessionId ?? undefined);
    },
    [approveAction, rdpSessionId],
  );

  const handleApproveAll = useCallback(async () => {
    // 快照当前待执行的 RDP 操作（按顺序）
    const queue = rdpActions.filter((a) => !a.executed && !a.rejected);
    if (queue.length === 0) return;
    for (const action of queue) {
      setBatchExecutingId(action.id);
      try {
        await approveAction(action.id, rdpSessionId ?? undefined);
      } catch {
        // 单步失败不中断队列，继续下一步
      }
    }
    setBatchExecutingId(null);
  }, [approveAction, rdpActions, rdpSessionId]);

  const sendGoalContextMessage = useCallback(async (target: string, round: number, check: boolean) => {
    const prompt = check
      ? `目标：${target}\n刚才执行了第 ${round} 轮操作，并已附加当前 RDP 截图。请先根据截图判断目标是否已经完成。如果已完成，直接回复"目标已完成"并简要说明；如果未完成，请输出下一步完整 <RDP_PLAN>。`
      : `目标：${target}\n已附加当前 RDP 截图。请根据截图给出实现该目标的第一步操作计划。必须输出完整 <RDP_PLAN> JSON，steps 和 operations 不能为空。步骤要精确，坐标基于 ${desktopWidth}x${desktopHeight} 分辨率。`;
    void sendDirectMessage(prompt, buildRdpAgentContext(desktopWidth, desktopHeight), getScreenshotImages());
  }, [desktopHeight, desktopWidth, getScreenshotImages, sendDirectMessage]);

  const handleStartGoal = useCallback(() => {
    const target = goalText.trim();
    if (!target || streaming) return;
    goalActiveRef.current = true;
    goalTargetRef.current = target;
    goalRoundRef.current = 0;
    goalLoopBusyRef.current = false;
    setGoalRunning(true);
    setGoalRound(0);
    setGoalText('');
    void sendGoalContextMessage(target, 0, false);
  }, [goalText, sendGoalContextMessage, streaming]);

  const handleStopGoal = useCallback(() => {
    goalActiveRef.current = false;
    goalLoopBusyRef.current = false;
    setGoalRunning(false);
  }, []);

  // 目标模式循环：当有新的待执行操作且目标模式运行中时，自动审批执行
  useEffect(() => {
    if (!goalActiveRef.current) return;
    if (goalLoopBusyRef.current) return;
    const pendingRdpActions = rdpActions.filter((a) => !a.executed && !a.rejected);
    if (pendingRdpActions.length === 0) return;

    // 自动执行所有待执行操作
    goalLoopBusyRef.current = true;
    void (async () => {
      try {
        for (const action of pendingRdpActions) {
          if (!goalActiveRef.current) return;
          setBatchExecutingId(action.id);
          try {
            await approveAction(action.id, rdpSessionId ?? undefined);
          } catch {
            // 忽略单步错误继续
          }
        }
        setBatchExecutingId(null);

        if (!goalActiveRef.current) return;
        // 等待操作在远程桌面生效
        await new Promise<void>((resolve) => setTimeout(resolve, 2500));

        if (!goalActiveRef.current) return;
        // 进入下一轮检查
        goalRoundRef.current += 1;
        setGoalRound(goalRoundRef.current);
        if (goalRoundRef.current >= GOAL_MAX_ROUNDS) {
          goalActiveRef.current = false;
          setGoalRunning(false);
          return;
        }
        void sendGoalContextMessage(goalTargetRef.current, goalRoundRef.current, true);
      } finally {
        goalLoopBusyRef.current = false;
        setBatchExecutingId(null);
      }
    })();
  }, [rdpActions, approveAction, rdpSessionId, sendGoalContextMessage]);

  // 目标模式：AI 检查轮回复不含 RDP_PLAN（无新操作）时判定完成
  useEffect(() => {
    if (!goalActiveRef.current || streaming) return;
    if (messages.length === 0) return;
    const lastMsg = messages[messages.length - 1];
    // 最后一条是 AI 回复、流式已结束、且没有 pendingActions（说明没有生成新操作）
    if (
      lastMsg.role === 'assistant'
      && !lastMsg.streaming
      && !(lastMsg.pendingActions?.length)
    ) {
      const timer = setTimeout(() => {
        if (!goalActiveRef.current) return;
        goalActiveRef.current = false;
        setGoalRunning(false);
      }, 1500);
      return () => clearTimeout(timer);
    }
  }, [messages, streaming]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  return (
    <div className="rdp-ai-panel">
      <div className="rdp-ai-toolbar">
        <span className="rdp-ai-toolbar-title">
          {tText('rdp.aiTitle')}
        </span>
        {rdpSessionId && (
          <span className="rdp-ai-toolbar-status">
            {desktopWidth}×{desktopHeight}
          </span>
        )}
        <button
          className={`rdp-ai-toolbar-btn ${goalMode ? 'rdp-ai-toolbar-btn-active' : ''}`}
          onClick={() => { setGoalMode((v) => !v); if (goalRunning) handleStopGoal(); }}
          title={tText('rdp.aiGoalMode')}
        >
          🎯
        </button>
        <button
          className="rdp-ai-toolbar-btn"
          onClick={() => newConversation()}
          title={tText('ai.newChat')}
        >
          ＋
        </button>
        <button
          className="rdp-ai-toolbar-btn"
          onClick={() => setShowHistory((value) => !value)}
          title={tText('ai.chatHistory')}
        >
          ☰
        </button>
        {onClose && (
          <button
            className="rdp-ai-toolbar-btn rdp-ai-toolbar-close"
            onClick={onClose}
            title={tText('rdp.aiClose')}
          >
            ✕
          </button>
        )}
      </div>

      {showHistory && (
        <div className="rdp-ai-history">
          {conversations.length === 0 && (
            <div className="rdp-ai-history-empty">{t('ai.noHistory')}</div>
          )}
          {conversations.map((conv) => (
            <div
              key={conv.id}
              className={`rdp-ai-history-item ${conv.id === activeConversationId ? 'rdp-ai-history-active' : ''}`}
              onClick={() => {
                void openConversation(conv.id);
                setShowHistory(false);
              }}
            >
              <span className="rdp-ai-history-title">{conv.title || t('ai.noTitle')}</span>
              <button
                className="rdp-ai-history-delete"
                onClick={(e) => {
                  e.stopPropagation();
                  void deleteConversation(conv.id);
                }}
              >
                ✗
              </button>
            </div>
          ))}
        </div>
      )}

      {goalMode && (
        <div className="rdp-ai-goal-bar">
          {goalRunning ? (
            <div className="rdp-ai-goal-running-info">
              <span className="rdp-ai-goal-label">🎯 {tText('rdp.aiGoalRunning')}</span>
              <span className="rdp-ai-goal-round">{tText('rdp.aiGoalRound', { round: goalRound, max: GOAL_MAX_ROUNDS })}</span>
              <button className="rdp-ai-goal-stop" onClick={handleStopGoal}>
                ⏹ {tText('rdp.aiGoalStop')}
              </button>
            </div>
          ) : (
            <>
              <input
                className="rdp-ai-goal-input"
                value={goalText}
                onChange={(e) => setGoalText(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') handleStartGoal(); }}
                placeholder={tText('rdp.aiGoalPlaceholder')}
              />
              <button
                className="rdp-ai-goal-start"
                onClick={handleStartGoal}
                disabled={!goalText.trim() || streaming}
              >
                ▶ {tText('rdp.aiGoalStart')}
              </button>
            </>
          )}
        </div>
      )}

      <div className="rdp-ai-messages">
        {messages.length === 0 && (
          <div className="rdp-ai-empty">
            <div className="rdp-ai-empty-icon">🖥️</div>
            <div className="rdp-ai-empty-title">{t('rdp.aiEmptyTitle')}</div>
            <div className="rdp-ai-empty-hint">{t('rdp.aiEmptyHint')}</div>
            <div className="rdp-ai-empty-examples">
              <button
                className="rdp-ai-example-btn"
                onClick={() => setInputText('打开记事本并输入"你好世界"')}
              >
                {tText('rdp.aiExampleNotepad')}
              </button>
              <button
                className="rdp-ai-example-btn"
                onClick={() => setInputText('打开开始菜单，搜索 cmd 并启动命令提示符')}
              >
                {tText('rdp.aiExampleCmd')}
              </button>
              <button
                className="rdp-ai-example-btn"
                onClick={() => setInputText('截图当前桌面（Win+Shift+S）')}
              >
                {tText('rdp.aiExampleScreenshot')}
              </button>
            </div>
          </div>
        )}
        {messages.map((msg) => (
          <RdpMessageContent key={msg.id} msg={msg} />
        ))}
        <div ref={messagesEndRef} />
      </div>

      {rdpActions.length > 0 && (
        <div className="rdp-ai-actions-section">
          <div className="rdp-ai-actions-header">
            <span>{tText('rdp.aiActions')}</span>
            {pendingActions.length > 0 && (
              <span className="rdp-ai-actions-badge">{pendingActions.length}</span>
            )}
            {pendingActions.length > 1 && !batchExecuting && (
              <button
                className="rdp-ai-actions-run-all"
                onClick={() => { void handleApproveAll(); }}
              >
                ▶ {tText('rdp.aiRunAll', { count: pendingActions.length })}
              </button>
            )}
            {batchExecuting && (
              <span className="rdp-ai-actions-batch-running">
                <span className="rdp-ai-action-dots" /> {tText('rdp.aiBatchRunning')}
              </span>
            )}
          </div>
          <div className="rdp-ai-actions-list">
            {rdpActions.map((action) => (
              <RdpActionCard
                key={action.id}
                action={action}
                onApprove={handleApproveAction}
                onReject={rejectAction}
                disabled={streaming}
                batchExecutingId={batchExecutingId}
              />
            ))}
          </div>
        </div>
      )}

      <div className="rdp-ai-input-bar">
        <textarea
          className="rdp-ai-input"
          value={inputText}
          onChange={(e) => setInputText(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={tText('rdp.aiInputPlaceholder')}
          rows={1}
        />
        <button
          className="rdp-ai-send"
          onClick={handleSend}
          disabled={streaming || !inputText.trim()}
        >
          {streaming ? '…' : '➤'}
        </button>
      </div>
    </div>
  );
};

export default memo(RdpAiPanel);
