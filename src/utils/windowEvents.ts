// 跨窗口事件常量。
// 独立窗口（VNC/RDP 等）通过这些事件与主窗口通信，
// 主窗口在 components/MainLayout.tsx 中监听并响应。

/** 独立窗口请求主窗口打开资产管理面板。payload: { sourceWindowLabel } */
export const OPEN_ASSET_MANAGER_EVENT = 'open-asset-manager';

export interface OpenAssetManagerPayload {
  /** 发起请求的窗口 label，用于让主窗口忽略自身发出的请求 */
  sourceWindowLabel?: string;
}
