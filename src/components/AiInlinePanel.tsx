import { useEffect, useRef, useState, useCallback, memo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { FC } from 'react';

interface StreamChunk {
  delta: string;
  done: boolean;
  model: string;
  conversation_id: string;
  message_id: string;
  client_request_id?: string;
}

interface AiInlinePanelProps {
  sessionId?: string;
  hostId?: string;
  visible: boolean;
  onClose: () => void;
  terminalBuffer?: string;
}

const AiInlinePanel: FC<AiInlinePanelProps> = ({
  sessionId,
  hostId,
  visible,
  onClose,
  terminalBuffer,
}) => {
  const [input, setInput] = useState('');
  const [response, setResponse] = useState('');
  const [streaming, setStreaming] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const requestIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (!visible) return;

    let disposed = false;
    let localUnlisten: UnlistenFn | null = null;

    const setup = async () => {
      localUnlisten = await listen<StreamChunk>('ai-stream-chunk', (event) => {
        if (disposed) return;
        const chunk = event.payload;
        if (!chunk.client_request_id || chunk.client_request_id !== requestIdRef.current) return;
        if (chunk.done) {
          setStreaming(false);
          requestIdRef.current = null;
          return;
        }
        setResponse((prev) => prev + chunk.delta);
      });
      if (disposed) {
        localUnlisten();
        return;
      }
    };

    void setup();

    return () => {
      disposed = true;
      localUnlisten?.();
    };
  }, [visible]);

  useEffect(() => {
    if (visible) {
      inputRef.current?.focus();
    } else {
      setInput('');
      setResponse('');
      setStreaming(false);
      requestIdRef.current = null;
    }
  }, [visible]);

  const handleSend = useCallback(async () => {
    if (!input.trim() || streaming) return;

    const prompt = input.trim();
    setInput('');
    setResponse('');
    setStreaming(true);
    const clientRequestId = crypto.randomUUID();
    requestIdRef.current = clientRequestId;

    const context = [
      '你是一个终端命令生成器。根据用户描述，生成可直接在终端执行的命令。只输出命令本身，不要解释。如果需要多条命令，用 && 或 ; 连接。',
    ];
    if (terminalBuffer) {
      context.push(`[终端最近输出]\n${terminalBuffer.split('\n').slice(-30).join('\n')}`);
    }

    try {
      await invoke('ai_chat_stream', {
        request: {
          messages: [
            { role: 'system', content: context.join('\n\n') },
            { role: 'user', content: prompt },
          ],
          scope: hostId ? 'ssh_host' : 'terminal_session',
          scopeId: hostId || sessionId || '',
          clientRequestId,
        },
      });
    } catch {
      setResponse('请求失败');
      setStreaming(false);
      requestIdRef.current = null;
    }
  }, [hostId, input, sessionId, streaming, terminalBuffer]);

  const handlePaste = useCallback(async () => {
    if (!response.trim() || !sessionId) return;
    const bracketedPaste = `\x1b[200~${response.trim()}\x1b[201~`;
    await invoke('terminal_write', { sessionId, data: bracketedPaste });
    onClose();
  }, [response, sessionId, onClose]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        void handleSend();
      }
      if (e.key === 'Escape') {
        onClose();
      }
    },
    [handleSend, onClose],
  );

  useEffect(() => {
    const handleGlobal = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && visible) {
        onClose();
      }
    };
    window.addEventListener('keydown', handleGlobal);
    return () => window.removeEventListener('keydown', handleGlobal);
  }, [visible, onClose]);

  if (!visible) return null;

  return (
    <div className="ai-inline-panel" ref={panelRef}>
      <div className="ai-inline-header">
        <span>AI 命令生成</span>
        <button className="ai-inline-close" onClick={onClose}>✗</button>
      </div>
      <div className="ai-inline-body">
        <div className="ai-inline-input-row">
          <textarea
            ref={inputRef}
            className="ai-inline-input"
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="描述你想执行的命令… (⌘I 打开/关闭)"
            rows={1}
            disabled={streaming}
          />
          <button
            className="ai-inline-send"
            onClick={() => void handleSend()}
            disabled={streaming || !input.trim()}
          >
            ➤
          </button>
        </div>
        {response && (
          <div className="ai-inline-response">
            <pre className="ai-inline-code">{response}</pre>
            <div className="ai-inline-actions">
              <button
                className="ai-inline-action"
                onClick={() => void handlePaste()}
                disabled={streaming}
              >
                ↵ 注入终端
              </button>
              <button
                className="ai-inline-action"
                onClick={() => navigator.clipboard.writeText(response)}
              >
                复制
              </button>
            </div>
          </div>
        )}
        {streaming && !response && (
          <div className="ai-inline-loading">生成中…</div>
        )}
      </div>
    </div>
  );
};

export default memo(AiInlinePanel);
