import { useState, useEffect, useRef } from 'react';
import { Modal, Button, Space, Select, Empty, Spin, message } from './ui';
import { PlayCircleOutlined, PauseCircleOutlined, ForwardOutlined } from './ui/icons';
import { invoke } from '@tauri-apps/api/core';

interface AsciinemaEvent {
  time: number;
  event_type: string;
  content: string;
}

interface AsciinemaRecording {
  header: { version: number; width: number; height: number; timestamp: number };
  events: AsciinemaEvent[];
  filepath: string;
}

interface RecordingMeta {
  host_id: string;
  host_name: string;
  filepath: string;
  duration: number;
}

interface Props {
  open: boolean;
  historyId: string;
  onClose: () => void;
}

export default function AsciinemaPlayer({ open, historyId, onClose }: Props) {
  const [recordings, setRecordings] = useState<RecordingMeta[]>([]);
  const [selectedHost, setSelectedHost] = useState<string | null>(null);
  const [recording, setRecording] = useState<AsciinemaRecording | null>(null);
  const [playing, setPlaying] = useState(false);
  const [currentLine, setCurrentLine] = useState(0);
  const [output, setOutput] = useState<string>('');
  const [speed, setSpeed] = useState(1);
  const [loading, setLoading] = useState(false);
  const timerRef = useRef<number | null>(null);
  const outputRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    if (open && historyId) {
      loadRecordings();
    }
    return () => { if (timerRef.current) clearTimeout(timerRef.current); };
  }, [open, historyId]);

  useEffect(() => {
    if (outputRef.current) {
      outputRef.current.scrollTop = outputRef.current.scrollHeight;
    }
  }, [output]);

  const loadRecordings = async () => {
    try {
      const list = await invoke<RecordingMeta[]>('list_recordings', { historyId });
      setRecordings(list);
      if (list.length > 0 && !selectedHost) {
        setSelectedHost(list[0].host_id);
        loadRecording(list[0].host_id);
      }
    } catch {
      setRecordings([]);
    }
  };

  const loadRecording = async (hostId: string) => {
    setLoading(true);
    try {
      const rec = await invoke<AsciinemaRecording | null>('read_asciinema_recording', {
        historyId,
        hostId,
      });
      setRecording(rec);
      setCurrentLine(0);
      setOutput('');
      setPlaying(false);
    } catch (e: unknown) {
      message.error(String(e));
    } finally {
      setLoading(false);
    }
  };

  const play = () => {
    if (!recording || recording.events.length === 0) return;
    setPlaying(true);

    const events = recording.events;
    let idx = currentLine;
    let accumulated = '';

    const playNext = () => {
      if (idx >= events.length || !playing) {
        setPlaying(false);
        return;
      }

      const event = events[idx];
      accumulated += event.content;
      setOutput(accumulated);
      setCurrentLine(idx + 1);
      idx++;

      if (idx < events.length) {
        const delay = Math.max(10, (events[idx].time - events[idx - 1].time) * 1000 / speed);
        timerRef.current = window.setTimeout(playNext, delay);
      } else {
        setPlaying(false);
      }
    };

    playNext();
  };

  const pause = () => {
    setPlaying(false);
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  };

  const skipToEnd = () => {
    pause();
    if (!recording) return;
    setOutput(recording.events.map((e) => e.content).join(''));
    setCurrentLine(recording.events.length);
  };

  const reset = () => {
    pause();
    setOutput('');
    setCurrentLine(0);
  };

  const progress = recording ? (currentLine / Math.max(recording.events.length, 1)) * 100 : 0;

  return (
    <Modal
      title="执行记录回放 (Asciinema)"
      open={open}
      onCancel={() => { pause(); onClose(); }}
      width={900}
      footer={<Button onClick={() => { pause(); onClose(); }}>关闭</Button>}
    >
      <div style={{ marginBottom: 12 }}>
        <Select
          style={{ width: 300 }}
          placeholder="选择主机"
          value={selectedHost || undefined}
          onChange={(v) => {
            const hostId = Array.isArray(v) ? v[0] : v;
            pause();
            reset();
            setSelectedHost(hostId);
            loadRecording(hostId);
          }}
          options={recordings.map((r) => ({
            value: r.host_id,
            label: `${r.host_name || r.host_id} (${r.duration.toFixed(1)}s)`,
          }))}
        />
      </div>

      {loading ? (
        <div style={{ textAlign: 'center', padding: 40 }}><Spin /></div>
      ) : recording ? (
        <div>
          {/* Progress bar */}
          <div style={{
            height: 4,
            background: '#f0f0f0',
            borderRadius: 2,
            marginBottom: 12,
            overflow: 'hidden',
          }}>
            <div style={{
              height: '100%',
              width: `${progress}%`,
              background: '#1677ff',
              transition: 'width 0.1s linear',
            }} />
          </div>

          {/* Terminal output */}
          <pre
            ref={outputRef}
            style={{
              background: '#1e1e1e',
              color: '#d4d4d4',
              padding: 16,
              borderRadius: 8,
              fontFamily: "'Menlo', 'Monaco', 'Courier New', monospace",
              fontSize: 13,
              lineHeight: 1.5,
              minHeight: 300,
              maxHeight: 450,
              overflow: 'auto',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-all',
              margin: 0,
            }}
          >
            {output || (playing ? '' : '点击播放按钮开始回放')}
          </pre>

          {/* Controls */}
          <div style={{ marginTop: 12, display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <Space>
              {!playing ? (
                <Button type="primary" icon={<PlayCircleOutlined />} onClick={play}
                  disabled={!recording || currentLine >= recording.events.length}>
                  {currentLine > 0 ? '继续' : '播放'}
                </Button>
              ) : (
                <Button danger icon={<PauseCircleOutlined />} onClick={pause}>暂停</Button>
              )}
              <Button icon={<ForwardOutlined />} onClick={skipToEnd}>跳到结尾</Button>
              <Button onClick={reset}>重置</Button>
            </Space>
            <Space>
              <span style={{ fontSize: 12, color: '#666' }}>速度:</span>
              <Select<number> size="small" value={speed} onChange={(value) => setSpeed(Array.isArray(value) ? value[0] : value)} style={{ width: 80 }}
                options={[
                  { value: 0.5, label: '0.5x' },
                  { value: 1, label: '1x' },
                  { value: 2, label: '2x' },
                  { value: 5, label: '5x' },
                  { value: 10, label: '10x' },
                ]}
              />
              <span style={{ fontSize: 12, color: '#999' }}>
                {currentLine}/{recording?.events.length || 0}
              </span>
            </Space>
          </div>
        </div>
      ) : (
        <Empty description="暂无录制数据" image={Empty.PRESENTED_IMAGE_SIMPLE}>
          <p style={{ fontSize: 12, color: '#999' }}>
            执行命令时将自动录制为 asciinema 格式
          </p>
        </Empty>
      )}
    </Modal>
  );
}
