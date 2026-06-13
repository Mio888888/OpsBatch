import { invoke } from '@tauri-apps/api/core';
import type { RdpOperation } from './aiActionParser';
import { getScancodeForKey } from '../pages/Rdp/rdpProtocol';
import type { RdpInputEvent } from '../pages/Rdp/rdpProtocol';

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

// 将单个字符转成 unicode 输入事件对（按下 + 释放）
function unicodeEvents(char: string): RdpInputEvent[] {
  return [
    { type: 'unicode', character: char, down: true },
    { type: 'unicode', character: char, down: false },
  ];
}

// 修饰键与特殊键的 scancode 映射（不依赖 rdpProtocol 的 getScancodeForKey，
// 因为后者对 Meta 单键返回空，且 agent-rdp 需要稳定支持这些键）。
const MODIFIER_SCANCODES: Record<string, { code: number; extended: boolean }> = {
  Meta: { code: 0x5b, extended: true },
  MetaLeft: { code: 0x5b, extended: true },
  MetaRight: { code: 0x5c, extended: true },
  Win: { code: 0x5b, extended: true },
  Windows: { code: 0x5b, extended: true },
  Control: { code: 0x1d, extended: false },
  ControlLeft: { code: 0x1d, extended: false },
  ControlRight: { code: 0x1d, extended: true },
  Ctrl: { code: 0x1d, extended: false },
  Shift: { code: 0x2a, extended: false },
  ShiftLeft: { code: 0x2a, extended: false },
  ShiftRight: { code: 0x36, extended: false },
  Alt: { code: 0x38, extended: false },
  AltLeft: { code: 0x38, extended: false },
  AltRight: { code: 0x38, extended: true },
  AltGr: { code: 0x38, extended: true },
};

function resolveScancode(key: string): { code: number; extended: boolean } | null {
  if (MODIFIER_SCANCODES[key]) return MODIFIER_SCANCODES[key];
  const mapped = getScancodeForKey(key, key.length === 1 ? `Key${key.toUpperCase()}` : key);
  return mapped ?? null;
}

// 将按键名/单字符转成 scancode 输入事件对
function scancodeEvents(key: string): RdpInputEvent[] | null {
  const mapped = resolveScancode(key);
  if (!mapped) return null;
  return [
    { type: 'key_scancode', code: mapped.code, extended: mapped.extended, down: true },
    { type: 'key_scancode', code: mapped.code, extended: mapped.extended, down: false },
  ];
}

// 组合键：依次按下所有键，再逆序释放
function comboKeyEvents(keys: string[]): RdpInputEvent[] {
  const events: RdpInputEvent[] = [];
  const mapped = keys.map((key) => ({ key, sc: resolveScancode(key) }));

  for (const { key, sc } of mapped) {
    if (sc) {
      events.push({ type: 'key_scancode', code: sc.code, extended: sc.extended, down: true });
    } else if (key.length === 1) {
      events.push({ type: 'unicode', character: key, down: true });
    }
  }
  for (let idx = mapped.length - 1; idx >= 0; idx -= 1) {
    const { key, sc } = mapped[idx];
    if (sc) {
      events.push({ type: 'key_scancode', code: sc.code, extended: sc.extended, down: false });
    } else if (key.length === 1) {
      events.push({ type: 'unicode', character: key, down: false });
    }
  }
  return events;
}

export function rdpOperationsToInputEvents(ops: RdpOperation[]): RdpInputEvent[][] {
  const batches: RdpInputEvent[][] = [];

  for (const op of ops) {
    switch (op.type) {
      case 'move':
        batches.push([{ type: 'mouse_move', x: op.x, y: op.y }]);
        break;
      case 'click': {
        const button = op.button ?? 0;
        // 先移动到目标位置
        batches.push([{ type: 'mouse_move', x: op.x, y: op.y }]);
        // 按下+释放合并为同一批次（原子执行，避免按下保持过久变成拖拽）
        const pressBatch: RdpInputEvent[] = [
          { type: 'mouse_button', x: op.x, y: op.y, button, down: true },
          { type: 'mouse_button', x: op.x, y: op.y, button, down: false },
        ];
        if (op.doubleClick) {
          pressBatch.push(
            { type: 'mouse_button', x: op.x, y: op.y, button, down: true },
            { type: 'mouse_button', x: op.x, y: op.y, button, down: false },
          );
        }
        batches.push(pressBatch);
        break;
      }
      case 'drag': {
        const button = op.button ?? 0;
        batches.push([{ type: 'mouse_move', x: op.fromX, y: op.fromY }]);
        batches.push([{ type: 'mouse_button', x: op.fromX, y: op.fromY, button, down: true }]);
        // 拖拽过程中间点，让远程更平滑
        const steps = 4;
        for (let step = 1; step < steps; step += 1) {
          const px = Math.round(op.fromX + ((op.toX - op.fromX) * step) / steps);
          const py = Math.round(op.fromY + ((op.toY - op.fromY) * step) / steps);
          batches.push([{ type: 'mouse_move', x: px, y: py }]);
        }
        batches.push([{ type: 'mouse_move', x: op.toX, y: op.toY }]);
        batches.push([{ type: 'mouse_button', x: op.toX, y: op.toY, button, down: false }]);
        break;
      }
      case 'scroll': {
        const vertical = op.vertical ?? true;
        batches.push([{ type: 'mouse_move', x: op.x, y: op.y }]);
        // RDP wheel 以 WHEEL_DELTA(120) 为单位，正数向上/负数向下
        const units = op.delta * 120;
        batches.push([
          { type: 'wheel', x: op.x, y: op.y, delta: units, vertical },
        ]);
        break;
      }
      case 'type': {
        const chars = Array.from(op.text);
        const batch: RdpInputEvent[] = [];
        for (const ch of chars) {
          batch.push(...unicodeEvents(ch));
        }
        if (batch.length > 0) batches.push(batch);
        break;
      }
      case 'key': {
        if (op.keys.length === 1) {
          const events = scancodeEvents(op.keys[0]);
          if (events) batches.push(events);
        } else if (op.keys.length > 1) {
          batches.push(comboKeyEvents(op.keys));
        }
        break;
      }
      default:
        break;
    }
  }

  return batches;
}

// 根据批次内容推断操作类型，用于决定延迟时长。
// UI 切换类操作（key/click/drag）需要更长等待，让远程窗口/菜单响应。
type BatchKind = 'key' | 'click' | 'drag' | 'scroll' | 'type' | 'move';

function classifyBatch(batch: RdpInputEvent[]): BatchKind {
  if (batch.some((e) => e.type === 'key_scancode')) return 'key';
  if (batch.some((e) => e.type === 'unicode')) return 'type';
  if (batch.some((e) => e.type === 'mouse_button')) {
    // 一个完整 click = move + down + up，含 down/up 判定为 click
    return 'click';
  }
  if (batch.some((e) => e.type === 'wheel')) return 'scroll';
  if (batch.some((e) => e.type === 'mouse_move')) return 'move';
  return 'move';
}

function delayForKind(kind: BatchKind): number {
  switch (kind) {
    case 'key':
      // 按键（尤其 Enter/Meta）常触发 UI 切换（开菜单、启动程序），需要较长等待
      return 500;
    case 'click':
      // 点击后窗口/控件可能需要聚焦或响应
      return 350;
    case 'drag':
      return 300;
    case 'type':
      // 文本输入：字符间短间隔即可
      return 40;
    case 'scroll':
      return 120;
    case 'move':
    default:
      return 80;
  }
}

export interface RdpExecutorOptions {
  sessionId: string;
  /** 步骤执行完成后的尾延迟，让远程 UI 稳定后再进入下一步（默认 800ms） */
  settleDelayMs?: number;
}

export async function executeRdpOperations(
  ops: RdpOperation[],
  options: RdpExecutorOptions,
): Promise<void> {
  const { sessionId, settleDelayMs = 800 } = options;
  const batches = rdpOperationsToInputEvents(ops);

  for (let batchIdx = 0; batchIdx < batches.length; batchIdx += 1) {
    const batch = batches[batchIdx];
    for (const event of batch) {
      await invoke('rdp_send_input', { sessionId, event });
    }
    const kind = classifyBatch(batch);
    // 批次内事件本身是原子的（如 click 的 down+up），无需额外间隔；
    // 批次间按操作类型等待，让远程桌面有时间响应。
    await sleep(delayForKind(kind));
  }

  // 步骤尾部缓冲：等待远程窗口/应用启动完成，避免下一步操作打在错误的焦点上
  await sleep(settleDelayMs);
}
