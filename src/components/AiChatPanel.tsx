import { useEffect, useRef, useState, useMemo, memo, useCallback } from 'react';
import { useAiChatStore, selectMessages, selectActiveConversationId, selectInputText, selectStreamingMessageId, type AiChatScope, type ChatMessage, type PendingAction } from '../stores/aiChat';
import { classifyTerminalFollowUp, formatTerminalCommandResult } from '../utils/terminalFollowUp';
import { extractEmptyCommandPlanNotice, stripCommandPlanBlock } from '../utils/aiActionParser';
import { useTranslation } from '../i18n';
import type { TerminalCommandExecutionResult } from './TerminalView';
import type { FC } from 'react';
import '../styles/panels/ai-panels.css';

const MESSAGE_BUFFER = 80;

const CommandPlanNotice: FC<{ notice: NonNullable<ChatMessage['commandPlanNotice']> }> = memo(({ notice }) => {
  const { t } = useTranslation();

  return (
    <div className="ai-command-plan-notice">
      <div className="ai-command-plan-notice-icon" aria-hidden="true">✓</div>
      <div className="ai-command-plan-notice-body">
        <div className="ai-command-plan-notice-title">{t('ai.noCommandPlanTitle')}</div>
        <div className="ai-command-plan-notice-summary">{notice.summary}</div>
      </div>
    </div>
  );
});
CommandPlanNotice.displayName = 'CommandPlanNotice';

const MessageContent: FC<{ msg: ChatMessage }> = memo(({ msg }) => {
  const { t } = useTranslation();
  const fallbackPlan = useMemo(
    () => (!msg.commandPlanNotice && !msg.streaming ? extractEmptyCommandPlanNotice(msg.content) : null),
    [msg.commandPlanNotice, msg.content, msg.streaming],
  );
  const displayContent = msg.role === 'assistant' ? stripCommandPlanBlock(msg.content) : msg.content;
  const content = fallbackPlan?.displayContent ?? displayContent;
  const notice = msg.commandPlanNotice ?? fallbackPlan?.commandPlanNotice;
  const hasActions = (msg.pendingActions?.filter((action) => !action.rejected).length ?? 0) > 0;
  const isEmpty = !content.trim() && !notice;

  return (
    <div className="ai-msg-content">
      {content && <div className="ai-msg-text">{content}</div>}
      {notice && <CommandPlanNotice notice={notice} />}
      {isEmpty && msg.streaming && (
        <div className="ai-msg-thinking">
          <span className="ai-msg-thinking-dots" />
          {t('ai.thinking')}
        </div>
      )}
      {isEmpty && !msg.streaming && hasActions && (
        <div className="ai-msg-plan-ready">{t('ai.commandPlanReady')}</div>
      )}
      {msg.streaming && !isEmpty && <span className="ai-msg-cursor" />}
    </div>
  );
});
MessageContent.displayName = 'MessageContent';

const MessageBubble: FC<{ msg: ChatMessage }> = memo(({ msg }) => {
  const { t } = useTranslation();
  const isUser = msg.role === 'user';
  if (msg.role === 'system') return null;

  return (
    <div className={`ai-msg ${isUser ? 'ai-msg-user' : 'ai-msg-assistant'}`}>
      <div className="ai-msg-role">{isUser ? t('ai.roleUser') : t('ai.roleAI')}</div>
      <MessageContent msg={msg} />
    </div>
  );
});
MessageBubble.displayName = 'MessageBubble';

const HistoricalMessage: FC<{ msg: ChatMessage }> = memo(({ msg }) => {
  const { t } = useTranslation();
  const isUser = msg.role === 'user';
  if (msg.role === 'system') return null;

  return (
    <div
      className={`ai-msg ${isUser ? 'ai-msg-user' : 'ai-msg-assistant'}`}
      style={{ contentVisibility: 'auto', containIntrinsicSize: '0 60px' }}
    >
      <div className="ai-msg-role">{isUser ? t('ai.roleUser') : t('ai.roleAI')}</div>
      <MessageContent msg={msg} />
    </div>
  );
});
HistoricalMessage.displayName = 'HistoricalMessage';

const CmdPendingItem: FC<{
  action: PendingAction;
  idx: number;
  sessionId?: string;
  onApprove: (id: string, sid?: string) => void;
  onReject: (id: string) => void;
}> = memo(({ action, idx, sessionId, onApprove, onReject }) => {
  const { t, tText } = useTranslation();
  const assessment = action.assessment;
  const isBlocked = assessment?.decision === 'BLOCK';

  return (
    <div className={`ai-cmd-item ai-cmd-item-pending ${isBlocked ? 'ai-cmd-item-blocked' : ''}`}>
      <div className="ai-cmd-desc">
        <span className="ai-cmd-order">{idx + 1}</span>
        {action.description}
      </div>
      {action.command && (
        <pre className="ai-cmd-code">{action.command}</pre>
      )}
      <div className="ai-cmd-policy">
        {action.assessmentLoading && (
          <span className="ai-cmd-policy-muted">{t('ai.assessingRisk')}</span>
        )}
        {assessment && (
          <>
            <span className={`ai-cmd-risk ai-cmd-risk-${assessment.risk_level}`}>
              {t(`ai.risk.${assessment.risk_level}` as never)}
            </span>
            <span className={`ai-cmd-decision ai-cmd-decision-${assessment.decision.toLowerCase()}`}>
              {t(`ai.decision.${assessment.decision.toLowerCase()}` as never)}
            </span>
            <span className="ai-cmd-rule">{assessment.matched_rule}</span>
          </>
        )}
      </div>
      {assessment?.reason && (
        <div className="ai-cmd-reason">{assessment.reason}</div>
      )}
      {action.assessmentError && (
        <div className="ai-cmd-reason ai-cmd-reason-error">{action.assessmentError}</div>
      )}
      <div className="ai-cmd-buttons">
        <button
          className="ai-cmd-approve"
          onClick={() => onApprove(action.id, sessionId)}
          title={tText(isBlocked ? 'ai.blockedTitle' : 'ai.approveTitle')}
          disabled={action.assessmentLoading || isBlocked}
        >
          ↵ {t('ai.approveAndInject')}
        </button>
        <button className="ai-cmd-reject" onClick={() => onReject(action.id)} title={tText('ai.rejectTitle')}>
          ✗ {t('ai.reject')}
        </button>
      </div>
    </div>
  );
});
CmdPendingItem.displayName = 'CmdPendingItem';

const CmdExecutingItem: FC<{ action: PendingAction; idx: number }> = memo(({ action, idx }) => {
  const { t } = useTranslation();

  return (
    <div className="ai-cmd-item ai-cmd-item-executing">
      <div className="ai-cmd-desc">
        <span className="ai-cmd-order">{idx + 1}</span>
        {action.description}
      </div>
      {action.command && (
        <pre className="ai-cmd-code">{action.command}</pre>
      )}
      <div className="ai-cmd-executing-status">
        <span className="ai-cmd-executing-dots" />
        <span>{t('ai.executing')}</span>
      </div>
    </div>
  );
});
CmdExecutingItem.displayName = 'CmdExecutingItem';

const CmdDoneItem: FC<{ action: PendingAction; idx: number }> = memo(({ action, idx }) => {
  const { t } = useTranslation();

  return (
    <div className="ai-cmd-item ai-cmd-item-done">
      <div className="ai-cmd-desc">
        <span className="ai-cmd-order">{idx + 1}</span>
        {action.description}
      </div>
      {action.command && (
        <pre className="ai-cmd-code">{action.command}</pre>
      )}
      <span className="ai-cmd-status">✓ {t('ai.executed')}</span>
    </div>
  );
});
CmdDoneItem.displayName = 'CmdDoneItem';

const AUTO_COMMAND_TIMEOUT_MS = 45 * 60 * 1000;

function getTerminalBufferText(
  getBuffer?: () => string,
): string {
  if (!getBuffer) return '';
  const raw = getBuffer();
  if (!raw) return '';
  const lines = raw.split('\n').slice(-80);
  return `[终端缓冲区（最近输出）]\n${lines.join('\n')}`;
}

interface AiChatPanelProps {
  hostId?: string;
  hostName?: string;
  hostIp?: string;
  sftpPath?: string;
  sessionId?: string;
  getTerminalBuffer?: () => string;
  executeTerminalCommand?: (command: string, options?: { timeoutMs?: number }) => Promise<TerminalCommandExecutionResult> | undefined;
}

const AiChatPanel: FC<AiChatPanelProps> = ({
  hostId,
  hostName,
  hostIp,
  sftpPath,
  sessionId,
  getTerminalBuffer,
  executeTerminalCommand,
}) => {
  const { t, tText } = useTranslation();
  const messages = useAiChatStore(selectMessages);
  const inputText = useAiChatStore(selectInputText);
  const streamingMessageId = useAiChatStore(selectStreamingMessageId);
  const streaming = Boolean(streamingMessageId);
  const setInputText = useAiChatStore((s) => s.setInputText);
  const sendMessage = useAiChatStore((s) => s.sendMessage);
  const approveAction = useAiChatStore((s) => s.approveAction);
  const rejectAction = useAiChatStore((s) => s.rejectAction);
  const initStreamListener = useAiChatStore((s) => s.initStreamListener);
  const destroyStreamListener = useAiChatStore((s) => s.destroyStreamListener);
  const conversations = useAiChatStore((s) => s.conversations);
  const activeConversationId = useAiChatStore(selectActiveConversationId);
  const loadConversations = useAiChatStore((s) => s.loadConversations);
  const openConversation = useAiChatStore((s) => s.openConversation);
  const newConversation = useAiChatStore((s) => s.newConversation);
  const deleteConversation = useAiChatStore((s) => s.deleteConversation);
  const sendDirectMessage = useAiChatStore((s) => s.sendDirectMessage);
  const activateSession = useAiChatStore((s) => s.activateSession);
  const [showHistory, setShowHistory] = useState(false);
  const [executingActionIds, setExecutingActionIds] = useState<Set<string>>(new Set());
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const executingActionIdsRef = useRef<Set<string>>(new Set());

  const allActions = useMemo(
    () => messages.flatMap((m) => (m.pendingActions ?? []).filter((a) => !a.rejected)),
    [messages],
  );
  const pendingActions = useMemo(
    () => allActions.filter((a) => !a.approved && !a.executed),
    [allActions],
  );
  const executingActions = useMemo(
    () => pendingActions.filter((a) => executingActionIds.has(a.id)),
    [pendingActions, executingActionIds],
  );
  const waitingActions = useMemo(
    () => pendingActions.filter((a) => !executingActionIds.has(a.id)),
    [pendingActions, executingActionIds],
  );
  const executedActions = useMemo(
    () => allActions.filter((a) => a.executed),
    [allActions],
  );
  const hasUnfinishedAction = pendingActions.length > 0
    || executingActions.length > 0;

  // Split messages: historical (rendered with content-visibility) + active (last N)
  const { historical, active } = useMemo(() => {
    if (messages.length <= MESSAGE_BUFFER) {
      return { historical: [] as ChatMessage[], active: messages };
    }
    return {
      historical: messages.slice(0, messages.length - MESSAGE_BUFFER),
      active: messages.slice(messages.length - MESSAGE_BUFFER),
    };
  }, [messages]);

  useEffect(() => {
    const scope: AiChatScope = hostId
      ? { scope: 'ssh_host', scopeId: hostId }
      : { scope: 'terminal_session', scopeId: sessionId || '' };
    if (sessionId) activateSession(sessionId, scope);
    void initStreamListener();
    void loadConversations(scope);
    return () => destroyStreamListener();
  }, [hostId, sessionId, activateSession, initStreamListener, destroyStreamListener, loadConversations]);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  const buildContext = (): string => {
    const parts: string[] = [];
    parts.push(`你是 OpsBatch 终端的 AI 运维助手。帮助用户诊断问题、生成命令、分析输出。回复简洁实用。

## 工作原则
1. **先观察再操作**：执行任何变更前，先用检查命令了解目标主机现状（系统版本、服务状态、磁盘/内存、已有配置等），不要跳过诊断直接给出修改方案。
2. **谨慎对待系统更新**：不要轻易建议 apt upgrade / yum update 等全量更新操作。仅在用户明确要求或确实需要修复安全漏洞时才提议，且必须说明影响（耗时、可能的重启、兼容性风险）。
3. **最小变更**：优先给出最小改动的修复方案，避免一次性大范围变更。
4. **风险提示**：涉及数据删除、服务重启、配置修改等操作时，先说明影响再提议命令。

## 命令格式
1. **选择与命令互斥**：需要用户在多个方案、目标、范围或风险选项中选择时，只解释选项并等待用户决定，不输出任何命令、COMMAND_PLAN、ACTION 或 shell 代码块。
2. **每轮一个动作**：需要执行操作时，每次回复只提供一个可审批命令动作，COMMAND_PLAN 的 steps 必须恰好包含一个步骤。一个动作内部允许使用 &&、;、换行、管道和重定向。
3. **等待结果再继续**：不要提前给出后续命令。收到当前命令的执行结果、退出码或异常状态后，再分析并决定下一步。

当需要执行命令时，优先在回复末尾输出结构化计划：
<COMMAND_PLAN>
{
  "version": 1,
  "summary": "一句话说明计划",
  "steps": [
    {
      "description": "步骤说明",
      "command": "要执行的命令",
      "intent": "observe|diagnose|change|verify|rollback",
      "expectedOutcome": "预期结果"
    }
  ]
}
</COMMAND_PLAN>

如果无法输出结构化计划，再使用兼容格式：
[ACTION:命令描述]
实际命令内容
[/ACTION]
用户可以在右侧命令确认栏审批这一条命令，命令执行前会经过 Rust 策略评估。`);

    if (hostName && hostIp) {
      parts.push(`当前连接: ${hostName} (${hostIp})`);
    }
    if (sftpPath) {
      parts.push(`SFTP 当前路径: ${sftpPath}`);
    }
    const buffer = getTerminalBufferText(getTerminalBuffer);
    if (buffer) {
      parts.push(buffer);
    }

    return parts.join('\n\n');
  };

  const handleSend = () => {
    void sendMessage(buildContext());
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleApproveAction = useCallback(async (actionId: string) => {
    const action = pendingActions.find((item) => item.id === actionId);
    const aiSessionId = sessionId;
    if (executingActionIdsRef.current.has(actionId)) {
      return;
    }
    if (action?.assessment?.decision === 'BLOCK' || action?.assessmentLoading) {
      await approveAction(actionId, sessionId);
      return;
    }

    if (!action?.command || !executeTerminalCommand) {
      await approveAction(actionId, sessionId);
      return;
    }

    executingActionIdsRef.current.add(actionId);
    setExecutingActionIds(new Set(executingActionIdsRef.current));
    try {
      const followUpContext = buildContext();
      const execution = executeTerminalCommand(action.command, { timeoutMs: AUTO_COMMAND_TIMEOUT_MS });
      if (!execution) {
        await approveAction(actionId, sessionId);
        return;
      }

      const result = await execution;
      await approveAction(actionId, undefined, aiSessionId);
      const commandOutput = formatTerminalCommandResult(result);
      const decision = classifyTerminalFollowUp(commandOutput);
      void sendDirectMessage(
        `[自动跟进] 当前命令已执行完毕，请分析这条命令的输出和退出状态，判断是否需要下一步操作：\n\n${decision.output || commandOutput}`,
        followUpContext,
        undefined,
        aiSessionId,
      );
    } finally {
      executingActionIdsRef.current.delete(actionId);
      setExecutingActionIds(new Set(executingActionIdsRef.current));
    }
  }, [approveAction, executeTerminalCommand, pendingActions, sendDirectMessage, sessionId]);

  return (
    <div className="ai-chat-panel">
      {/* Left column: Chat */}
      <div className="ai-chat-left">
        <div className="ai-chat-toolbar">
          <button
            className="ai-chat-toolbar-btn"
            onClick={() => newConversation()}
            title={tText('ai.newChat')}
          >
            ＋
          </button>
          <button
            className="ai-chat-toolbar-btn"
            onClick={() => setShowHistory(!showHistory)}
            title={tText('ai.chatHistory')}
          >
            ☰
          </button>
          <span className="ai-chat-toolbar-title">
            {hostName ? `${hostName}` : t('ai.assistantTitle')}
          </span>
        </div>

        {showHistory && (
          <div className="ai-chat-history">
            {conversations.length === 0 && (
              <div className="ai-chat-history-empty">{t('ai.noHistory')}</div>
            )}
            {conversations.map((conv) => (
              <div
                key={conv.id}
                className={`ai-chat-history-item ${conv.id === activeConversationId ? 'ai-chat-history-active' : ''}`}
                onClick={() => {
                  void openConversation(conv.id);
                  setShowHistory(false);
                }}
              >
                <span className="ai-chat-history-title">
                  {conv.title || t('ai.noTitle')}
                </span>
                <button
                  className="ai-chat-history-delete"
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

        <div className="ai-chat-messages">
          {messages.length === 0 && (
            <div className="ai-chat-empty">
              <div className="ai-chat-empty-icon">⌨</div>
              <div>{t('ai.emptyPrompt')}</div>
              <div className="ai-chat-empty-hint">
                {t('ai.emptyHint')}
              </div>
            </div>
          )}
          {historical.map((msg) => (
            <HistoricalMessage key={msg.id} msg={msg} />
          ))}
          {active.map((msg) => (
            <MessageBubble key={msg.id} msg={msg} />
          ))}
          <div ref={messagesEndRef} />
        </div>

        <div className="ai-chat-input-bar">
          <textarea
            className="ai-chat-input"
            value={inputText}
            onChange={(e) => setInputText(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={tText('ai.inputPlaceholder')}
            disabled={streaming || hasUnfinishedAction}
            rows={1}
          />
          <button
            className="ai-chat-send"
            onClick={handleSend}
            disabled={streaming || hasUnfinishedAction || !inputText.trim()}
          >
            {streaming ? '…' : '➤'}
          </button>
        </div>
      </div>

      {/* Right column: Command Approval */}
      <div className="ai-cmd-panel">
        <div className="ai-cmd-header">
          <span className="ai-cmd-title">{t('ai.commandConfirm')}</span>
          {pendingActions.length > 0 && (
            <span className="ai-cmd-badge">{pendingActions.length}</span>
          )}
        </div>
        <div className="ai-cmd-list">
          {waitingActions.length === 0 && executingActions.length === 0 && executedActions.length === 0 && (
            <div className="ai-cmd-empty">
              <div className="ai-cmd-empty-icon">⏳</div>
              <div>{t('ai.waitingForCommand')}</div>
            </div>
          )}
          {executingActions.map((action, idx) => (
            <CmdExecutingItem
              key={action.id}
              action={action}
              idx={idx}
            />
          ))}
          {waitingActions.map((action, idx) => (
            <CmdPendingItem
              key={action.id}
              action={action}
              idx={executingActions.length + idx}
              sessionId={sessionId}
              onApprove={(id) => { void handleApproveAction(id); }}
              onReject={rejectAction}
            />
          ))}
          {executedActions.length > 0 && (
            <div className="ai-cmd-done-section">
              {executedActions.map((action, idx) => (
                <CmdDoneItem key={action.id} action={action} idx={idx} />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default memo(AiChatPanel);
