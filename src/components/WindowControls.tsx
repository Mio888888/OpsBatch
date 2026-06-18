import { useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Maximize2, Minus, X } from 'lucide-react';

interface WindowControlsProps {
  className?: string;
}

function runWindowCommand(command: () => Promise<void>) {
  void command().catch((error) => {
    console.error('Window command failed:', error);
  });
}

export default function WindowControls({ className = '' }: WindowControlsProps) {
  const minimize = useCallback(() => {
    runWindowCommand(async () => {
      const window = getCurrentWindow();
      const settings = await invoke<Record<string, string>>('get_general_settings');
      if (window.label === 'main' && settings.minimizeToTray !== 'false') {
        await window.hide();
        return;
      }
      await window.minimize();
    });
  }, []);

  const toggleMaximize = useCallback(() => {
    runWindowCommand(() => getCurrentWindow().toggleMaximize());
  }, []);

  const close = useCallback(() => {
    runWindowCommand(async () => {
      const window = getCurrentWindow();
      if (window.label === 'main') {
        await window.close();
        return;
      }
      await window.destroy();
    });
  }, []);

  return (
    <div className={`window-controls ${className}`.trim()} aria-label="窗口控制">
      <button type="button" className="window-control window-control-close" onClick={close} aria-label="关闭">
        <X className="window-control-glyph" size={7} strokeWidth={3} aria-hidden="true" />
      </button>
      <button type="button" className="window-control window-control-minimize" onClick={minimize} aria-label="最小化">
        <Minus className="window-control-glyph" size={7} strokeWidth={3} aria-hidden="true" />
      </button>
      <button type="button" className="window-control window-control-maximize" onClick={toggleMaximize} aria-label="最大化或还原">
        <Maximize2 className="window-control-glyph" size={7} strokeWidth={3} aria-hidden="true" />
      </button>
    </div>
  );
}
