import type { TranslationKey } from '../../../i18n';

interface NodeTypeEntry {
  readonly value: string;
  readonly labelKey: TranslationKey;
  readonly fallbackLabel: string;
  readonly color: string;
}

interface NodeGroup {
  readonly labelKey: TranslationKey;
  readonly fallbackLabel: string;
  readonly types: readonly NodeTypeEntry[];
}

export const NODE_GROUPS: readonly NodeGroup[] = [
  {
    labelKey: 'nodePalette.flowControl',
    fallbackLabel: '流程控制',
    types: [
      { value: 'start', labelKey: 'nodePalette.start', fallbackLabel: '开始', color: '#52c41a' },
      { value: 'end', labelKey: 'nodePalette.end', fallbackLabel: '结束', color: '#ff4d4f' },
    ],
  },
  {
    labelKey: 'nodePalette.target',
    fallbackLabel: '目标',
    types: [
      { value: 'selectHost', labelKey: 'nodePalette.selectHost', fallbackLabel: '选择主机', color: '#1677ff' },
    ],
  },
  {
    labelKey: 'nodePalette.executeOps',
    fallbackLabel: '执行操作',
    types: [
      { value: 'command', labelKey: 'nodePalette.executeCommand', fallbackLabel: '执行命令', color: '#1677ff' },
      { value: 'script', labelKey: 'nodePalette.executeScript', fallbackLabel: '执行脚本', color: '#52c41a' },
      { value: 'quickAction', labelKey: 'nodePalette.quickAction', fallbackLabel: '快捷指令', color: '#fa8c16' },
      { value: 'transfer', labelKey: 'nodePalette.fileTransfer', fallbackLabel: '文件传输', color: '#722ed1' },
    ],
  },
  {
    labelKey: 'nodePalette.logicControl',
    fallbackLabel: '逻辑控制',
    types: [
      { value: 'condition', labelKey: 'nodePalette.condition', fallbackLabel: '条件判断', color: '#13c2c2' },
      { value: 'switch', labelKey: 'nodePalette.switch', fallbackLabel: '多分支判断', color: '#722ed1' },
      { value: 'delay', labelKey: 'nodePalette.delay', fallbackLabel: '等待延时', color: '#8c8c8c' },
      { value: 'confirm', labelKey: 'nodePalette.manualConfirm', fallbackLabel: '人工确认', color: '#eb2f96' },
    ],
  },
  {
    labelKey: 'nodePalette.rollback',
    fallbackLabel: '回滚',
    types: [
      { value: 'rollback', labelKey: 'nodePalette.rollbackOp', fallbackLabel: '回滚操作', color: '#ff4d4f' },
    ],
  },
];

export const NODE_TYPES: NodeTypeEntry[] = NODE_GROUPS.flatMap((g) => g.types);
