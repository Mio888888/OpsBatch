import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { CanvasNode, Connection } from '../../stores/workflow';

/* ---------- Node result ---------- */

export interface NodeResult {
  success: boolean;
  output: string;
  exitCode: number;
}

/* ---------- Progress events ---------- */

export type ProgressEvent =
  | { type: 'log'; message: string }
  | { type: 'node_start'; nodeId: string; nodeName: string }
  | { type: 'node_complete'; nodeId: string; nodeName: string; success: boolean; output: string; exitCode: number }
  | { type: 'level_start'; level: number; total: number; count: number }
  | { type: 'done' };

/* ---------- Template resolution ---------- */

function resolveTemplates(
  template: string,
  lastResult: NodeResult | null,
  resultsMap: Map<string, NodeResult>,
): string {
  let resolved = template;

  // {{lastResult}} -> full output
  resolved = resolved.replace(/\{\{lastResult\}\}/g, lastResult?.output ?? '');

  // {{lastResult.output}}
  resolved = resolved.replace(/\{\{lastResult\.output\}\}/g, lastResult?.output ?? '');

  // {{lastResult.exitCode}}
  resolved = resolved.replace(/\{\{lastResult\.exitCode\}\}/g, String(lastResult?.exitCode ?? ''));

  // {{lastResult.success}}
  resolved = resolved.replace(/\{\{lastResult\.success\}\}/g, String(lastResult?.success ?? ''));

  // {{result.<nodeId>}} -> output of a specific node
  resolved = resolved.replace(/\{\{result\.([a-zA-Z0-9_-]+)\}\}/g, (_, nodeId: string) => {
    return resultsMap.get(nodeId)?.output ?? '';
  });

  // {{result.<nodeId>.output}}
  resolved = resolved.replace(/\{\{result\.([a-zA-Z0-9_-]+)\.output\}\}/g, (_, nodeId: string) => {
    return resultsMap.get(nodeId)?.output ?? '';
  });

  // {{result.<nodeId>.exitCode}}
  resolved = resolved.replace(/\{\{result\.([a-zA-Z0-9_-]+)\.exitCode\}\}/g, (_, nodeId: string) => {
    return String(resultsMap.get(nodeId)?.exitCode ?? '');
  });

  // {{result.<nodeId>.success}}
  resolved = resolved.replace(/\{\{result\.([a-zA-Z0-9_-]+)\.success\}\}/g, (_, nodeId: string) => {
    return String(resultsMap.get(nodeId)?.success ?? '');
  });

  return resolved;
}

/* ---------- Condition evaluation ---------- */

function evaluateCondition(expression: string, lastResult: NodeResult | null, resultsMap: Map<string, NodeResult>): boolean {
  // First resolve templates
  let resolved = resolveTemplates(expression, lastResult, resultsMap);

  // Simple condition evaluation
  // Handle common patterns: "0 === 0", "true", "false", "1 === 0"
  resolved = resolved.trim();

  if (resolved === 'true') return true;
  if (resolved === 'false') return false;

  // Handle comparison operators
  const compMatch = resolved.match(/^(!?\s*['"]?.*?['"]?)\s*(===|!==|==|!=|>=|<=|>|<)\s*(['"]?.*?['"]?)$/);
  if (compMatch) {
    const [, leftRaw, op, rightRaw] = compMatch;
    const left = leftRaw.trim().replace(/^['"]|['"]$/g, '');
    const right = rightRaw.trim().replace(/^['"]|['"]$/g, '');

    const leftNum = Number(left);
    const rightNum = Number(right);
    const useNum = !isNaN(leftNum) && !isNaN(rightNum);

    switch (op) {
      case '===': return useNum ? leftNum === rightNum : left === right;
      case '!==': return useNum ? leftNum !== rightNum : left !== right;
      case '==': return useNum ? leftNum == rightNum : left == right;
      case '!=': return useNum ? leftNum != rightNum : left != right;
      case '>=': return useNum ? leftNum >= rightNum : false;
      case '<=': return useNum ? leftNum <= rightNum : false;
      case '>': return useNum ? leftNum > rightNum : false;
      case '<': return useNum ? leftNum < rightNum : false;
    }
  }

  // Handle negation
  if (resolved.startsWith('!')) {
    const inner = resolved.slice(1).trim();
    if (inner === 'true') return false;
    if (inner === 'false') return true;
  }

  // Default: if expression resolved to something truthy-looking, pass
  return resolved !== '0' && resolved !== '' && resolved !== 'false';
}

/* ---------- Main executor ---------- */

interface ExecutionResult {
  name: string;
  ok: boolean;
  msg: string;
  nodeId: string;
  nodeResult: NodeResult;
}

export async function executeWorkflow(
  nodes: CanvasNode[],
  connections: Connection[],
  hostIds: string[],
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>,
  waitForConfirmation: (nodeName: string, description: string) => Promise<boolean>,
  onProgress: (event: ProgressEvent) => void,
): Promise<void> {
  const enabledNodes = nodes.filter((n) => n.enabled);
  if (enabledNodes.length === 0) {
    onProgress({ type: 'log', message: '工作流没有启用的步骤' });
    return;
  }

  onProgress({ type: 'log', message: `开始执行工作流（${enabledNodes.length} 个步骤）` });

  const resultsMap = new Map<string, NodeResult>();
  // Track condition results: nodeId -> true/false
  const conditionResults = new Map<string, boolean>();
  // Track switch results: nodeId -> matched case index (e.g., "case-0")
  const switchResults = new Map<string, string>();
  let lastResult: NodeResult | null = null;

  // Build adjacency info
  const nodeMap = new Map(enabledNodes.map((n) => [n.id, n]));
  const incomingEdges = new Map<string, Connection[]>();
  const outgoingEdges = new Map<string, Connection[]>();

  for (const conn of connections) {
    if (!nodeMap.has(conn.fromId) || !nodeMap.has(conn.toId)) continue;
    if (!incomingEdges.has(conn.toId)) incomingEdges.set(conn.toId, []);
    if (!outgoingEdges.has(conn.fromId)) outgoingEdges.set(conn.fromId, []);
    incomingEdges.get(conn.toId)!.push(conn);
    outgoingEdges.get(conn.fromId)!.push(conn);
  }

  // Compute in-degree for initial ready nodes
  const inDegree = new Map<string, number>();
  for (const n of enabledNodes) {
    inDegree.set(n.id, 0);
  }
  for (const conn of connections) {
    if (!nodeMap.has(conn.fromId) || !nodeMap.has(conn.toId)) continue;
    inDegree.set(conn.toId, (inDegree.get(conn.toId) || 0) + 1);
  }

  // Nodes with no incoming edges are initially ready
  let readyNodes = enabledNodes.filter((n) => (inDegree.get(n.id) || 0) === 0);
  const processed = new Set<string>();
  let level = 0;
  const totalLevels = enabledNodes.length; // upper bound

  while (readyNodes.length > 0) {
    level++;
    onProgress({ type: 'level_start', level, total: totalLevels, count: readyNodes.length });

    const nextReady: CanvasNode[] = [];

    for (const step of readyNodes) {
      processed.add(step.id);
      onProgress({ type: 'node_start', nodeId: step.id, nodeName: step.name });

      const r = await executeNode(step, hostIds, invoke, waitForConfirmation, lastResult, resultsMap);

      const nodeResult: NodeResult = {
        success: r.ok,
        output: r.msg,
        exitCode: r.ok ? 0 : 1,
      };
      resultsMap.set(step.id, nodeResult);
      lastResult = nodeResult;

      onProgress({
        type: 'node_complete',
        nodeId: step.id,
        nodeName: step.name,
        success: r.ok,
        output: r.msg,
        exitCode: nodeResult.exitCode,
      });

      if (!r.ok && r.msg === '用户取消') {
        onProgress({ type: 'log', message: '工作流已被用户终止' });
        onProgress({ type: 'done' });
        return;
      }

      // For condition nodes, record the result so we can branch
      if (step.type === 'condition') {
        const passed = r.ok;
        conditionResults.set(step.id, passed);
        onProgress({
          type: 'log',
          message: `条件节点「${step.name}」评估结果: ${passed ? '是 (true)' : '否 (false)'}`,
        });
      }

      // For switch nodes, record the matched case index
      if (step.type === 'switch' && r.nodeResult.output) {
        // The switch executor returns the matched case key in the output (e.g., "case-1")
        const matchKey = r.nodeResult.output;
        switchResults.set(step.id, matchKey);
        onProgress({
          type: 'log',
          message: `多分支节点「${step.name}」匹配分支: ${matchKey}`,
        });
      }

      // Determine which downstream nodes are now ready
      const outEdges = outgoingEdges.get(step.id) || [];
      for (const edge of outEdges) {
        const targetId = edge.toId;
        if (processed.has(targetId)) continue;

        // Check branch filtering: if this edge comes from a condition node's branch,
        // only include the target if the branch matches the condition result
        if (step.type === 'condition' && edge.sourceHandle) {
          const conditionResult = conditionResults.get(step.id);
          if (conditionResult === undefined) continue;

          if (edge.sourceHandle === 'true' && !conditionResult) continue;
          if (edge.sourceHandle === 'false' && conditionResult) continue;
        }

        // Check switch branch filtering: only follow the matched case edge
        if (step.type === 'switch' && edge.sourceHandle) {
          const matchedCase = switchResults.get(step.id);
          if (matchedCase === undefined) continue;

          // Only follow the edge whose sourceHandle matches the selected case
          if (edge.sourceHandle !== matchedCase) continue;
        }

        // Decrement in-degree. A node is ready when all its unfiltered incoming edges are satisfied.
        const currentDegree = (inDegree.get(targetId) || 0) - 1;
        inDegree.set(targetId, currentDegree);

        if (currentDegree <= 0 && !processed.has(targetId)) {
          const targetNode = nodeMap.get(targetId);
          if (targetNode && !nextReady.some((n) => n.id === targetId)) {
            nextReady.push(targetNode);
          }
        }
      }
    }

    readyNodes = nextReady;
  }

  onProgress({ type: 'log', message: '工作流执行完成' });
  onProgress({ type: 'done' });
}

async function executeNode(
  step: CanvasNode,
  hostIds: string[],
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>,
  waitForConfirmation: (nodeName: string, description: string) => Promise<boolean>,
  lastResult: NodeResult | null,
  resultsMap: Map<string, NodeResult>,
): Promise<ExecutionResult> {
  const noop = { name: step.name, ok: true, msg: step.type, nodeId: step.id, nodeResult: { success: true, output: step.type, exitCode: 0 } };

  if (step.type === 'confirm') {
    const config = resolveTemplates(step.config || '请确认是否继续执行', lastResult, resultsMap);
    const confirmed = await waitForConfirmation(step.name, config);
    return confirmed
      ? { name: step.name, ok: true, msg: '已确认', nodeId: step.id, nodeResult: { success: true, output: '已确认', exitCode: 0 } }
      : { name: step.name, ok: false, msg: '用户取消', nodeId: step.id, nodeResult: { success: false, output: '用户取消', exitCode: 1 } };
  }

  if (step.type === 'start' || step.type === 'end' || step.type === 'selectHost') {
    return noop;
  }

  if (step.type === 'command' || step.type === 'quickAction') {
    if (!step.config) return { ...noop, msg: '跳过(无配置)' };
    const command = resolveTemplates(step.config, lastResult, resultsMap);
    try {
      const taskId = crypto.randomUUID();

      // Listen for output events to capture the actual output
      let outputText = '';
      let exitCode = 0;
      let success = true;

      const unlisteners: UnlistenFn[] = [];

      let settled = false;
      let listenerRegistrationPromise = Promise.resolve();
      const outputPromise = new Promise<void>((resolveOutput) => {
        listenerRegistrationPromise = Promise.all([
          listen(`exec:${taskId}:output`, (event) => {
            const data = event.payload as {
              hostId: string; hostName: string; status: string;
              output: string; exitCode: number; duration: number;
            };
            // Accumulate output from all hosts, take the last one with content
            if (data.output) {
              outputText += (outputText ? '\n' : '') + `[${data.hostName || data.hostId}] ${data.output}`;
            }
            if (data.status === 'failed' || data.status === 'timeout') {
              success = false;
              exitCode = data.exitCode || 1;
            }
          }),
          listen(`exec:${taskId}:done`, () => {
            if (!settled) {
              settled = true;
              resolveOutput();
            }
          }),
        ]).then((listeners) => { unlisteners.push(...listeners); });
      });

      // Set a timeout so we don't hang forever
      const timeoutPromise = new Promise<void>((resolveTimeout) => {
        setTimeout(resolveTimeout, 120_000);
      });

      await listenerRegistrationPromise;
      try {
        await invoke<string>('execute_command', {
          taskId,
          hostIds,
          command,
          concurrency: 10,
          timeout: 30,
        });

        await Promise.race([outputPromise, timeoutPromise]);
      } finally {
        for (const un of unlisteners) un();
      }

      return {
        name: step.name,
        ok: success,
        msg: outputText || (success ? '成功' : '失败'),
        nodeId: step.id,
        nodeResult: { success, output: outputText || (success ? '成功' : '失败'), exitCode },
      };
    } catch (e: unknown) {
      return {
        name: step.name,
        ok: false,
        msg: String(e),
        nodeId: step.id,
        nodeResult: { success: false, output: String(e), exitCode: 1 },
      };
    }
  }

  if (step.type === 'script') {
    if (!step.config) return { ...noop, msg: '跳过(无配置)' };
    const script = resolveTemplates(step.config, lastResult, resultsMap);
    try {
      const taskId = crypto.randomUUID();

      let outputText = '';
      let exitCode = 0;
      let success = true;

      const unlisteners: UnlistenFn[] = [];

      let settled = false;
      let listenerRegistrationPromise = Promise.resolve();
      const outputPromise = new Promise<void>((resolveOutput) => {
        listenerRegistrationPromise = Promise.all([
          listen(`exec:${taskId}:output`, (event) => {
            const data = event.payload as {
              hostId: string; hostName: string; status: string;
              output: string; exitCode: number; duration: number;
            };
            if (data.output) {
              outputText += (outputText ? '\n' : '') + `[${data.hostName || data.hostId}] ${data.output}`;
            }
            if (data.status === 'failed' || data.status === 'timeout') {
              success = false;
              exitCode = data.exitCode || 1;
            }
          }),
          listen(`exec:${taskId}:done`, () => {
            if (!settled) {
              settled = true;
              resolveOutput();
            }
          }),
        ]).then((listeners) => { unlisteners.push(...listeners); });
      });

      const timeoutPromise = new Promise<void>((resolveTimeout) => {
        setTimeout(resolveTimeout, 180_000);
      });

      await listenerRegistrationPromise;
      try {
        await invoke<string>('execute_command', {
          taskId,
          hostIds,
          command: script,
          concurrency: 10,
          timeout: 60,
        });

        await Promise.race([outputPromise, timeoutPromise]);
      } finally {
        for (const un of unlisteners) un();
      }

      return {
        name: step.name,
        ok: success,
        msg: outputText || (success ? '成功' : '失败'),
        nodeId: step.id,
        nodeResult: { success, output: outputText || (success ? '成功' : '失败'), exitCode },
      };
    } catch (e: unknown) {
      return {
        name: step.name,
        ok: false,
        msg: String(e),
        nodeId: step.id,
        nodeResult: { success: false, output: String(e), exitCode: 1 },
      };
    }
  }

  if (step.type === 'transfer') {
    if (!step.config) return { ...noop, msg: '跳过(无配置)' };
    const resolvedConfig = resolveTemplates(step.config, lastResult, resultsMap);
    try {
      const { localPath, remotePath, direction } = JSON.parse(resolvedConfig);
      await invoke<string>('file_transfer', {
        request: {
          direction: direction || 'upload',
          hostIds,
          localPath,
          remotePath,
          timeout: 120,
        },
      });
      return {
        name: step.name,
        ok: true,
        msg: '传输成功',
        nodeId: step.id,
        nodeResult: { success: true, output: '传输成功', exitCode: 0 },
      };
    } catch (e: unknown) {
      return {
        name: step.name,
        ok: false,
        msg: String(e),
        nodeId: step.id,
        nodeResult: { success: false, output: String(e), exitCode: 1 },
      };
    }
  }

  if (step.type === 'delay') {
    const seconds = parseInt(step.config) || 5;
    await new Promise((r) => setTimeout(r, seconds * 1000));
    return {
      name: step.name,
      ok: true,
      msg: `等待${seconds}s`,
      nodeId: step.id,
      nodeResult: { success: true, output: `等待${seconds}s`, exitCode: 0 },
    };
  }

  if (step.type === 'condition') {
    if (!step.config) {
      return {
        name: step.name,
        ok: true,
        msg: '条件通过(无条件表达式)',
        nodeId: step.id,
        nodeResult: { success: true, output: '条件通过(无条件表达式)', exitCode: 0 },
      };
    }
    const resolvedExpr = resolveTemplates(step.config, lastResult, resultsMap);
    const passed = evaluateCondition(step.config, lastResult, resultsMap);
    const msg = passed ? `条件通过: ${resolvedExpr}` : `条件不满足: ${resolvedExpr}`;
    // Condition node always succeeds - the branching logic is handled by the executor
    // which reads this node's result to decide which branch to follow
    return {
      name: step.name,
      ok: passed,
      msg,
      nodeId: step.id,
      nodeResult: { success: passed, output: msg, exitCode: passed ? 0 : 1 },
    };
  }

  if (step.type === 'switch') {
    if (!step.config) {
      return {
        name: step.name,
        ok: true,
        msg: 'case-0',
        nodeId: step.id,
        nodeResult: { success: true, output: 'case-0', exitCode: 0 },
      };
    }
    try {
      const cfg = JSON.parse(step.config);
      const expression: string = cfg.expression || '';
      const cases: Array<{ label: string; value: string }> = cfg.cases || [];

      if (cases.length === 0) {
        return {
          name: step.name,
          ok: true,
          msg: 'case-0',
          nodeId: step.id,
          nodeResult: { success: true, output: 'case-0', exitCode: 0 },
        };
      }

      // Resolve the expression
      const resolvedValue = resolveTemplates(expression, lastResult, resultsMap).trim();

      // Find matching case
      let matchIndex = -1;
      let defaultIndex = -1;

      for (let i = 0; i < cases.length; i++) {
        if (cases[i].value === '*') {
          if (defaultIndex === -1) defaultIndex = i;
          continue;
        }
        if (cases[i].value === resolvedValue) {
          matchIndex = i;
          break;
        }
      }

      // Fall through to default if no match
      if (matchIndex === -1) matchIndex = defaultIndex;
      // If still no match, use first case
      if (matchIndex === -1) matchIndex = 0;

      const matchKey = `case-${matchIndex}`;
      const caseLabel = cases[matchIndex]?.label || matchKey;

      return {
        name: step.name,
        ok: true,
        msg: `匹配分支「${caseLabel}」: ${expression} → ${resolvedValue}`,
        nodeId: step.id,
        // Store the case key in output so the executor can read it
        nodeResult: { success: true, output: matchKey, exitCode: 0 },
      };
    } catch {
      return {
        name: step.name,
        ok: true,
        msg: 'case-0',
        nodeId: step.id,
        nodeResult: { success: true, output: 'case-0', exitCode: 0 },
      };
    }
  }

  if (step.type === 'rollback') {
    if (!step.config) return { ...noop, msg: '回滚(无配置)' };
    const rollbackCmd = resolveTemplates(step.config, lastResult, resultsMap);
    try {
      const taskId = crypto.randomUUID();

      let outputText = '';
      let exitCode = 0;
      let success = true;

      const unlisteners: UnlistenFn[] = [];

      let settled = false;
      let listenerRegistrationPromise = Promise.resolve();
      const outputPromise = new Promise<void>((resolveOutput) => {
        listenerRegistrationPromise = Promise.all([
          listen(`exec:${taskId}:output`, (event) => {
            const data = event.payload as {
              hostId: string; hostName: string; status: string;
              output: string; exitCode: number; duration: number;
            };
            if (data.output) {
              outputText += (outputText ? '\n' : '') + `[${data.hostName || data.hostId}] ${data.output}`;
            }
            if (data.status === 'failed' || data.status === 'timeout') {
              success = false;
              exitCode = data.exitCode || 1;
            }
          }),
          listen(`exec:${taskId}:done`, () => {
            if (!settled) {
              settled = true;
              resolveOutput();
            }
          }),
        ]).then((listeners) => { unlisteners.push(...listeners); });
      });

      const timeoutPromise = new Promise<void>((resolveTimeout) => {
        setTimeout(resolveTimeout, 120_000);
      });

      await listenerRegistrationPromise;
      try {
        await invoke<string>('execute_command', {
          taskId,
          hostIds,
          command: rollbackCmd,
          concurrency: 10,
          timeout: 30,
        });

        await Promise.race([outputPromise, timeoutPromise]);
      } finally {
        for (const un of unlisteners) un();
      }

      return {
        name: step.name,
        ok: success,
        msg: outputText || (success ? '回滚成功' : '回滚失败'),
        nodeId: step.id,
        nodeResult: { success, output: outputText || (success ? '回滚成功' : '回滚失败'), exitCode },
      };
    } catch (e: unknown) {
      return {
        name: step.name,
        ok: false,
        msg: String(e),
        nodeId: step.id,
        nodeResult: { success: false, output: String(e), exitCode: 1 },
      };
    }
  }

  return noop;
}

export function buildExecutionLevels(nodes: CanvasNode[], connections: Connection[]): CanvasNode[][] {
  const nodeMap = new Map(nodes.map((n) => [n.id, n]));
  const inDegree = new Map(nodes.map((n) => [n.id, 0]));
  const children = new Map(nodes.map((n) => [n.id, [] as string[]]));

  for (const conn of connections) {
    if (nodeMap.has(conn.fromId) && nodeMap.has(conn.toId)) {
      inDegree.set(conn.toId, (inDegree.get(conn.toId) || 0) + 1);
      children.get(conn.fromId)?.push(conn.toId);
    }
  }

  let queue = nodes.filter((n) => (inDegree.get(n.id) || 0) === 0);
  const levels: CanvasNode[][] = [];
  const processed = new Set<string>();

  while (queue.length > 0) {
    levels.push([...queue]);
    for (const n of queue) processed.add(n.id);

    const nextQueue: CanvasNode[] = [];
    for (const n of queue) {
      for (const childId of children.get(n.id) || []) {
        inDegree.set(childId, (inDegree.get(childId) || 0) - 1);
        if ((inDegree.get(childId) || 0) === 0 && !processed.has(childId)) {
          const child = nodeMap.get(childId);
          if (child) nextQueue.push(child);
        }
      }
    }
    queue = nextQueue;
  }

  for (const n of nodes) {
    if (!processed.has(n.id)) {
      levels.push([n]);
    }
  }

  return levels;
}
