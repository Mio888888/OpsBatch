const ANSI_SEQUENCE_RE = /\x1b\[[0-?]*[ -/]*[@-~]|\x1b\].*?(?:\x07|\x1b\\)/g;
const CONTROL_SEQUENCE_RE = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g;
const SHELL_PROMPT_RE = /^[\w.-]+@[\w.-]+:[^\n]*[#$]\s*$/;
const SIMPLE_PROMPT_RE = /^(?:[$#]|❯|➜)\s*$/;

const ERROR_RE = /\b(?:error|failed|failure|fatal|exception|traceback|panic|permission denied|access denied|not found|no such file|command not found|segmentation fault|timed out|timeout|connection refused|unauthorized|forbidden|exit code|curl:\s*\(\d+\)|npm ERR!|ERR!|E:\s|Err:)\b/i;
const CJK_ERROR_RE = /(?:错误|失败|拒绝|无权限|不存在|未找到|超时|异常)/;
const INTERACTIVE_RE = /(?:password|passphrase|enter\s+.+|input\s+.+|select\s+.+|choose\s+.+|confirm\s+.+|continue\?|proceed\?|are you sure|\[[YyNn]\/[^\]]+\]|\([^)]*(?:yes\/no|y\/n)[^)]*\)|press any key|请输入|请选择|确认|是否|密码|口令)/i;
const INTERACTIVE_TAIL_RE = /(?:password|passphrase|请输入|请选择|确认|是否|密码|口令)[^\n]*[:?]\s*$/i;

const SHELL_TRACE_RE = /^\+\s+(?:sh\s+-c|bash\s+-c|sudo\b|env\b|apt(?:-get)?\b|yum\b|dnf\b|brew\b|docker\b|curl\b|wget\b|chmod\b|install\b|echo\b|cat\b|tee\b|mkdir\b|rm\b|mv\b|cp\b|sed\b|awk\b|grep\b|gpg\b|systemctl\b|snap\b|pip\b|npm\b|pnpm\b|yarn\b|apk\b)/i;
const LONG_COMMAND_RE = /^(?:\+?\s*)?(?:(?:sudo|env|sh\s+-c|bash\s+-c|apt-get|apt|yum|dnf|brew|docker|curl|wget|chmod|install|echo|cat|tee|mkdir|rm|mv|cp|sed|awk|grep|gpg|pip|npm|pnpm|yarn|apk|DEBIAN_FRONTEND=|[A-Z_]+=[^\s]+)\b)/i;
const INSTALL_NOISE_RE = /^(?:\+?\s*)?(?:Get:\d+|Hit:\d+|Ign:\d+|Fetched\b|Reading package lists|Building dependency tree|Reading state information|Selecting previously unselected|Preparing to unpack|Unpacking|Setting up|Processing triggers|update-alternatives|debconf: delaying package configuration|Collecting\s+|Downloading\s+|Installing collected packages|Successfully (?:built|installed)|Pulling fs layer|Waiting|Extracting|Verifying Checksum|Download complete|Pull complete|Status: Downloaded|Removing intermediate container|Step \d+\/\d+|--->|==>|Already up-to-date|Package .* is already installed|Installed:|Updated:|Complete!$|Loaded plugins:)/i;
const INSTALL_CONTEXT_RE = /^(?:Get:\d+|Hit:\d+|Ign:\d+|Fetched\b|Reading package lists|Building dependency tree|Reading state information|The following .*packages|Suggested packages|Recommended packages|Need to get\b|After this operation\b|Selecting previously unselected|Preparing to unpack|Unpacking|Setting up|Processing triggers|Collecting\s+|Downloading\s+|Installing collected packages|Pulling fs layer|Step \d+\/\d+|Sending build context to Docker daemon|#\d+\s|Successfully tagged\b|exporting to image\b)/i;
const INSTALL_DETAIL_NOISE_RE = /^(?:The following .*packages|Suggested packages|Recommended packages|Need to get\b|After this operation\b|Additional disk space|Sending build context to Docker daemon|#\d+\s|=>\s|CACHED\s|DONE\s|naming to docker\.io|exporting layers|exporting manifest|exporting config|exporting attestation manifest|exporting manifest list|exporting to image|Successfully tagged\b|[a-z0-9][a-z0-9+.:~_-]+(?:\s+[a-z0-9][a-z0-9+.:~_-]+){1,})$/i;
const PROGRESS_NOISE_RE = /(?:\b\d{1,3}%\b|\[[#=>.\-\s]{8,}\]|[#=]{12,}|^\s*(?:\||\/|-|\\)\s*$|^\s*\d+(?:\.\d+)?\s*(?:B|KB|MB|GB|KiB|MiB|GiB)\/?s?\b)/i;
const PACKAGE_SUMMARY_RE = /^(?:\d+\s+)?(?:upgraded|newly installed|to remove|not upgraded|packages? can be upgraded|files? and directories currently installed)\b/i;
const VALUEFUL_COMPLETION_RE = /\b(?:created|started|stopped|restarted|enabled|disabled|listening|active \(running\)|version|successfully|completed|done|ok|passed)\b/i;

export interface TerminalFollowUpDecision {
  shouldFollowUp: boolean;
  output: string;
  reason: 'empty' | 'noise' | 'error' | 'interactive' | 'meaningful';
}

function cleanTerminalText(text: string): string {
  return text
    .replace(ANSI_SEQUENCE_RE, '')
    .replace(/\r(?!\n)/g, '\n')
    .replace(CONTROL_SEQUENCE_RE, '')
    .replace(/\x1b\[200~|\x1b\[201~/g, '');
}

function isPromptLine(line: string): boolean {
  return SHELL_PROMPT_RE.test(line) || SIMPLE_PROMPT_RE.test(line);
}

function hasErrorSignal(line: string): boolean {
  return ERROR_RE.test(line) || CJK_ERROR_RE.test(line);
}

function hasInteractiveSignal(line: string): boolean {
  return INTERACTIVE_RE.test(line) || INTERACTIVE_TAIL_RE.test(line);
}

function isLongCommandEcho(line: string): boolean {
  return line.length >= 140 && LONG_COMMAND_RE.test(line);
}

function hasInstallContext(lines: string[]): boolean {
  return lines.some((line) => INSTALL_CONTEXT_RE.test(line));
}

function isNoiseLine(line: string, installContext = false): boolean {
  if (!line || isPromptLine(line)) return true;
  if (hasErrorSignal(line) || hasInteractiveSignal(line)) return false;
  return SHELL_TRACE_RE.test(line)
    || isLongCommandEcho(line)
    || INSTALL_NOISE_RE.test(line)
    || (installContext && INSTALL_DETAIL_NOISE_RE.test(line))
    || PROGRESS_NOISE_RE.test(line)
    || PACKAGE_SUMMARY_RE.test(line);
}

function limitText(text: string, maxChars: number): string {
  if (text.length <= maxChars) return text;
  return text.slice(text.length - maxChars);
}

export function classifyTerminalFollowUp(rawBuffer: string): TerminalFollowUpDecision {
  const cleaned = cleanTerminalText(rawBuffer);
  const lines = cleaned
    .split(/\n+/)
    .map((line) => line.trim())
    .filter(Boolean)
    .slice(-100);

  if (lines.length === 0) {
    return { shouldFollowUp: false, output: '', reason: 'empty' };
  }

  const relevantLines = lines.filter((line) => !isPromptLine(line));
  const outputLines = (relevantLines.length > 0 ? relevantLines : lines).slice(-80);
  const hasError = outputLines.some(hasErrorSignal);
  const hasInteractive = outputLines.some(hasInteractiveSignal);
  const output = limitText(outputLines.join('\n'), 12_000).trim();

  if (!output) {
    return { shouldFollowUp: false, output: '', reason: 'empty' };
  }

  if (hasError) {
    return { shouldFollowUp: true, output, reason: 'error' };
  }

  if (hasInteractive) {
    return { shouldFollowUp: true, output, reason: 'interactive' };
  }

  const installContext = hasInstallContext(outputLines);
  const noiseLines = outputLines.filter((line) => isNoiseLine(line, installContext));
  const meaningfulLines = outputLines.filter((line) => !isNoiseLine(line, installContext));
  const shellTraceLines = outputLines.filter((line) => SHELL_TRACE_RE.test(line));
  const meaningfulText = meaningfulLines.join('\n').trim();

  if (meaningfulLines.length === 0) {
    return { shouldFollowUp: false, output: '', reason: 'noise' };
  }

  const noiseRatio = noiseLines.length / outputLines.length;
  const traceRatio = shellTraceLines.length / outputLines.length;
  const meaningfulChars = meaningfulText.length;

  if (traceRatio >= 0.6 && meaningfulLines.length <= 2 && meaningfulChars < 240) {
    return { shouldFollowUp: false, output: '', reason: 'noise' };
  }

  if (noiseRatio >= 0.8 && meaningfulChars < 240) {
    return { shouldFollowUp: false, output: '', reason: 'noise' };
  }

  if (
    meaningfulLines.length >= 2
    || meaningfulChars >= 120
    || meaningfulLines.some((line) => VALUEFUL_COMPLETION_RE.test(line))
  ) {
    return {
      shouldFollowUp: true,
      output: limitText(meaningfulText || output, 8_000).trim(),
      reason: 'meaningful',
    };
  }

  return { shouldFollowUp: false, output: '', reason: 'noise' };
}

export function stripTerminalControl(text: string): string {
  return cleanTerminalText(text);
}
