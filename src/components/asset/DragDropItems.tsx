import { useCallback, useEffect, useRef } from 'react';
import { useDraggable, useDroppable } from '@dnd-kit/core';
import { FolderOpenOutlined } from '../ui/icons';
import type { ReactNode } from 'react';
import type { Host } from '../../types';
import { getGroupDropId, getHostDragId } from './constants';

interface DroppableGroupTitleProps {
  groupId: string;
  name: string;
  hostCount: number;
  depth: number;
  children?: ReactNode;
  onContextMenu: (event: React.MouseEvent, groupId: string) => void;
}

export function DroppableGroupTitle({
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

interface DraggableHostTitleProps {
  host: Host;
  children: ReactNode;
}

export function DraggableHostTitle({ host, children }: DraggableHostTitleProps) {
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
