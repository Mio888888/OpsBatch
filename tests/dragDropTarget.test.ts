import assert from 'node:assert/strict';
import test from 'node:test';
import {
  isPointInsideRectInEitherCoordinateSpace,
  shouldAcceptExternalFileDrop,
} from '../src/utils/dragDropTarget.ts';

const targetRect = {
  left: 500,
  right: 900,
  top: 120,
  bottom: 520,
};

test('accepts drop paths when a previous over event marked the remote list as target', () => {
  assert.equal(shouldAcceptExternalFileDrop({
    paths: ['/Users/me/Desktop/report.csv'],
    isOverTarget: true,
  }), true);
});

test('rejects drop paths when there is no target evidence', () => {
  assert.equal(shouldAcceptExternalFileDrop({
    paths: ['/Users/me/Desktop/report.csv'],
    isOverTarget: false,
  }), false);
});

test('accepts coordinates reported as logical pixels', () => {
  assert.equal(isPointInsideRectInEitherCoordinateSpace(
    { x: 640, y: 300 },
    targetRect,
    2,
  ), true);
});

test('accepts coordinates reported as physical pixels on high DPI displays', () => {
  assert.equal(isPointInsideRectInEitherCoordinateSpace(
    { x: 1280, y: 600 },
    targetRect,
    2,
  ), true);
});
