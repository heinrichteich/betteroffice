import { canvasPointToModel, modelPointToCanvas } from '@betteroffice/vsdx';
import type { Affine, ModelPoint } from '@betteroffice/vsdx';
export interface DragStart { canvas: ModelPoint; model: ModelPoint; resize: boolean; pin: ModelPoint; size: { width: number; height: number }; parentTransforms?: readonly Affine[]; angle?: number; flipX?: boolean; flipY?: boolean; pointerId?: number; startX?: number; startY?: number; }
const MIN_SHAPE_INCHES = 0.01;
export const passedDragThreshold = (startX: number, startY: number, clientX: number, clientY: number, threshold = 4): boolean => Math.hypot(clientX - startX, clientY - startY) >= threshold;
export const resolveDragGeometry = (start: DragStart, release: ModelPoint): { x: number; y: number; width: number; height: number } => {
  const toParent = (point: ModelPoint) => (start.parentTransforms ?? []).reduce((local, transform) => canvasPointToModel(transform, local.x, local.y), point);
  const origin = toParent(start.model);
  const end = toParent(release);
  const deltaX = end.x - origin.x;
  const deltaY = end.y - origin.y;
  if (start.resize) {
    const cos = Math.cos(start.angle ?? 0), sin = Math.sin(start.angle ?? 0);
    const widthDelta = (cos * deltaX + sin * deltaY) * (start.flipX ? -1 : 1);
    const heightDelta = (-sin * deltaX + cos * deltaY) * (start.flipY ? -1 : 1);
    return { x: start.pin.x, y: start.pin.y, width: Math.max(MIN_SHAPE_INCHES, start.size.width + widthDelta), height: Math.max(MIN_SHAPE_INCHES, start.size.height + heightDelta) };
  }
  return { x: start.pin.x + deltaX, y: start.pin.y + deltaY, width: start.size.width, height: start.size.height };
};
const applyForward = (transform: Affine, point: ModelPoint): ModelPoint => ({ x: transform.a * point.x + transform.c * point.y + transform.e, y: transform.b * point.x + transform.d * point.y + transform.f });
export const previewOutline = (start: DragStart, release: ModelPoint, paintTransform: Affine): ModelPoint[] => {
  const geometry = resolveDragGeometry(start, release);
  const cos = Math.cos(start.angle ?? 0), sin = Math.sin(start.angle ?? 0);
  const halfWidth = geometry.width / 2, halfHeight = geometry.height / 2;
  const offsets: ReadonlyArray<readonly [number, number]> = [[-halfWidth, -halfHeight], [halfWidth, -halfHeight], [halfWidth, halfHeight], [-halfWidth, halfHeight]];
  return offsets.map(([offsetX, offsetY]) => {
    const flippedX = start.flipX ? -offsetX : offsetX;
    const flippedY = start.flipY ? -offsetY : offsetY;
    const parent = { x: flippedX * cos - flippedY * sin + geometry.x, y: flippedX * sin + flippedY * cos + geometry.y };
    const page = (start.parentTransforms ?? []).reduceRight((point, transform) => applyForward(transform, point), parent);
    return modelPointToCanvas(paintTransform, page.x, page.y);
  });
};
export const paintDragPreview = (context: CanvasRenderingContext2D, corners: readonly ModelPoint[], dpr: number, scale: number): void => {
  if (!corners.length) return;
  context.save();
  try {
    context.setTransform(dpr * scale, 0, 0, dpr * scale, 0, 0);
    context.strokeStyle = '#0f6cbd';
    context.lineWidth = 1;
    context.setLineDash([4, 4]);
    context.beginPath();
    context.moveTo(corners[0].x, corners[0].y);
    for (let index = 1; index < corners.length; index += 1) context.lineTo(corners[index].x, corners[index].y);
    context.closePath();
    context.stroke();
  } finally { context.restore(); }
};
