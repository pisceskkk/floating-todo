import { openConflicts } from './sync-ui.js';
const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
export function openLanSettings(store, onBack = null) {
  const overlay = document.createElement('div'); overlay.className = 'overlay sync-overlay';
  const irohDefault = store.doc.lan?.irohEnabled !== false;
  overlay.innerHTML = `<div class="settings sync-settings">
    <div class="sheet-header">${onBack ? '<button type="button" class="icon-btn sm" data-lan-back aria-label="返回设置">‹</button>' : ''}<h2>局域网与跨网同步</h2><button type="button" class="icon-btn sm" data-lan-close aria-label="关闭">×</button></div>
    <p class="sync-help">支持局域网、Tailscale 与 Iroh 跨网直连/中继同步。两台设备均需开启同步并完成短码配对。</p>
    <label class="set-row"><span>开启局域网与跨网同步</span><input class="toggle-switch" type="checkbox" data-lan-enabled ${store.doc.lan?.enabled ? 'checked' : ''}></label>
    <label class="sync-field">设备名称<input class="lan-input" data-lan-name maxlength="40" value="${esc(store.doc.lan?.name === '我的待办' ? '' : store.doc.lan?.name || '')}" ${store.doc.lan?.enabled ? 'disabled' : ''}></label>
    <label class="set-row"><span>开启 Iroh 跨网同步（免 Tailscale）</span><input class="toggle-switch" type="checkbox" data-lan-iroh ${irohDefault ? 'checked' : ''}></label>
    <label class="sync-field">自定义中继节点（可选）<input class="lan-input" data-lan-relay placeholder="留空使用默认公共中继，或输入 https://relay.example.com" value="${esc(store.doc.lan?.relayUrl || '')}"></label>
    <button class="data-btn" data-lan-save>应用设置</button>
    <p class="sync-help" data-lan-status></p>
    <section class="settings-group"><h3>本机配对码与跨网地址</h3><p class="sync-help">仅向你要同步的设备提供。短码只用于建立配对，重置短码不会撤销已配对设备。</p>
      <p data-lan-codes>开启后显示</p>
      <p class="sync-help" data-lan-iroh-addr style="word-break:break-all"></p>
      <div class="sync-actions"><button class="link-btn" data-lan-copy-iroh style="display:none">复制跨网地址</button><button class="link-btn" data-lan-rotate>刷新临时码</button><button class="link-btn" data-lan-fixed>重置固定码</button></div></section>
    <section class="settings-group"><h3>发现的设备</h3><div data-lan-candidates></div>
      <label class="sync-field">设备地址<input class="lan-input" data-lan-url placeholder="http://192.168.1.10:47831 或 iroh://..."></label>
      <button class="link-btn" data-lan-discover>查找此地址</button>
      <label class="sync-field">对方短码<input class="lan-input" data-lan-code inputmode="numeric" autocomplete="off" maxlength="8" placeholder="输入对方的 6 位或 8 位短码"></label>
      <p class="sync-help">配对即同意合并双方全部数据；同一事项的冲突版本会保留。</p><button class="data-btn" data-lan-pair>配对并同步</button></section>
    <section class="settings-group"><h3>已配对设备</h3><div data-lan-peers></div></section>
    <div class="sync-actions"><button class="link-btn" data-lan-now>立即同步</button><button class="link-btn" data-lan-conflicts>处理同步冲突</button></div>
    <p class="sync-error" data-lan-message role="alert"></p></div>`;
  document.body.append(overlay);
  const message = text => { overlay.querySelector('[data-lan-message]').textContent = text; };
  const nameInput = overlay.querySelector('[data-lan-name]');
  let nameEdited = false;
  let lastIrohAddress = '';
  nameInput.addEventListener('input', () => { nameEdited = true; });
  if (!nameInput.value && store.native) store.lanDeviceName().then(name => {
    if (overlay.isConnected && !nameEdited) nameInput.value = name;
  });
  let busy = false, refreshing = false;
  async function refresh() {
    if (!overlay.isConnected || refreshing) return;
    refreshing = true;
    try {
      if (!store.native) { message('局域网同步仅在桌面应用中可用'); return; }
      const s = await store.invoke('lan_status');
      const irohInfo = s.iroh || {};
      const irohText = irohInfo.enabled
        ? ` · Iroh 跨网${irohInfo.relayUrl ? '（自建中继）' : '（公共中继）'}`
        : '';
      overlay.querySelector('[data-lan-status]').textContent = s.enabled ? `已开启 · TCP ${s.port}${irohText}${store.lanError ? ' · ' + store.lanError : ''}` : '已关闭';
      overlay.querySelector('[data-lan-name]').disabled = !!s.enabled;
      lastIrohAddress = (s.enabled && irohInfo.enabled && irohInfo.address) ? irohInfo.address : '';
      const irohAddrEl = overlay.querySelector('[data-lan-iroh-addr]');
      const copyIrohBtn = overlay.querySelector('[data-lan-copy-iroh]');
      if (lastIrohAddress) {
        const relayHint = irohInfo.relayUrl
          ? `自建中继：${irohInfo.relayUrl}`
          : (irohInfo.homeRelay ? `当前公共中继：${irohInfo.homeRelay}` : '正在连接公共中继…');
        irohAddrEl.textContent = `跨网地址：${lastIrohAddress}（${relayHint}）`;
        copyIrohBtn.style.display = '';
      } else {
        irohAddrEl.textContent = '';
        copyIrohBtn.style.display = 'none';
      }
      overlay.querySelector('[data-lan-candidates]').innerHTML = s.candidates?.map(c => `<button class="data-btn" data-lan-select="${esc(c.url)}">${esc(c.name)} · ${esc(c.url)}</button>`).join('') || '<p class="sync-help">暂未发现设备，可输入局域网 IP、Tailscale IP 或 iroh:// 跨网地址查找。</p>';
      overlay.querySelector('[data-lan-peers]').innerHTML = s.peers?.map(p => `<div class="lan-peer"><strong>${esc(p.name)}</strong><p class="sync-help">${esc(p.url)}<br>${esc(p.error || (p.lastSuccess ? '最近同步：' + new Date(p.lastSuccess).toLocaleTimeString() : '等待同步'))}</p><button class="link-btn" data-lan-revoke="${esc(p.id)}">撤销配对</button></div>`).join('') || '<p class="sync-help">暂无配对设备。</p>';
      if (s.enabled) {
        const c = await store.invoke('lan_codes');
        overlay.querySelector('[data-lan-codes]').textContent = `临时码 ${c.dynamic}（${Math.max(0, Math.ceil((c.expires - Date.now()) / 1000))} 秒） · 固定码 ${c.fixed}`;
      } else overlay.querySelector('[data-lan-codes]').textContent = '开启后显示';
    } catch (e) { message(String(e.message || e)); }
    finally { refreshing = false; }
  }
  const timer = setInterval(() => { if (!overlay.isConnected) clearInterval(timer); else if (!busy) refresh(); }, 1000);
  overlay.addEventListener('click', async e => {
    if (e.target === overlay || e.target.closest('[data-lan-close]')) { clearInterval(timer); overlay.remove(); return; }
    if (onBack && e.target.closest('[data-lan-back]')) { clearInterval(timer); overlay.remove(); onBack(); return; }
    const b = e.target.closest('button'); if (!b || busy) return;
    if (b.hasAttribute('data-lan-select')) { overlay.querySelector('[data-lan-url]').value = b.dataset.lanSelect; overlay.querySelector('[data-lan-code]').focus(); return; }
    if (b.hasAttribute('data-lan-copy-iroh')) {
      if (lastIrohAddress) {
        try { await navigator.clipboard.writeText(lastIrohAddress); message('已复制本机 Iroh 跨网地址'); }
        catch { message('复制失败，请手动选中复制'); }
      }
      return;
    }
    if (b.hasAttribute('data-lan-conflicts')) { clearInterval(timer); overlay.remove(); openConflicts(store); return; }
    busy = true; b.disabled = true; message('');
    try {
      if (b.hasAttribute('data-lan-save')) {
        await store.configureLan(
          overlay.querySelector('[data-lan-enabled]').checked,
          overlay.querySelector('[data-lan-name]').value.trim(),
          overlay.querySelector('[data-lan-iroh]').checked,
          overlay.querySelector('[data-lan-relay]').value.trim(),
        );
      }
      if (b.hasAttribute('data-lan-rotate') || b.hasAttribute('data-lan-fixed')) await store.invoke('lan_rotate', { fixed: b.hasAttribute('data-lan-fixed') });
      if (b.hasAttribute('data-lan-revoke')) await store.invoke('lan_revoke', { id: b.dataset.lanRevoke });
      if (b.hasAttribute('data-lan-discover')) await store.invoke('lan_discover', { url: overlay.querySelector('[data-lan-url]').value.trim() });
      if (b.hasAttribute('data-lan-pair')) {
        const code = overlay.querySelector('[data-lan-code]').value.trim();
        await store.invoke('lan_pair', { url: overlay.querySelector('[data-lan-url]').value.trim(), mode: code.length === 6 ? 'dynamic' : 'fixed', code });
        overlay.querySelector('[data-lan-code]').value = ''; await store.syncLan(); message('配对成功，已开始合并数据');
      }
      if (b.hasAttribute('data-lan-now')) { await store.syncLan(); if (store.lanError) throw new Error(store.lanError); }
      await refresh();
    } catch (e) { message(String(e.message || e)); }
    finally { b.disabled = false; busy = false; }
  });
  refresh();
}
