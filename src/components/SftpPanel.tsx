import { useState, useEffect, useCallback, useRef, memo, useMemo, useLayoutEffect } from 'react';
import { createPortal } from 'react-dom';
import { useVirtualizer } from '@tanstack/react-virtual';
import { invoke } from '@tauri-apps/api/core';
import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useSftpStore, type FileEntry } from '../stores/sftp';
import PortForwardPanel from './PortForwardPanel';
import { useTranslation } from '../i18n';
import type { FC, ReactNode } from 'react';

const SFTP_FILE_ROW_HEIGHT = 23;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function formatSize(bytes: number): string {
  if (bytes === 0) return '-';
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024 / 1024).toFixed(1)}G`;
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)}M`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)}K`;
  return `${bytes}B`;
}

function formatSpeed(bytesPerMs: number): string {
  if (bytesPerMs === 0) return '';
  const bps = bytesPerMs * 1000;
  if (bps >= 1024 * 1024) return `${(bps / 1024 / 1024).toFixed(1)} MB/s`;
  if (bps >= 1024) return `${(bps / 1024).toFixed(0)} KB/s`;
  return `${bps.toFixed(0)} B/s`;
}

function formatDuration(ms: number | null): string {
  if (ms === null) return '';
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function fileIcon(entry: FileEntry): string {
  if (entry.is_dir) return '📁';
  const ext = entry.name.split('.').pop()?.toLowerCase() || '';
  const map: Record<string, string> = {
    txt: '📄', md: '📝', json: '📋', js: '📜', ts: '📜', tsx: '📜', jsx: '📜',
    py: '🐍', rs: '🦀', go: '🔵', java: '☕', c: '🔧', h: '🔧', cpp: '🔧',
    html: '🌐', css: '🎨', scss: '🎨',
    jpg: '🖼️', jpeg: '🖼️', png: '🖼️', gif: '🖼️', svg: '🖼️', webp: '🖼️', ico: '🖼️',
    mp3: '🎵', wav: '🎵', flac: '🎵', aac: '🎵', ogg: '🎵',
    mp4: '🎬', avi: '🎬', mkv: '🎬', mov: '🎬', webm: '🎬',
    zip: '📦', gz: '📦', tar: '📦', bz2: '📦', xz: '📦', '7z': '📦', rar: '📦',
    pdf: '📕',
    ttf: '🔤', otf: '🔤', woff: '🔤', woff2: '🔤',
  };
  return map[ext] || '📄';
}

function isPreviewable(entry: FileEntry): boolean {
  if (entry.is_dir) return false;
  const ext = entry.name.split('.').pop()?.toLowerCase() || '';
  const textExts = new Set([
    'txt', 'md', 'json', 'js', 'ts', 'tsx', 'jsx', 'py', 'rs', 'go', 'java',
    'c', 'h', 'cpp', 'hpp', 'html', 'css', 'scss', 'less', 'yaml', 'yml',
    'toml', 'ini', 'cfg', 'conf', 'sh', 'bash', 'zsh', 'fish', 'sql', 'xml',
    'csv', 'log', 'env', 'gitignore', 'dockerfile', 'makefile',
  ]);
  const imageExts = new Set(['jpg', 'jpeg', 'png', 'gif', 'svg', 'webp', 'ico', 'bmp']);
  const audioExts = new Set(['mp3', 'wav', 'flac', 'aac', 'ogg']);
  const videoExts = new Set(['mp4', 'webm', 'ogg']);
  const fontExts = new Set(['ttf', 'otf', 'woff', 'woff2']);
  return textExts.has(ext) || imageExts.has(ext) || audioExts.has(ext) || videoExts.has(ext) || fontExts.has(ext) || ext === 'pdf';
}

function isArchive(entry: FileEntry): boolean {
  if (entry.is_dir) return false;
  const name = entry.name.toLowerCase();
  return /\.(tar\.gz|tar\.bz2|tar\.xz|tgz|tbz2|txz|tar|zip|7z|rar)$/.test(name);
}

function isImage(entry: FileEntry): boolean {
  const ext = entry.name.split('.').pop()?.toLowerCase() || '';
  return ['jpg', 'jpeg', 'png', 'gif', 'svg', 'webp', 'ico', 'bmp'].includes(ext);
}

function isText(entry: FileEntry): boolean {
  const ext = entry.name.split('.').pop()?.toLowerCase() || '';
  const textExts = new Set([
    'txt', 'md', 'json', 'js', 'ts', 'tsx', 'jsx', 'py', 'rs', 'go', 'java',
    'c', 'h', 'cpp', 'hpp', 'html', 'css', 'scss', 'less', 'yaml', 'yml',
    'toml', 'ini', 'cfg', 'conf', 'sh', 'bash', 'zsh', 'fish', 'sql', 'xml',
    'csv', 'log', 'env',
  ]);
  return textExts.has(ext);
}

function isAudio(entry: FileEntry): boolean {
  const ext = entry.name.split('.').pop()?.toLowerCase() || '';
  return ['mp3', 'wav', 'flac', 'aac', 'ogg'].includes(ext);
}

function isVideo(entry: FileEntry): boolean {
  const ext = entry.name.split('.').pop()?.toLowerCase() || '';
  return ['mp4', 'webm', 'ogg'].includes(ext);
}

function toHexDump(bytes: Uint8Array, maxLines = 16): string {
  const lines: string[] = [];
  for (let i = 0; i < Math.min(bytes.length, maxLines * 16); i += 16) {
    const hex = Array.from(bytes.subarray(i, Math.min(i + 16, bytes.length)))
      .map((b) => b.toString(16).padStart(2, '0'))
      .join(' ');
    const ascii = Array.from(bytes.subarray(i, Math.min(i + 16, bytes.length)))
      .map((b) => (b >= 32 && b <= 126 ? String.fromCharCode(b) : '.'))
      .join('');
    lines.push(`${i.toString(16).padStart(8, '0')}  ${hex.padEnd(47)}  |${ascii}|`);
  }
  return lines.join('\n');
}

function joinPath(base: string, name: string): string {
  if (base.endsWith('/')) return `${base}${name}`;
  return `${base}/${name}`;
}

const CONTEXT_MENU_VIEWPORT_MARGIN = 8;

function clampNumber(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

function getViewportSafeMenuPosition(
  anchorX: number,
  anchorY: number,
  menuWidth: number,
  menuHeight: number,
): { left: number; top: number } {
  const margin = CONTEXT_MENU_VIEWPORT_MARGIN;
  const maxLeft = Math.max(margin, window.innerWidth - menuWidth - margin);
  const maxTop = Math.max(margin, window.innerHeight - menuHeight - margin);
  const preferredLeft = anchorX + menuWidth + margin > window.innerWidth
    ? anchorX - menuWidth
    : anchorX;
  const preferredTop = anchorY + menuHeight + margin > window.innerHeight
    ? anchorY - menuHeight
    : anchorY;

  return {
    left: clampNumber(preferredLeft, margin, maxLeft),
    top: clampNumber(preferredTop, margin, maxTop),
  };
}

// ---------------------------------------------------------------------------
// FileRow (memoized)
// ---------------------------------------------------------------------------

interface FileRowProps {
  entry: FileEntry;
  isSelected: boolean;
  onClick: () => void;
  onDoubleClick: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
  onDragStart: (e: React.DragEvent) => void;
}

const FileRow: FC<FileRowProps> = memo(({ entry, isSelected, onClick, onDoubleClick, onContextMenu, onDragStart }) => (
  <div
    className={`sftp-file-row ${isSelected ? 'sftp-file-selected' : ''}`}
    onClick={onClick}
    onDoubleClick={onDoubleClick}
    onContextMenu={onContextMenu}
    draggable={!entry.is_dir}
    onDragStart={onDragStart}
  >
    <span className="sftp-file-icon">{fileIcon(entry)}</span>
    <span className={`sftp-file-name ${entry.is_dir ? 'sftp-file-dir' : ''}`}>{entry.name}</span>
    <span className="sftp-file-size">{entry.is_dir ? '' : formatSize(entry.size)}</span>
    <span className="sftp-file-modified">{entry.modified || ''}</span>
  </div>
), (prev, next) =>
  prev.entry.path === next.entry.path
  && prev.entry.name === next.entry.name
  && prev.entry.size === next.entry.size
  && prev.entry.is_dir === next.entry.is_dir
  && prev.entry.modified === next.entry.modified
  && prev.isSelected === next.isSelected
);

FileRow.displayName = 'FileRow';

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface SftpPanelProps {
  hostId: string;
  hideTabBar?: boolean;
  forceTab?: 'sftp' | 'forward';
}

// ---------------------------------------------------------------------------
// FilePane
// ---------------------------------------------------------------------------

interface FilePaneProps {
  side: 'local' | 'remote';
  hostId: string;
  onContextMenu: (side: 'local' | 'remote', entry: FileEntry, event: React.MouseEvent) => void;
}

const FilePane: FC<FilePaneProps> = memo(({ side, hostId, onContextMenu }) => {
  const { tText } = useTranslation();
  const path = useSftpStore((s) => side === 'local' ? s.localPath : s.remotePath);
  const entries = useSftpStore((s) => side === 'local' ? s.localEntries : s.remoteEntries);
  const loading = useSftpStore((s) => side === 'local' ? s.localLoading : s.remoteLoading);
  const error = useSftpStore((s) => side === 'local' ? s.localError : s.remoteError);
  const selected = useSftpStore((s) => side === 'local' ? s.selectedLocal : s.selectedRemote);
  const setSelected = useSftpStore((s) => side === 'local' ? s.setSelectedLocal : s.setSelectedRemote);
  const navigateLocal = useSftpStore((s) => s.navigateLocal);
  const navigateRemote = useSftpStore((s) => s.navigateRemote);
  const goLocalUp = useSftpStore((s) => s.goLocalUp);
  const goRemoteUp = useSftpStore((s) => s.goRemoteUp);
  const refreshLocal = useSftpStore((s) => s.refreshLocal);
  const refreshRemote = useSftpStore((s) => s.refreshRemote);
  const upload = useSftpStore((s) => s.upload);
  const download = useSftpStore((s) => s.download);
  const remotePath = useSftpStore((s) => s.remotePath);
  const localPath = useSftpStore((s) => s.localPath);
  const localEntries = useSftpStore((s) => s.localEntries);
  const localLoading = useSftpStore((s) => s.localLoading);

  const navigate = side === 'local' ? navigateLocal : (p: string) => navigateRemote(hostId, p);
  const goUp = side === 'local' ? goLocalUp : () => goRemoteUp(hostId);
  const refresh = side === 'local' ? refreshLocal : () => refreshRemote(hostId);
  const transferFn = side === 'local'
    ? (entry: FileEntry) => upload(hostId, entry.path, remotePath)
    : (entry: FileEntry) => download(hostId, entry.path, localPath);
  const [pathInput, setPathInput] = useState(path);
  const [dragOver, setDragOver] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setPathInput(path);
  }, [path]);

  useEffect(() => {
    if (side === 'local' && localEntries.length === 0 && !localLoading) {
      invoke<string>('local_home_dir').then((home) => {
        navigateLocal(home);
      }).catch(() => {});
    }
  }, [side]);

  const handleNavigate = useCallback(() => {
    if (pathInput.trim()) {
      navigate(pathInput.trim());
    }
  }, [pathInput, navigate]);

  const openInIde = useCallback(async (entry: FileEntry) => {
    const existing = await WebviewWindow.getByLabel('editor');
    if (existing) {
      await existing.destroy();
    }
    const baseUrl = window.location.origin;
    const mode = entry.is_dir ? 'dir' : 'file';
    const url = `${baseUrl}/editor?hostId=${encodeURIComponent(hostId)}&path=${encodeURIComponent(entry.path)}&mode=${mode}`;
    const title = tText('sftp.editTitle', { name: entry.name, side: side === 'local' ? tText('sftp.local') : tText('sftp.remote') });
    const webview = new WebviewWindow('editor', {
      url,
      title,
      width: 1200,
      height: 800,
      minWidth: 800,
      minHeight: 600,
      decorations: false,
      transparent: true,
      backgroundColor: '#00000000',
    });
    webview.once('tauri://error', (e) => {
      console.error('Failed to open editor window:', e);
    });
  }, [hostId, side]);

  const handleDoubleClick = useCallback((entry: FileEntry) => {
    if (entry.is_dir) {
      navigate(entry.path);
    } else {
      openInIde(entry);
    }
  }, [navigate, openInIde]);

  const handleDragStart = useCallback((entry: FileEntry) => (e: React.DragEvent) => {
    e.dataTransfer.setData('application/x-sftp-file', JSON.stringify({ side, entry }));
    e.dataTransfer.effectAllowed = 'copy';
  }, [side]);

  const handleDrop = useCallback(async (e: React.DragEvent) => {
    e.preventDefault();
    setDragOver(false);
    const raw = e.dataTransfer.getData('application/x-sftp-file');
    if (!raw) return;
    const { side: sourceSide, entry } = JSON.parse(raw) as { side: string; entry: FileEntry };
    if (sourceSide === side) return;
    if (entry.is_dir) return;
    await transferFn(entry);
  }, [side, transferFn]);

  const selectedPath = selected?.path ?? null;
  const selectedIndex = useMemo(() => {
    if (!selectedPath) return -1;
    return entries.findIndex((entry) => entry.path === selectedPath);
  }, [entries, selectedPath]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp' && e.key !== 'Enter') return;
    if (!entries.length) return;

    if (e.key === 'Enter') {
      if (!selected) return;
      e.preventDefault();
      handleDoubleClick(selected);
      return;
    }

    const idx = selectedIndex >= 0 ? selectedIndex : -1;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      const next = entries[Math.min(idx + 1, entries.length - 1)];
      if (next) setSelected(next);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      const prev = entries[Math.max(idx - 1, 0)];
      if (prev) setSelected(prev);
    }
  }, [selected, selectedIndex, entries, handleDoubleClick, setSelected]);

  const rowVirtualizer = useVirtualizer<HTMLDivElement, HTMLDivElement>({
    count: !loading && !error ? entries.length : 0,
    getScrollElement: () => listRef.current,
    estimateSize: () => SFTP_FILE_ROW_HEIGHT,
    overscan: 8,
  });

  useEffect(() => {
    if (selectedIndex >= 0) {
      rowVirtualizer.scrollToIndex(selectedIndex, { align: 'auto' });
    }
  }, [rowVirtualizer, selectedIndex]);

  const makeRowCallbacks = useCallback((entry: FileEntry) => ({
    onClick: () => setSelected(entry),
    onDoubleClick: () => handleDoubleClick(entry),
    onContextMenu: (e: React.MouseEvent) => onContextMenu(side, entry, e),
    onDragStart: handleDragStart(entry),
  }), [setSelected, handleDoubleClick, onContextMenu, side, handleDragStart]);

  const label = side === 'local' ? tText('sftp.local') : tText('sftp.remote');

  return (
    <div className={`sftp-pane ${dragOver ? 'sftp-pane-drag-over' : ''}`}>
      <div className="sftp-pane-header">
        <span className="sftp-pane-label">{label}</span>
        <div className="sftp-path-bar">
          <input
            className="sftp-path-input"
            value={pathInput}
            onChange={(e) => setPathInput(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && handleNavigate()}
            placeholder="/"
          />
          <button className="sftp-btn-icon" onClick={goUp} title={tText('sftp.parentDir')}>↑</button>
          <button className="sftp-btn-icon" onClick={() => refresh()} title={tText('common.refresh')}>↻</button>
        </div>
      </div>
      <div
        className="sftp-file-list"
        ref={listRef}
        tabIndex={0}
        onKeyDown={handleKeyDown}
        onDragOver={(e) => { e.preventDefault(); setDragOver(true); }}
        onDragLeave={() => setDragOver(false)}
        onDrop={handleDrop}
      >
        {loading && <div className="sftp-loading">{tText('sftp.loadingFiles')}</div>}
        {error && <div className="sftp-error">{error}</div>}
        {!loading && !error && entries.length > 0 && (
          <div
            className="sftp-file-list-spacer"
            style={{ height: rowVirtualizer.getTotalSize(), position: 'relative' }}
          >
            {rowVirtualizer.getVirtualItems().map((virtualRow) => {
              const entry = entries[virtualRow.index];
              if (!entry) return null;
              const cbs = makeRowCallbacks(entry);
              return (
                <div
                  key={entry.path}
                  className="sftp-file-row-virtual"
                  style={{ transform: `translateY(${virtualRow.start}px)` }}
                >
                  <FileRow
                    entry={entry}
                    isSelected={selectedPath === entry.path}
                    onClick={cbs.onClick}
                    onDoubleClick={cbs.onDoubleClick}
                    onContextMenu={cbs.onContextMenu}
                    onDragStart={cbs.onDragStart}
                  />
                </div>
              );
            })}
          </div>
        )}
        {!loading && !error && entries.length === 0 && (
          <div className="sftp-empty">{tText('sftp.emptyDir')}</div>
        )}
      </div>
    </div>
  );
});
FilePane.displayName = 'FilePane';

// ---------------------------------------------------------------------------
// FilePreview
// ---------------------------------------------------------------------------

const FilePreview: FC = memo(() => {
  const { tText } = useTranslation();
  const previewFile = useSftpStore((s) => s.previewFile);
  const previewLoading = useSftpStore((s) => s.previewLoading);
  const closePreview = useSftpStore((s) => s.closePreview);

  const { entry, data } = previewFile || { entry: null, data: null };

  const previewBytes = useMemo(() => {
    if (!entry || !data) return null;
    return Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
  }, [entry, data]);

  const objectUrl = useMemo(() => {
    if (!entry || !previewBytes || (!isImage(entry) && !isAudio(entry) && !isVideo(entry))) return null;
    const ext = entry.name.split('.').pop()?.toLowerCase() || '';

    if (isImage(entry)) {
      const mimeMap: Record<string, string> = {
        jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png',
        gif: 'image/gif', svg: 'image/svg+xml', webp: 'image/webp',
        ico: 'image/x-icon', bmp: 'image/bmp',
      };
      return URL.createObjectURL(new Blob([previewBytes], { type: mimeMap[ext] || 'image/png' }));
    }

    if (isAudio(entry)) {
      const mimeMap: Record<string, string> = {
        mp3: 'audio/mpeg', wav: 'audio/wav', flac: 'audio/flac',
        aac: 'audio/aac', ogg: 'audio/ogg',
      };
      return URL.createObjectURL(new Blob([previewBytes], { type: mimeMap[ext] || 'audio/mpeg' }));
    }

    const mimeMap: Record<string, string> = {
      mp4: 'video/mp4', webm: 'video/webm', ogg: 'video/ogg',
    };
    return URL.createObjectURL(new Blob([previewBytes], { type: mimeMap[ext] || 'video/mp4' }));
  }, [entry, previewBytes]);

  useEffect(() => {
    return () => {
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [objectUrl]);

  if (!previewFile && !previewLoading) return null;

  let content: ReactNode = null;

  if (previewLoading) {
    content = <div className="sftp-preview-loading">{tText('sftp.loadingFiles')}</div>;
  } else if (entry && previewBytes) {
    if (isImage(entry) && objectUrl) {
      content = <img src={objectUrl} alt={entry.name} className="sftp-preview-image" />;
    } else if (isText(entry)) {
      const text = new TextDecoder('utf-8', { fatal: false }).decode(previewBytes);
      content = (
        <pre className="sftp-preview-text">
          <code>{text.length > 100000 ? text.slice(0, 100000) + tText('sftp.truncated') : text}</code>
        </pre>
      );
    } else if (isAudio(entry) && objectUrl) {
      content = <audio controls src={objectUrl} className="sftp-preview-audio" />;
    } else if (isVideo(entry) && objectUrl) {
      content = <video controls src={objectUrl} className="sftp-preview-video" />;
    } else {
      content = <pre className="sftp-preview-hex">{toHexDump(previewBytes)}</pre>;
    }
  }

  return (
    <div className="sftp-preview">
      <div className="sftp-preview-header">
        <span className="sftp-preview-title">{entry?.name || tText('sftp.preview')}</span>
        <button className="sftp-btn-icon" onClick={closePreview}>✗</button>
      </div>
      <div className="sftp-preview-body">{content}</div>
    </div>
  );
});
FilePreview.displayName = 'FilePreview';

// ---------------------------------------------------------------------------
// TransferQueue
// ---------------------------------------------------------------------------

const TransferQueue: FC = memo(() => {
  const { tText } = useTranslation();
  const transfers = useSftpStore((s) => s.transfers);
  const visibleTransfers = useMemo(() => transfers.slice(-20).reverse(), [transfers]);
  const { active, completed } = useMemo(() => transfers.reduce(
    (stats, transfer) => {
      if (transfer.status === 'transferring' || transfer.status === 'pending') {
        stats.active += 1;
      } else if (transfer.status === 'done' || transfer.status === 'error') {
        stats.completed += 1;
      }
      return stats;
    },
    { active: 0, completed: 0 },
  ), [transfers]);

  if (transfers.length === 0) return null;

  return (
    <div className="sftp-transfers">
      <div className="sftp-transfers-header">
        {tText('sftp.transferQueue', { active, completed })}
      </div>
      <div className="sftp-transfers-list">
        {visibleTransfers.map((t) => (
          <div key={t.id} className={`sftp-transfer-item sftp-transfer-${t.status}`}>
            <span className="sftp-transfer-icon">{t.direction === 'upload' ? '↑' : '↓'}</span>
            <span className="sftp-transfer-name">{t.fileName}</span>
            <span className="sftp-transfer-size">{formatSize(t.fileSize)}</span>
            {t.status === 'transferring' && (
              <div className="sftp-transfer-progress">
                <div className="sftp-transfer-progress-bar" style={{ width: `${t.progress}%` }} />
              </div>
            )}
            {t.status === 'done' && (
              <span className="sftp-transfer-done">
                {formatDuration(t.durationMs)} · {formatSpeed(t.speed)}
              </span>
            )}
            {t.status === 'error' && (
              <span className="sftp-transfer-error-text" title={t.error || ''}>{tText('sftp.transferFailed')}</span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
});
TransferQueue.displayName = 'TransferQueue';

// ---------------------------------------------------------------------------
// ContextMenu
// ---------------------------------------------------------------------------

interface ContextMenuState {
  x: number;
  y: number;
  side: 'local' | 'remote';
  entry: FileEntry;
}

const FileContextMenu: FC<{
  menu: ContextMenuState;
  hostId: string;
  onClose: () => void;
}> = ({ menu, hostId, onClose }) => {
  const { tText } = useTranslation();
  const previewLocal = useSftpStore((s) => s.previewLocal);
  const previewRemote = useSftpStore((s) => s.previewRemote);
  const uploadFn = useSftpStore((s) => s.upload);
  const downloadFn = useSftpStore((s) => s.download);
  const localRemove = useSftpStore((s) => s.localRemove);
  const remoteRemove = useSftpStore((s) => s.remoteRemove);
  const localRename = useSftpStore((s) => s.localRename);
  const remoteRename = useSftpStore((s) => s.remoteRename);
  const extractArchive = useSftpStore((s) => s.extractArchive);
  const localPath = useSftpStore((s) => s.localPath);
  const remotePath = useSftpStore((s) => s.remotePath);
  const localMkdir = useSftpStore((s) => s.localMkdir);
  const remoteMkdir = useSftpStore((s) => s.remoteMkdir);
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState(() => ({ left: menu.x, top: menu.y }));

  const handlePreview = () => {
    if (menu.side === 'local') {
      previewLocal(menu.entry);
    } else {
      previewRemote(hostId, menu.entry);
    }
    onClose();
  };

  const handleTransfer = () => {
    if (menu.side === 'local') {
      uploadFn(hostId, menu.entry.path, remotePath);
    } else {
      downloadFn(hostId, menu.entry.path, localPath);
    }
    onClose();
  };

  const handleDelete = async () => {
    const name = menu.entry.name;
    if (!confirm(tText('sftp.deleteConfirm', { name }))) return;
    if (menu.side === 'local') {
      await localRemove(menu.entry.path);
    } else {
      await remoteRemove(hostId, menu.entry.path, menu.entry.is_dir);
    }
    onClose();
  };

  const handleRename = async () => {
    const newName = prompt(tText('sftp.renamePrompt'), menu.entry.name);
    if (!newName || newName === menu.entry.name) return;
    const parent = menu.entry.path.replace(/\/[^/]*$/, '');
    const newPath = joinPath(parent, newName);
    if (menu.side === 'local') {
      await localRename(menu.entry.path, newPath);
    } else {
      await remoteRename(hostId, menu.entry.path, newPath);
    }
    onClose();
  };

  const handleExtract = async () => {
    await extractArchive(hostId, menu.entry.path, '');
    onClose();
  };

  const handleMkdir = async () => {
    const name = prompt(tText('sftp.newFolderPrompt'));
    if (!name) return;
    const newPath = joinPath(menu.side === 'local' ? localPath : remotePath, name);
    if (menu.side === 'local') {
      await localMkdir(newPath);
    } else {
      await remoteMkdir(hostId, newPath);
    }
    onClose();
  };

  const handleIdeOpen = async () => {
    onClose();
    const existing = await WebviewWindow.getByLabel('editor');
    if (existing) {
      await existing.destroy();
    }
    const mode = menu.entry.is_dir ? 'dir' : 'file';
    const baseUrl = window.location.origin;
    const url = `${baseUrl}/editor?hostId=${encodeURIComponent(hostId)}&path=${encodeURIComponent(menu.entry.path)}&mode=${mode}`;
    const title = menu.entry.is_dir
      ? tText('sftp.editTitle', { name: `${menu.entry.name}/`, side: tText('sftp.remote') })
      : tText('sftp.editTitle', { name: menu.entry.name, side: tText('sftp.remote') });
    const webview = new WebviewWindow('editor', {
      url,
      title,
      width: 1200,
      height: 800,
      minWidth: 800,
      minHeight: 600,
      decorations: false,
      transparent: true,
      backgroundColor: '#00000000',
    });
    webview.once('tauri://error', (e) => {
      console.error('Failed to open editor window:', e);
    });
  };

  const items: { label: string; action: () => void; separator?: boolean }[] = [];

  if (menu.side === 'remote') {
    items.push({ label: menu.entry.is_dir ? tText('sftp.ideOpenDir') : tText('sftp.ideOpenFile'), action: handleIdeOpen });
  }

  if (!menu.entry.is_dir) {
    if (isPreviewable(menu.entry)) {
      items.push({ label: tText('sftp.preview'), action: handlePreview });
    }
    items.push({ label: menu.side === 'local' ? tText('sftp.uploadToRemote') : tText('sftp.downloadToLocal'), action: handleTransfer });
    if (isArchive(menu.entry) && menu.side === 'remote') {
      items.push({ label: tText('sftp.extract'), action: handleExtract });
    }
  }

  items.push({ label: tText('sftp.rename'), action: handleRename });
  items.push({ label: tText('common.delete'), action: handleDelete });
  items.push({ label: tText('sftp.newFolder'), action: handleMkdir, separator: true });

  useLayoutEffect(() => {
    const element = menuRef.current;
    if (!element) return;

    const rect = element.getBoundingClientRect();
    setPosition(getViewportSafeMenuPosition(menu.x, menu.y, rect.width, rect.height));
  }, [items.length, menu.entry.is_dir, menu.entry.name, menu.side, menu.x, menu.y]);

  return createPortal(
    <div
      className="sftp-context-menu-overlay"
      onClick={onClose}
      onContextMenu={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      <div
        ref={menuRef}
        className="sftp-context-menu"
        style={{ left: position.left, top: position.top }}
        onClick={(e) => e.stopPropagation()}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
        }}
      >
        {items.map((item, i) => (
          <div key={i}>
            {item.separator && <div className="sftp-context-separator" />}
            <button className="sftp-context-item" onClick={item.action}>{item.label}</button>
          </div>
        ))}
      </div>
    </div>,
    document.body,
  );
};

// ---------------------------------------------------------------------------
// BookmarkBar
// ---------------------------------------------------------------------------

const BookmarkBar: FC<{ hostId: string }> = memo(({ hostId }) => {
  const { tText } = useTranslation();
  const bookmarks = useSftpStore((s) => s.bookmarks);
  const localPath = useSftpStore((s) => s.localPath);
  const remotePath = useSftpStore((s) => s.remotePath);
  const addBookmark = useSftpStore((s) => s.addBookmark);
  const removeBookmark = useSftpStore((s) => s.removeBookmark);
  const navigateLocal = useSftpStore((s) => s.navigateLocal);
  const navigateRemote = useSftpStore((s) => s.navigateRemote);
  const [showAdd, setShowAdd] = useState(false);

  const navigate = (bookmark: typeof bookmarks[0]) => {
    if (bookmark.side === 'local') {
      navigateLocal(bookmark.path);
    } else {
      navigateRemote(hostId, bookmark.path);
    }
  };

  const addCurrent = (side: 'local' | 'remote') => {
    const path = side === 'local' ? localPath : remotePath;
    const name = path.split('/').pop() || path;
    addBookmark({ id: '', name, path, side });
    setShowAdd(false);
  };

  return (
    <div className="sftp-bookmarks">
      {bookmarks.map((bm) => (
        <button
          key={bm.id}
          className="sftp-bookmark-btn"
          onClick={() => navigate(bm)}
          title={`${bm.side === 'local' ? tText('sftp.local') : tText('sftp.remote')}: ${bm.path}`}
        >
          {bm.side === 'local' ? '■' : '□'} {bm.name}
          <span
            className="sftp-bookmark-remove"
            onClick={(e) => { e.stopPropagation(); removeBookmark(bm.id); }}
          >✗</span>
        </button>
      ))}
      <button className="sftp-bookmark-add" onClick={() => setShowAdd(!showAdd)}>
        {tText('sftp.addBookmark')}
      </button>
      {showAdd && (
        <span className="sftp-bookmark-add-menu">
          <button onClick={() => addCurrent('local')}>{tText('sftp.localCurrentDir')}</button>
          <button onClick={() => addCurrent('remote')}>{tText('sftp.remoteCurrentDir')}</button>
        </span>
      )}
    </div>
  );
});
BookmarkBar.displayName = 'BookmarkBar';

// ---------------------------------------------------------------------------
// SftpPanel (main)
// ---------------------------------------------------------------------------

export const SftpPanel: FC<SftpPanelProps> = ({ hostId, hideTabBar, forceTab }) => {
  const { tText } = useTranslation();
  const initSession = useSftpStore((s) => s.initSession);
  const navigateRemote = useSftpStore((s) => s.navigateRemote);
  const transfers = useSftpStore((s) => s.transfers);
  const [initialized, setInitialized] = useState(false);
  const [initError, setInitError] = useState('');
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [splitRatio, setSplitRatio] = useState(0.5);
  const [showTransfers, setShowTransfers] = useState(true);
  const [activeTab, setActiveTab] = useState<'sftp' | 'forward'>(forceTab || 'sftp');
  const panelRef = useRef<HTMLDivElement>(null);
  const draggingSplit = useRef(false);

  const initializeSession = useCallback(async () => {
    setInitialized(false);
    setInitError('');
    try {
      const home = await initSession(hostId);
      navigateRemote(hostId, home);
      setInitialized(true);
    } catch (e: unknown) {
      setInitError(e instanceof Error ? e.message : String(e));
    }
  }, [hostId, initSession, navigateRemote]);

  useEffect(() => {
    let cancelled = false;
    setInitialized(false);
    setInitError('');
    initSession(hostId).then((home) => {
      if (cancelled) return;
      navigateRemote(hostId, home);
      setInitialized(true);
    }).catch((e: unknown) => {
      if (cancelled) return;
      setInitError(e instanceof Error ? e.message : String(e));
    });
    return () => { cancelled = true; };
  }, [hostId, initSession, navigateRemote]);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    draggingSplit.current = true;
    const panel = panelRef.current;
    if (!panel) return;

    const onMouseMove = (ev: MouseEvent) => {
      if (!draggingSplit.current || !panel) return;
      const rect = panel.getBoundingClientRect();
      const ratio = (ev.clientX - rect.left) / rect.width;
      setSplitRatio(Math.max(0.2, Math.min(0.8, ratio)));
    };

    const onMouseUp = () => {
      draggingSplit.current = false;
      window.removeEventListener('mousemove', onMouseMove);
      window.removeEventListener('mouseup', onMouseUp);
    };

    window.addEventListener('mousemove', onMouseMove);
    window.addEventListener('mouseup', onMouseUp);
  }, []);

  const handleContextMenu = useCallback((side: 'local' | 'remote', entry: FileEntry, e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setContextMenu({ x: e.clientX, y: e.clientY, side, entry });
  }, []);

  const transferStats = useMemo(() => transfers.reduce(
    (stats, transfer) => {
      stats.total += 1;
      if (transfer.status === 'transferring' || transfer.status === 'pending') {
        stats.active += 1;
      }
      return stats;
    },
    { active: 0, total: 0 },
  ), [transfers]);

  if (!initialized) {
    return <div className="sftp-panel sftp-panel-loading">
      {initError ? (
        <div className="sftp-error">
          <div>{tText('sftp.initFailed', { error: initError })}</div>
          <button className="sftp-btn" onClick={() => { void initializeSession(); }}>{tText('sftp.retry')}</button>
        </div>
      ) : (
        <div className="sftp-skeleton">
          <div className="sftp-skeleton-bar" style={{ width: '40%' }} />
          <div className="sftp-skeleton-row">
            <div className="sftp-skeleton-block" style={{ width: '48%', height: '120px' }} />
            <div className="sftp-skeleton-block" style={{ width: '48%', height: '120px' }} />
          </div>
        </div>
      )}
    </div>;
  }

  return (
    <div className="sftp-panel" ref={panelRef}>
      {transferStats.total > 0 && (
        <button
          className={`sftp-panel-toggle-transfers${showTransfers ? ' sftp-panel-toggle-transfers-active' : ''}`}
          onClick={() => setShowTransfers(v => !v)}
          title={showTransfers ? tText('sftp.hideTransferQueue') : tText('sftp.showTransferQueue')}
        >
          {transferStats.active > 0 ? transferStats.active : '✓'}
        </button>
      )}
      {!hideTabBar && (
        <div className="sftp-tab-bar">
          <button
            className={`sftp-tab-btn ${activeTab === 'sftp' ? 'sftp-tab-btn-active' : ''}`}
            onClick={() => setActiveTab('sftp')}
          >
            SFTP
          </button>
          <button
            className={`sftp-tab-btn ${activeTab === 'forward' ? 'sftp-tab-btn-active' : ''}`}
            onClick={() => setActiveTab('forward')}
          >
            {tText('sftp.portForward')}
          </button>
        </div>
      )}
      {activeTab === 'sftp' ? (
        <>
          <BookmarkBar hostId={hostId} />
          <div className="sftp-panes">
            <div
              className="sftp-pane-local"
              style={{ width: `${splitRatio * 100}%` }}
            >
              <FilePane side="local" hostId={hostId} onContextMenu={handleContextMenu} />
            </div>
            <div className="sftp-splitter" onMouseDown={handleMouseDown} />
            <div
              className="sftp-pane-remote"
              style={{ width: `${(1 - splitRatio) * 100}%` }}
            >
              <FilePane side="remote" hostId={hostId} onContextMenu={handleContextMenu} />
            </div>
          </div>
          <FilePreview />
          {showTransfers && <TransferQueue />}
          {contextMenu && (
            <FileContextMenu
              menu={contextMenu}
              hostId={hostId}
              onClose={() => setContextMenu(null)}
            />
          )}
        </>
      ) : (
        <PortForwardPanel hostId={hostId} />
      )}
    </div>
  );
};

export default SftpPanel;
