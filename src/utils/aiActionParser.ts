import { invoke } from '@tauri-apps/api/core';

// RDP 操作：AI agent 输出的远程桌面操作指令
export type RdpOperation =
  | { type: 'click'; x: number; y: number; button?: number; doubleClick?: boolean }
  | { type: 'move'; x: number; y: number }
  | { type: 'drag'; fromX: number; fromY: number; toX: number; toY: number; button?: number }
  | { type: 'scroll'; x: number; y: number; delta: number; vertical?: boolean }
  | { type: 'type'; text: string }
  | { type: 'key'; keys: string[] };

export interface ParsedPendingAction {
  id: string;
  type: 'command' | 'file_inspect' | 'diagnose' | 'rdp_action';
  description: string;
  command?: string;
  rdpOperations?: RdpOperation[];
  source?: 'action_block' | 'fence' | 'command_plan' | 'rdp_plan';
  intent?: string;
  expectedOutcome?: string;
  assessment?: AiActionAssessment;
  assessmentLoading?: boolean;
  assessmentError?: string;
  approved: boolean;
  executed: boolean;
  rejected?: boolean;
}

export interface ParsedCommandPlanNotice {
  version: number;
  summary: string;
}

export interface ParsedAiActionsResult {
  displayContent: string;
  actions: ParsedPendingAction[];
  commandPlanNotice?: ParsedCommandPlanNotice;
  rdpParseError?: string;
}

export interface AiActionAssessment {
  decision: 'SAFE' | 'CONFIRM' | 'BLOCK';
  risk_level: 'low' | 'medium' | 'high' | 'critical';
  risk_score: number;
  matched_rule: string;
  reason: string;
  capabilities: string[];
}

const SHELL_SCRIPT_FENCE_LANGUAGES = new Set([
  'bash',
  'sh',
  'shell',
  'zsh',
]);

const TERMINAL_FENCE_LANGUAGES = new Set([
  'console',
  'terminal',
]);

const COMMON_SHELL_COMMANDS = new Set([
  'apt',
  'apt-get',
  'awk',
  'bash',
  'cat',
  'cd',
  'chmod',
  'chown',
  'cp',
  'curl',
  'df',
  'dnf',
  'docker',
  'du',
  'echo',
  'export',
  'find',
  'free',
  'git',
  'grep',
  'head',
  'helm',
  'id',
  'journalctl',
  'kubectl',
  'less',
  'ls',
  'mkdir',
  'mv',
  'netstat',
  'nginx',
  'node',
  'npm',
  'ping',
  'pip',
  'pip3',
  'pnpm',
  'printf',
  'ps',
  'psql',
  'pwd',
  'python',
  'python3',
  'redis-cli',
  'rm',
  'rsync',
  'scp',
  'sed',
  'sh',
  'ss',
  'sudo',
  'systemctl',
  'tail',
  'tar',
  'top',
  'touch',
  'uname',
  'unzip',
  'vim',
  'vi',
  'wget',
  'whoami',
  'yum',
  'yarn',
  'zip',
  'zsh',
]);

interface FenceBlock {
  language: string;
  code: string;
  startIndex: number;
}

interface ValidatedCommandPlan {
  version: number;
  summary: string;
  steps: ValidatedCommandPlanStep[];
}

interface ValidatedCommandPlanStep {
  id: string;
  description: string;
  command: string;
  intent: string;
  expected_outcome: string;
}

interface CommandPlanCandidate {
  rawPlan: string;
  startIndex: number;
  endIndex: number;
}

export async function parseAiPendingActionsAsync(
  content: string,
  rdpContext?: { width: number; height: number },
): Promise<ParsedAiActionsResult> {
  // RDP 场景：优先尝试解析 RDP 操作计划
  if (rdpContext) {
    const rdpResult = parseRdpActions(content, rdpContext);
    if (rdpResult) {
      return {
        displayContent: rdpResult.displayContent,
        actions: rdpResult.actions,
        rdpParseError: rdpResult.parseError,
      };
    }
  }

  const candidate = extractCommandPlanCandidate(content);
  if (!candidate) return parseAiPendingActions(content);

  const commandPlanNotice = parseEmptyCommandPlanNotice(candidate.rawPlan);
  if (commandPlanNotice) {
    return {
      displayContent: removeCommandPlanFromDisplay(content, candidate).trim(),
      actions: [],
      commandPlanNotice,
    };
  }

  try {
    const plan = await invoke<ValidatedCommandPlan>('ai_validate_command_plan', {
      rawPlan: candidate.rawPlan,
    });
    return {
      displayContent: removeCommandPlanFromDisplay(content, candidate).trim(),
      actions: plan.steps.map((step) => ({
        id: step.id,
        type: 'command',
        description: step.description,
        command: step.command,
        source: 'command_plan',
        intent: step.intent,
        expectedOutcome: step.expected_outcome,
        approved: false,
        executed: false,
      })),
    };
  } catch {
    return parseAiPendingActions(content);
  }
}

export function parseAiPendingActions(content: string): ParsedAiActionsResult {
  const actions: ParsedPendingAction[] = [];
  const seenCommands = new Set<string>();
  const actionRegex = /\[ACTION(?::([^\]]+))?\]([\s\S]*?)\[\/ACTION\]/g;
  let actionMatch: RegExpExecArray | null;

  while ((actionMatch = actionRegex.exec(content)) !== null) {
    const tag = (actionMatch[1] ?? '').trim();
    const body = actionMatch[2].trim();
    const colonIdx = tag.indexOf(':');
    const description = colonIdx > 0 ? tag.slice(0, colonIdx).trim() : tag;
    const rawCommand = colonIdx > 0 && !body ? tag.slice(colonIdx + 1).trim() : body;
    addCommandAction(actions, seenCommands, description, rawCommand);
  }

  const displayContent = content.replace(/\[ACTION(?::([^\]]+))?\]([\s\S]*?)\[\/ACTION\]/g, '').trim();
  const contentWithoutActionBlocks = displayContent;

  for (const block of extractFenceBlocks(contentWithoutActionBlocks)) {
    const command = getShellCommandFromFence(block.language, block.code);
    if (!command) continue;
    const description = extractDescriptionBeforeFence(contentWithoutActionBlocks, block.startIndex);
    addCommandAction(actions, seenCommands, description, command, 'fence');
  }

  return { displayContent, actions };
}

export function extractEmptyCommandPlanNotice(content: string): ParsedAiActionsResult | null {
  const candidate = extractCommandPlanCandidate(content);
  if (!candidate) return null;

  const commandPlanNotice = parseEmptyCommandPlanNotice(candidate.rawPlan);
  if (!commandPlanNotice) return null;

  return {
    displayContent: removeCommandPlanFromDisplay(content, candidate).trim(),
    actions: [],
    commandPlanNotice,
  };
}

function addCommandAction(
  actions: ParsedPendingAction[],
  seenCommands: Set<string>,
  description: string,
  rawCommand: string,
  source: ParsedPendingAction['source'] = 'action_block',
): void {
  const command = sanitizeShellCommand(rawCommand);
  const commandKey = normalizeCommandForDedupe(command);
  if (!command || !commandKey || seenCommands.has(commandKey)) return;

  seenCommands.add(commandKey);
  actions.push({
    id: crypto.randomUUID(),
    type: 'command',
    description: description || '执行终端命令',
    command,
    source,
    approved: false,
    executed: false,
  });
}

function extractCommandPlanCandidate(content: string): CommandPlanCandidate | null {
  const tagged = content.match(/<COMMAND_PLAN>\s*([\s\S]*?)\s*<\/COMMAND_PLAN>/i);
  if (tagged && typeof tagged.index === 'number') {
    return {
      rawPlan: tagged[1].trim(),
      startIndex: tagged.index,
      endIndex: tagged.index + tagged[0].length,
    };
  }

  for (const block of extractFenceBlocks(content)) {
    if (block.language !== 'json') continue;
    if (!looksLikeCommandPlanJson(block.code)) continue;
    return {
      rawPlan: block.code.trim(),
      startIndex: block.startIndex,
      endIndex: findFenceEnd(content, block.startIndex),
    };
  }

  return null;
}

function looksLikeCommandPlanJson(raw: string): boolean {
  try {
    const parsed = JSON.parse(raw) as { steps?: unknown };
    return Array.isArray(parsed.steps);
  } catch {
    return false;
  }
}

function findFenceEnd(content: string, fenceStart: number): number {
  const rest = content.slice(fenceStart);
  const match = rest.match(/(?:\r?\n)?(```|~~~)[ \t]*(?=\r?\n|$)/);
  if (!match || typeof match.index !== 'number') return content.length;
  return fenceStart + match.index + match[0].length;
}

function removeRange(content: string, start: number, end: number): string {
  return `${content.slice(0, start)}${content.slice(end)}`;
}

function removeCommandPlanFromDisplay(content: string, candidate: CommandPlanCandidate): string {
  const after = content.slice(candidate.endIndex);
  const trailingOnlyPunctuation = after.match(/^\s*[。.!！]*\s*$/);
  const endIndex = trailingOnlyPunctuation ? content.length : candidate.endIndex;
  return removeRange(content, candidate.startIndex, endIndex);
}

function parseEmptyCommandPlanNotice(raw: string): ParsedCommandPlanNotice | null {
  try {
    const parsed = JSON.parse(raw) as {
      version?: unknown;
      summary?: unknown;
      steps?: unknown;
    };
    if (parsed.version !== 1) return null;
    if (!Array.isArray(parsed.steps) || parsed.steps.length !== 0) return null;
    if (typeof parsed.summary !== 'string') return null;

    const summary = parsed.summary.trim();
    if (!summary || summary.length > 200) return null;

    return {
      version: parsed.version,
      summary,
    };
  } catch {
    return null;
  }
}

function extractFenceBlocks(content: string): FenceBlock[] {
  const blocks: FenceBlock[] = [];
  const fenceRegex = /(^|\n)(```|~~~)([^\r\n]*)\r?\n([\s\S]*?)(?:\r?\n)?\2[ \t]*(?=\r?\n|$)/g;
  let fenceMatch: RegExpExecArray | null;

  while ((fenceMatch = fenceRegex.exec(content)) !== null) {
    blocks.push({
      language: normalizeFenceLanguage(fenceMatch[3]),
      code: fenceMatch[4],
      startIndex: fenceMatch.index + fenceMatch[1].length,
    });
  }

  return blocks;
}

function normalizeFenceLanguage(info: string): string {
  const token = info.trim().split(/\s+/)[0] ?? '';
  return token
    .replace(/^\{?\.?/, '')
    .replace(/[},].*$/, '')
    .toLowerCase();
}

function getShellCommandFromFence(language: string, code: string): string {
  if (SHELL_SCRIPT_FENCE_LANGUAGES.has(language)) return code;
  if (TERMINAL_FENCE_LANGUAGES.has(language)) {
    const promptedCommands = extractTerminalTranscriptCommands(code);
    if (promptedCommands.length > 0) return promptedCommands.join('\n');
    return looksLikeShellCommands(code) ? code : '';
  }
  if (language) return '';
  return looksLikeShellCommands(code) ? code : '';
}

function extractTerminalTranscriptCommands(command: string): string[] {
  return command
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map((line) => extractShellPromptCommand(line))
    .filter((line): line is string => Boolean(line));
}

function sanitizeShellCommand(command: string): string {
  const rawLines = command
    .replace(/\r\n?/g, '\n')
    .split('\n')
    .map((line) => line.replace(/[ \t]+$/g, ''));
  const promptedLines = rawLines
    .map((line) => extractShellPromptCommand(line))
    .filter((line): line is string => line !== null);
  const linesToClean = promptedLines.length > 0
    ? promptedLines
    : rawLines.map((line) => stripShellPrompt(line));

  return linesToClean
    .filter((line) => {
      const trimmed = line.trim();
      return trimmed.length > 0 && !isPureShellComment(trimmed);
    })
    .join('\n')
    .trim();
}

function stripShellPrompt(line: string): string {
  return extractShellPromptCommand(line) ?? line;
}

function extractShellPromptCommand(line: string): string | null {
  const simplePrompt = line.match(/^\s*(?:\$|>)\s+(.+)$/);
  if (simplePrompt) return simplePrompt[1];

  const userHostPrompt = line.match(/^\s*(?:\[[^\]]+\]\s*)?[A-Za-z0-9._-]+@[A-Za-z0-9._-]+:[^#$\n]*[#$]\s+(.+)$/);
  if (userHostPrompt) return userHostPrompt[1];

  return null;
}

function isPureShellComment(trimmedLine: string): boolean {
  return trimmedLine.startsWith('#');
}

const STRUCTURED_DATA_START_RE = /^\s*(?:[{[}\]]|---\s*$|\w+:\s*(?:$|[\w'"[{]))/;

function looksLikeShellCommands(code: string): boolean {
  const lines = sanitizeShellCommand(code).split('\n').map((line) => line.trim()).filter(Boolean);
  if (lines.length === 0) return false;

  let shellLikeLines = 0;
  for (const line of lines) {
    if (isLikelyShellLine(line)) shellLikeLines += 1;
  }

  if (shellLikeLines === 0) return false;
  if (lines.length === 1) return shellLikeLines === 1 && !STRUCTURED_DATA_START_RE.test(lines[0]);
  return shellLikeLines >= Math.ceil(lines.length / 2);
}

function isLikelyShellLine(line: string): boolean {
  if (/^(?:[A-Za-z_][A-Za-z0-9_]*=\S+\s+)*[A-Za-z_][A-Za-z0-9_]*=/.test(line)) return true;
  if (/^(?:\.\.?\/|~\/|\/)[^\s]+/.test(line)) return true;
  if (/[|&]{2}|\||>>?|<\s*\S+/.test(line)) return true;

  const firstToken = line.split(/\s+/)[0];
  return COMMON_SHELL_COMMANDS.has(firstToken);
}

function normalizeCommandForDedupe(command: string): string {
  return sanitizeShellCommand(command).replace(/\s+/g, ' ').trim();
}

function extractDescriptionBeforeFence(content: string, fenceStartIndex: number): string {
  const lines = content.slice(0, fenceStartIndex).split('\n');

  for (let i = lines.length - 1; i >= 0; i -= 1) {
    const line = lines[i].trim();
    if (!line || line.startsWith('```') || line.startsWith('~~~')) continue;

    const candidate = line
      .replace(/^#{1,6}\s+/, '')
      .replace(/^[-*+]\s+/, '')
      .replace(/^\d+[.)]\s+/, '')
      .replace(/[：:]\s*$/, '')
      .trim();

    if (candidate) return candidate.length > 80 ? `${candidate.slice(0, 77)}...` : candidate;
  }

  return '执行终端命令';
}

// ---------------------------------------------------------------------------
// agent-rdp: RDP 操作计划解析与验证
// ---------------------------------------------------------------------------

const RDP_PLAN_FENCE_LANGUAGES = new Set(['rdp', 'rdp-plan', 'rdp_actions', 'rdp-ops']);

interface RawRdpPlan {
  version?: unknown;
  summary?: unknown;
  steps?: unknown;
}

interface RawRdpPlanStep {
  description?: unknown;
  operations?: unknown;
  intent?: unknown;
  expected_outcome?: unknown;
}

interface ValidatedRdpPlan {
  version: number;
  summary: string;
  steps: ValidatedRdpPlanStep[];
}

interface ValidatedRdpPlanStep {
  id: string;
  description: string;
  operations: RdpOperation[];
  intent: string;
  expected_outcome: string;
}

const RDP_PLAN_MAX_STEPS = 12;
const RDP_PLAN_MAX_OPS_PER_STEP = 30;
const RDP_ALLOWED_INTENTS = new Set([
  'click',
  'type',
  'key',
  'scroll',
  'navigate',
  'launch',
  'operate',
  'interact',
]);

// 常用按键名映射到 RDP scancode 体系（仅用于描述与校验，实际发送由前端键盘事件处理）
const RDP_SUPPORTED_KEY_NAMES = new Set([
  'Enter', 'Tab', 'Escape', 'Backspace', 'Delete', 'Insert',
  'Home', 'End', 'PageUp', 'PageDown',
  'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight',
  'Space',
  'Shift', 'Control', 'Alt', 'Meta',
  'ShiftLeft', 'ShiftRight', 'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'MetaLeft', 'MetaRight',
  'CapsLock',
  'F1', 'F2', 'F3', 'F4', 'F5', 'F6', 'F7', 'F8', 'F9', 'F10', 'F11', 'F12',
  // 修饰键别名（与 executor MODIFIER_SCANCODES 保持一致）
  'Win', 'Windows', 'Ctrl', 'AltGr',
]);

// 尝试从可能被截断/不规范的 RDP_PLAN 标签块中提取 JSON 文本。
// 流式输出或模型截断时，闭合标签可能是 </RDP_PLAN>、</RDP、或不完整。
function extractRdpPlanCandidate(content: string): { rawPlan: string; startIndex: number; endIndex: number; extractError?: string } | null {
  // 1. 完整闭合标签（标准情况）
  const tagged = content.match(/<RDP_PLAN>\s*([\s\S]*?)\s*<\/RDP_PLAN>/i);
  if (tagged && typeof tagged.index === 'number') {
    return {
      rawPlan: tagged[1].trim(),
      startIndex: tagged.index,
      endIndex: tagged.index + tagged[0].length,
    };
  }

  // 2. 开标签存在但闭合不完整（截断/不规范）：用括号平衡提取 JSON
  // 宽容匹配开标签：接受 <RDP_PLAN> 后跟空白、或 <RDP_PLAN 直接换行
  const openMatch = content.match(/<RDP_PLAN\s*>?/i);
  if (openMatch && typeof openMatch.index === 'number') {
    const afterOpen = openMatch.index + openMatch[0].length;
    const jsonText = sliceBalancedJson(content, afterOpen);
    if (jsonText) {
      // 扩展 endIndex 到包含后续闭合标签（完整或不完整），避免残留显示
      const tail = content.slice(jsonText.endIndex);
      const closeMatch = tail.match(/<\/RDP_PLAN?>?/i);
      const endIndex = closeMatch && typeof closeMatch.index === 'number'
        ? jsonText.endIndex + closeMatch.index + closeMatch[0].length
        : jsonText.endIndex;
      return {
        rawPlan: jsonText.text.trim(),
        startIndex: openMatch.index,
        endIndex,
      };
    }
    // JSON 提取失败但确实有开标签：返回错误诊断
    return {
      rawPlan: '',
      startIndex: openMatch.index,
      endIndex: content.length,
      extractError: '检测到 <RDP_PLAN> 标签但无法提取有效 JSON（可能格式错误或被截断）',
    };
  }

  // 3. fence 代码块
  for (const block of extractFenceBlocks(content)) {
    if (!RDP_PLAN_FENCE_LANGUAGES.has(block.language)) continue;
    if (!looksLikeRdpPlanJson(block.code)) continue;
    return {
      rawPlan: block.code.trim(),
      startIndex: block.startIndex,
      endIndex: findFenceEnd(content, block.startIndex),
    };
  }

  return null;
}

// 从 startIndex 起查找第一个完整 JSON 对象（花括号平衡），容忍中间空白。
// 用于流式截断/闭合标签缺失的场景。若 JSON 不完整则返回 null。
function sliceBalancedJson(content: string, startIndex: number): { text: string; endIndex: number } | null {
  let depth = 0;
  let inString = false;
  let escape = false;
  let firstBrace = -1;
  const openStack: string[] = [];

  for (let idx = startIndex; idx < content.length; idx += 1) {
    const ch = content[idx];
    if (inString) {
      if (escape) {
        escape = false;
      } else if (ch === '\\') {
        escape = true;
      } else if (ch === '"') {
        inString = false;
      }
      continue;
    }
    if (ch === '"') {
      inString = true;
      continue;
    }
    if (ch === '{' || ch === '[') {
      if (depth === 0 && ch === '{') firstBrace = idx;
      depth += 1;
      openStack.push(ch);
    } else if (ch === '}' || ch === ']') {
      depth -= 1;
      openStack.pop();
      if (depth === 0 && firstBrace >= 0) {
        return { text: content.slice(firstBrace, idx + 1), endIndex: idx + 1 };
      }
    }
  }

  if (firstBrace >= 0 && openStack.length > 0) {
    const partial = content.slice(firstBrace);
    const repaired = repairTruncatedJson(partial, openStack);
    if (repaired) {
      return { text: repaired, endIndex: content.length };
    }
  }

  return null;
}

function repairTruncatedJson(partial: string, openStack: string[]): string | null {
  let repaired = partial;

  let inString = false;
  let escape = false;
  for (let idx = 0; idx < repaired.length; idx += 1) {
    const ch = repaired[idx];
    if (inString) {
      if (escape) escape = false;
      else if (ch === '\\') escape = true;
      else if (ch === '"') inString = false;
    } else if (ch === '"') inString = true;
  }
  if (inString) repaired += '"';

  repaired = repaired.replace(/[,\s]+$/, '');
  repaired = repaired.replace(/:\s*$/, '');
  repaired = repaired.replace(/,\s*"[A-Za-z_]+"\s*:\s*$/, '');

  for (let idx = openStack.length - 1; idx >= 0; idx -= 1) {
    repaired += openStack[idx] === '{' ? '}' : ']';
  }

  return repaired;
}


function looksLikeRdpPlanJson(raw: string): boolean {
  try {
    const parsed = JSON.parse(raw) as RawRdpPlan;
    return Array.isArray(parsed.steps);
  } catch {
    return false;
  }
}

function validateRdpOperations(rawOps: unknown, maxCoords: { width: number; height: number }): RdpOperation[] {
  if (!Array.isArray(rawOps)) throw new Error('RDP operations 必须是数组');
  if (rawOps.length === 0) throw new Error('RDP operations 不能为空');
  if (rawOps.length > RDP_PLAN_MAX_OPS_PER_STEP) {
    throw new Error(`RDP operations 单步不能超过 ${RDP_PLAN_MAX_OPS_PER_STEP} 个操作`);
  }

  const result: RdpOperation[] = [];
  for (const raw of rawOps) {
    if (!raw || typeof raw !== 'object') throw new Error('RDP operation 必须是对象');
    const op = raw as Record<string, unknown>;
    const type = op.type;

    const clampCoord = (v: unknown, max: number): number => {
      const n = Math.floor(Number(v));
      if (!Number.isFinite(n)) throw new Error('RDP 坐标必须是数字');
      if (n < 0 || n >= max) throw new Error(`RDP 坐标超出范围: ${n} (允许 0..${max - 1})`);
      return n;
    };

    switch (type) {
      case 'click':
      case 'move': {
        const x = clampCoord(op.x, maxCoords.width);
        const y = clampCoord(op.y, maxCoords.height);
        if (type === 'click') {
          const button = op.button === undefined ? 0 : Math.floor(Number(op.button));
          if (![0, 1, 2].includes(button)) throw new Error(`RDP click button 不支持: ${button}`);
          const doubleClick = Boolean(op.doubleClick);
          result.push({ type: 'click', x, y, button, doubleClick });
        } else {
          result.push({ type: 'move', x, y });
        }
        break;
      }
      case 'drag': {
        const fromX = clampCoord(op.fromX ?? op.from_x, maxCoords.width);
        const fromY = clampCoord(op.fromY ?? op.from_y, maxCoords.height);
        const toX = clampCoord(op.toX ?? op.to_x, maxCoords.width);
        const toY = clampCoord(op.toY ?? op.to_y, maxCoords.height);
        const button = op.button === undefined ? 0 : Math.floor(Number(op.button));
        if (![0, 2].includes(button)) throw new Error(`RDP drag button 不支持: ${button}`);
        result.push({ type: 'drag', fromX, fromY, toX, toY, button });
        break;
      }
      case 'scroll': {
        const x = clampCoord(op.x, maxCoords.width);
        const y = clampCoord(op.y, maxCoords.height);
        const delta = Math.floor(Number(op.delta));
        if (!Number.isFinite(delta) || delta === 0) throw new Error('RDP scroll delta 必须是非零整数');
        if (Math.abs(delta) > 100) throw new Error('RDP scroll delta 不能超过 ±100');
        const vertical = op.vertical === undefined ? true : Boolean(op.vertical);
        result.push({ type: 'scroll', x, y, delta, vertical });
        break;
      }
      case 'type': {
        const text = String(op.text ?? '');
        if (text.length === 0) throw new Error('RDP type text 不能为空');
        if (text.length > 500) throw new Error('RDP type text 不能超过 500 字符');
        if (text.includes('\0')) throw new Error('RDP type text 包含非法字符');
        result.push({ type: 'type', text });
        break;
      }
      case 'key': {
        const rawKeys = Array.isArray(op.keys) ? op.keys : (typeof op.keys === 'string' ? [op.keys] : []);
        if (rawKeys.length === 0) throw new Error('RDP key keys 不能为空');
        if (rawKeys.length > 6) throw new Error('RDP key 组合键不能超过 6 个');
        const keys: string[] = [];
        for (const k of rawKeys) {
          const keyName = String(k);
          if (keyName.length === 1) {
            keys.push(keyName);
          } else if (RDP_SUPPORTED_KEY_NAMES.has(keyName)) {
            keys.push(keyName);
          } else {
            throw new Error(`RDP 不支持的按键: ${keyName}`);
          }
        }
        result.push({ type: 'key', keys });
        break;
      }
      default:
        throw new Error(`RDP operation type 不支持: ${String(type)}`);
    }
  }

  return result;
}

// 宽松解析：先尝试严格 JSON.parse，失败则尝试规范化常见 AI 输出问题。
// 主要处理：summary/description 等字符串值里未转义的 ASCII 双引号，
// 以及被截断的 JSON（补全花括号/引号）。
function parsePlanJsonLoose(raw: string): RawRdpPlan | null {
  try {
    return JSON.parse(raw);
  } catch {
    // 尝试逐字符扫描，重建合法 JSON：识别字符串边界时，
    // 用「下一个结构字符前是否有引号」来判断字符串是否闭合。
    const repaired = rebuildJsonStructure(raw);
    if (repaired) {
      try {
        return JSON.parse(repaired);
      } catch {
        return null;
      }
    }
    return null;
  }
}

// 通过状态机重建 JSON：当遇到疑似字符串内部的未转义引号时，
// 检查其后是否紧跟合法的 JSON 结构字符（, } ] : 或 EOF），
// 若不是则判定为内部引号并转义。
function rebuildJsonStructure(raw: string): string | null {
  let result = '';
  let idx = 0;
  const len = raw.length;
  while (idx < len) {
    const ch = raw[idx];
    if (ch !== '"') {
      result += ch;
      idx += 1;
      continue;
    }
    // 进入字符串：找到所有候选闭合引号，选第一个「后跟结构字符」的作为真正闭合
    result += '"';
    idx += 1;
    let closed = false;
    while (idx < len) {
      const inner = raw[idx];
      if (inner === '\\' && idx + 1 < len) {
        result += raw.slice(idx, idx + 2);
        idx += 2;
        continue;
      }
      if (inner !== '"') {
        result += inner;
        idx += 1;
        continue;
      }
      // 遇到引号：判断是否为真正闭合
      const after = raw[idx + 1];
      const nextNonSpace = after === undefined ? '' : after;
      const isStructural = /[,}\]:\s]/.test(nextNonSpace) || idx + 1 >= len;
      if (isStructural) {
        result += '"';
        idx += 1;
        closed = true;
        break;
      }
      // 内部未转义引号：转义
      result += '\\"';
      idx += 1;
    }
    if (!closed) {
      // 字符串未闭合（截断）：补闭合引号
      result += '"';
    }
  }
  return result;
}

export function validateRdpPlan(
  raw: string,
  maxCoords: { width: number; height: number },
): ValidatedRdpPlan {
  const plan = parsePlanJsonLoose(raw);
  // eslint-disable-next-line no-console
  console.log('[validateRdpPlan] parsed:', { version: plan?.version, summary: plan?.summary, stepsLen: (plan?.steps as unknown[])?.length, rawPreview: raw.slice(0, 200) });
  if (!plan) throw new Error('RdpPlan JSON 解析失败');
  // version 宽容校验：接受缺失（默认 1）、数字 1、字符串 "1"
  const versionNum = Number(plan.version ?? 1);
  if (versionNum !== 1) throw new Error(`RdpPlan version 必须为 1，实际为 ${String(plan.version)}`);

  let summary = String(plan.summary ?? '').trim();
  // summary 宽容处理：为空时尝试从 steps 推导，而非直接拒绝
  if (!summary && Array.isArray(plan.steps) && plan.steps.length > 0) {
    summary = String(plan.steps[0]?.description ?? '').trim().slice(0, 120) || 'RDP 操作';
  }
  if (!summary) throw new Error('RdpPlan summary 和 steps 均为空，无法解析操作计划');
  if (summary.length > 200) throw new Error('RdpPlan summary 不能超过 200 字符');

  if (!Array.isArray(plan.steps) || plan.steps.length === 0) {
    throw new Error('RdpPlan steps 必须包含 1 到 12 个步骤');
  }
  if (plan.steps.length > RDP_PLAN_MAX_STEPS) {
    throw new Error(`RdpPlan steps 不能超过 ${RDP_PLAN_MAX_STEPS} 个步骤`);
  }

  const steps: ValidatedRdpPlanStep[] = [];
  for (const rawStep of plan.steps as RawRdpPlanStep[]) {
    const description = String(rawStep.description ?? '').trim();
    if (!description) throw new Error('RdpPlan step description 不能为空');
    if (description.length > 120) throw new Error('RdpPlan step description 不能超过 120 字符');

    const operations = validateRdpOperations(rawStep.operations, maxCoords);

    const intent = String(rawStep.intent ?? 'operate')
      .trim()
      .toLowerCase();
    if (!RDP_ALLOWED_INTENTS.has(intent)) throw new Error(`RdpPlan step intent 不支持: ${intent}`);

    const expectedOutcome = String(rawStep.expected_outcome ?? '').trim().slice(0, 240);

    steps.push({
      id: crypto.randomUUID(),
      description,
      operations,
      intent,
      expected_outcome: expectedOutcome,
    });
  }

  return { version: 1, summary, steps };
}

export interface ParsedRdpActionsResult {
  displayContent: string;
  actions: ParsedPendingAction[];
  parseError?: string;
}

export function parseRdpActions(
  content: string,
  maxCoords: { width: number; height: number },
): ParsedRdpActionsResult | null {
  const candidate = extractRdpPlanCandidate(content);
  // 诊断日志：帮助定位 AI 输出格式问题
  // eslint-disable-next-line no-console
  console.log('[parseRdpActions] candidate:', candidate ? { rawPlanLen: candidate.rawPlan.length, rawPlanPreview: candidate.rawPlan.slice(0, 200), extractError: candidate.extractError } : null);
  if (!candidate) return null;

  // 开标签存在但 JSON 提取失败的诊断
  if (candidate.extractError) {
    const displayContentErr = removeRange(content, candidate.startIndex, candidate.endIndex).replace(/^\s+|\s+$/g, '');
    return { displayContent: displayContentErr, actions: [], parseError: candidate.extractError };
  }

  let plan: ValidatedRdpPlan;
  let parseError: string | undefined;
  try {
    plan = validateRdpPlan(candidate.rawPlan, maxCoords);
  } catch (e) {
    parseError = e instanceof Error ? e.message : String(e);
    const displayContentErr = removeRange(content, candidate.startIndex, candidate.endIndex).replace(/^\s+|\s+$/g, '');
    return { displayContent: displayContentErr, actions: [], parseError };
  }

  const displayContent = removeRange(content, candidate.startIndex, candidate.endIndex).replace(/^\s+|\s+$/g, '');

  const actions: ParsedPendingAction[] = plan.steps.map((step) => ({
    id: step.id,
    type: 'rdp_action',
    description: step.description,
    rdpOperations: step.operations,
    source: 'rdp_plan',
    intent: step.intent,
    expectedOutcome: step.expected_outcome,
    approved: false,
    executed: false,
  }));

  return { displayContent, actions };
}
