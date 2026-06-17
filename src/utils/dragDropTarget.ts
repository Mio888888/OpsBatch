export interface DragPoint {
  x: number;
  y: number;
}

export interface DragRect {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

export function isPointInsideRectInEitherCoordinateSpace(
  point: DragPoint,
  rect: DragRect,
  devicePixelRatio = 1,
): boolean {
  return isPointInsideRect(point, rect)
    || isPointInsideRect({ x: point.x / devicePixelRatio, y: point.y / devicePixelRatio }, rect);
}

export function shouldAcceptExternalFileDrop({
  paths,
  isOverTarget,
  position,
  targetRect,
  devicePixelRatio = 1,
}: {
  paths: string[];
  isOverTarget: boolean;
  position?: DragPoint | null;
  targetRect?: DragRect | null;
  devicePixelRatio?: number;
}): boolean {
  if (paths.length === 0) return false;
  if (isOverTarget) return true;
  if (!position || !targetRect) return false;
  return isPointInsideRectInEitherCoordinateSpace(position, targetRect, devicePixelRatio);
}

function isPointInsideRect(point: DragPoint, rect: DragRect): boolean {
  return point.x >= rect.left
    && point.x <= rect.right
    && point.y >= rect.top
    && point.y <= rect.bottom;
}
