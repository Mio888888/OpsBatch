import { invoke } from "@tauri-apps/api/core";

// 调用后端切换当前窗口的 DevTools；后端仅在 debug 构建下生效
async function toggleDevtoolsForCurrentWindow() {
  try {
    await invoke("toggle_devtools");
  } catch (error) {
    // 非开发环境或权限不足时静默失败，避免干扰用户
    console.warn("[devtools] toggle failed:", error);
  }
}

// 判断按键组合是否为打开 DevTools 的标准快捷键
function isDevtoolsShortcut(event: KeyboardEvent): boolean {
  const key = event.key;
  // F12：Windows/Linux 通用
  if (key === "F12") return true;
  // Ctrl+Shift+I（Windows/Linux）或 Cmd+Option+I（macOS）
  const isMac = navigator.platform.toLowerCase().includes("mac");
  if (isMac) {
    return event.metaKey && event.altKey && (key === "i" || key === "I");
  }
  return event.ctrlKey && event.shiftKey && (key === "i" || key === "I");
}

// 注册全局 DevTools 快捷键，仅在开发环境生效，返回卸载函数
export function registerDevtoolsShortcut(): () => void {
  // import.meta.env.DEV 在 Vite 中表示当前为开发构建
  if (!import.meta.env.DEV) {
    return () => {};
  }

  const handler = (event: KeyboardEvent) => {
    if (!isDevtoolsShortcut(event)) return;
    event.preventDefault();
    void toggleDevtoolsForCurrentWindow();
  };

  window.addEventListener("keydown", handler, { capture: true });
  return () => {
    window.removeEventListener("keydown", handler, { capture: true } as EventListenerOptions);
  };
}
