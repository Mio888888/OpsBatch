import { useState, useCallback, useEffect } from 'react';
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  useReactFlow,
  ReactFlowProvider,
  type Node,
  type Edge,
  type Connection as XyConnection,
  BackgroundVariant,
  MarkerType,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';

import { Button, Space, Tag, Modal, Input, InputNumber, message } from '../../components/ui';
import { SaveOutlined, PlayCircleOutlined } from '../../components/ui/icons';
import { useTranslation } from '../../i18n';
import { useWorkflowStore } from '../../stores/workflow';
import type { WorkflowRecord, CanvasNode, Connection } from '../../stores/workflow';
import type { WfNodeData, NodeExecStatus } from './components/custom-nodes';
import WfNode from './components/custom-nodes';
import NodePalette from './components/NodePalette';
import { ConfigForm } from './components/NodeEditModal';
import { NODE_TYPES } from './components/nodeTypes';
import { executeWorkflow, type ProgressEvent } from './workflowExecutor';
import ExecutionLogPanel from './components/ExecutionLogPanel';
import { collectWorkflowHostIds, createWorkflowLogEntry, type LogEntry } from './workflowExecutionLogs';
import './workflow.css';

const nodeTypes = { wfNode: WfNode };

function toXyNodes(nodes: CanvasNode[], execStatus?: Map<string, NodeExecStatus>): Node<WfNodeData>[] {
  return nodes.map((n) => ({
    id: n.id,
    type: 'wfNode',
    position: { x: n.x, y: n.y },
    data: {
      name: n.name,
      nodeType: n.type,
      config: n.config,
      enabled: n.enabled,
      execStatus: execStatus?.get(n.id),
    },
  }));
}

function toXyEdges(connections: Connection[]): Edge[] {
  return connections.map((c) => {
    const edge: Edge = {
      id: c.id,
      source: c.fromId,
      target: c.toId,
      type: 'smoothstep',
      markerEnd: { type: MarkerType.ArrowClosed, color: '#999' },
    };
    if (c.sourceHandle) {
      edge.sourceHandle = c.sourceHandle;
      if (c.sourceHandle === 'true') {
        edge.markerEnd = { type: MarkerType.ArrowClosed, color: '#52c41a' };
        edge.style = { stroke: '#52c41a' };
      } else if (c.sourceHandle === 'false') {
        edge.markerEnd = { type: MarkerType.ArrowClosed, color: '#ff4d4f' };
        edge.style = { stroke: '#ff4d4f' };
      }
    }
    return edge;
  });
}

function fromXyNodes(xyNodes: Node<WfNodeData>[]): CanvasNode[] {
  return xyNodes.map((n) => ({
    id: n.id,
    type: n.data.nodeType,
    name: n.data.name,
    config: n.data.config,
    x: n.position.x,
    y: n.position.y,
    enabled: n.data.enabled,
    width: 160,
    height: 56,
  }));
}

function fromXyEdges(xyEdges: Edge[]): Connection[] {
  return xyEdges.map((e) => ({
    id: e.id,
    fromId: e.source,
    toId: e.target,
    sourceHandle: e.sourceHandle || undefined,
  }));
}

interface Props {
  workflow: WorkflowRecord;
  onBack: () => void;
}

interface ContextMenuState {
  x: number;
  y: number;
  nodeId?: string;
  edgeId?: string;
}

function WorkflowEditorInner({ workflow, onBack }: Props) {
  const { tText } = useTranslation();
  const { saveWorkflow } = useWorkflowStore();
  const { screenToFlowPosition } = useReactFlow();

  const [execStatus, setExecStatus] = useState<Map<string, NodeExecStatus>>(new Map());
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [logCollapsed, setLogCollapsed] = useState(false);
  const [executing, setExecuting] = useState(false);
  const [defaultNodeIntervalSeconds, setDefaultNodeIntervalSeconds] = useState(workflow.settings.defaultNodeIntervalSeconds);

  const [xyNodes, setXyNodes, onNodesChange] = useNodesState(toXyNodes(workflow.nodes));
  const [xyEdges, setXyEdges, onEdgesChange] = useEdgesState(toXyEdges(workflow.connections));
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);

  useEffect(() => {
    setDefaultNodeIntervalSeconds(workflow.settings.defaultNodeIntervalSeconds);
  }, [workflow.id, workflow.settings.defaultNodeIntervalSeconds]);

  useEffect(() => {
    setXyNodes((nds) =>
      nds.map((n) => ({
        ...n,
        data: { ...n.data, execStatus: execStatus.get(n.id) },
      }))
    );
  }, [execStatus, setXyNodes]);

  const addNode = useCallback((type: string) => {
    const typeInfo = NODE_TYPES.find((t) => t.value === type);
    const id = `node-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
    const position = screenToFlowPosition({
      x: window.innerWidth / 2 - 80 + (Math.random() - 0.5) * 100,
      y: window.innerHeight / 2 - 28 + (Math.random() - 0.5) * 80,
    });
    const newNode: Node<WfNodeData> = {
      id,
      type: 'wfNode',
      position,
      data: { name: typeInfo?.fallbackLabel || type, nodeType: type, config: '', enabled: true },
    };
    setXyNodes((nds) => [...nds, newNode]);
  }, [setXyNodes, screenToFlowPosition]);

  const onConnect = useCallback((connection: XyConnection) => {
    if (!connection.source || !connection.target) return;
    if (connection.source === connection.target) return;
    const edge: Edge = {
      id: `edge-${connection.source}-${connection.target}${connection.sourceHandle ? `-${connection.sourceHandle}` : ''}`,
      source: connection.source,
      target: connection.target,
      sourceHandle: connection.sourceHandle || undefined,
      type: 'smoothstep',
      markerEnd: { type: MarkerType.ArrowClosed, color: '#999' },
    };
    if (connection.sourceHandle === 'true') {
      edge.markerEnd = { type: MarkerType.ArrowClosed, color: '#52c41a' };
      edge.style = { stroke: '#52c41a' };
    } else if (connection.sourceHandle === 'false') {
      edge.markerEnd = { type: MarkerType.ArrowClosed, color: '#ff4d4f' };
      edge.style = { stroke: '#ff4d4f' };
    }
    setXyEdges((eds) => {
      if (eds.some((e) => e.source === edge.source && e.target === edge.target && e.sourceHandle === edge.sourceHandle)) return eds;
      return [...eds, edge];
    });
  }, [setXyEdges]);

  const onNodeClick = useCallback((_: React.MouseEvent, node: Node) => {
    setSelectedNodeId(node.id);
  }, []);

  const onNodeContextMenu = useCallback((event: React.MouseEvent, node: Node) => {
    event.preventDefault();
    setContextMenu({ x: event.clientX, y: event.clientY, nodeId: node.id });
  }, []);

  const onEdgeContextMenu = useCallback((event: React.MouseEvent, edge: Edge) => {
    event.preventDefault();
    setContextMenu({ x: event.clientX, y: event.clientY, edgeId: edge.id });
  }, []);

  const onPaneClick = useCallback(() => {
    setContextMenu(null);
    setSelectedNodeId(null);
  }, []);

  const handleContextDelete = useCallback(() => {
    if (!contextMenu) return;
    if (contextMenu.nodeId) {
      setXyNodes((nds) => nds.filter((n) => n.id !== contextMenu.nodeId));
      setXyEdges((eds) => eds.filter((e) => e.source !== contextMenu.nodeId && e.target !== contextMenu.nodeId));
    } else if (contextMenu.edgeId) {
      setXyEdges((eds) => eds.filter((e) => e.id !== contextMenu.edgeId));
    }
    setContextMenu(null);
  }, [contextMenu, setXyNodes, setXyEdges]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setContextMenu(null);
        setSelectedNodeId(null);
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, []);

  const updateSelectedNode = useCallback((field: string, value: unknown) => {
    setXyNodes((nds) => nds.map((n) => {
      if (n.id !== selectedNodeId) return n;
      return { ...n, data: { ...n.data, [field]: value } };
    }));
  }, [selectedNodeId, setXyNodes]);

  const updateSelectedNodeConfig = useCallback((config: string) => {
    setXyNodes((nds) => nds.map((n) => {
      if (n.id !== selectedNodeId) return n;
      return { ...n, data: { ...n.data, config } };
    }));
  }, [selectedNodeId, setXyNodes]);

  const deleteSelectedNode = useCallback(() => {
    if (!selectedNodeId) return;
    setXyNodes((nds) => nds.filter((n) => n.id !== selectedNodeId));
    setXyEdges((eds) => eds.filter((e) => e.source !== selectedNodeId && e.target !== selectedNodeId));
    setSelectedNodeId(null);
  }, [selectedNodeId, setXyNodes, setXyEdges]);

  const handleSave = useCallback(async () => {
    const nodes = fromXyNodes(xyNodes);
    const connections = fromXyEdges(xyEdges);
    await saveWorkflow(workflow.id, workflow.name, workflow.description, nodes, connections, 'ready', {
      defaultNodeIntervalSeconds,
    });
    message.success(tText('wfEditor.saved'));
  }, [xyNodes, xyEdges, workflow, saveWorkflow, tText, defaultNodeIntervalSeconds]);

  const [confirmOpen, setConfirmOpen] = useState(false);
  const [confirmName, setConfirmName] = useState('');
  const [confirmDesc, setConfirmDesc] = useState('');
  const [confirmResolve, setConfirmResolve] = useState<((v: boolean) => void) | null>(null);

  const waitForConfirmation = useCallback((nodeName: string, description: string): Promise<boolean> => {
    return new Promise((resolve) => {
      setConfirmName(nodeName);
      setConfirmDesc(description);
      setConfirmOpen(true);
      setConfirmResolve(() => resolve);
    });
  }, []);

  const hostIds = collectWorkflowHostIds(fromXyNodes(xyNodes));

  const handleExecute = useCallback(async () => {
    const nodes = fromXyNodes(xyNodes);
    const hids = collectWorkflowHostIds(nodes);
    if (hids.length === 0) {
      message.warning(tText('wfEditor.addHostNode'));
      return;
    }

    setExecStatus(new Map());
    setLogs([]);
    setLogCollapsed(false);
    setExecuting(true);

    const connections = fromXyEdges(xyEdges);
    const { invoke } = await import('@tauri-apps/api/core');

    await executeWorkflow(
      nodes,
      connections,
      hids,
      invoke as <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>,
      waitForConfirmation,
      (event: ProgressEvent) => {
        switch (event.type) {
          case 'log':
          case 'level_start':
            break;

          case 'node_start':
            setExecStatus((prev) => {
              const next = new Map(prev);
              next.set(event.nodeId, 'running');
              return next;
            });
            break;

          case 'node_complete': {
            const status: NodeExecStatus = event.success ? 'success' : 'fail';
            setExecStatus((prev) => {
              const next = new Map(prev);
              next.set(event.nodeId, status);
              return next;
            });
            break;
          }

          case 'done':
            setExecuting(false);
            break;
        }

        const logEntry = createWorkflowLogEntry(event, {
          startExecute: tText('wfEditor.startExecute'),
          executeSuccess: tText('wfEditor.executeSuccess'),
          executeFail: tText('wfEditor.executeFail'),
          stageInfo: (values) => tText('wfEditor.stageInfo', values),
        });
        if (logEntry) {
          setLogs((prev) => [...prev, logEntry]);
        }
      },
      defaultNodeIntervalSeconds,
    );
  }, [xyNodes, xyEdges, waitForConfirmation, tText, defaultNodeIntervalSeconds]);

  const handleBack = useCallback(() => {
    onBack();
  }, [onBack]);

  const selectedNode = selectedNodeId ? xyNodes.find((n) => n.id === selectedNodeId) : null;

  return (
    <div className="workflow-editor">
      <div className="workflow-editor-header">
        <Space>
          <Button onClick={handleBack}>{tText('wfEditor.backToList')}</Button>
          <h2 style={{ margin: 0, fontSize: 16 }}>{workflow.name}</h2>
          {workflow.status === 'ready' && <Tag color="green">{tText('wfList.ready')}</Tag>}
          {hostIds.length > 0 && <Tag color="blue">{tText('wfEditor.hostSelected', { count: hostIds.length })}</Tag>}
          {executing && <Tag color="orange">{tText('wfEditor.executing')}</Tag>}
        </Space>
        <Space>
          <div className="wf-editor-interval-control" title={tText('wfEditor.defaultNodeIntervalHint')}>
            <span>{tText('wfEditor.defaultNodeInterval')}</span>
            <InputNumber
              min={0}
              value={defaultNodeIntervalSeconds}
              onChange={(value) => setDefaultNodeIntervalSeconds(Math.max(0, value ?? 0))}
              disabled={executing}
              addonAfter="s"
            />
          </div>
          <Button icon={<SaveOutlined />} onClick={handleSave} disabled={executing}>{tText('wfEditor.save')}</Button>
          <Button type="primary" icon={<PlayCircleOutlined />}
            disabled={xyNodes.length === 0 || executing}
            onClick={handleExecute}>
            {tText('wfEditor.executeWorkflow')}
          </Button>
        </Space>
      </div>

      <div className="workflow-editor-body">
        <NodePalette onAddNode={addNode} />

        <div className="workflow-canvas-area">
          <div style={{ flex: 1, position: 'relative' }}>
            <ReactFlow
              nodes={xyNodes}
              edges={xyEdges}
              onNodesChange={onNodesChange}
              onEdgesChange={onEdgesChange}
              onConnect={onConnect}
              onNodeClick={onNodeClick}
              onNodeContextMenu={onNodeContextMenu}
              onEdgeContextMenu={onEdgeContextMenu}
              onPaneClick={onPaneClick}
              nodeTypes={nodeTypes}
              fitView
              minZoom={0.3}
              maxZoom={2}
              style={{ background: 'var(--color-surface-soft)' }}
            >
              <Background variant={BackgroundVariant.Dots} gap={20} size={1} color="#d9d9d9" />
              <Controls showInteractive={false} />
              <MiniMap
                nodeColor={(n) => {
                  const d = n.data as WfNodeData;
                  const info = NODE_TYPES.find((t) => t.value === d?.nodeType);
                  return info?.color || '#d9d9d9';
                }}
              />
            </ReactFlow>

            {contextMenu && (
              <div
                className="wf-context-menu"
                style={{ left: contextMenu.x, top: contextMenu.y }}
              >
                <button
                  type="button"
                  className="wf-context-menu-item wf-context-menu-item--danger"
                  onClick={handleContextDelete}
                >
                  {contextMenu.nodeId ? tText('wfEditor.deleteNode') : tText('wfEditor.deleteConnection')}
                </button>
              </div>
            )}

            {xyNodes.length === 0 && (
              <div className="wf-canvas-empty">
                <div className="wf-canvas-empty-icon">+</div>
                <div>{tText('wfEditor.canvasEmpty')}</div>
                <div style={{ fontSize: 12, marginTop: 4 }}>{tText('wfEditor.canvasEmptyHint')}</div>
              </div>
            )}
          </div>
        </div>

        {selectedNode && (
          <div className="wf-edit-panel">
            <div className="wf-edit-panel-header">
              <span>{tText('wfEditor.editNode')}</span>
              <button type="button" className="wf-edit-panel-close" onClick={() => setSelectedNodeId(null)}>x</button>
            </div>
            <div className="wf-edit-panel-body">
              <div className="wf-edit-field">
                <label>{tText('wfEditor.nodeName')}</label>
                <Input
                  value={selectedNode.data.name}
                  onChange={(e) => updateSelectedNode('name', e.target.value)}
                />
              </div>
              <div className="wf-edit-section">
                <label className="wf-edit-checkbox-label">
                  <input
                    type="checkbox"
                    checked={selectedNode.data.enabled}
                    onChange={(e) => updateSelectedNode('enabled', e.target.checked)}
                  />
                  {tText('wfEditor.enableNode')}
                </label>
              </div>
              <div className="wf-edit-section">
                <div className="wf-edit-section-label">{tText('wfEditor.config')}</div>
                <ConfigForm
                  type={selectedNode.data.nodeType}
                  config={selectedNode.data.config}
                  onChange={(v) => updateSelectedNodeConfig(v)}
                />
              </div>
              <div className="wf-edit-panel-footer">
                <Button danger onClick={deleteSelectedNode}>{tText('wfEditor.deleteNode')}</Button>
              </div>
            </div>
          </div>
        )}
      </div>

      {logs.length > 0 && (
        <ExecutionLogPanel
          logs={logs}
          collapsed={logCollapsed}
          onToggle={() => setLogCollapsed((v) => !v)}
          labels={{ execLog: tText('wfEditor.execLog'), noLogs: tText('wfEditor.noLogs') }}
        />
      )}

      <Modal
        title={tText('wfEditor.manualConfirm')}
        open={confirmOpen}
        onOk={() => { setConfirmOpen(false); confirmResolve?.(true); setConfirmResolve(null); }}
        onCancel={() => { setConfirmOpen(false); confirmResolve?.(false); setConfirmResolve(null); }}
        okText={tText('wfEditor.confirmContinue')}
        cancelText={tText('wfEditor.terminateWorkflow')}
        cancelButtonProps={{ danger: true }}
        maskClosable={false}
        closable={false}
      >
        <p style={{ fontWeight: 600, fontSize: 16, marginBottom: 8 }}>{confirmName}</p>
        {confirmDesc && <p style={{ color: '#666', marginBottom: 12 }}>{confirmDesc}</p>}
        <p>{tText('wfEditor.confirmContinueHint')}</p>
      </Modal>
    </div>
  );
}

export default function WorkflowEditor(props: Props) {
  return (
    <ReactFlowProvider>
      <WorkflowEditorInner {...props} />
    </ReactFlowProvider>
  );
}
