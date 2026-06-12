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
