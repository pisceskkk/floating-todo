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
