# 移动端（Android / iOS）架构与开发文档

本文档基于 `floating-todo` 现有架构（**Tauri v2 + 原生 ES Module 前端 + Rust 后端**），定义在沿用当前技术栈的前提下，实现 Android 与 iOS 移动端的完整技术方案、交互映射规则、数据同步互通机制及系统桌面小部件（Home Screen Widget）的开发规范。

---

## 1. 产品形态与交互映射设计

移动端不复刻桌面端的“跨应用常驻透明窗口”与“应用内折叠迷你栏”，而是将桌面端的两种形态自然映射为移动操作系统的原生标准形态：

| 桌面端（macOS / Windows）形态 | 移动端（Android / iOS）等价形态 | 职责与交互说明 |
| :--- | :--- | :--- |
| **展开主面板（`panel` 模式）** | **App 启动后的全屏主页面** | 开启 App 后直接展示完整的「今天 / 明天 / 后天」三日视图、「每日任务（打卡+连续天数）」、分类过滤（`+项目 #标签`）、底部备忘录及同步设置。 |
| **悬浮 Mini 栏（`mini` / `stack` / `tasks` / `quick`）** | **系统桌面小部件（Home Screen Widget）** | 作为手机主屏幕组件常驻，提供待办计数、近期任务展示、直接勾选完成，以及点击「+」一键唤起 App 并聚焦快速输入框。 |
| **全局快捷键（`Cmd/Ctrl+Shift+Space`）** | **小部件「+」按钮 / App 图标长按快捷入口** | 通过 Deep Link（`floatingtodo://quick-capture`）拉起主应用并自动弹出软键盘、聚焦顶部快速输入框。 |
| **窗口顶栏控制（置顶 / 鼠标穿透 / 固定桌面 / 左下角拖拽缩放）** | **移除 / 隐藏** | 移动端无多窗口叠加穿透与自由像素缩放概念，主界面适配系统状态栏与底部安全区域（Safe Area）。 |

### 1.1 操作逻辑一致性保障
由于沿用现有 Tauri v2 前端代码（`src/*.js`），以下核心操作体验在移动端与桌面端保持 **100% 一致**：
1. **自然语言快速捕获**：支持在顶部输入框输入 `明天 下午3点半 提交周报 +工作 #核心`，自动解析日期分组、24 小时制提醒时间（`15:30`）、创建或关联项目与标签。
2. **符号自动补全**：输入 `+` 或 `#` 时实时弹出项目/标签候选列表与新建着色提示（复用 [src/classification.js](file:///Users/sk/Documents/floating-todo/src/classification.js) 与 [src/classification-ui.js](file:///Users/sk/Documents/floating-todo/src/classification-ui.js)）。
3. **跨天滚动与打卡连续天数**：基于固定同步空间时区（默认 `Asia/Shanghai`）计算跨天滚转，过期未完成任务自动归集至「今天」，每日任务独立记录 `taskId/YYYY-MM-DD` 完成状态并计算 `streak`（复用 [src/sync-model.js](file:///Users/sk/Documents/floating-todo/src/sync-model.js)）。
4. **手动拖拽排序**：移动端通过长按任务触发 Pointer / Touch 拖拽排序，复用 [src/todo-order.js](file:///Users/sk/Documents/floating-todo/src/todo-order.js) 与 `order` 实体浮点位置算法。
5. **三方同步冲突解决**：多端并发修改同一任务时保留分支，在「处理同步冲突」弹窗中提供保留、替换、编辑或另存为新任务（复用 [src/sync-ui.js](file:///Users/sk/Documents/floating-todo/src/sync-ui.js)）。

---

## 2. 总体技术架构

在单一代码库（Monorepo）下，通过 Tauri v2 构建桌面端与移动端主应用，并通过共享的 Rust 核心层（`todo-v3.sqlite3`）支撑原生桌面小部件：

```
┌─────────────────────────────────────────────────────────────────────────┐
│                       前端层 (src/ 原生 ES Modules)                      │
│  main.js / sync-model.js / sync-store.js / classification.js / lan-ui.js │
│  - 桌面端：启用 window-controller.js (自由缩放 + Mini 栏模式切换)          │
│  - 移动端：锁定全屏 Panel 视图 + Safe Area 适配 + Deep Link 聚焦输入        │
└───────────────────────────────────┬─────────────────────────────────────┘
                                    │ Tauri IPC (invoke / event)
┌───────────────────────────────────▼─────────────────────────────────────┐
│                     Rust 核心与桥接层 (src-tauri / crates)               │
│  - sync_storage.rs : SQLite 事务存储 + WebDAV 客户端 (reqwest + rustls)  │
│  - crates/lan-sync : 局域网/Tailscale 点对点加密同步 (SPAKE2 + AES-GCM)   │
│  - widget_core.rs  : (新增) Rust 视图投影与快捷打卡写入，供原生小部件调用   │
│  - 凭据适配层       : Desktop(keyring) / iOS(Keychain) / Android(Keystore)│
└─────────────────┬─────────────────────────────────────┬─────────────────┘
                  │ 共享 SQLite 数据库 (WAL 模式)          │
                  ▼                                     ▼
┌───────────────────────────────────┐ ┌───────────────────────────────────┐
│   存储文件 (App Group / Data Dir)  │ │     原生系统桌面小部件 (Widgets)    │
│  - todo-v3.sqlite3 (版本图+队列)   │◄┤  - iOS: WidgetKit + AppIntents    │
│  - lan-vault.sqlite3 (配对加密库)  │ │  - Android: AppWidget (Glance)    │
└───────────────────────────────────┘ └───────────────────────────────────┘
```

---

## 3. 现有代码改造清单（Tauri v2 移动端适配）

### 3.1 Rust 后端条件编译与权限拆分（`src-tauri`）

1. **隔离桌面专属托盘与窗口命令**（[src-tauri/src/lib.rs](file:///Users/sk/Documents/floating-todo/src-tauri/src/lib.rs)）
   - `Cargo.toml` 中将 `tray-icon` 特性移至桌面端条件依赖，或在 `lib.rs` 中将 `Menu`、`MenuItem`、`TrayIconBuilder` 以及 `toggle_window` 全部包裹在 `#[cfg(desktop)]` 块内。
   - `set_desktop_mode`、`set_window_screen_position`、`set_window_surface` 在移动端保持空操作返回 `Ok(())` / `Ok(false)`。
2. **拆分 Capabilities 权限配置**（[src-tauri/capabilities/](file:///Users/sk/Documents/floating-todo/src-tauri/capabilities/default.json)）
   - 在 `src-tauri/capabilities/default.json` 中移除桌面专属权限，保留 `core:default`、`core:event:default`、`opener:default`、`notification:default`、`dialog:default`。
   - 新建 `src-tauri/capabilities/desktop.json`，设置 `"platforms": ["macOS", "windows", "linux"]`，放置 `global-shortcut:*`、`autostart:*`、`core:window:allow-start-dragging`、`core:window:allow-set-ignore-cursor-events` 等桌面窗口权限。

### 3.2 跨平台安全凭据存储改造（WebDAV 密码与 LAN 配对密钥）

当前 [sync_storage.rs](file:///Users/sk/Documents/floating-todo/src-tauri/src/sync_storage.rs#L146-L160) 与 [lan.rs](file:///Users/sk/Documents/floating-todo/src-tauri/src/lan.rs#L84-L93) 使用 `keyring` crate（仅开启了 `apple-native` 和 `windows-native`）。需封装统一的 `credential_store` 模块：

- **macOS / Windows**：继续使用 `keyring::Entry`。
- **iOS**：使用 `keyring` 的 `apple-native`（基于 iOS Keychain Services），并配置 Keychain Access Group，以便 Widget Extension 在后台需要触发 WebDAV 同步时也能安全读取凭据。
- **Android**：通过 JNI 或 Tauri Android Plugin 调用 `EncryptedSharedPreferences`（基于 Android Keystore 硬件密钥包装），按 `(service, account)` 存取 WebDAV 密码与 `lan-vault.sqlite3` 的 AES-256-GCM 包装主密钥。

### 3.3 数据库路径与 App Group 共享（支撑小部件直读直写）

在 [src-tauri/src/sync_storage.rs](file:///Users/sk/Documents/floating-todo/src-tauri/src/sync_storage.rs#L10-L18) 的 `connection(app: &AppHandle)` 中：
- **Android**：`app.path().app_data_dir()` 位于应用私有沙盒目录 `/data/data/com.gaopengfei.floating_todo/files/`，同一 APK 内的 `AppWidgetProvider` 与主 Activity 天生共享该路径，无需迁移路径。
- **iOS**：主 App 与 Widget Extension 处于不同沙盒路径，必须通过 **App Group**（`group.com.gaopengfei.floating-todo`）共享数据目录：
  - 使用 `NSFileManager.defaultManager().containerURLForSecurityApplicationGroupIdentifier("group.com.gaopengfei.floating-todo")` 获取共享根目录存放 `todo-v3.sqlite3`。
  - SQLite 已启用 `PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;` 和 `busy_timeout(5s)`，且 `save_document` 内置了 `expected_generation` 乐观锁检查（[sync_storage.rs:79](file:///Users/sk/Documents/floating-todo/src-tauri/src/sync_storage.rs#L79)），可安全防止主应用与桌面小部件并发写入覆盖。

### 3.4 导入与导出备份改造

[src-tauri/src/lib.rs](file:///Users/sk/Documents/floating-todo/src-tauri/src/lib.rs#L30-L64) 中的 `export_data` / `import_data` 当前使用了 `blocking_save_file()` 和 `PathBuf`：
- **移动端导出**：生成 `floating-todo-backup.json` 写入 `app_cache_dir()` 后，调用系统原生分享面板（iOS `UIActivityViewController` / Android `ACTION_SEND` Intent）或移动端异步文件保存对话框。
- **移动端导入**：在移动端使用异步 `pick_file()`，并通过 `tauri-plugin-fs` 读取 Android `content://` URI 或 iOS Security-Scoped URL 内容返回给前端。

### 3.5 前端移动端视图适配（`src/`）

1. **禁用窗口折叠控制器**：
   - 在 [src/main.js](file:///Users/sk/Documents/floating-todo/src/main.js) 中检测运行环境（如 `const isMobile = /Android|iPhone|iPad|iPod/i.test(navigator.userAgent)`）。
   - 移动端始终锁定 `windowController.mode = 'panel'`，不加载 `attachResize` 左下角缩放手柄，隐藏顶部「收起为迷你栏」「鼠标穿透」「置顶」「固定在桌面」按钮，同时在「设置」弹窗中隐藏桌面专属外观项（窗口透明度、鼠标穿透、开机自启、检查桌面安装包更新）。
2. **移动端全屏与安全区域 CSS**（[src/styles.css](file:///Users/sk/Documents/floating-todo/src/styles.css)）：
   - 桌面端为了渲染自定义窗口阴影，在四周留有 `12px` 透明边距（Gutter）和固定圆角。
   - 移动端通过 `html.platform-mobile` 类名移除四周透明阴影边距，让主容器铺满 `100vw × 100dvh`，并适配刘海屏与底部横条：
     ```css
     html.platform-mobile .app-shell {
       width: 100vw;
       height: 100dvh;
       border-radius: 0;
       padding-top: env(safe-area-inset-top);
       padding-bottom: env(safe-area-inset-bottom);
     }
     ```
3. **软键盘弹出与触摸拖拽体验**：
   - 监听 `visualViewport` 尺寸变化，确保编辑底部备忘录或底部任务时输入框不被软键盘遮挡。
   - 增大移动端任务复选框与操作按钮的热区（最小 `44×44px`），避免误触。

---

## 4. 数据同步与跨端互通方案

移动端与桌面端共用同一套 **v3 文档结构与不可变 Revision DAG 协议**（[docs/webdav-sync.md](file:///Users/sk/Documents/floating-todo/docs/webdav-sync.md)、[docs/lan-sync.md](file:///Users/sk/Documents/floating-todo/docs/lan-sync.md)），无需架设任何中心化后端即可实现多端互通。

### 4.1 WebDAV 同步（跨网/异地异步互通）
- **完全复用**：直接使用 [src-tauri/src/sync_storage.rs](file:///Users/sk/Documents/floating-todo/src-tauri/src/sync_storage.rs) 的 `dav_request` 与 [src/sync-store.js](file:///Users/sk/Documents/floating-todo/src/sync-store.js) 的 `ensureSpace -> pull -> push -> pull` 流程。
- **触发时机**：
  1. App 启动或从后台切回前台（监听 `visibilitychange` / `focus` 事件）时立即拉取；
  2. 本地新增/修改/打卡后延迟 `2.5s` 自动上传；
  3. 用户在移动端下拉列表或点击设置中的「立即同步」。

### 4.2 局域网与 Tailscale 同步（近场/虚拟局域网秒级互通）
直接复用 `crates/lan-sync` 原生库（TCP `47831` + mDNS `_floatingtodo._tcp.local.` + SPAKE2 短码配对 + AES-256-GCM 传输），针对移动端操作系统做如下适配：

1. **系统权限声明**：
   - **iOS (`Info.plist`)**：
     - `NSLocalNetworkUsageDescription`：说明用于发现并同步同一局域网下的桌面端待办数据。
     - `NSBonjourServices`：包含 `_floatingtodo._tcp`。
   - **Android (`AndroidManifest.xml`)**：
     - `android.permission.INTERNET`
     - `android.permission.ACCESS_WIFI_STATE`
     - `android.permission.CHANGE_WIFI_MULTICAST_STATE`（在 `lan_start` 时通过 JNI 获取 `WifiManager.MulticastLock`，`lan_stop` 时释放，否则 Android 默认过滤 UDP 5353 多播包）。
2. **默认设备名称获取**（[src-tauri/src/lan.rs:31](file:///Users/sk/Documents/floating-todo/src-tauri/src/lan.rs#L31)）：
   - 补充 `#[cfg(target_os = "ios")]` 与 `#[cfg(target_os = "android")]` 分支，读取手机设备型号/名称（如 `iPhone` / `Pixel`）作为局域网广播的默认名称。
3. **Tailscale 互联机制**：
   - 移动端沙盒内无 `tailscale` CLI 可执行文件，[crates/lan-sync/src/lib.rs:647](file:///Users/sk/Documents/floating-todo/crates/lan-sync/src/lib.rs#L647) 的 `tailscale()` CLI 探测会自然跳过。
   - **发现路径**：当手机与电脑同时开启 Tailscale 客户端时，**桌面端**可通过自身的 Tailscale CLI 自动发现手机的 `100.64.x.x:47831` 节点并显示在桌面端候选列表中；或者在手机端手动输入一次电脑的 `http://100.64.x.x:47831` 完成首次配对，后续配对关系及地址持久化在 `lan-vault.sqlite3` 中，无需再次输入。
4. **前后台生命周期管理**：
   - 移动操作系统会冻结后台进程的 Socket 监听。因此前端监听 `document.addEventListener('visibilitychange', ...)`：
     - **进入前台（`visible`）**：若 `doc.lan.enabled` 为真，确保 `lan_start` 已启动并立即调用 `lan_poll()` 向所有已配对桌面端拉取最新变更及发送通知。
     - **退入后台（`hidden`）**：等待当前正在进行的同步批次（最多 4 秒）完成后调用 `lan_stop()` 释放端口与 `MulticastLock`，避免后台耗电或被系统强杀导致 Socket 状态异常。

---

## 5. 系统桌面小部件（Home Screen Widget）技术方案

桌面端的悬浮 Mini 栏在移动端转化为**系统桌面小部件**。由于 Android 和 iOS 的桌面小部件运行在系统桌面进程/独立 Extension 进程中，**不能加载 Tauri WebView**，必须使用原生 UI 组件（iOS SwiftUI WidgetKit / Android Jetpack Glance 或 RemoteViews）+ **Rust 共享数据层**实现。

### 5.1 小部件尺寸与桌面 Mini 栏对应关系

| 小部件规格 | 对应桌面端 Mini 模式 | 展示内容 | 交互行为 |
| :--- | :--- | :--- | :--- |
| **小号（Small / 2×2）** | `mini`（迷你状态条） | 今日待办剩余数量 / 已完成进度环 + 顶部第一条待办标题 + 「+」按钮 | 点击「+」唤起 App 快速输入；点击卡片主体打开 App 今日视图。 |
| **中号（Medium / 4×2）** | `stack`（近期任务堆叠） | 展示优先度最高的前 3 条未完成待办（含时间标签、项目色点）+ 「+」按钮 | **直接点击任务左侧复选框在桌面原地完成打卡**；点击「+」唤起快速输入。 |
| **大号（Large / 4×4）** | `tasks`（完整列表预览） | 上半区：今日待办列表（前 5~6 条）；下半区：每日打卡任务及当前连续天数（`🔥 N天`） | 支持在桌面直接勾选完成普通待办与每日打卡；顶部提供「+ 新建」与「🔄 同步」按钮。 |

### 5.2 下沉 Rust 轻量数据投影与写入模块（`widget_core`）

目前版本图投影（`projectView`）和版本创建（`writeRevision`）位于前端 [src/sync-model.js](file:///Users/sk/Documents/floating-todo/src/sync-model.js)。为了让原生小部件在**不启动 WebView** 的情况下也能直接读取数据库并原地勾选完成任务，在 Rust 端新增 `widget_core` 模块（约 250 行 Rust 代码），暴露两个 C-ABI / JNI 接口供 Swift 与 Kotlin 调用：

1. `widget_load_snapshot(db_path: &str, today: &str) -> String`
   - 读取 `todo-v3.sqlite3` 中的 `document.json`。
   - 按照与 [src/sync-model.js](file:///Users/sk/Documents/floating-todo/src/sync-model.js) 完全相同的拓扑排序与 `compareRevision` 规则，投影出今日未完成/已完成任务列表、每日打卡列表及统计计数，返回精简 JSON 供原生小部件渲染。
2. `widget_toggle_task(db_path: &str, task_id: &str, today: &str) -> Result<(), String>`
   - 开启 SQLite `Immediate` 事务，校验并递增 `localGeneration`。
   - 若目标为 `one_off` 任务，生成一条 `completed: !prev` 的新 `task` Revision；若目标为 `daily` 任务，生成一条 `entityId = "{taskId}/{today}"` 的 `completion` Revision。
   - 将新 Revision 追加至 `document`、`revisions` 表和 `outbox` 待上传队列，确保下次打开 App 或后台同步时，小部件上的打卡记录能作为标准版本同步到桌面端。

### 5.3 主应用与小部件的双向刷新机制

- **主应用修改数据 ➔ 刷新小部件**：
  在 [src-tauri/src/sync_storage.rs](file:///Users/sk/Documents/floating-todo/src-tauri/src/sync_storage.rs#L40) 的 `storage_save` 提交成功后，除了调用 `crate::lan::publish(&app, &json)`，同时调用平台刷新钩子：
  - **iOS**：通过 `objc2` 或 Swift 桥接调用 `WidgetCenter.shared.reloadAllTimelines()`。
  - **Android**：通过 JNI 发送 `AppWidgetManager.ACTION_APPWIDGET_UPDATE` 广播刷新所有桌面小部件。
- **小部件原地勾选 ➔ 通知主应用 & 触发同步**：
  - 用户在桌面小部件点击复选框后，原生小部件调用 `widget_toggle_task` 写入 SQLite（`localGeneration + 1`）。
  - 若此时主应用正处于后台存活状态，当用户切回主应用触发 `focus` / `visibilitychange` 时，前端调用 `syncStore.reloadFromDisk()` 重新加载最新的 `todo-v3.sqlite3` 并刷新界面，同时触发 WebDAV / 局域网同步将小部件产生的 `outbox` 版本推送给桌面端。

---

## 6. 唤醒协议（Deep Link）与本地通知提醒

### 6.1 小部件快速新建（Quick Capture Deep Link）
1. 注册自定义 URL Scheme：`floatingtodo://`（使用 `tauri-plugin-deep-link`）。
2. 定义两条路由：
   - `floatingtodo://open?day=today`：点击小部件空白处，打开 App 并切换到「今天」标签页。
   - `floatingtodo://quick-capture`：点击小部件右上角「+」按钮，打开 App 后直接触发与桌面端全局快捷键相同的 `quick-capture` 事件（聚焦顶部快速输入框并弹出系统软键盘）。

### 6.2 移动端定时提醒通知
- 桌面端依赖进程常驻和前端定时器触发 `tauri-plugin-notification`。
- 移动端进程退后台后 JS 定时器会暂停，因此在每次 `storage_save` 时，由原生层根据今日与明日带有 `time`（如 `15:30`）且未完成的任务，向系统注册**精确本地定时通知**（iOS `UNCalendarNotificationTrigger` / Android `AlarmManager` 或 `WorkManager`），确保锁屏或杀后台状态下依然准时提醒。

---

## 7. 目录结构规划

改造完成后，项目在保持现有桌面端结构不变的基础上，增量扩展如下目录：

```text
floating-todo/
├── crates/
│   └── lan-sync/                 # [复用] 局域网/Tailscale 同步核心库
├── src/                          # [复用+适配] 前端代码，增加移动端全屏布局与 visibility 生命周期
├── src-tauri/
│   ├── capabilities/
│   │   ├── default.json          # [改造] 跨平台通用权限
│   │   └── desktop.json          # [新增] 桌面专属权限 (global-shortcut, autostart, window drag)
│   ├── gen/
│   │   ├── android/              # [tauri android init 生成] Android 工程与 AppWidget Kotlin 代码
│   │   └── apple/                # [tauri ios init 生成] Xcode 工程与 WidgetKit Swift Extension
│   └── src/
│       ├── lib.rs                # [改造] 托盘与窗口命令增加 #[cfg(desktop)] 隔离
│       ├── sync_storage.rs       # [改造] 支持 iOS App Group 路径与跨平台凭据存储
│       ├── credential_store.rs   # [新增] 统一封装 Desktop Keyring / iOS Keychain / Android Keystore
│       ├── widget_core.rs        # [新增] 供 iOS/Android 桌面小部件直读直写 SQLite 的 Rust 核心逻辑
│       └── lan.rs                # [改造] 补充移动端默认设备名与 Android MulticastLock 钩子
```

---

## 8. 分阶段开发与验收计划

### 阶段一：核心层解耦与移动端编译打通（Milestone 1）
- [ ] 在 `src-tauri/src/lib.rs` 与 `Cargo.toml` 中使用 `#[cfg(desktop)]` 隔离系统托盘（`TrayIconBuilder`）和桌面专有插件。
- [ ] 拆分 `capabilities/default.json` 与 `capabilities/desktop.json`。
- [ ] 实现 `credential_store.rs`，解决 Android 下无 `keyring` 后端的问题。
- [ ] 运行 `npx tauri android init` 与 `npx tauri ios init`，完成双端基础安装包编译并在模拟器启动。

### 阶段二：主界面移动端适配与双通道同步验收（Milestone 2）
- [ ] 前端识别移动端环境：固定为 `panel` 全屏展开模式，隐藏桌面窗口控制项（穿透、置顶、缩放手柄、Mini 栏切换按钮）。
- [ ] 适配移动端 CSS 安全区域（`safe-area-inset`）、触摸长按排序与软键盘弹出视口自适应。
- [ ] 适配移动端 JSON 备份导入/导出，以及前后台切换时的 WebDAV 自动拉取与局域网同步生命周期（含 Android `MulticastLock` 与 iOS 本地网络权限配置）。
- [ ] **验收标准**：Android / iOS 真机与 macOS / Windows 桌面端通过 WebDAV 及局域网 6 位/8 位短码配对成功，双向增删改任务、每日打卡、备忘录与分类标签秒级互通，三方冲突解决正常。

### 阶段三：Rust 小部件核心与 Android / iOS 桌面小部件开发（Milestone 3）
- [ ] 实现 `src-tauri/src/widget_core.rs`（Rust 版 `projectView` 快照投影与 `widget_toggle_task` 原子打卡写入），编写单元测试与 JS 版 `sync-model.js` 输出对齐。
- [ ] 开发 **iOS WidgetKit Extension**（配置 App Group 共享 `todo-v3.sqlite3`，实现小/中/大三种尺寸及 iOS 17+ `AppIntent` 原地勾选交互）。
- [ ] 开发 **Android AppWidget**（实现列表渲染、`PendingIntent` 原地勾选交互与尺寸自适应）。
- [ ] 接入 `floatingtodo://quick-capture` Deep Link 与移动端系统定时通知。
- [ ] **验收标准**：在手机桌面小部件上直接勾选待办或每日打卡，打开 App 状态即时一致，且自动通过 WebDAV / 局域网同步至电脑桌面悬浮窗。
