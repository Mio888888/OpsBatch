import { createPortal } from 'react-dom';
import type { TerminalTabCloseMode } from '../../utils/terminalTabs';

export interface TerminalSplitContextMenu {
  tabKey: string;
  terminalKey: string;
  x: number;
  y: number;
  selectedText?: string;
}

export interface TerminalTabContextMenu {
  kind: 'tab' | 'list';
  tabKey?: string;
  x: number;
  y: number;
}

interface SplitContextMenuProps {
  menu: TerminalSplitContextMenu | null;
  copyLabel: string;
  pasteLabel: string;
  aiAnalyzeLabel: string;
  openSplitLabel: string;
  onCopySelection: (selectedText: string) => void;
  onPasteClipboard: (terminalKey: string) => void;
  onAiAnalyzeSelection: (tabKey: string, selectedText: string) => void;
  onOpenSplit: (tabKey: string) => void;
}

interface TabContextMenuProps {
  menu: TerminalTabContextMenu | null;
  labels: {
    newConnection: string;
    closeAll: string;
    closeOthers: string;
    closeLeft: string;
    closeRight: string;
  };
  onNewConnection: () => void;
  onCloseAll: () => void;
  onCloseTabsByMode: (targetKey: string, mode: TerminalTabCloseMode) => void;
}

export function SplitContextMenu({
  menu,
  copyLabel,
  pasteLabel,
  aiAnalyzeLabel,
  openSplitLabel,
  onCopySelection,
  onPasteClipboard,
  onAiAnalyzeSelection,
  onOpenSplit,
}: SplitContextMenuProps) {
  if (!menu) return null;

  return createPortal(
    <div
      className="terminal-context-menu"
      style={{ left: menu.x, top: menu.y }}
      onClick={(event) => event.stopPropagation()}
    >
      {menu.selectedText ? (
        <button type="button" className="terminal-context-menu-item" onClick={() => onCopySelection(menu.selectedText || '')}>
          {copyLabel}
        </button>
      ) : (
        <button type="button" className="terminal-context-menu-item" onClick={() => onPasteClipboard(menu.terminalKey)}>
          {pasteLabel}
        </button>
      )}
      {menu.selectedText && (
        <>
          <div className="terminal-context-menu-separator" />
          <button type="button" className="terminal-context-menu-item terminal-context-menu-item-ai" onClick={() => onAiAnalyzeSelection(menu.tabKey, menu.selectedText || '')}>
            🔍 {aiAnalyzeLabel}
          </button>
        </>
      )}
      <div className="terminal-context-menu-separator" />
      <button type="button" className="terminal-context-menu-item" onClick={() => onOpenSplit(menu.tabKey)}>
        {openSplitLabel}
      </button>
    </div>,
    document.body,
  );
}

export function TabContextMenu({
  menu,
  labels,
  onNewConnection,
  onCloseAll,
  onCloseTabsByMode,
}: TabContextMenuProps) {
  if (!menu) return null;

  return createPortal(
    <div
      className="terminal-context-menu terminal-tab-context-menu"
      style={{ left: menu.x, top: menu.y }}
      onClick={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
    >
      {menu.kind === 'list' ? (
        <>
          <button type="button" className="terminal-context-menu-item" onClick={onNewConnection}>
            {labels.newConnection}
          </button>
          <div className="terminal-context-menu-separator" />
          <button type="button" className="terminal-context-menu-item terminal-context-menu-item-danger" onClick={onCloseAll}>
            {labels.closeAll}
          </button>
        </>
      ) : (
        <>
          <button type="button" className="terminal-context-menu-item terminal-context-menu-item-danger" onClick={() => menu.tabKey && onCloseTabsByMode(menu.tabKey, 'all')}>
            {labels.closeAll}
          </button>
          <button type="button" className="terminal-context-menu-item" onClick={() => menu.tabKey && onCloseTabsByMode(menu.tabKey, 'others')}>
            {labels.closeOthers}
          </button>
          <div className="terminal-context-menu-separator" />
          <button type="button" className="terminal-context-menu-item" onClick={() => menu.tabKey && onCloseTabsByMode(menu.tabKey, 'left')}>
            {labels.closeLeft}
          </button>
          <button type="button" className="terminal-context-menu-item" onClick={() => menu.tabKey && onCloseTabsByMode(menu.tabKey, 'right')}>
            {labels.closeRight}
          </button>
        </>
      )}
    </div>,
    document.body,
  );
}
