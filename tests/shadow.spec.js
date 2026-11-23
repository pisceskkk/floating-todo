import { test, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import { newDocument, commitView, dateInZone } from '../src/sync-model.js';

const CACHE_KEY = 'floating-todo/v3-preview';
function buildDoc(extraSettings = {}) {
  const doc = newDocument();
  const base = {
    schemaVersion: 3, selectedDay: 'today', projects: [], tags: [], colorCounters: { projects: 0, tags: 0 },
    itemsByDay: { today: [], tomorrow: [], dayAfterTomorrow: [] }, recurringItems: [],
    settings: { appearance: 'light', opacity: 0.78 }, memo: { text: '', expanded: false }, lastActiveDate: dateInZone(),
  };
  const view = {
    ...base,
    itemsByDay: { today: [{ id: 'a', title: '检查圆角与分类', detail: '', time: null, completed: false, createdAt: 1, projectId: 'p', tagIds: ['t'] }], tomorrow: [], dayAfterTomorrow: [] },
    projects: [{ id: 'p', name: '桌面工具', color: '#397fe5' }],
    tags: [{ id: 't', name: '重要', color: '#ae65d6' }],
    settings: { ...base.settings, ...extraSettings },
  };
  commitView(doc, base, view);
  return doc;
}

async function boot(page, extraSettings = {}) {
  const rawDoc = JSON.stringify(buildDoc(extraSettings));
  await page.addInitScript(({ key, rawDoc }) => {
    if (!localStorage.getItem(key)) localStorage.setItem(key, rawDoc);
    window.__TAURI__ = { core: { invoke: async (cmd, args) => {
      if (cmd === 'export_data') window.__backup = args.json;
      if (cmd === 'import_data') return window.__import;
    } } };
  }, { key: CACHE_KEY, rawDoc });
  await page.goto('/'); await expect(page.locator('.row')).toBeVisible();
}
async function settings(page) { await page.locator('[data-act="settings"]').click(); }
async function closeSettings(page) { await page.locator('[data-s="close"]').click(); }
const shadow = page => page.locator('.panel').evaluate(el => getComputedStyle(el).boxShadow);

test('surface style defaults to glass on non-Windows, shadow toggle appears only in flat mode and persists', async ({ page }) => {
  await boot(page);
  await expect(page.locator('html')).toHaveAttribute('data-surface', 'glass');
  await expect(page.locator('html')).toHaveAttribute('data-shadow', 'on');
  expect(await shadow(page)).not.toBe('none');

  // Glass mode hides the flat-only shadow toggle.
  await settings(page);
  await expect(page.locator('[data-shadow-row]')).toBeHidden();

  // Switching to flat reveals the shadow toggle and removes backdrop blur.
  await page.locator('[data-surface="flat"]').click();
  await expect(page.locator('html')).toHaveAttribute('data-surface', 'flat');
  await expect(page.locator('[data-shadow-row]')).toBeVisible();

  const toggle = page.getByRole('switch', { name: '窗口阴影' });
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  await toggle.click();
  expect(await shadow(page)).toBe('none');
  await expect(toggle).toHaveAttribute('aria-checked', 'false');

  await closeSettings(page); await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-surface', 'flat');
  await expect(page.locator('html')).toHaveAttribute('data-shadow', 'off');

  await settings(page); await page.locator('[data-s="export"]').click();
  const exported = await page.evaluate(() => JSON.parse(window.__backup).settings);
  expect(exported.surfaceStyle).toBe('flat');
  expect(exported.shadowEnabled).toBe(false);

  await page.getByRole('switch', { name: '窗口阴影' }).click();
  await page.evaluate(() => { window.__import = window.__backup; }); await page.locator('[data-s="import"]').click();
  await page.locator('[data-import-confirm]').click();
  await expect(page.locator('html')).toHaveAttribute('data-shadow', 'on');
});

test('rounded surface and overlays fit the transparent gutter in glass/flat light/dark and compact layouts', async ({ page }) => {
  await boot(page);
  const output = resolve('test-results/shadow-previews'); mkdirSync(output, { recursive: true });
  for (const surface of ['glass', 'flat']) {
    for (const theme of ['light', 'dark']) {
      await settings(page);
      await page.locator(`button[data-surface="${surface}"]`).click();
      await page.locator(`[data-ap="${theme}"]`).click();
      await closeSettings(page);

      const panel = await page.locator('.panel').boundingBox();
      expect(panel.x).toBe(12); expect(panel.y).toBe(12); expect(panel.width).toBe(316);
      const metrics = await page.locator('.panel').evaluate(el => ({
        radius: getComputedStyle(el).borderRadius,
        clip: getComputedStyle(el, '::before').clipPath,
        blur: getComputedStyle(el, '::before').backdropFilter,
      }));
      expect(metrics.radius).toBe('24px');
      expect(metrics.clip).toBe('inset(0px round 24px)');
      if (surface === 'glass') expect(metrics.blur).toContain('blur(28px)');
      else expect(metrics.blur).toBe('none');

      await page.screenshot({ path: `${output}/${surface}-${theme}-on.png`, omitBackground: true });
      await settings(page);
      await expect(page.locator('.overlay')).toHaveCSS('border-radius', '24px');
      await expect(page.locator('.overlay')).toHaveCSS('overflow', 'hidden');
      await page.screenshot({ path: `${output}/${surface}-${theme}-settings.png`, omitBackground: true });
      if (surface === 'flat') {
        await page.getByRole('switch', { name: '窗口阴影' }).click(); await closeSettings(page);
        await page.screenshot({ path: `${output}/${surface}-${theme}-off.png`, omitBackground: true });
        expect(await shadow(page)).toBe('none');
        await settings(page); await page.getByRole('switch', { name: '窗口阴影' }).click(); await closeSettings(page);
      } else {
        await closeSettings(page);
      }
    }
  }
  await settings(page); await page.locator('[data-s="categories"]').click();
  await page.setViewportSize({ width: 220, height: 180 });
  const dialog = await page.getByRole('dialog').boundingBox();
  expect(dialog.x).toBeGreaterThanOrEqual(12); expect(dialog.x + dialog.width).toBeLessThanOrEqual(208);
  expect(dialog.y + dialog.height).toBeLessThanOrEqual(168);
  expect(await page.getByRole('dialog').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
  await page.screenshot({ path: `${output}/compact-manager.png`, omitBackground: true });
});
