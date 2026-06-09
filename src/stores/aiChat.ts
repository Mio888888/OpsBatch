import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { parseAiPendingActionsAsync } from '../utils/aiActionParser';
import type { AiActionAssessment, ParsedCommandPlanNotice, ParsedPendingAction } from '../utils/aiActionParser';

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  model?: string;
  streaming?: boolean;
  pendingActions?: PendingAction[];
  commandPlanNotice?: ParsedCommandPlanNotice;
}

export interface PendingAction extends ParsedPendingAction {}

export interface Conversation {
  id: string;
  title: string;
  scope: string;
  scope_id: string;
  model: string;
  created_at: string;
  updated_at: string;
}

interface StreamChunk {
  delta: string;
  done: boolean;
  model: string;
  conversation_id: string;
  message_id: string;
  client_request_id?: string;
}

export interface AiChatScope {
  scope: string;
  scopeId: string;
}

interface SessionState {
  messages: ChatMessage[];
  activeConversationId: string | null;
  streamingMessageId: string | null;
  inputText: string;
  scope: AiChatScope;
  activeRequestId: string | null;
}

interface AiChatState {
  activeSessionId: string | null;
  sessions: Record<string, SessionState>;
  conversations: Conversation[];
  inlineVisible: boolean;
  inlineInputText: string;
  bottomTab: 'ai' | 'sftp' | 'forward' | 'commands' | 'scripts';
  _unlisten: UnlistenFn | null;

  setBottomTab: (tab: 'ai' | 'sftp' | 'forward' | 'commands' | 'scripts') => void;
  setInputText: (text: string) => void;
  setInlineVisible: (visible: boolean) => void;
  setInlineInputText: (text: string) => void;
  activateSession: (sessionId: string, scope?: AiChatScope) => void;
  clearSession: (sessionId: string) => void;
  loadConversations: (scope?: AiChatScope) => Promise<void>;
  openConversation: (id: string) => Promise<void>;
  newConversation: () => void;
  deleteConversation: (id: string) => Promise<void>;
  sendMessage: (context?: string) => Promise<void>;
  sendDirectMessage: (text: string, context?: string) => Promise<void>;
  sendMessageInline: () => Promise<void>;
  initStreamListener: () => Promise<void>;
  destroyStreamListener: () => void;
  approveAction: (actionId: string, sessionId?: string) => Promise<void>;
  rejectAction: (actionId: string) => void;
}

// Selectors for active session
export function selectMessages(state: AiChatState): ChatMessage[] {
  return getSession(state.sessions, state.activeSessionId).messages;
}
export function selectActiveConversationId(state: AiChatState): string | null {
  return getSession(state.sessions, state.activeSessionId).activeConversationId;
}
export function selectStreamingMessageId(state: AiChatState): string | null {
  return getSession(state.sessions, state.activeSessionId).streamingMessageId;
}
export function selectInputText(state: AiChatState): string {
  return getSession(state.sessions, state.activeSessionId).inputText;
}

const _EMPTY_SESSION: SessionState = {
  messages: [],
  activeConversationId: null,
  streamingMessageId: null,
  inputText: '',
  scope: { scope: 'global', scopeId: '' },
  activeRequestId: null,
};

function emptySession(): SessionState {
  return _EMPTY_SESSION;
}

function getSession(sessions: Record<string, SessionState>, id: string | null): SessionState {
  if (!id) return emptySession();
  return sessions[id] ?? emptySession();
}

function normalizeScope(scope?: AiChatScope): AiChatScope {
  return {
    scope: scope?.scope?.trim() || 'global',
    scopeId: scope?.scopeId?.trim() || '',
  };
}

function sameScope(a: AiChatScope, conversation: Conversation): boolean {
  return a.scope === conversation.scope && a.scopeId === conversation.scope_id;
}

function buildScopedRequest(
  session: SessionState,
  messages: Pick<ChatMessage, 'role' | 'content'>[],
  clientRequestId: string,
) {
  return {
    messages,
    conversationId: session.activeConversationId || undefined,
    scope: session.scope.scope,
    scopeId: session.scope.scopeId,
    clientRequestId,
  };
}

let _listenerPromise: Promise<UnlistenFn> | null = null;
const streamDeltaBuffer = new Map<string, string>();
const streamDeltaTimers = new Map<string, number>();

function flushStreamDelta(sessionId: string, setState: (fn: (prev: AiChatState) => Partial<AiChatState> | AiChatState) => void) {
  const delta = streamDeltaBuffer.get(sessionId);
  if (!delta) return;
  streamDeltaBuffer.delete(sessionId);
  const timer = streamDeltaTimers.get(sessionId);
  if (timer) {
    window.cancelAnimationFrame(timer);
    streamDeltaTimers.delete(sessionId);
  }
  setState((prev) => {
    const s = getSession(prev.sessions, sessionId);
    if (!s.streamingMessageId) return prev;
    return {
      sessions: {
        ...prev.sessions,
        [sessionId]: {
          ...s,
          messages: s.messages.map((m) =>
            m.id === s.streamingMessageId ? { ...m, content: m.content + delta } : m,
          ),
        },
      },
    };
  });
}

function enqueueStreamDelta(sessionId: string, delta: string, setState: (fn: (prev: AiChatState) => Partial<AiChatState> | AiChatState) => void) {
  if (!delta) return;
  streamDeltaBuffer.set(sessionId, (streamDeltaBuffer.get(sessionId) || '') + delta);
  if (streamDeltaTimers.has(sessionId)) return;
  const timer = window.requestAnimationFrame(() => {
    streamDeltaTimers.delete(sessionId);
    flushStreamDelta(sessionId, setState);
  });
  streamDeltaTimers.set(sessionId, timer);
}

function blockedAssessment(error: string): AiActionAssessment {
  return {
    decision: 'BLOCK',
    risk_level: 'critical',
    risk_score: 100,
    matched_rule: 'ASSESSMENT_FAILED',
    reason: error,
    capabilities: ['policy_error'],
  };
}

async function assessPendingAction(action: PendingAction): Promise<PendingAction> {
  if (action.type !== 'command' || !action.command) return action;
  try {
    const assessment = await invoke<AiActionAssessment>('ai_assess_action', {
      command: action.command,
    });
    return { ...action, assessment, assessmentLoading: false, assessmentError: undefined };
  } catch (e) {
    const error = `策略评估失败: ${String(e)}`;
    return {
      ...action,
      assessment: blockedAssessment(error),
      assessmentLoading: false,
      assessmentError: error,
    };
  }
}

async function recordActionEvent(
  action: PendingAction,
  event: 'approved' | 'rejected' | 'blocked',
  sessionId: string | undefined,
  conversationId: string | null,
): Promise<void> {
  if (action.type !== 'command' || !action.command || !action.assessment) return;
  await invoke('ai_record_action_event', {
    event: {
      actionId: action.id,
      event,
      command: action.command,
      conversationId: conversationId || undefined,
      sessionId: sessionId || undefined,
      assessment: action.assessment,
    },
  });
}

export const useAiChatStore = create<AiChatState>((set, get) => ({
  activeSessionId: null,
  sessions: {},
  conversations: [],
  inlineVisible: false,
  inlineInputText: '',
  bottomTab: 'sftp',
  _unlisten: null,

  setBottomTab: (tab) => set({ bottomTab: tab }),
  setInlineVisible: (visible) => set({ inlineVisible: visible }),
  setInlineInputText: (text) => set({ inlineInputText: text }),

  setInputText: (text) => {
    const sid = get().activeSessionId;
    if (!sid) return;
    set((prev) => ({
      sessions: { ...prev.sessions, [sid]: { ...getSession(prev.sessions, sid), inputText: text } },
    }));
  },

  activateSession: (sessionId, scope) => {
    const normalizedScope = normalizeScope(scope);
    set((prev) => {
      const sessions = { ...prev.sessions };
      const existing = sessions[sessionId];
      if (!existing) {
        sessions[sessionId] = {
          ...emptySession(),
          scope: normalizedScope,
        };
      } else if (
        existing.scope.scope !== normalizedScope.scope
        || existing.scope.scopeId !== normalizedScope.scopeId
      ) {
        sessions[sessionId] = {
          ...existing,
          scope: normalizedScope,
          activeConversationId: null,
          messages: [],
          streamingMessageId: null,
          activeRequestId: null,
        };
      }
      return { activeSessionId: sessionId, sessions };
    });
  },

  clearSession: (sessionId) => {
    set((prev) => {
      const sessions = { ...prev.sessions };
      delete sessions[sessionId];
      return { sessions };
    });
  },

  loadConversations: async (scope) => {
    const sid = get().activeSessionId;
    const requestScope = normalizeScope(scope ?? getSession(get().sessions, sid).scope);
    try {
      const convs = await invoke<Conversation[]>('ai_list_conversations', {
        scope: requestScope.scope,
        scopeId: requestScope.scopeId,
      });
      const activeSession = getSession(get().sessions, get().activeSessionId);
      if (
        activeSession.scope.scope === requestScope.scope
        && activeSession.scope.scopeId === requestScope.scopeId
      ) {
        set({ conversations: convs });
      }
    } catch {}
  },

  openConversation: async (id) => {
    const sid = get().activeSessionId;
    if (!sid) return;
    try {
      const [conversation, msgs] = await invoke<[Conversation, ChatMessage[]]>('ai_get_conversation', {
        conversationId: id,
      });
      const session = getSession(get().sessions, sid);
      if (!sameScope(session.scope, conversation)) return;
      set((prev) => ({
        sessions: {
          ...prev.sessions,
          [sid]: {
            ...getSession(prev.sessions, sid),
            activeConversationId: id,
            messages: msgs,
            streamingMessageId: null,
            activeRequestId: null,
          },
        },
      }));
    } catch {}
  },

  newConversation: () => {
    const sid = get().activeSessionId;
    if (!sid) return;
    set((prev) => ({
      sessions: {
        ...prev.sessions,
        [sid]: {
          ...getSession(prev.sessions, sid),
          activeConversationId: null,
          messages: [],
          streamingMessageId: null,
          activeRequestId: null,
        },
      },
    }));
  },

  deleteConversation: async (id) => {
    const sid = get().activeSessionId;
    try {
      await invoke('ai_delete_conversation', { conversationId: id });
      if (sid) {
        const session = getSession(get().sessions, sid);
        if (session.activeConversationId === id) {
          set((prev) => ({
            sessions: {
              ...prev.sessions,
              [sid]: {
                ...getSession(prev.sessions, sid),
                activeConversationId: null,
                messages: [],
                streamingMessageId: null,
                activeRequestId: null,
              },
            },
          }));
        }
      }
      await get().loadConversations();
    } catch {}
  },

  sendMessage: async (context?: string) => {
    const state = get();
    const sid = state.activeSessionId;
    if (!sid) return;
    const session = getSession(state.sessions, sid);
    if (session.streamingMessageId) return;
    const text = session.inputText.trim();
    if (!text) return;
    const clientRequestId = crypto.randomUUID();

    const userMsg: ChatMessage = {
      id: crypto.randomUUID(),
      role: 'user',
      content: text,
    };

    const assistantMsgId = crypto.randomUUID();
    const assistantMsg: ChatMessage = {
      id: assistantMsgId,
      role: 'assistant',
      content: '',
      streaming: true,
    };

    const prevMessages = session.messages;
    const allMessages = [...prevMessages, userMsg, assistantMsg];

    set((prev) => ({
      sessions: {
        ...prev.sessions,
        [sid]: {
          ...getSession(prev.sessions, sid),
          messages: allMessages,
          inputText: '',
          streamingMessageId: assistantMsgId,
          activeRequestId: clientRequestId,
        },
      },
    }));

    const apiMessages = [...prevMessages, userMsg].map((m) => ({
      role: m.role,
      content: m.content,
    }));

    if (context) {
      apiMessages.unshift({ role: 'system', content: context });
    }

    try {
      await invoke('ai_chat_stream', {
        request: buildScopedRequest(session, apiMessages, clientRequestId),
      });
    } catch (e) {
      set((prev) => ({
        sessions: {
          ...prev.sessions,
          [sid]: {
            ...getSession(prev.sessions, sid),
            messages: allMessages.map((m) =>
              m.id === assistantMsgId
                ? { ...m, content: `错误: ${e}`, streaming: false }
                : m,
            ),
            streamingMessageId: null,
            activeRequestId: null,
          },
        },
      }));
    }
  },

  sendDirectMessage: async (text: string, context?: string) => {
    const state = get();
    const sid = state.activeSessionId;
    if (!sid) return;
    const session = getSession(state.sessions, sid);
    if (session.streamingMessageId) return;
    const clientRequestId = crypto.randomUUID();

    const userMsg: ChatMessage = {
      id: crypto.randomUUID(),
      role: 'user',
      content: text,
    };

    const assistantMsgId = crypto.randomUUID();
    const assistantMsg: ChatMessage = {
      id: assistantMsgId,
      role: 'assistant',
      content: '',
      streaming: true,
    };

    const allMessages = [...session.messages, userMsg, assistantMsg];

    set((prev) => ({
      sessions: {
        ...prev.sessions,
        [sid]: {
          ...getSession(prev.sessions, sid),
          messages: allMessages,
          streamingMessageId: assistantMsgId,
          activeRequestId: clientRequestId,
        },
      },
    }));

    const apiMessages = [...session.messages, userMsg].map((m) => ({
      role: m.role,
      content: m.content,
    }));

    if (context) {
      apiMessages.unshift({ role: 'system', content: context });
    }

    try {
      await invoke('ai_chat_stream', {
        request: buildScopedRequest(session, apiMessages, clientRequestId),
      });
    } catch (e) {
      set((prev) => ({
        sessions: {
          ...prev.sessions,
          [sid]: {
            ...getSession(prev.sessions, sid),
            messages: allMessages.map((m) =>
              m.id === assistantMsgId
                ? { ...m, content: `错误: ${e}`, streaming: false }
                : m,
            ),
            streamingMessageId: null,
            activeRequestId: null,
          },
        },
      }));
    }
  },

  sendMessageInline: async () => {
    const state = get();
    const sid = state.activeSessionId;
    if (!sid) return;
    const session = getSession(state.sessions, sid);
    if (session.streamingMessageId) return;
    const text = state.inlineInputText.trim();
    if (!text) return;
    const clientRequestId = crypto.randomUUID();

    const assistantMsgId = crypto.randomUUID();
    const userMsg: ChatMessage = {
      id: crypto.randomUUID(),
      role: 'user',
      content: text,
    };
    const assistantMsg: ChatMessage = {
      id: assistantMsgId,
      role: 'assistant',
      content: '',
      streaming: true,
    };

    set((prev) => ({
      sessions: {
        ...prev.sessions,
        [sid]: {
          ...getSession(prev.sessions, sid),
          messages: [...getSession(prev.sessions, sid).messages, userMsg, assistantMsg],
          streamingMessageId: assistantMsgId,
          activeRequestId: clientRequestId,
        },
      },
      inlineInputText: '',
    }));

    try {
      await invoke('ai_chat_stream', {
        request: buildScopedRequest(session, [{ role: 'user', content: text }], clientRequestId),
      });
    } catch (e) {
      set((prev) => ({
        sessions: {
          ...prev.sessions,
          [sid]: {
            ...getSession(prev.sessions, sid),
            messages: getSession(prev.sessions, sid).messages.map((m) =>
              m.id === assistantMsgId
                ? { ...m, content: `错误: ${e}`, streaming: false }
                : m,
            ),
            streamingMessageId: null,
            activeRequestId: null,
          },
        },
      }));
    }
  },

  initStreamListener: async () => {
    if (!_listenerPromise) {
      _listenerPromise = listen<StreamChunk>('ai-stream-chunk', (event) => {
        const chunk = event.payload;
        const state = get();
        const sid = chunk.client_request_id
          ? Object.entries(state.sessions).find(([, session]) => session.activeRequestId === chunk.client_request_id)?.[0]
          : state.activeSessionId;

        if (!sid) return;
        const session = getSession(state.sessions, sid);
        if (!session.streamingMessageId) return;

        if (chunk.done) {
          flushStreamDelta(sid, set);
          const targetMessageId = session.streamingMessageId;
          const targetMsg = session.messages.find((m) => m.id === targetMessageId);
          const rawContent = targetMsg?.content ?? '';

          set((prev) => {
            const s = getSession(prev.sessions, sid);
            if (
              !s.streamingMessageId
              || (chunk.client_request_id && s.activeRequestId !== chunk.client_request_id)
            ) {
              return prev;
            }

            return {
              sessions: {
                ...prev.sessions,
                [sid]: {
                  ...s,
                  messages: s.messages.map((m) =>
                    m.id === s.streamingMessageId
                      ? {
                        ...m,
                        streaming: false,
                        model: chunk.model,
                      }
                      : m,
                  ),
                  streamingMessageId: null,
                  activeRequestId: null,
                  activeConversationId: s.activeConversationId || chunk.conversation_id,
                },
              },
            };
          });
          void parseAiPendingActionsAsync(rawContent).then(({ displayContent, actions, commandPlanNotice }) => {
            const initialActions = actions.map((action) => ({
              ...action,
              assessmentLoading: action.type === 'command',
            }));
            set((prev) => {
              const s = getSession(prev.sessions, sid);
              return {
                sessions: {
                  ...prev.sessions,
                  [sid]: {
                    ...s,
                    messages: s.messages.map((m) =>
                      m.id === targetMessageId
                        ? {
                          ...m,
                          content: displayContent,
                          pendingActions: initialActions.length > 0 ? initialActions : undefined,
                          commandPlanNotice,
                        }
                        : m,
                    ),
                  },
                },
              };
            });
            if (actions.length === 0) return;
            void Promise.all(actions.map(assessPendingAction)).then((assessedActions) => {
              set((prev) => {
                const s = getSession(prev.sessions, sid);
                return {
                  sessions: {
                    ...prev.sessions,
                    [sid]: {
                      ...s,
                      messages: s.messages.map((m) =>
                        m.id === targetMessageId
                          ? { ...m, pendingActions: assessedActions }
                          : m,
                      ),
                    },
                  },
                };
              });
            });
          });
          if (get().activeSessionId === sid) {
            void get().loadConversations();
          }
          return;
        }

        enqueueStreamDelta(sid, chunk.delta, set);
      });
    }
    const unlisten = await _listenerPromise;
    set({ _unlisten: unlisten });
  },

  destroyStreamListener: () => {
    // Keep listener alive for app lifetime
  },

  approveAction: async (actionId: string, sessionId?: string) => {
    const state = get();
    const sid = state.activeSessionId;
    if (!sid) return;
    const session = getSession(state.sessions, sid);
    const msg = session.messages.find((m) => m.pendingActions?.some((a) => a.id === actionId));
    if (!msg?.pendingActions) return;

    const action = msg.pendingActions.find((a) => a.id === actionId);
    if (!action || action.approved || action.executed) return;
    if (action.assessment?.decision === 'BLOCK') {
      await recordActionEvent(action, 'blocked', sessionId, session.activeConversationId);
      return;
    }
    await recordActionEvent(action, 'approved', sessionId, session.activeConversationId);

    if (action.type === 'command' && action.command && sessionId) {
      try {
        const bracketedPaste = `\x1b[200~${action.command}\x1b[201~\r`;
        await invoke('terminal_write', { sessionId, data: bracketedPaste });
      } catch {
        return;
      }
    }

    set((prev) => ({
      sessions: {
        ...prev.sessions,
        [sid]: {
          ...getSession(prev.sessions, sid),
          messages: getSession(prev.sessions, sid).messages.map((m) =>
            m.pendingActions?.some((a) => a.id === actionId)
              ? {
                ...m,
                pendingActions: m.pendingActions?.map((a) =>
                  a.id === actionId ? { ...a, approved: true, executed: true } : a,
                ),
              }
              : m,
          ),
        },
      },
    }));
  },

  rejectAction: (actionId: string) => {
    const state = get();
    const sid = state.activeSessionId;
    if (!sid) return;
    const currentSession = getSession(state.sessions, sid);
    const action = currentSession.messages
      .flatMap((m) => m.pendingActions ?? [])
      .find((a) => a.id === actionId);
    set((prev) => ({
      sessions: {
        ...prev.sessions,
        [sid]: {
          ...getSession(prev.sessions, sid),
          messages: getSession(prev.sessions, sid).messages.map((m) =>
            m.pendingActions?.some((a) => a.id === actionId)
              ? {
                ...m,
                pendingActions: m.pendingActions?.map((a) =>
                  a.id === actionId ? { ...a, approved: false, executed: false, rejected: true } : a,
                ),
              }
              : m,
          ),
        },
      },
    }));
    if (action) {
      void recordActionEvent(action, 'rejected', undefined, currentSession.activeConversationId);
    }
  },
}));
