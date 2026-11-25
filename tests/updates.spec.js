import { test, expect } from '@playwright/test';

async function boot(page, { fail = false, canInstall = true, installFail = false } = {}) {
  await page.addInitScript(({ fail, canInstall, installFail }) => {
    window.updateCalls = 0;
    window.installCalls = 0;
    window.updateFail = fail;
    window.installFail = installFail;
    window.openedUrls = [];
    window.eventListeners = {};
    window.emitTauriEvent = (name, payload) => {
      for (const fn of window.eventListeners[name] || []) fn({ payload });
    };
    window.__TAURI__ = {
      core: { invoke: async name => {
        if (name === 'installed_version') return '0.4.0-rc.5';
        if (name === 'install_update') {
          window.installCalls++;
          if (window.installFail) throw new Error('下载中断，请检查网络后重试');
          return new Promise(resolve => { window.finishInstall = resolve; });
        }
        if (name !== 'check_for_updates') return;
        window.updateCalls++;
        if (window.updateFail) throw new Error('无法连接 GitHub，请检查网络后重试');
        return {
          currentVersion: '0.4.0-rc.5',
          version: '0.4.0-rc.10',
          url: 'https://github.com/pisceskkk/floating-todo/releases/tag/v0.4.0-rc.10',
          canInstall,
          assetName: canInstall ? 'floating-todo_0.4.0-rc.10_aarch64.dmg' : null,
          assetSize: canInstall ? 20 * 1024 * 1024 : null,
        };
      } },
      event: {
        listen: async (event, handler) => {
          window.eventListeners[event] = window.eventListeners[event] || [];
          window.eventListeners[event].push(handler);
          return () => {
            window.eventListeners[event] = (window.eventListeners[event] || []).filter(h => h !== handler);
          };
        },
      },
      opener: { openUrl: async url => window.openedUrls.push(url) },
    };
  }, { fail, canInstall, installFail });
  await page.goto('/');
  await expect(page.locator('#main-input')).toBeVisible();
}

test('automatically discovers releases, persists indicator across renders and opens release', async ({ page }) => {
  await boot(page, { canInstall: false });
  await expect(page.locator('body')).toHaveClass(/update-available/, { timeout: 10000 });
  await page.locator('[data-act="settings"]').click();
  await expect(page.locator('[data-update-status]')).toContainText('0.4.0-rc.10');
  await expect(page.locator('[data-installed-version]')).toHaveText('0.4.0-rc.5');
  await expect(page.locator('[data-update-install]')).toBeHidden();
  await page.locator('[data-update-open]').click();
  expect(await page.evaluate(() => window.openedUrls)).toEqual(['https://github.com/pisceskkk/floating-todo/releases/tag/v0.4.0-rc.10']);
  await page.locator('[data-s="close"]').click();
  await expect(page.locator('body')).toHaveClass(/update-available/);
});

test('in-app update downloads with progress and recovers on failure', async ({ page }) => {
  await boot(page, { canInstall: true, installFail: true });
  await expect(page.locator('body')).toHaveClass(/update-available/, { timeout: 10000 });
  await page.locator('[data-act="settings"]').click();
  const installBtn = page.locator('[data-update-install]');
  await expect(installBtn).toBeVisible();
  await expect(installBtn).toHaveText('立即更新');

  // First attempt fails and remains retryable.
  await installBtn.click();
  await expect(page.locator('[data-update-status]')).toContainText('更新失败');
  await expect(installBtn).toBeEnabled();

  // Retry succeeds and reflects live progress events.
  await page.evaluate(() => { window.installFail = false; });
  await installBtn.click();
  await expect(installBtn).toBeDisabled();
  await expect(page.locator('[data-update-progress]')).toBeVisible();

  await page.evaluate(() => {
    window.emitTauriEvent('update-progress', {
      stage: 'downloading',
      downloaded: 10 * 1024 * 1024,
      total: 20 * 1024 * 1024,
      percent: 50,
      message: '正在下载新版本 0.4.0-rc.10…',
    });
  });
  await expect(page.locator('[data-update-status]')).toContainText('50%');

  await page.evaluate(() => {
    window.emitTauriEvent('update-progress', {
      stage: 'installing',
      downloaded: 20 * 1024 * 1024,
      total: 20 * 1024 * 1024,
      percent: 100,
      message: '正在替换应用…',
    });
  });
  await expect(page.locator('[data-update-status]')).toContainText('正在替换应用');

  await page.evaluate(() => window.finishInstall());
  expect(await page.evaluate(() => window.installCalls)).toBe(2);
});

test('failed checks are retryable without losing todo input', async ({ page }) => {
  await boot(page, { fail: true });
  await page.locator('#main-input').fill('未提交的草稿');
  await page.locator('[data-act="settings"]').click();
  await expect(page.locator('[data-installed-version]')).toHaveText('0.4.0-rc.5');
  await page.locator('[data-update-check]').click();
  await expect(page.locator('[data-update-status]')).toContainText('检查失败');
  await expect(page.locator('[data-installed-version]')).toHaveText('0.4.0-rc.5');
  await page.evaluate(() => { window.updateFail = false; });
  await page.locator('[data-update-check]').click();
  await expect(page.locator('[data-update-status]')).toContainText('发现新版本');
  await page.locator('[data-s="close"]').click();
  await expect(page.locator('#main-input')).toHaveValue('未提交的草稿');
});

test('checker coalesces requests and repeats after six hours', async ({ page }) => {
  await page.goto('/');
  await page.clock.install();
  await page.evaluate(async () => {
    const { createUpdateChecker } = await import('/update-ui.js');
    window.checks = 0;
    window.checker = createUpdateChecker({
      invoke: () => { window.checks++; return new Promise(resolve => { window.finishUpdate = resolve; }); },
      openExternal: () => {}, notice: () => {},
    });
    window.checker.start();
  });
  await page.clock.fastForward(5000);
  await page.evaluate(() => { window.checker.check(); window.checker.check(); });
  expect(await page.evaluate(() => window.checks)).toBe(1);
  await page.evaluate(async () => {
    window.finishUpdate({ currentVersion: '0.4.0', version: null, canInstall: false });
    await window.checker.check();
  });
  await page.clock.fastForward(6 * 60 * 60 * 1000);
  expect(await page.evaluate(() => window.checks)).toBe(2);
  await page.evaluate(() => window.checker.stop());
});
