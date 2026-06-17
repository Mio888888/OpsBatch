import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from '../i18n';
import type { FC } from 'react';
import '../styles/panels/port-forward.css';

type ForwardType = 'local' | 'remote' | 'dynamic';
type ForwardStatus = 'active' | 'suspended' | 'error';

interface ForwardConfig {
  forward_type: ForwardType;
  local_addr: string;
  remote_addr: string;
  label: string | null;
}

interface ForwardEntry {
  id: string;
  host_id: string;
  config: ForwardConfig;
  status: ForwardStatus;
  error: string | null;
  bytes_sent: number;
  bytes_received: number;
  connected_at: number | null;
}

const FORWARD_TYPES: { value: ForwardType; labelKey: 'portForward.localForward' | 'portForward.remoteForward' | 'portForward.dynamicForward' }[] = [
  { value: 'local', labelKey: 'portForward.localForward' },
  { value: 'remote', labelKey: 'portForward.remoteForward' },
  { value: 'dynamic', labelKey: 'portForward.dynamicForward' },
];

const STATUS_MAP: Record<ForwardStatus, { labelKey: 'portForward.statusActive' | 'portForward.statusSuspended' | 'portForward.statusError'; className: string }> = {
  active: { labelKey: 'portForward.statusActive', className: 'pf-status-active' },
  suspended: { labelKey: 'portForward.statusSuspended', className: 'pf-status-suspended' },
  error: { labelKey: 'portForward.statusError', className: 'pf-status-error' },
};

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0B';
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)}M`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)}K`;
  return `${bytes}B`;
}

interface PortForwardPanelProps {
  hostId: string;
}

const PortForwardPanel: FC<PortForwardPanelProps> = ({ hostId }) => {
  const { tText } = useTranslation();
  const [forwards, setForwards] = useState<ForwardEntry[]>([]);
  const [showForm, setShowForm] = useState(false);
  const [formType, setFormType] = useState<ForwardType>('local');
  const [formLocalAddr, setFormLocalAddr] = useState('127.0.0.1:');
  const [formRemoteAddr, setFormRemoteAddr] = useState('');
  const [formLabel, setFormLabel] = useState('');

  const loadForwards = useCallback(async () => {
    try {
      const list = await invoke<ForwardEntry[]>('forward_list', { hostId });
      setForwards(list);
    } catch {
      setForwards([]);
    }
  }, [hostId]);

  useEffect(() => {
    void loadForwards();
  }, [loadForwards]);

  useEffect(() => {
    const unlisten = listen<ForwardEntry>(`forward-status:${hostId}`, (event) => {
      const entry = event.payload;
      setForwards((prev) => {
        const idx = prev.findIndex((f) => f.id === entry.id);
        if (idx >= 0) {
          const next = [...prev];
          next[idx] = entry;
          return next;
        }
        return [...prev, entry];
      });
    });

    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [hostId]);

  const handleAdd = useCallback(async () => {
    try {
      const entry = await invoke<ForwardEntry>('forward_add', {
        hostId,
        config: {
          forward_type: formType,
          local_addr: formLocalAddr,
          remote_addr: formRemoteAddr,
          label: formLabel || null,
        },
      });
      setForwards((prev) => [...prev, entry]);
      setShowForm(false);
      setFormLocalAddr('127.0.0.1:');
      setFormRemoteAddr('');
      setFormLabel('');
    } catch (e) {
      console.error('forward_add failed:', e);
    }
  }, [hostId, formType, formLocalAddr, formRemoteAddr, formLabel]);

  const handleRemove = useCallback(async (forwardId: string) => {
    try {
      await invoke('forward_remove', { hostId, forwardId });
      setForwards((prev) => prev.filter((f) => f.id !== forwardId));
    } catch (e) {
      console.error('forward_remove failed:', e);
    }
  }, [hostId]);

  const handleStop = useCallback(async (forwardId: string) => {
    try {
      await invoke('forward_stop', { hostId, forwardId });
    } catch (e) {
      console.error('forward_stop failed:', e);
    }
  }, [hostId]);

  return (
    <div className="pf-panel">
      <div className="pf-header">
        <span className="pf-title">{tText('portForward.title')}</span>
        <button
          className="pf-add-btn"
          onClick={() => setShowForm(!showForm)}
        >
          {showForm ? tText('common.cancel') : tText('portForward.addNew')}
        </button>
      </div>

      {showForm && (
        <div className="pf-form">
          <div className="pf-form-row">
            <select
              className="pf-form-select"
              value={formType}
              onChange={(e) => setFormType(e.target.value as ForwardType)}
            >
              {FORWARD_TYPES.map((t) => (
                <option key={t.value} value={t.value}>{tText(t.labelKey)}</option>
              ))}
            </select>
          </div>
          <div className="pf-form-row">
            <input
              className="pf-form-input"
              placeholder={formType === 'dynamic' ? tText('portForward.localAddrDynamic') : tText('portForward.localAddr')}
              value={formLocalAddr}
              onChange={(e) => setFormLocalAddr(e.target.value)}
            />
            {formType !== 'dynamic' && (
              <>
                <span className="pf-form-arrow">→</span>
                <input
                  className="pf-form-input"
                  placeholder={tText('portForward.remoteAddr')}
                  value={formRemoteAddr}
                  onChange={(e) => setFormRemoteAddr(e.target.value)}
                />
              </>
            )}
          </div>
          <div className="pf-form-row">
            <input
              className="pf-form-input"
              placeholder={tText('portForward.notePlaceholder')}
              value={formLabel}
              onChange={(e) => setFormLabel(e.target.value)}
            />
            <button
              className="pf-form-submit"
              onClick={handleAdd}
              disabled={!formLocalAddr || (formType !== 'dynamic' && !formRemoteAddr)}
            >
              {tText('portForward.confirm')}
            </button>
          </div>
        </div>
      )}

      <div className="pf-list">
        {forwards.length === 0 ? (
          <div className="pf-empty">{tText('portForward.noRules')}</div>
        ) : (
          forwards.map((f) => {
            const statusInfo = STATUS_MAP[f.status] || STATUS_MAP.error;
            return (
              <div key={f.id} className={`pf-item pf-item-${f.status}`}>
                <div className="pf-item-main">
                  <span className={`pf-item-type pf-type-${f.config.forward_type}`}>
                    {f.config.forward_type === 'local' ? 'L' : f.config.forward_type === 'remote' ? 'R' : 'D'}
                  </span>
                  <span className="pf-item-addr">
                    {f.config.forward_type === 'dynamic'
                      ? f.config.local_addr
                      : `${f.config.local_addr} → ${f.config.remote_addr}`}
                  </span>
                  {f.config.label && <span className="pf-item-label">{f.config.label}</span>}
                </div>
                <div className="pf-item-meta">
                  <span className={`pf-item-status ${statusInfo.className}`}>
                    {tText(statusInfo.labelKey)}
                  </span>
                  {(f.bytes_sent > 0 || f.bytes_received > 0) && (
                    <span className="pf-item-stats">
                      ↑{formatBytes(f.bytes_sent)} ↓{formatBytes(f.bytes_received)}
                    </span>
                  )}
                  {f.error && <span className="pf-item-error" title={f.error}>!</span>}
                  <div className="pf-item-actions">
                    {f.status === 'active' && (
                      <button className="pf-action-btn" onClick={() => handleStop(f.id)} title={tText('portForward.stop')}>
                        ■
                      </button>
                    )}
                    <button className="pf-action-btn pf-action-remove" onClick={() => handleRemove(f.id)} title={tText('common.delete')}>
                      ✗
                    </button>
                  </div>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
};

export default PortForwardPanel;
