/** 危险命令规则（正则预编译后缓存） */

export interface DangerRuleRaw {
  id: string;
  name: string;
  pattern: string;
  enabled: boolean;
  is_builtin: boolean;
}

export interface CompiledDangerRule {
  name: string;
  regex: RegExp;
}

/**
 * 将原始规则列表编译为正则缓存，跳过无效 pattern。
 * 仅在规则变更时调用一次。
 */
export function compileDangerRules(rules: DangerRuleRaw[]): CompiledDangerRule[] {
  const result: CompiledDangerRule[] = [];
  for (const rule of rules) {
    if (!rule.enabled) continue;
    try {
      result.push({ name: rule.name, regex: new RegExp(rule.pattern) });
    } catch {
      // 跳过无效正则
    }
  }
  return result;
}

/**
 * 用预编译正则检测命令是否匹配危险规则，返回匹配的规则名称列表。
 */
export function checkDangerousCommand(compiled: CompiledDangerRule[], cmd: string): string[] {
  const matched: string[] = [];
  for (const { name, regex } of compiled) {
    if (regex.test(cmd)) {
      matched.push(name);
    }
  }
  return matched;
}

/**
 * 与后端 settings.rs 内置规则保持同步的默认 pattern 列表。
 * 作为前端 fallback：终端初始化时立即生效，不依赖 invoke('list_danger_rules') 异步返回。
 * 后端规则加载成功后会替换为完整列表（含用户自定义规则）。
 */
const DEFAULT_DANGER_RULE_PATTERNS: DangerRuleRaw[] = [
  { id: 'builtin-rm-rf-root', name: 'rm -rf /', pattern: 'rm\\s+-rf\\s+/', enabled: true, is_builtin: true },
  { id: 'builtin-rm-rf-home', name: 'rm -rf ~', pattern: 'rm\\s+-rf\\s+~', enabled: true, is_builtin: true },
  { id: 'builtin-fork-bomb', name: 'fork bomb', pattern: ':\\(\\)\\{\\s*:\\|:\\&\\s*\\}\\s*;', enabled: true, is_builtin: true },
  { id: 'builtin-dd', name: 'dd overwrite', pattern: 'dd\\s+if=', enabled: true, is_builtin: true },
  { id: 'builtin-mkfs', name: 'mkfs', pattern: 'mkfs\\.', enabled: true, is_builtin: true },
  { id: 'builtin-dev-sda', name: '/dev/sda redirect', pattern: '>\\s*/dev/sda', enabled: true, is_builtin: true },
  { id: 'builtin-chmod-777', name: 'chmod 777 /', pattern: 'chmod\\s+-R\\s+777\\s+/', enabled: true, is_builtin: true },
  { id: 'builtin-chown-root', name: 'chown /', pattern: 'chown\\s+-R\\s+\\w+\\s+/', enabled: true, is_builtin: true },
  { id: 'builtin-shutdown', name: 'shutdown', pattern: 'shutdown\\s+', enabled: true, is_builtin: true },
  { id: 'builtin-reboot', name: 'reboot', pattern: 'reboot\\b', enabled: true, is_builtin: true },
  { id: 'builtin-init', name: 'init 0/6', pattern: 'init\\s+[06]\\b', enabled: true, is_builtin: true },
  { id: 'builtin-drop-db', name: 'drop database', pattern: '(?:DROP|drop)\\s+(?:DATABASE|database|SCHEMA|schema)', enabled: true, is_builtin: true },
  { id: 'builtin-truncate', name: 'truncate table', pattern: '(?:TRUNCATE|truncate)\\s+(?:TABLE|table)?\\s*\\w', enabled: true, is_builtin: true },
  { id: 'builtin-systemctl-critical', name: 'systemctl stop critical', pattern: 'systemctl\\s+(?:stop|disable|mask)\\s+(?:sshd|nginx|docker|firewalld|NetworkManager|systemd)\\b', enabled: true, is_builtin: true },
  { id: 'builtin-iptables-flush', name: 'iptables flush', pattern: 'iptables\\s+-F', enabled: true, is_builtin: true },
  { id: 'builtin-rm-var-log', name: 'rm -rf var/log', pattern: 'rm\\s+-rf\\s+/var/log', enabled: true, is_builtin: true },
  { id: 'builtin-wipefs', name: 'wipefs', pattern: 'wipefs\\s+-a', enabled: true, is_builtin: true },
  { id: 'builtin-passwd', name: '> /etc/passwd', pattern: '>\\s*/etc/passwd', enabled: true, is_builtin: true },
  { id: 'builtin-mv-devnull', name: 'mv /* /dev/null', pattern: 'mv\\s+/\\S*\\s+/dev/null', enabled: true, is_builtin: true },
];

/**
 * 模块加载时预编译的默认规则，作为 fallback 立即可用。
 */
export const DEFAULT_COMPILED_DANGER_RULES: CompiledDangerRule[] = compileDangerRules(DEFAULT_DANGER_RULE_PATTERNS);
