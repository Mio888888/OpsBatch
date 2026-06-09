import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import gsap from 'gsap';
import { useGSAP } from '@gsap/react';
import {
  DndContext,
  PointerSensor,
  pointerWithin,
  rectIntersection,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
} from '@dnd-kit/core';
import type { CollisionDetection, DragEndEvent, DragOverEvent } from '@dnd-kit/core';
import {
  Button,
  Empty,
  Form,
  Input,
  InputNumber,
  Layout,
  Modal,
  Popconfirm,
  Select,
  Spin,
  Tooltip,
  Tree,
  TreeSelect,
  message,
} from './ui';
import {
  ApartmentOutlined,
  CloudServerOutlined,
  CloseOutlined,
  CodeOutlined,
  DeleteOutlined,
  EditOutlined,
  FolderOpenOutlined,
  GithubOutlined,
  PlusOutlined,
  ProfileOutlined,
  ReloadOutlined,
  SearchOutlined,
  SettingOutlined,
  UploadOutlined,
} from './ui/icons';
import type { Key, ReactNode } from 'react';
import type { DataNode, TreeSelectOption } from './ui';
import CloudImport from './CloudImport';
import WindowControls from './WindowControls';
import { useTranslation } from '../i18n';
import type { TranslationKey } from '../i18n';
import { useAssetsStore } from '../stores/assets';
import { useThemeStore } from '../stores/theme';
import type { AssetGroup, Host } from '../types';
import { requestKeychainNotice } from '../utils/keychainNotice';

const { Content } = Layout;

gsap.registerPlugin(useGSAP);

function useSystemReduceMotion() {
  const [systemReduceMotion, setSystemReduceMotion] = useState(
    () => window.matchMedia('(prefers-reduced-motion: reduce)').matches,
  );

  useEffect(() => {
    const motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    const update = () => setSystemReduceMotion(motionQuery.matches);
    motionQuery.addEventListener('change', update);
    update();
    return () => motionQuery.removeEventListener('change', update);
  }, []);

  return systemReduceMotion;
}

const workModes: Array<{ key: string; icon: ReactNode; labelKey: TranslationKey }> = [
  { key: '/terminal', icon: <CodeOutlined />, labelKey: 'nav.terminal' },
  { key: '/workflow', icon: <ApartmentOutlined />, labelKey: 'nav.workflow' },
  { key: '/github', icon: <GithubOutlined />, labelKey: 'nav.github' },
];

interface HostTreeNode extends DataNode {
  children?: HostTreeNode[];
  nodeType: 'group' | 'host';
  groupId?: string;
  host?: Host;
  depth?: number;
}

interface DroppableGroupTitleProps {
  groupId: string;
  name: string;
  hostCount: number;
  depth: number;
  children?: ReactNode;
  onContextMenu: (event: React.MouseEvent, groupId: string) => void;
}

interface DraggableHostTitleProps {
  host: Host;
  children: ReactNode;
}

interface HostFormValues {
  name: string;
  ip: string;
  port: number;
  authType: Host['authType'];
  username: string;
  password?: string;
  privateKey?: string;
  os: Host['os'];
  tags?: string[];
  groupId?: string;
  remark?: string;
  jumpChain?: string[];
}

const DEFAULT_GROUP_ID = '__default__';
const GROUP_DROP_ID_PREFIX = 'asset-group:';
const HOST_DRAG_ID_PREFIX = 'asset-host:';
const SECRET_PLACEHOLDER = '***keychain***';

function editableSecret(value?: string): string | undefined {
  return value && value !== SECRET_PLACEHOLDER ? value : undefined;
}

function submittedSecret(value?: string): string | undefined {
  const trimmed = value?.trim();
  return trimmed && trimmed !== SECRET_PLACEHOLDER ? value : undefined;
}

function secretDebugState(value?: string) {
  return {
    present: Boolean(value),
    placeholder: value === SECRET_PLACEHOLDER,
    length: value?.length ?? 0,
  };
}

function hostUsesStoredSecret(host: Pick<Host, 'authType' | 'password' | 'privateKey' | 'jumpChain'>) {
  if (host.authType === 'password' && host.password === SECRET_PLACEHOLDER) return true;
  if (host.authType === 'key' && host.privateKey === SECRET_PLACEHOLDER) return true;
  return host.jumpChain.length > 0;
}

function hostFormStoresSecret(host: Pick<Host, 'password' | 'privateKey'>) {
  return Boolean(host.password || host.privateKey);
}

function getGroupDropId(groupId: string) {
  return `${GROUP_DROP_ID_PREFIX}${groupId}`;
}

function getHostDragId(hostId: string) {
  return `${HOST_DRAG_ID_PREFIX}${hostId}`;
}

function getGroupIdFromDropId(id: string) {
  return id.startsWith(GROUP_DROP_ID_PREFIX) ? id.slice(GROUP_DROP_ID_PREFIX.length) : null;
}

function getHostIdFromDragId(id: string) {
  return id.startsWith(HOST_DRAG_ID_PREFIX) ? id.slice(HOST_DRAG_ID_PREFIX.length) : null;
}

function DroppableGroupTitle({
  groupId,
  name,
  hostCount,
  depth,
  children,
  onContextMenu,
}: DroppableGroupTitleProps) {
  const { isOver, setNodeRef } = useDroppable({
    id: getGroupDropId(groupId),
    data: { type: 'group', groupId },
  });

  return (
    <div
      ref={setNodeRef}
      className={`asset-tree-group-title${isOver ? ' asset-tree-group-title-drag-over' : ''}`}
      style={{ marginLeft: -depth * 12, paddingLeft: depth * 12 + 4 }}
      onContextMenu={(event) => onContextMenu(event, groupId)}
    >
      <FolderOpenOutlined />
      <span>{name}</span>
      <span className="asset-tree-count">{hostCount}</span>
      {children}
    </div>
  );
}

function DraggableHostTitle({ host, children }: DraggableHostTitleProps) {
  const pointerDownRef = useRef<{ x: number; y: number } | null>(null);
  const suppressClickAfterDragRef = useRef(false);
  const suppressClickResetRef = useRef<number | null>(null);
  const { attributes, isDragging, listeners, setNodeRef, transform } = useDraggable({
    id: getHostDragId(host.id),
    data: { type: 'host', hostId: host.id },
  });

  useEffect(() => {
    if (isDragging) {
      if (suppressClickResetRef.current !== null) {
        window.clearTimeout(suppressClickResetRef.current);
        suppressClickResetRef.current = null;
      }
      suppressClickAfterDragRef.current = true;
      return undefined;
    }

    if (!suppressClickAfterDragRef.current) return undefined;

    const timeoutId = window.setTimeout(() => {
      suppressClickAfterDragRef.current = false;
      suppressClickResetRef.current = null;
    }, 120);
    suppressClickResetRef.current = timeoutId;

    return () => window.clearTimeout(timeoutId);
  }, [isDragging]);

  const handlePointerDownCapture = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) {
      pointerDownRef.current = null;
      return;
    }
    pointerDownRef.current = { x: event.clientX, y: event.clientY };
  }, []);

  const handleClickCapture = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    const pointerDown = pointerDownRef.current;
    pointerDownRef.current = null;
    const movedAfterPointerDown = pointerDown
      ? Math.hypot(event.clientX - pointerDown.x, event.clientY - pointerDown.y) >= 4
      : false;

    if (!suppressClickAfterDragRef.current && !movedAfterPointerDown) return;

    suppressClickAfterDragRef.current = false;
    if (suppressClickResetRef.current !== null) {
      window.clearTimeout(suppressClickResetRef.current);
      suppressClickResetRef.current = null;
    }
    event.preventDefault();
    event.stopPropagation();
  }, []);

  return (
    <div
      ref={setNodeRef}
      className={`asset-tree-draggable-host${isDragging ? ' asset-tree-draggable-host-dragging' : ''}`}
      style={{
        transform: transform ? `translate3d(${transform.x}px, ${transform.y}px, 0)` : undefined,
      }}
      onPointerDownCapture={handlePointerDownCapture}
      onClickCapture={handleClickCapture}
      {...listeners}
      {...attributes}
    >
      {children}
    </div>
  );
}

function getActiveMode(pathname: string) {
  if (pathname === '/terminal') return '/terminal';
  if (pathname === '/workflow') return '/workflow';
  if (pathname === '/github') return '/github';
  if (pathname === '/assets' || pathname === '/commands' || pathname === '/quick-actions') return '';
  return '';
}

export default function MainLayout({ children }: { children: ReactNode }) {
  const { t, tText } = useTranslation();
  const location = useLocation();
  const navigate = useNavigate();
  const layoutRef = useRef<HTMLDivElement>(null);
  const assetPanelRef = useRef<HTMLElement>(null);
  const mainPanelRef = useRef<HTMLElement>(null);
  const hasPlayedIntroRef = useRef(false);
  const assetPanelIntroPlayedRef = useRef(false);
  const previousActiveModeRef = useRef(getActiveMode(location.pathname));
  const previousPathnameRef = useRef(location.pathname);
  const motionSettingsLoaded = useThemeStore((s) => s.loaded);
  const reduceMotionSetting = useThemeStore((s) => s.reduceMotion);
  const systemReduceMotion = useSystemReduceMotion();
  const reduceMotion = !motionSettingsLoaded || reduceMotionSetting || systemReduceMotion;
  const activeMode = getActiveMode(location.pathname);
  const isTerminal = location.pathname === '/terminal' || location.pathname === '/assets' || location.pathname === '/';
  const [assetPanelVisible, setAssetPanelVisible] = useState(
    () => new URLSearchParams(location.search).get('assets') === '1',
  );
  const closeAssetPanel = useCallback(() => {
    setAssetPanelVisible(false);
    if (new URLSearchParams(location.search).get('assets') === '1') {
      navigate(location.pathname, { replace: true });
    }
  }, [location.pathname, location.search, navigate]);
  const [assetSearchText, setAssetSearchText] = useState('');
  const [assetExpandedKeys, setAssetExpandedKeys] = useState<Key[]>([]);
  const [userCollapsedKeys, setUserCollapsedKeys] = useState<Set<string>>(new Set());
  const [hostModalOpen, setHostModalOpen] = useState(false);
  const [editingHost, setEditingHost] = useState<Host | null>(null);
  const [cloudImportOpen, setCloudImportOpen] = useState(false);
  const [hostForm] = Form.useForm<HostFormValues>();

  // Group context menu & modal
  const [ctxMenu, setCtxMenu] = useState<{
    x: number;
    y: number;
    target: { type: 'empty' } | { type: 'group'; groupId: string } | { type: 'host'; hostId: string };
  } | null>(null);
  const [groupModalOpen, setGroupModalOpen] = useState(false);
  const [editingGroup, setEditingGroup] = useState<AssetGroup | null>(null);
  const [editingDefaultGroup, setEditingDefaultGroup] = useState(false);
  const [newGroupParentId, setNewGroupParentId] = useState<string | undefined>(undefined);
  const [groupForm] = Form.useForm<{ name: string }>();
  const ctxMenuRef = useRef<HTMLDivElement>(null);
  const lastDragOverGroupIdRef = useRef<string | null>(null);
  const dndSensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }));
  const assetTreeCollisionDetection = useCallback<CollisionDetection>((args) => {
    const pointerCollisions = pointerWithin(args);
    return pointerCollisions.length > 0 ? pointerCollisions : rectIntersection(args);
  }, []);

  const hosts = useAssetsStore((s) => s.hosts);
  const groups = useAssetsStore((s) => s.groups);
  // @ts-expect-error — tags removed from UI but kept in store for future use
  const tags = useAssetsStore((s) => s.tags);
  const selectedHostIds = useAssetsStore((s) => s.selectedHostIds);
  const loading = useAssetsStore((s) => s.loading);
  const loadHosts = useAssetsStore((s) => s.loadHosts);
  const addHost = useAssetsStore((s) => s.addHost);
  const updateHost = useAssetsStore((s) => s.updateHost);
  const deleteHost = useAssetsStore((s) => s.deleteHost);
  const setSelectedHostIds = useAssetsStore((s) => s.setSelectedHostIds);
  const loadTags = useAssetsStore((s) => s.loadTags);
  const loadGroups = useAssetsStore((s) => s.loadGroups);
  const addGroup = useAssetsStore((s) => s.addGroup);
  const updateGroup = useAssetsStore((s) => s.updateGroup);
  const deleteGroup = useAssetsStore((s) => s.deleteGroup);
  const defaultGroupName = useAssetsStore((s) => s.defaultGroupName);
  const setDefaultGroupName = useAssetsStore((s) => s.setDefaultGroupName);
  const displayedDefaultGroupName = defaultGroupName || tText('assets.defaultGroup');

  useEffect(() => {
    const params = new URLSearchParams(location.search);
    if (params.get('assets') === '1') {
      setAssetPanelVisible(true);
    }
  }, [location.search]);

  useEffect(() => {
    if (assetPanelVisible) {
      void loadHosts();
      void loadTags();
      void loadGroups();
    }
  }, [assetPanelVisible, loadHosts, loadTags, loadGroups]);

  useEffect(() => {
    if (!assetPanelVisible && hosts.length === 0) {
      void loadHosts();
    }
  }, [assetPanelVisible, hosts.length, loadHosts]);

  const lastOpenHostRef = useRef('');
  const openHostTerminal = useCallback(async (host: Host) => {
    if (lastOpenHostRef.current === host.id) return;
    if (hostUsesStoredSecret(host) && !(await requestKeychainNotice())) return;
    lastOpenHostRef.current = host.id;
    window.setTimeout(() => {
      if (lastOpenHostRef.current === host.id) lastOpenHostRef.current = '';
    }, 300);
    closeAssetPanel();
    navigate('/terminal', {
      state: {
        openHost: {
          requestId: `${host.id}-${Date.now()}-${Math.random().toString(36).slice(2)}`,
          hostId: host.id,
          name: host.name,
          ip: host.ip,
        },
      },
    });
  }, [closeAssetPanel, navigate, requestKeychainNotice]);

  const hostIdSet = useMemo(() => new Set(hosts.map((h) => h.id)), [hosts]);

  const filteredHosts = useMemo(() => {
    const query = assetSearchText.trim().toLowerCase();
    if (!query) return hosts;
    return hosts.filter((host) => {
      const fields = [host.name, host.ip, host.username, host.remark].join(' ').toLowerCase();
      return fields.includes(query);
    });
  }, [assetSearchText, hosts]);

  const handleAssetTreeCheck = useCallback((checkedKeys: Key[] | { checked: Key[]; halfChecked: Key[] }) => {
    const keys = Array.isArray(checkedKeys) ? checkedKeys : checkedKeys.checked;
    setSelectedHostIds(keys.map(String).filter((key) => hostIdSet.has(key)));
  }, [hostIdSet, setSelectedHostIds]);

  // Right-click context menu
  const handleGroupContextMenu = useCallback((e: React.MouseEvent, groupId: string) => {
    e.preventDefault();
    e.stopPropagation();
    setCtxMenu({ x: e.clientX, y: e.clientY, target: { type: 'group', groupId } });
  }, []);

  useEffect(() => {
    if (!ctxMenu) return;
    const close = (e: MouseEvent) => {
      if (ctxMenuRef.current && !ctxMenuRef.current.contains(e.target as Node)) {
        setCtxMenu(null);
      }
    };
    document.addEventListener('mousedown', close);
    return () => document.removeEventListener('mousedown', close);
  }, [ctxMenu]);

  const handleCtxAddGroup = useCallback((parentId?: string) => {
    setCtxMenu(null);
    setEditingGroup(null);
    setEditingDefaultGroup(false);
    setNewGroupParentId(parentId);
    groupForm.resetFields();
    setGroupModalOpen(true);
  }, [groupForm]);

  const handleCtxEditGroup = useCallback(() => {
    if (!ctxMenu || ctxMenu.target.type !== 'group') return;
    const target = ctxMenu.target as { type: 'group'; groupId: string };
    setCtxMenu(null);
    if (target.groupId === DEFAULT_GROUP_ID) {
      setEditingGroup(null);
      setEditingDefaultGroup(true);
      groupForm.resetFields();
      groupForm.setFieldsValue({ name: displayedDefaultGroupName });
    } else {
      const group = groups.find((g) => g.id === target.groupId);
      if (!group) return;
      setEditingGroup(group);
      setEditingDefaultGroup(false);
      groupForm.resetFields();
      groupForm.setFieldsValue({ name: group.name });
    }
    setGroupModalOpen(true);
  }, [ctxMenu, groups, groupForm, displayedDefaultGroupName]);

  const handleCtxDeleteGroup = useCallback(async () => {
    if (!ctxMenu || ctxMenu.target.type !== 'group') return;
    const groupId = ctxMenu.target.groupId;
    setCtxMenu(null);
    if (!window.confirm(tText('assets.deleteGroupConfirm'))) return;
    try {
      await deleteGroup(groupId);
      message.success(tText('assets.groupDeleted'));
    } catch (e: unknown) {
      message.error(tText('common.deleteFailed', { error: String(e) }));
    }
  }, [ctxMenu, deleteGroup, tText]);

  const handleSaveGroup = useCallback(async () => {
    try {
      const values = await groupForm.validateFields();
      if (editingDefaultGroup) {
        setDefaultGroupName(values.name);
        message.success(tText('assets.groupUpdated'));
      } else if (editingGroup) {
        await updateGroup({ ...editingGroup, name: values.name });
        message.success(tText('assets.groupUpdated'));
      } else {
        await addGroup(values.name, newGroupParentId);
        message.success(tText('assets.groupCreated'));
      }
      setGroupModalOpen(false);
      setEditingGroup(null);
      setEditingDefaultGroup(false);
      groupForm.resetFields();
    } catch {
      // form validation
    }
  }, [addGroup, editingGroup, editingDefaultGroup, groupForm, newGroupParentId, setDefaultGroupName, tText, updateGroup]);

  const openNewHostModal = useCallback((groupId?: string) => {
    setEditingHost(null);
    hostForm.resetFields();
    hostForm.setFieldsValue({
      port: 22,
      os: 'linux',
      authType: 'password',
      username: 'root',
      groupId: groupId ?? DEFAULT_GROUP_ID,
      remark: '',
    });
    setHostModalOpen(true);
  }, [hostForm]);

  const openEditHostModal = useCallback((host: Host) => {
    setEditingHost(host);
    hostForm.resetFields();
    console.info('[host-secret] open edit host modal', {
      hostId: host.id,
      authType: host.authType,
      password: secretDebugState(host.password),
      privateKey: secretDebugState(host.privateKey),
    });
    hostForm.setFieldsValue({
      name: host.name,
      ip: host.ip,
      port: host.port,
      authType: host.authType,
      username: host.username,
      password: editableSecret(host.password),
      privateKey: editableSecret(host.privateKey),
      os: host.os,
      groupId: host.groupId ?? DEFAULT_GROUP_ID,
      remark: host.remark,
      jumpChain: host.jumpChain ?? [],
    });
    setHostModalOpen(true);
  }, [hostForm]);

  const closeHostModal = useCallback(() => {
    setHostModalOpen(false);
    setEditingHost(null);
    hostForm.resetFields();
  }, [hostForm]);

  const handleSaveHost = useCallback(async () => {
    try {
      const values = await hostForm.validateFields();
      const normalizedHost: Omit<Host, 'id' | 'createdAt' | 'updatedAt'> = {
        name: values.name,
        ip: values.ip,
        port: values.port,
        authType: values.authType,
        username: values.username,
        password: submittedSecret(values.password),
        privateKey: submittedSecret(values.privateKey),
        os: values.os,
        tags: values.tags ?? [],
        groupId: values.groupId && values.groupId !== DEFAULT_GROUP_ID ? values.groupId : undefined,
        remark: values.remark ?? '',
        jumpChain: values.jumpChain ?? [],
      };
      console.info('[host-secret] submit host form', {
        hostId: editingHost?.id ?? '(new)',
        authType: values.authType,
        editing: Boolean(editingHost),
        rawPassword: secretDebugState(values.password),
        submittedPassword: secretDebugState(normalizedHost.password),
        rawPrivateKey: secretDebugState(values.privateKey),
        submittedPrivateKey: secretDebugState(normalizedHost.privateKey),
      });

      try {
        if (hostFormStoresSecret(normalizedHost) && !(await requestKeychainNotice())) return;
        if (editingHost) {
          await updateHost({ ...editingHost, ...normalizedHost });
          message.success(tText('assets.hostUpdated'));
        } else {
          await addHost(normalizedHost);
          message.success(tText('assets.hostAdded'));
        }
        closeHostModal();
      } catch (e: unknown) {
        message.error(tText('common.operationFailed', { error: String(e) }));
      }
    } catch {
      // 表单校验失败时由项目 Form 展示字段错误
    }
  }, [addHost, closeHostModal, editingHost, hostForm, requestKeychainNotice, tText, updateHost]);

  const handleDeleteHost = useCallback(async (hostId: string) => {
    try {
      await deleteHost(hostId);
      message.success(tText('assets.hostDeleted'));
    } catch (e: unknown) {
      message.error(tText('common.deleteFailed', { error: String(e) }));
    }
  }, [deleteHost, tText]);

  const handleAssetDragStart = useCallback(() => {
    lastDragOverGroupIdRef.current = null;
  }, []);

  const handleAssetDragOver = useCallback((event: DragOverEvent) => {
    const groupId = event.over ? getGroupIdFromDropId(String(event.over.id)) : null;
    if (!groupId || lastDragOverGroupIdRef.current === groupId) return;
    lastDragOverGroupIdRef.current = groupId;
  }, []);

  const handleAssetDragEnd = useCallback((event: DragEndEvent) => {
    const hostId = getHostIdFromDragId(String(event.active.id));
    const dropGroupId = event.over ? getGroupIdFromDropId(String(event.over.id)) : null;
    lastDragOverGroupIdRef.current = null;

    if (!hostId || !dropGroupId) return;

    const host = hosts.find((h) => h.id === hostId);
    if (!host) return;

    const newGroupId = dropGroupId === DEFAULT_GROUP_ID ? undefined : dropGroupId;
    if (host.groupId === newGroupId) return;

    void updateHost({ ...host, groupId: newGroupId })
      .catch((error: unknown) => {
        message.error(tText('common.moveFailed', { error: String(error) }));
      });
  }, [hosts, tText, updateHost]);

  const handleAssetDragCancel = useCallback(() => {
    lastDragOverGroupIdRef.current = null;
  }, []);

  const renderHostTitle = useCallback((host: Host) => {
    return (
      <div
        className="asset-tree-host-title"
        onClick={(event) => {
          if (event.defaultPrevented || event.button !== 0 || event.ctrlKey) return;
          void openHostTerminal(host);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setCtxMenu({ x: e.clientX, y: e.clientY, target: { type: 'host', hostId: host.id } });
        }}
      >
        <div className="asset-tree-host-main">
          <div className="asset-tree-host-text">
            <span className="asset-tree-host-name">{host.name}</span>
            <span className="asset-tree-host-meta">{host.username}@{host.ip}:{host.port}</span>
            <div className="asset-tree-host-details">
              <span>{host.os === 'linux' ? 'Linux' : 'Windows'}</span>
              {host.remark ? <span>{host.remark}</span> : null}
            </div>
          </div>
        </div>
        <div className="asset-tree-host-actions">
          <Tooltip title={t('assets.openTerminal')}>
            <button
              type="button"
              className="asset-tree-host-icon-button"
              onClick={(event) => {
                event.stopPropagation();
                void openHostTerminal(host);
              }}
            >
              <CodeOutlined />
            </button>
          </Tooltip>
          <Tooltip title={t('common.edit')}>
            <button
              type="button"
              className="asset-tree-host-icon-button"
              onClick={(event) => {
                event.stopPropagation();
                openEditHostModal(host);
              }}
            >
              <EditOutlined />
            </button>
          </Tooltip>
          <Popconfirm
            title={t('assets.deleteHostConfirm')}
            onConfirm={() => {
              void handleDeleteHost(host.id);
            }}
          >
            <button
              type="button"
              className="asset-tree-host-icon-button asset-tree-host-icon-button-danger"
              aria-label={tText('assets.deleteHostAria')}
              title={tText('common.delete')}
              onClick={(event) => event.stopPropagation()}
            >
              <DeleteOutlined />
            </button>
          </Popconfirm>
        </div>
      </div>
    );
  }, [handleDeleteHost, openEditHostModal, openHostTerminal, t, tText]);

  const assetTreeData = useMemo<HostTreeNode[]>(() => {
    const hostsByGroup = new Map<string, Host[]>();
    for (const host of filteredHosts) {
      const gid = host.groupId ?? DEFAULT_GROUP_ID;
      let list = hostsByGroup.get(gid);
      if (!list) { list = []; hostsByGroup.set(gid, list); }
      list.push(host);
    }

    const groupsByParent = new Map<string, AssetGroup[]>();
    for (const group of groups) {
      const parentId = group.parentId ?? '';
      let list = groupsByParent.get(parentId);
      if (!list) { list = []; groupsByParent.set(parentId, list); }
      list.push(group);
    }

    const buildGroupNode = (groupId: string, name: string, depth = 0): HostTreeNode => {
      const groupHosts = hostsByGroup.get(groupId) ?? [];
      const childGroups = groupsByParent.get(groupId) ?? [];
      return {
        key: groupId,
        selectable: false,
        isLeaf: false,
        nodeType: 'group',
        groupId,
        depth,
        title: (
          <DroppableGroupTitle
            groupId={groupId}
            name={name}
            hostCount={groupHosts.length}
            depth={depth}
            onContextMenu={handleGroupContextMenu}
          />
        ),
        children: [
          ...childGroups.map((cg) => buildGroupNode(cg.id, cg.name, depth + 1)),
          ...groupHosts.map<HostTreeNode>((host) => ({
            key: host.id,
            title: (
              <DraggableHostTitle host={host}>
                {renderHostTitle(host)}
              </DraggableHostTitle>
            ),
            isLeaf: true,
            nodeType: 'host',
            host,
            depth: depth + 1,
          })),
        ],
      };
    };

    const groupNodes: HostTreeNode[] = [];

    // Default group first
    groupNodes.push(buildGroupNode(DEFAULT_GROUP_ID, displayedDefaultGroupName));

    // Top-level custom groups (no parentId)
    for (const group of groupsByParent.get('') ?? []) {
      groupNodes.push(buildGroupNode(group.id, group.name));
    }

    return groupNodes;
  }, [filteredHosts, groups, displayedDefaultGroupName, renderHostTitle, handleGroupContextMenu]);

  // Build a flat tree structure for the group picker (TreeSelect)
  const groupTreeData = useMemo<TreeSelectOption[]>(() => {
    const groupById = new Map(groups.map((group) => [group.id, group]));
    const groupsByParent = new Map<string, AssetGroup[]>();
    for (const group of groups) {
      const parentId = group.parentId ?? '';
      let list = groupsByParent.get(parentId);
      if (!list) { list = []; groupsByParent.set(parentId, list); }
      list.push(group);
    }

    const toOption = (gId: string): TreeSelectOption => {
      const group = groupById.get(gId);
      const label = gId === DEFAULT_GROUP_ID ? tText('assets.defaultGroup') : (group?.name ?? '?');
      const children = (groupsByParent.get(gId) ?? []).map((cg) => toOption(cg.id));
      return { key: gId, title: label, children: children.length > 0 ? children : undefined };
    };
    return [
      toOption(DEFAULT_GROUP_ID),
      ...(groupsByParent.get('') ?? []).map((g) => toOption(g.id)),
    ];
  }, [groups, tText]);

  // Auto-expand groups that contain hosts
  const autoExpandedKeys = useMemo(() => {
    const groupsWithHosts = new Set<string>();
    const parentByGroup = new Map(groups.map((group) => [group.id, group.parentId]));

    for (const host of hosts) {
      const groupId = host.groupId ?? DEFAULT_GROUP_ID;
      groupsWithHosts.add(groupId);
      let parentId = parentByGroup.get(groupId);
      while (parentId) {
        groupsWithHosts.add(parentId);
        parentId = parentByGroup.get(parentId);
      }
    }

    return groupsWithHosts;
  }, [hosts, groups]);

  const effectiveExpandedKeys = useMemo<Key[]>(() => {
    const keys = new Set<Key>(autoExpandedKeys);
    for (const k of assetExpandedKeys) keys.add(k);
    for (const k of userCollapsedKeys) keys.delete(k);
    return [...keys];
  }, [autoExpandedKeys, assetExpandedKeys, userCollapsedKeys]);

  const openBatchTerminal = useCallback(async () => {
    if (selectedHostIds.length === 0) return;
    if (!(await requestKeychainNotice())) return;
    await invoke('open_managed_window', { kind: 'batch-terminal', hostIds: selectedHostIds });
  }, [requestKeychainNotice, selectedHostIds]);

  const openBatchTransfer = useCallback(async () => {
    if (selectedHostIds.length === 0) return;
    if (!(await requestKeychainNotice())) return;
    await invoke('open_managed_window', { kind: 'batch-transfer', hostIds: selectedHostIds });
  }, [requestKeychainNotice, selectedHostIds]);

  const openSettingsWindow = useCallback(async () => {
    await invoke('open_managed_window', { kind: 'settings' });
  }, []);

  const openLogWindow = useCallback(async () => {
    await invoke('open_managed_window', { kind: 'global-log' });
  }, []);

  useGSAP(() => {
    const root = layoutRef.current;
    if (!root) return;

    const topbar = root.querySelector<HTMLElement>('.workbench-topbar');
    const shell = root.querySelector<HTMLElement>('.workbench-shell');
    const introTargets = [topbar, shell].filter((target): target is HTMLElement => Boolean(target));

    if (reduceMotion || hasPlayedIntroRef.current) {
      gsap.set(introTargets, { autoAlpha: 1, clearProps: 'transform,opacity,visibility' });
      return;
    }

    hasPlayedIntroRef.current = true;
    const timeline = gsap.timeline({ defaults: { duration: 0.42, ease: 'power3.out' } });
    if (topbar) timeline.from(topbar, { y: -8, autoAlpha: 0 }, 0);
    if (shell) timeline.from(shell, { y: 10, autoAlpha: 0 }, 0.08);

    return () => timeline.kill();
  }, { scope: layoutRef, dependencies: [motionSettingsLoaded], revertOnUpdate: true });

  useGSAP(() => {
    if (previousActiveModeRef.current === activeMode) return;
    previousActiveModeRef.current = activeMode;

    const activeTab = layoutRef.current?.querySelector<HTMLElement>('.workbench-mode-tab-active');
    if (!activeTab) return;

    if (reduceMotion) {
      gsap.set(activeTab, { clearProps: 'transform' });
      return;
    }

    const tween = gsap.fromTo(
      activeTab,
      { scale: 0.96 },
      { scale: 1, duration: 0.24, ease: 'back.out(1.6)', clearProps: 'transform' },
    );
    return () => tween.kill();
  }, { scope: layoutRef, dependencies: [activeMode, reduceMotion], revertOnUpdate: true });

  useGSAP(() => {
    const pathnameChanged = previousPathnameRef.current !== location.pathname;
    previousPathnameRef.current = location.pathname;
    if (!pathnameChanged || isTerminal) return;

    const panel = mainPanelRef.current;
    if (!panel) return;

    if (reduceMotion) {
      gsap.set(panel, { autoAlpha: 1, clearProps: 'transform,opacity,visibility' });
      return;
    }

    const tween = gsap.fromTo(
      panel,
      { y: 8, autoAlpha: 0.86 },
      { y: 0, autoAlpha: 1, duration: 0.26, ease: 'power2.out', clearProps: 'transform,opacity,visibility' },
    );
    return () => tween.kill();
  }, { scope: mainPanelRef, dependencies: [location.pathname, reduceMotion], revertOnUpdate: true });

  useGSAP(() => {
    const panel = assetPanelRef.current;
    if (!assetPanelVisible || !panel) {
      assetPanelIntroPlayedRef.current = false;
      return;
    }

    const chromeTargets = Array.from(panel.querySelectorAll<HTMLElement>(
      '.asset-sidebar-header, .asset-sidebar-panel > .ui-input-prefix, .asset-sidebar-action-grid, .asset-sidebar-selection, .asset-sidebar-loading, .ui-empty',
    ));
    const rowTargets = Array.from(panel.querySelectorAll<HTMLElement>('.asset-tree .ui-tree-node')).slice(0, 18);
    const detailTargets = [...chromeTargets, ...rowTargets];

    if (reduceMotion) {
      assetPanelIntroPlayedRef.current = true;
      gsap.set([panel, ...detailTargets], { autoAlpha: 1, clearProps: 'transform,opacity,visibility' });
      return;
    }

    if (assetPanelIntroPlayedRef.current) {
      const rowTween = rowTargets.length > 0
        ? gsap.from(rowTargets, {
          y: 4,
          autoAlpha: 0,
          duration: 0.16,
          stagger: 0.018,
          ease: 'power2.out',
          clearProps: 'transform,opacity,visibility',
        })
        : null;
      return () => rowTween?.kill();
    }

    assetPanelIntroPlayedRef.current = true;
    const timeline = gsap.timeline({ defaults: { ease: 'power2.out' } });
    timeline.from(panel, { x: -18, autoAlpha: 0, duration: 0.32, ease: 'power3.out' });
    timeline.from(detailTargets, {
      y: 6,
      autoAlpha: 0,
      duration: 0.2,
      stagger: 0.025,
      clearProps: 'transform,opacity,visibility',
    }, '-=0.16');

    return () => timeline.kill();
  }, { scope: layoutRef, dependencies: [assetPanelVisible, assetTreeData.length, reduceMotion], revertOnUpdate: true });

  useGSAP(() => {
    const menu = ctxMenuRef.current;
    if (!ctxMenu || !menu) return;

    if (reduceMotion) {
      gsap.set(menu, { autoAlpha: 1, scale: 1, clearProps: 'transform,opacity,visibility' });
      return;
    }

    const tween = gsap.fromTo(
      menu,
      { autoAlpha: 0, scale: 0.97, y: -2, transformOrigin: 'left top' },
      { autoAlpha: 1, scale: 1, y: 0, duration: 0.14, ease: 'power2.out', clearProps: 'transform,opacity,visibility' },
    );
    return () => tween.kill();
  }, { scope: layoutRef, dependencies: [ctxMenu, reduceMotion], revertOnUpdate: true });

  return (
    <div ref={layoutRef} className="app-layout workbench-layout">
      <header
        className="workbench-topbar"
        onMouseDown={(e) => {
          const target = e.target as HTMLElement;
          if (target.closest('button, nav')) return;
          getCurrentWindow().startDragging();
        }}
      >
        <div className="workbench-titlebar-left">
          <WindowControls />
          <Tooltip title={assetPanelVisible ? t('assets.closePanel') : t('assets.openPanel')}>
            <button
              type="button"
              className={`tool-icon-button${assetPanelVisible ? ' tool-icon-button-active' : ''}`}
              onClick={() => assetPanelVisible ? closeAssetPanel() : setAssetPanelVisible(true)}
              aria-label={assetPanelVisible ? tText('assets.closePanel') : tText('assets.openPanel')}
            >
              <CloudServerOutlined />
            </button>
          </Tooltip>
        </div>

        <nav className="workbench-mode-tabs" aria-label={tText('nav.workModes')}>
          {workModes.map((mode) => (
            <button
              key={mode.key}
              type="button"
              className={`workbench-mode-tab${activeMode === mode.key ? ' workbench-mode-tab-active' : ''}`}
              onClick={() => navigate(mode.key)}
            >
              {mode.icon}
              <span>{t(mode.labelKey)}</span>
            </button>
          ))}
        </nav>

        <div className="workbench-tools">
          <Tooltip title={t('nav.globalLog')}>
            <button
              type="button"
              className="tool-icon-button"
              onClick={openLogWindow}
              aria-label={tText('nav.globalLog')}
            >
              <ProfileOutlined />
            </button>
          </Tooltip>
          <Tooltip title={t('nav.settings')}>
            <button type="button" className="tool-icon-button" onClick={openSettingsWindow} aria-label={tText('nav.settings')}>
              <SettingOutlined />
            </button>
          </Tooltip>
        </div>
      </header>

      <div className="workbench-body">
        {assetPanelVisible && (
          <aside ref={assetPanelRef} className="workbench-asset-panel" aria-label={tText('assets.title')}>
            <div className="asset-sidebar-panel">
              <div className="asset-sidebar-header">
                <div>
                  <span className="asset-sidebar-eyebrow">{t('assets.eyebrow')}</span>
                  <h2>{t('assets.title')}</h2>
                </div>
                <div className="asset-sidebar-header-actions">
                  <Tooltip title={t('assets.refresh')}>
                    <Button
                      type="text"
                      size="small"
                      icon={<ReloadOutlined spin={loading} />}
                      loading={loading}
                      onClick={() => {
                        void loadHosts();
                      }}
                    />
                  </Tooltip>
                  <Tooltip title={t('assets.hide')}>
                    <Button
                      type="text"
                      size="small"
                      icon={<CloseOutlined />}
                      onClick={() => closeAssetPanel()}
                    />
                  </Tooltip>
                </div>
              </div>

              <Input
                size="small"
                allowClear
                prefix={<SearchOutlined />}
                placeholder={tText('assets.searchPlaceholder')}
                value={assetSearchText}
                onChange={(event) => setAssetSearchText(event.target.value)}
              />

              <div className="asset-sidebar-action-grid asset-sidebar-action-grid-4">
                <Button size="small" type="primary" icon={<PlusOutlined />} onClick={() => openNewHostModal()}>{t('common.add')}</Button>
                <Button size="small" icon={<CloudServerOutlined />} onClick={() => setCloudImportOpen(true)}>{t('assets.cloudImport')}</Button>
                <Button
                  size="small"
                  type="primary"
                  icon={<CodeOutlined />}
                  disabled={selectedHostIds.length === 0}
                  onClick={openBatchTerminal}
                >
                  {t('assets.broadcastTerminal')}
                </Button>
                <Button
                  size="small"
                  icon={<UploadOutlined />}
                  disabled={selectedHostIds.length === 0}
                  onClick={openBatchTransfer}
                >
                  {t('assets.batchUpload')}
                </Button>
              </div>

              <div className="asset-sidebar-selection">
                {t('assets.selectedSummary', { selected: selectedHostIds.length, visible: filteredHosts.length, total: hosts.length })}
              </div>

              <div
                className="asset-tree-scroll"
                onContextMenu={(e) => {
                  if ((e.target as HTMLElement).closest('.asset-tree-group-title')) return;
                  e.preventDefault();
                  setCtxMenu({ x: e.clientX, y: e.clientY, target: { type: 'empty' } });
                }}
              >
                {loading && hosts.length === 0 ? (
                  <div className="asset-sidebar-loading">
                    <Spin size="small" />
                    <span>{t('assets.loadingHosts')}</span>
                  </div>
                ) : assetTreeData.length > 0 ? (
                  <DndContext
                    sensors={dndSensors}
                    collisionDetection={assetTreeCollisionDetection}
                    onDragStart={handleAssetDragStart}
                    onDragOver={handleAssetDragOver}
                    onDragEnd={handleAssetDragEnd}
                    onDragCancel={handleAssetDragCancel}
                  >
                    <Tree
                      className="asset-tree"
                      blockNode
                      checkable
                      selectable={false}
                      checkedKeys={selectedHostIds}
                      expandedKeys={effectiveExpandedKeys}
                      treeData={assetTreeData}
                      onCheck={handleAssetTreeCheck}
                      onExpand={(keys) => {
                        setAssetExpandedKeys(keys);
                        const collapsed = new Set(effectiveExpandedKeys.map(String).filter((k) => !keys.map(String).includes(k)));
                        setUserCollapsedKeys((prev) => {
                          const next = new Set(prev);
                          for (const c of collapsed) next.add(c);
                          for (const k of keys.map(String)) next.delete(k);
                          return next;
                        });
                      }}
                    />
                  </DndContext>
                ) : (
                  <Empty
                    image={Empty.PRESENTED_IMAGE_SIMPLE}
                    description={assetSearchText ? t('assets.noMatchedHosts') : t('assets.noHosts')}
                  />
                )}
              </div>

              {/* Right-click context menu */}
              {ctxMenu && (
                <div
                  ref={ctxMenuRef}
                  className="asset-ctx-menu"
                  style={{ position: 'fixed', left: ctxMenu.x, top: ctxMenu.y, zIndex: 1000 }}
                >
                  {ctxMenu.target.type === 'host' && (() => {
                    const target = ctxMenu.target as { type: 'host'; hostId: string };
                    const host = hosts.find((h) => h.id === target.hostId);
                    if (!host) return null;
                    return (
                      <>
                        <button type="button" className="asset-ctx-menu-item"
                          onClick={() => { setCtxMenu(null); void openHostTerminal(host); }}
                        ><CodeOutlined /> {t('assets.openTerminal')}</button>
                        <button type="button" className="asset-ctx-menu-item"
                          onClick={() => { setCtxMenu(null); openEditHostModal(host); }}
                        ><EditOutlined /> {t('common.edit')}</button>
                        <div className="asset-ctx-menu-divider" />
                        <button type="button" className="asset-ctx-menu-item"
                          onClick={() => {
                            setCtxMenu(null);
                            void updateHost({ ...host, groupId: undefined });
                          }}
                        ><FolderOpenOutlined /> {t('assets.moveToDefaultGroup')}</button>
                        {groups.map((g) => (
                          <button
                            key={g.id}
                            type="button"
                            className={`asset-ctx-menu-item${host.groupId === g.id ? ' asset-ctx-menu-item-active' : ''}`}
                            onClick={() => {
                              setCtxMenu(null);
                              if (host.groupId !== g.id) void updateHost({ ...host, groupId: g.id });
                            }}
                          ><FolderOpenOutlined /> {g.name}</button>
                        ))}
                        <div className="asset-ctx-menu-divider" />
                        <button type="button" className="asset-ctx-menu-item asset-ctx-menu-item-danger"
                          onClick={() => {
                            setCtxMenu(null);
                            if (window.confirm(tText('assets.deleteHostNamedConfirm', { name: host.name }))) void handleDeleteHost(host.id);
                          }}
                        ><DeleteOutlined /> {t('common.delete')}</button>
                      </>
                    );
                  })()}

                  {ctxMenu.target.type === 'group' && (() => {
                    const target = ctxMenu.target as { type: 'group'; groupId: string };
                    const { groupId } = target;
                    const isDefault = groupId === DEFAULT_GROUP_ID;
                    return (
                      <>
                        {!isDefault && (
                          <button type="button" className="asset-ctx-menu-item"
                            onClick={() => { setCtxMenu(null); openNewHostModal(groupId); }}
                          ><PlusOutlined /> {t('assets.addHostToGroup')}</button>
                        )}
                        <button type="button" className="asset-ctx-menu-item"
                          onClick={() => handleCtxAddGroup(isDefault ? undefined : groupId)}
                        ><FolderOpenOutlined /> {t('assets.newGroup')}</button>
                        <button type="button" className="asset-ctx-menu-item"
                          onClick={() => handleCtxAddGroup(isDefault ? undefined : groupId)}
                        ><FolderOpenOutlined /> {t('assets.newSubgroup')}</button>
                        <div className="asset-ctx-menu-divider" />
                        <button type="button" className="asset-ctx-menu-item"
                          onClick={handleCtxEditGroup}
                        ><EditOutlined /> {t('assets.editGroup')}</button>
                        {!isDefault && (
                          <button type="button" className="asset-ctx-menu-item asset-ctx-menu-item-danger"
                            onClick={handleCtxDeleteGroup}
                          ><DeleteOutlined /> {t('assets.deleteGroup')}</button>
                        )}
                      </>
                    );
                  })()}

                  {ctxMenu.target.type === 'empty' && (
                    <button type="button" className="asset-ctx-menu-item"
                      onClick={() => handleCtxAddGroup()}
                    ><FolderOpenOutlined /> {t('assets.newGroup')}</button>
                  )}
                </div>
              )}

              <Modal
                title={editingHost ? t('assets.editHost') : t('assets.addHost')}
                open={hostModalOpen}
                onOk={() => {
                  void handleSaveHost();
                }}
                onCancel={closeHostModal}
                width={520}
                className="asset-host-modal"
                destroyOnHidden
              >
                <Form form={hostForm} layout="vertical" className="asset-host-form">
                  {/* ── 连接信息 ── */}
                  <div className="asset-host-form-grid asset-host-form-grid-name">
                    <Form.Item name="name" label={t('assets.hostName')} rules={[{ required: true, message: tText('common.required') }]}>
                      <Input placeholder={tText('assets.hostNamePlaceholder')} />
                    </Form.Item>
                    <Form.Item name="os" label={t('assets.system')} rules={[{ required: true, message: tText('common.required') }]}>
                      <Select options={[
                        { value: 'linux', label: 'Linux' },
                        { value: 'windows', label: 'Windows' },
                      ]} />
                    </Form.Item>
                  </div>
                  <div className="asset-host-form-grid asset-host-form-grid-address">
                    <Form.Item name="ip" label={t('assets.hostAddress')} rules={[{ required: true, message: tText('common.required') }]}>
                      <Input placeholder={tText('assets.hostAddressPlaceholder')} />
                    </Form.Item>
                    <Form.Item name="port" label={t('assets.port')} rules={[{ required: true, message: tText('common.required') }]}>
                      <InputNumber min={1} max={65535} />
                    </Form.Item>
                  </div>

                  {/* ── 认证 ── */}
                  <div className="asset-host-form-grid asset-host-form-grid-auth">
                    <Form.Item name="authType" label={t('assets.auth')} rules={[{ required: true, message: tText('common.required') }]}>
                      <Select options={[
                        { value: 'password', label: t('assets.passwordAuth') },
                        { value: 'key', label: t('assets.keyAuth') },
                      ]} />
                    </Form.Item>
                    <Form.Item name="username" label={t('assets.username')} rules={[{ required: true, message: tText('common.required') }]}>
                      <Input placeholder="root" />
                    </Form.Item>
                    <Form.Item noStyle shouldUpdate>
                      {() => {
                        const authType = hostForm.getFieldValue('authType');
                        if (authType === 'key') {
                          return (
                            <Form.Item name="privateKey" label={t('assets.sshPrivateKey')}>
                              <Input.TextArea rows={1} placeholder={tText('assets.pastePrivateKey')} />
                            </Form.Item>
                          );
                        }
                        return (
                          <Form.Item name="password" label={t('assets.password')}>
                            <Input.Password placeholder={editingHost ? tText('assets.passwordKeepPlaceholder') : tText('assets.passwordPlaceholder')} />
                          </Form.Item>
                        );
                      }}
                    </Form.Item>
                  </div>

                  {/* ── 元数据 ── */}
                  <div className="asset-host-form-grid asset-host-form-grid-meta">
                    <Form.Item name="groupId" label={t('assets.group')}>
                      <TreeSelect
                        options={groupTreeData}
                        placeholder={tText('assets.selectGroup')}
                      />
                    </Form.Item>
                    <Form.Item name="remark" label={t('assets.remark')}>
                      <Input placeholder={tText('assets.remarkPlaceholder')} />
                    </Form.Item>
                  </div>

                  {/* ── 跳板链 ── */}
                  <Form.Item name="jumpChain" label={t('assets.jumpChain')} extra={t('assets.jumpChainExtra')} className="asset-host-form-jump">
                    <Select
                      mode="multiple"
                      allowClear
                      placeholder={tText('assets.directNoJump')}
                      options={hosts.map((h) => ({ value: h.id, label: `${h.name} (${h.ip})` }))}
                    />
                  </Form.Item>
                </Form>
              </Modal>

              {/* Group Modal */}
              <Modal
                title={editingGroup ? t('assets.editGroup') : t('assets.newGroup')}
                open={groupModalOpen}
                onOk={() => { void handleSaveGroup(); }}
                onCancel={() => { setGroupModalOpen(false); setEditingGroup(null); }}
                destroyOnHidden
              >
                <Form form={groupForm} layout="vertical" style={{ marginTop: 16 }}>
                  <Form.Item name="name" label={t('assets.groupName')} rules={[{ required: true, message: tText('common.required') }]}>
                    <Input placeholder={tText('assets.groupName')} />
                  </Form.Item>
                </Form>
              </Modal>

              <CloudImport open={cloudImportOpen} onClose={() => setCloudImportOpen(false)} onImported={loadHosts} />
            </div>
          </aside>
        )}

        <Layout className="workbench-shell">
          <Content className="workbench-content">
            <main ref={mainPanelRef} className="workbench-main-panel">{children}</main>
          </Content>
        </Layout>
      </div>
    </div>
  );
}
