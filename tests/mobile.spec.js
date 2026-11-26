import { test, expect } from '@playwright/test';

test.use({
  viewport: { width: 390, height: 844 },
  userAgent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148',
  hasTouch: true,
});

test('mobile mode locks fullscreen panel, hides desktop controls, and handles deep links', async ({ page }) => {
  await page.goto('/');

  // Mobile class & fullscreen panel without desktop inset/border-radius
  await expect(page.locator('html')).toHaveClass(/platform-mobile/);
  const viewportMeta = await page.locator('meta[name="viewport"]').getAttribute('content');
  expect(viewportMeta).toContain('user-scalable=no');
  expect(viewportMeta).toContain('maximum-scale=1.0');
  const panelBox = await page.locator('#panel').boundingBox();
  expect(Math.round(panelBox.width)).toBe(390);
  expect(Math.round(panelBox.height)).toBe(844);
  // Bright crisp white in light mode (opaque, not gray translucent desktop glass)
  const lightOpacity = await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue('--opacity').trim());
  expect(lightOpacity).toBe('1');
  const mainInputFontSize = await page.locator('#main-input').evaluate(el => parseFloat(getComputedStyle(el).fontSize));
  expect(mainInputFontSize).toBeGreaterThanOrEqual(16);

  // Desktop collapse pill and resize handles are hidden/absent
  await expect(page.locator('[data-act="collapse"]')).toHaveCount(0);
  await expect(page.locator('.resize').first()).toBeHidden();

  // Open settings and verify desktop-only items are hidden
  await page.locator('[data-act="settings"]').click();
  await expect(page.locator('.settings')).toBeVisible();
  await expect(page.locator('[data-s="alwaysOnTop"]')).toHaveCount(0);
  await expect(page.locator('[data-s="clickThrough"]')).toHaveCount(0);
  await expect(page.locator('#autostart-sw')).toHaveCount(0);
  await expect(page.locator('#op-range')).toHaveCount(0);
  await expect(page.locator('.settings [data-surface="glass"]')).toHaveCount(0);
  await expect(page.locator('[data-update-panel]')).toHaveCount(0);
  // Mobile-relevant settings remain visible
  await expect(page.locator('[data-s="memoEnabled"]')).toBeVisible();
  await expect(page.locator('[data-s="lan"]')).toBeVisible();
  await expect(page.locator('[data-s="sync"]')).toBeVisible();
  await page.locator('[data-s="close"]').click();

  // Deep link: open tomorrow tab
  await page.evaluate(() => {
    window.dispatchEvent(new CustomEvent('floatingtodo-deep-link', { detail: 'floatingtodo://open?day=tomorrow' }));
  });
  await expect(page.locator('.day-seg[data-day="tomorrow"]')).toHaveClass(/sel/);

  // Deep link: quick-capture focuses main input
  await page.evaluate(() => {
    window.dispatchEvent(new CustomEvent('floatingtodo-deep-link', { detail: 'floatingtodo://quick-capture' }));
  });
  await expect(page.locator('#main-input')).toBeFocused();

  // Add a task and toggle completion
  await page.locator('#main-input').fill('移动端验收任务 15:30');
  await page.locator('#main-input').press('Enter');
  await expect(page.locator('.row', { hasText: '移动端验收任务' })).toBeVisible();
  await page.locator('.row', { hasText: '移动端验收任务' }).locator('.check').click();
  await expect(page.locator('.row', { hasText: '移动端验收任务' })).toHaveClass(/done/);
});

test('iOS keyboard lifts only the main composer without pushing up the rest of the UI', async ({ page }) => {
  await page.addInitScript(() => {
    const listeners = { resize: new Set(), scroll: new Set() };
    const state = { width: 390, height: 844, offsetTop: 0, offsetLeft: 0, pageTop: 0, pageLeft: 0, scale: 1 };
    const vv = {
      get width() { return state.width; },
      get height() { return state.height; },
      get offsetTop() { return state.offsetTop; },
      get offsetLeft() { return state.offsetLeft; },
      get pageTop() { return state.pageTop; },
      get pageLeft() { return state.pageLeft; },
      get scale() { return state.scale; },
      addEventListener(type, fn) { listeners[type]?.add(fn); },
      removeEventListener(type, fn) { listeners[type]?.delete(fn); },
    };
    Object.defineProperty(window, 'visualViewport', { value: vv, configurable: true });
    window.__setMockVisualViewport = (next) => {
      Object.assign(state, next);
      listeners.resize.forEach(fn => fn());
      listeners.scroll.forEach(fn => fn());
    };
  });

  await page.goto('/');

  const beforePanel = await page.locator('#panel').boundingBox();
  const beforeHeader = await page.locator('.header').boundingBox();
  const beforeDayPicker = await page.locator('.day-picker').boundingBox();
  const beforeEmpty = await page.locator('.empty').boundingBox();
  const beforeComposer = await page.locator('.main-composer').boundingBox();

  // Focus main input and simulate iOS keyboard opening (height 844 -> 508, WKWebView offsetTop 0)
  await page.locator('#main-input').focus();
  await page.evaluate(() => window.__setMockVisualViewport({ height: 508, offsetTop: 0 }));

  await expect(page.locator('html')).toHaveClass(/mobile-composer-lifted/);
  await expect.poll(async () => {
    const box = await page.locator('.main-composer').boundingBox();
    return Math.round(box.y + box.height);
  }).toBe(498);

  const afterPanel = await page.locator('#panel').boundingBox();
  const afterHeader = await page.locator('.header').boundingBox();
  const afterDayPicker = await page.locator('.day-picker').boundingBox();
  const afterEmpty = await page.locator('.empty').boundingBox();
  const afterComposer = await page.locator('.main-composer').boundingBox();

  // Panel and surrounding UI stay completely stationary (not shrunk or pushed up)
  expect(Math.round(afterPanel.height)).toBe(Math.round(beforePanel.height));
  expect(Math.round(afterHeader.y)).toBe(Math.round(beforeHeader.y));
  expect(Math.round(afterDayPicker.y)).toBe(Math.round(beforeDayPicker.y));
  expect(Math.round(afterEmpty.y)).toBe(Math.round(beforeEmpty.y));

  // Only the main composer is lifted to sit right above the keyboard (508 - 10 = 498)
  expect(afterComposer.y).toBeLessThan(beforeComposer.y - 250);
  expect(Math.round(afterComposer.y + afterComposer.height)).toBe(498);

  // Simulate iOS WKWebView panning visualViewport.offsetTop > 0: panel compensates via top offset
  await page.evaluate(() => window.__setMockVisualViewport({ height: 508, offsetTop: 120 }));
  await expect.poll(async () => page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue('--mobile-vv-offset-top').trim()
  )).toBe('120px');

  // Keyboard closes: everything returns to resting state
  await page.evaluate(() => window.__setMockVisualViewport({ height: 844, offsetTop: 0 }));

  await expect(page.locator('html')).not.toHaveClass(/mobile-composer-lifted/);
  await expect.poll(async () => {
    const box = await page.locator('.main-composer').boundingBox();
    return Math.round(box.y);
  }).toBe(Math.round(beforeComposer.y));

  // Android native IME inset bridge (without shrinking visualViewport) lifts only the main composer
  await page.locator('#main-input').focus();
  await page.evaluate(() => {
    window.__nativeKeyboardInset = 320;
    window.dispatchEvent(new CustomEvent('floatingtodo-keyboard-inset', { detail: 320 }));
  });
  await expect(page.locator('html')).toHaveClass(/mobile-composer-lifted/);
  await expect.poll(async () => {
    const box = await page.locator('.main-composer').boundingBox();
    return Math.round(box.y + box.height);
  }).toBe(844 - 320 - 10);

  const androidEmpty = await page.locator('.empty').boundingBox();
  expect(Math.round(androidEmpty.y)).toBe(Math.round(beforeEmpty.y));

  await page.evaluate(() => {
    window.__nativeKeyboardInset = 0;
    window.dispatchEvent(new CustomEvent('floatingtodo-keyboard-inset', { detail: 0 }));
  });
  await expect(page.locator('html')).not.toHaveClass(/mobile-composer-lifted/);
  await expect.poll(async () => {
    const box = await page.locator('.main-composer').boundingBox();
    return Math.round(box.y);
  }).toBe(Math.round(beforeComposer.y));
});
