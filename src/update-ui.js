export const RELEASES_URL = 'https://github.com/pisceskkk/floating-todo/releases';
const INTERVAL = 6 * 60 * 60 * 1000;

function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

function formatProgress(progress) {
  if (!progress) return '正在下载并安装更新…';
  if (progress.stage === 'downloading') {
    if (progress.total > 0) {
      const pct = progress.percent ?? Math.min(100, Math.floor((progress.downloaded * 100) / progress.total));
      return `${progress.message} ${pct}% (${formatBytes(progress.downloaded)} / ${formatBytes(progress.total)})`;
    }
    if (progress.downloaded > 0) {
      return `${progress.message} ${formatBytes(progress.downloaded)}`;
    }
  }
  return progress.message || '正在更新…';
}

export function createUpdateChecker({ invoke, listen, openExternal, notice }) {
  let result = null;
  let installedVersion = '';
  let versionRequest = null;
  let versionError = false;
  let pending = null;
  let installing = null;
  let progress = null;
  let error = '';
  let notifiedVersion = '';
  let timer;
  const panels = new Set();

  function refresh() {
    document.body.classList.toggle('update-available', !!result?.version);
    for (const panel of panels) {
      if (!panel.isConnected) { panels.delete(panel); continue; }
      panel.querySelector('[data-installed-version]').textContent = installedVersion || (
        !invoke ? '仅桌面应用可用' : versionError ? '读取失败，请重新打开设置' : '读取中…');
      panel.querySelector('[data-update-status]').textContent = installing
        ? formatProgress(progress)
        : pending
          ? '正在检查更新…'
          : error || (
            result?.version
              ? `发现新版本 ${result.version}`
              : result ? '暂无更新' : '启动时及每 6 小时自动检查');

      const progressWrap = panel.querySelector('[data-update-progress]');
      const progressBar = panel.querySelector('[data-update-progress-bar]');
      if (progressWrap && progressBar) {
        const showProgress = !!installing && !!progress;
        progressWrap.hidden = !showProgress;
        if (showProgress) {
          const isDeterminate = progress.stage === 'downloading' && progress.total > 0;
          const pct = isDeterminate
            ? (progress.percent ?? Math.min(100, Math.floor((progress.downloaded * 100) / progress.total)))
            : 100;
          progressBar.style.width = `${pct}%`;
          progressBar.classList.toggle('indeterminate', !isDeterminate && progress.stage !== 'restarting');
        } else {
          progressBar.style.width = '0%';
          progressBar.classList.remove('indeterminate');
        }
      }

      const installBtn = panel.querySelector('[data-update-install]');
      if (installBtn) {
        const canInstall = !!(result?.version && result?.canInstall);
        installBtn.hidden = !canInstall;
        installBtn.disabled = !!installing || !!pending || !invoke;
        installBtn.textContent = installing ? '正在更新…' : '立即更新';
      }
      panel.querySelector('[data-update-check]').disabled = !!pending || !!installing || !invoke;
      panel.querySelector('[data-update-open]').textContent =
        result?.version && !result?.canInstall ? '前往下载新版' : '查看 Releases';
    }
  }

  function check() {
    if (installing) return installing;
    if (pending) return pending;
    if (!invoke) { error = '请在桌面应用中检查更新'; refresh(); return Promise.resolve(); }
    error = '';
    pending = Promise.resolve().then(() => invoke('check_for_updates')).then(value => {
      if (!value || typeof value.currentVersion !== 'string') throw new Error('更新信息不可用');
      result = value;
      if (result.version && result.version !== notifiedVersion) {
        notifiedVersion = result.version;
        const actionHint = result.canInstall ? '一键更新' : '下载';
        notice(`发现新版本 ${result.version}，可在设置 → 应用更新中${actionHint}`);
      }
    }).catch(reason => {
      error = `检查失败：${String(reason?.message || reason)}`;
    }).finally(() => { pending = null; refresh(); });
    refresh();
    return pending;
  }

  function install() {
    if (installing) return installing;
    if (!invoke || !result?.version || !result?.canInstall) return Promise.resolve();
    error = '';
    progress = {
      stage: 'downloading',
      downloaded: 0,
      total: result.assetSize ?? null,
      percent: 0,
      message: `正在准备下载 ${result.version}…`,
    };
    let unlisten = null;
    installing = (async () => {
      if (typeof listen === 'function') {
        try {
          unlisten = await listen('update-progress', event => {
            if (event?.payload) {
              progress = event.payload;
              refresh();
            }
          });
        } catch {
          // Progress listener is best-effort; install can still proceed.
        }
      }
      try {
        await invoke('install_update');
      } catch (reason) {
        error = `更新失败：${String(reason?.message || reason)}`;
        progress = null;
      } finally {
        if (typeof unlisten === 'function') {
          try { unlisten(); } catch { /* ignore */ }
        }
        installing = null;
        refresh();
      }
    })();
    refresh();
    return installing;
  }

  function loadInstalledVersion() {
    if (!invoke || installedVersion || versionRequest) return;
    versionError = false;
    versionRequest = Promise.resolve().then(() => invoke('installed_version')).then(version => {
      if (typeof version !== 'string' || !version) throw new Error('版本信息不可用');
      installedVersion = version;
    }).catch(() => { versionError = true; }).finally(() => {
      versionRequest = null;
      refresh();
    });
  }

  function mount(panel) {
    panel.innerHTML = `<h3>应用更新</h3>
      <div class="set-row"><span>当前安装版本</span><span data-installed-version class="update-version"></span></div>
      <p data-update-status role="status" class="update-status"></p>
      <div class="update-progress" data-update-progress hidden><div class="update-progress-bar" data-update-progress-bar></div></div>
      <p class="sync-help">更新来源：pisceskkk/floating-todo Releases</p>
      <div class="set-row update-actions">
        <button class="data-btn update-install-btn" data-update-install hidden>立即更新</button>
        <button class="data-btn" data-update-check>检查更新</button>
        <button class="data-btn" data-update-open>查看 Releases</button>
      </div>`;
    panels.add(panel);
    loadInstalledVersion();
    panel.querySelector('[data-update-install]').onclick = install;
    panel.querySelector('[data-update-check]').onclick = check;
    panel.querySelector('[data-update-open]').onclick = () => openExternal(result?.url || RELEASES_URL);
    refresh();
  }

  return {
    mount, check, install,
    start() {
      if (!invoke || timer) return;
      // Give application startup and storage migration time to finish first.
      timer = setTimeout(() => { check(); timer = setInterval(check, INTERVAL); }, 5000);
    },
    stop() { clearTimeout(timer); clearInterval(timer); timer = null; },
  };
}
