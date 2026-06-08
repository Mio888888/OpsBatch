import { invoke } from '@tauri-apps/api/core';

export interface ParsedPendingAction {
  id: string;
  type: 'command' | 'file_inspect' | 'diagnose';
  description: string;
  command?: string;
  source?: 'action_block' | 'fence' | 'command_plan';
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

export async function parseAiPendingActionsAsync(content: string): Promise<ParsedAiActionsResult> {
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
