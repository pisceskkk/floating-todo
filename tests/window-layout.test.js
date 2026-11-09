import test from 'node:test';
import assert from 'node:assert/strict';
import { anchoredBounds, floatingBounds, southwestResize, BAR_SIZE, PANEL_DEFAULT, CAPTURE_SIZE } from '../src/window-layout.js';

for (const scale of [1, 1.25, 1.5, 2]) {
  for (const horizontal of ['left', 'right']) for (const vertical of ['up', 'down']) {
    test(`floating ${horizontal}/${vertical} retains bar anchor at ${scale} DPI`, () => {
      const area = { position: { x: -1920 * scale, y: -100 * scale }, size: { width: 1920 * scale, height: 1080 * scale } };
      const bar = { x: Math.round((horizontal === 'right' ? -1910 : -198) * scale), y: Math.round((vertical === 'down' ? -90 : 902) * scale), width: 188 * scale, height: 68 * scale, scale };
      const { bounds, direction } = floatingBounds(bar, CAPTURE_SIZE, area);
      assert.deepEqual(direction, { horizontal, vertical });
      assert.equal(horizontal === 'left' ? bounds.x + bounds.width : bounds.x, horizontal === 'left' ? bar.x + bar.width : bar.x);
      assert.equal(vertical === 'up' ? bounds.y + bounds.height : bounds.y, vertical === 'up' ? bar.y + bar.height : bar.y);
      assert.ok(bounds.x >= area.position.x && bounds.y >= area.position.y);
      assert.ok(bounds.x + bounds.width <= 0 && bounds.y + bounds.height <= 980 * scale);
    });
  }
}

for (const scale of [1, 1.25, 1.5, 2]) {
  test(`southwest gesture requires both midlines at ${scale * 100}% scale`, () => {
    const frame = { x: -600, y: 80, width: 340 * scale, height: 560 * scale, scale };
    const start = { x: frame.x, y: frame.y + frame.height };
    const center = { x: frame.x + frame.width - 110 * scale, y: frame.y + 90 * scale };
    const bottom = frame.y + 180 * scale;
    const left = frame.x + frame.width - 220 * scale;
    for (const pointer of [
      { x: left, y: bottom },
      { x: center.x + 1, y: bottom },
      { x: left, y: center.y - 1 },
      { x: center.x, y: center.y },
      { x: center.x + 1, y: center.y },
      { x: center.x, y: center.y - 1 },
    ]) assert.equal(southwestResize(frame, start, pointer).collapse, false);
    const result = southwestResize(frame, start, { x: center.x + 1, y: center.y - 1 });
    assert.equal(result.collapse, true);
    assert.deepEqual(result.bounds, { ...frame, x: left, width: 220 * scale, height: 180 * scale });
    // Moving back out of the minimum dimensions disarms the gesture.
    assert.equal(southwestResize(frame, start, { x: start.x, y: bottom }).collapse, false);
  });
}

test('collapse and expand preserve the right edge at noninteger DPI and negative monitor coordinates', () => {
  const frame = { x: -900, y: 90, width: 510, height: 840, scale: 1.5 };
  const mini = anchoredBounds(frame, BAR_SIZE);
  assert.equal(mini.x + mini.width, frame.x + frame.width);
  assert.equal(mini.y, frame.y);
  assert.deepEqual(anchoredBounds(mini, PANEL_DEFAULT), frame);
});

test('expanding near a work-area edge keeps the panel accessible', () => {
  const frame = { x: -200, y: 1000, width: 188, height: 68, scale: 1 };
  const area = { position: { x: -1920, y: 0 }, size: { width: 1920, height: 1040 } };
  assert.deepEqual(anchoredBounds(frame, PANEL_DEFAULT, area), { x: -352, y: 480, width: 340, height: 560, scale: 1 });
});
