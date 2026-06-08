import { memo } from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { NODE_TYPES } from './nodeTypes';
import { useTranslation } from '../../../i18n';

export type NodeExecStatus = 'running' | 'success' | 'fail' | undefined;

export interface WfNodeData {
  name: string;
  nodeType: string;
  config: string;
  enabled: boolean;
  execStatus?: NodeExecStatus;
  [key: string]: unknown;
}

const TYPE_COLORS: Record<string, string> = {};
for (const t of NODE_TYPES) {
  TYPE_COLORS[t.value] = t.color;
}

interface SwitchCase {
  label: string;
  value: string;
}

interface SwitchConfig {
  expression: string;
  cases: SwitchCase[];
}

const SWITCH_CASE_PALETTE = [
  '#52c41a', // green
  '#1677ff', // blue
  '#fa8c16', // orange
  '#eb2f96', // magenta
  '#13c2c2', // cyan
  '#722ed1', // purple
  '#ff4d4f', // red
  '#8c8c8c', // gray
];

function parseSwitchConfig(config: string): SwitchConfig | null {
  if (!config) return null;
  try {
    const parsed = JSON.parse(config);
    if (parsed && Array.isArray(parsed.cases)) return parsed as SwitchConfig;
    return null;
  } catch {
    return null;
  }
}

const NO_INPUT = new Set(['start']);
const NO_OUTPUT = new Set(['end']);

function StatusIndicator({ status }: { status: NodeExecStatus }) {
  if (!status) return null;

  if (status === 'running') {
    return <span className="wf-node-status wf-node-status--running" />;
  }
  if (status === 'success') {
    return <span className="wf-node-status wf-node-status--success" />;
  }
  if (status === 'fail') {
    return <span className="wf-node-status wf-node-status--fail" />;
  }
  return null;
}

function SwitchBranchHandles({ config }: { config: string }) {
  const parsed = parseSwitchConfig(config);
  const cases: SwitchCase[] = parsed?.cases && parsed.cases.length > 0
    ? parsed.cases
    : [{ label: 'default', value: '*' }];

  return (
    <div className="wf-node-branches wf-node-switch-branches">
      {cases.map((c, i) => {
        const color = SWITCH_CASE_PALETTE[i % SWITCH_CASE_PALETTE.length];
        return (
          <div key={`case-${i}`} className="wf-node-branch wf-node-switch-branch">
            <span className="wf-node-branch-label wf-node-switch-label" style={{ color }}>
              {c.label}
            </span>
            <Handle
              type="source"
              position={Position.Right}
              id={`case-${i}`}
              className="wf-port wf-port--switch"
              style={{ borderColor: color }}
            />
          </div>
        );
      })}
    </div>
  );
}

function WfNode({ data, selected }: NodeProps) {
  const d = data as WfNodeData;
  const { tText } = useTranslation();
  const typeInfo = NODE_TYPES.find((t) => t.value === d.nodeType);
  const color = TYPE_COLORS[d.nodeType] || '#d9d9d9';
  const isStart = d.nodeType === 'start';
  const isEnd = d.nodeType === 'end';
  const isSelectHost = d.nodeType === 'selectHost';
  const isCondition = d.nodeType === 'condition';
  const isSwitch = d.nodeType === 'switch';

  const execClass = d.execStatus ? ` wf-node--exec-${d.execStatus}` : '';

  return (
    <div className={`wf-node wf-node--${d.nodeType}${selected ? ' wf-node--selected' : ''}${d.enabled ? '' : ' wf-node--disabled'}${isStart ? ' wf-node--pill' : ''}${isEnd ? ' wf-node--pill' : ''}${isSelectHost ? ' wf-node--host' : ''}${execClass}`}>
      {!NO_INPUT.has(d.nodeType) && <Handle type="target" position={Position.Left} className="wf-port" />}
      <div className="wf-node-header">
        <div className="wf-node-dot" style={{ background: color }} />
        <div style={{ flex: 1, overflow: 'hidden' }}>
          <div className="wf-node-title">{d.name}</div>
          <div className="wf-node-type">{typeInfo ? tText(typeInfo.labelKey) : d.nodeType}</div>
        </div>
        <StatusIndicator status={d.execStatus} />
      </div>
      {isCondition ? (
        <div className="wf-node-branches">
          <div className="wf-node-branch wf-node-branch--true">
            <span className="wf-node-branch-label">{tText('customNode.yes')}</span>
            <Handle type="source" position={Position.Right} id="true" className="wf-port wf-port--true" style={{ top: '30%' }} />
          </div>
          <div className="wf-node-branch wf-node-branch--false">
            <span className="wf-node-branch-label">{tText('customNode.no')}</span>
            <Handle type="source" position={Position.Right} id="false" className="wf-port wf-port--false" style={{ top: '70%' }} />
          </div>
        </div>
      ) : isSwitch ? (
        <SwitchBranchHandles config={d.config} />
      ) : (
        !NO_OUTPUT.has(d.nodeType) && <Handle type="source" position={Position.Right} id="default" className="wf-port" />
      )}
    </div>
  );
}

export default memo(WfNode);
