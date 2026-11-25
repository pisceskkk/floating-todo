#[cfg(desktop)]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter,
};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
mod credential_store;
mod lan;
mod sync_storage;
mod updates;
mod widget_core;

#[cfg(target_os = "macos")]
mod macos_surface;

#[cfg(target_os = "windows")]
mod windows_desktop;

/// 显示或隐藏悬浮窗。
#[cfg(desktop)]
fn toggle_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
    }
}

/// 导出数据到用户选择的文件（移动端写入应用缓存目录以便原生分享）。
#[tauri::command]
async fn export_data(app: AppHandle, json: String) -> Result<bool, String> {
    #[cfg(desktop)]
    {
        let file = app
            .dialog()
            .file()
            .add_filter("JSON", &["json"])
            .set_file_name("floating-todo-backup.json")
            .blocking_save_file();
        match file {
            Some(p) => {
                let path = p.into_path().map_err(|e| e.to_string())?;
                std::fs::write(path, json).map_err(|e| e.to_string())?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
    #[cfg(mobile)]
    {
        let dir = app.path().app_cache_dir().map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("floating-todo-backup.json");
        std::fs::write(&path, json).map_err(|e| e.to_string())?;
        Ok(true)
    }
}

/// 从用户选择的文件导入数据，返回文件内容。
#[tauri::command]
async fn import_data(app: AppHandle) -> Result<Option<String>, String> {
    #[cfg(desktop)]
    {
        let file = app
            .dialog()
            .file()
            .add_filter("JSON", &["json"])
            .blocking_pick_file();
        match file {
            Some(p) => {
                let path = p.into_path().map_err(|e| e.to_string())?;
                let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
                Ok(Some(content))
            }
            None => Ok(None),
        }
    }
    #[cfg(mobile)]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.dialog()
            .file()
            .add_filter("JSON", &["json"])
            .pick_file(move |file| {
                let _ = tx.send(file);
            });
        match rx.await.map_err(|e| e.to_string())? {
            Some(p) => {
                let path = p.into_path().map_err(|e| e.to_string())?;
                let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
                Ok(Some(content))
            }
            None => Ok(None),
        }
    }
}

/// 将窗口放入 Windows 桌面层，使其在 Win+D 显示桌面后仍然可见。
#[tauri::command]
fn set_desktop_mode(app: AppHandle, enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "main window not found".to_string())?;
        let hwnd = window.hwnd().map_err(|error| error.to_string())?;
        windows_desktop::set_desktop_mode(windows::Win32::Foundation::HWND(hwnd.0), enabled)
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, enabled);
        Ok(())
    }
}

#[tauri::command]
fn set_window_screen_position(window: tauri::WebviewWindow, x: i32, y: i32) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let hwnd = window.hwnd().map_err(|error| error.to_string())?;
        windows_desktop::set_screen_position(windows::Win32::Foundation::HWND(hwnd.0), x, y)
    }
    #[cfg(all(desktop, not(target_os = "windows")))]
    {
        window
            .set_position(tauri::PhysicalPosition::new(x, y))
            .map_err(|error| error.to_string())
    }
    #[cfg(not(desktop))]
    {
        let _ = (window, x, y);
        Ok(())
    }
}

/// 在 macOS 上切换原生毛玻璃材质（NSVisualEffectView）与精准内边距裁剪；其他平台为空操作。
#[tauri::command]
fn set_window_surface(
    window: tauri::WebviewWindow,
    vibrancy: bool,
    theme: String,
    radius: f64,
    inset: f64,
) -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    {
        macos_surface::apply_surface(&window, vibrancy, &theme, radius, inset)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, vibrancy, theme, radius, inset);
        Ok(false)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .manage(lan::LanState::default())
        .manage(updates::UpdateState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init());

    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_global_shortcut::Builder::new().build())
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                Some(vec![]),
            ));
    }

    builder
        .invoke_handler(tauri::generate_handler![
            lan::lan_device_name, lan::lan_start, lan::lan_stop, lan::lan_status, lan::lan_codes, lan::lan_rotate,
            lan::lan_revoke, lan::lan_discover, lan::lan_pair, lan::lan_pull, lan::lan_poll,
            sync_storage::storage_load,
            sync_storage::storage_save,
            sync_storage::sync_set_password,
            sync_storage::dav_request,
            updates::check_for_updates,
            updates::install_update,
            updates::installed_version,
            widget_core::widget_snapshot,
            widget_core::widget_toggle,
            export_data,
            import_data,
            set_desktop_mode,
            set_window_screen_position,
            set_window_surface
        ])
        .setup(|app| {
            #[cfg(not(desktop))]
            let _ = app;
            // macOS：作为状态栏小组件运行，不占用程序坞。
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            #[cfg(desktop)]
            {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.show();
                    let _ = win.set_focus();
                }

                // 全局快捷键：CmdOrCtrl+Shift+Space 唤起并聚焦输入框（快速捕获）。
                use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
                let handle = app.handle().clone();
                let _ = app.global_shortcut().on_shortcut(
                    "CommandOrControl+Shift+Space",
                    move |_app, _shortcut, event| {
                        if event.state == ShortcutState::Pressed {
                            if let Some(win) = handle.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.set_focus();
                                let _ = win.emit("quick-capture", ());
                            }
                        }
                    },
                );

                let toggle_item =
                    MenuItem::with_id(app, "toggle", "显示/隐藏小组件", true, None::<&str>)?;
                #[cfg(target_os = "windows")]
                let desktop_item =
                    MenuItem::with_id(app, "desktop", "切换固定在桌面", true, None::<&str>)?;
                let passthrough_item =
                    MenuItem::with_id(app, "passthrough", "切换鼠标穿透", true, None::<&str>)?;
                let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
                #[cfg(target_os = "windows")]
                let menu = Menu::with_items(
                    app,
                    &[&toggle_item, &desktop_item, &passthrough_item, &quit_item],
                )?;
                #[cfg(not(target_os = "windows"))]
                let menu = Menu::with_items(app, &[&toggle_item, &passthrough_item, &quit_item])?;

                let _tray = TrayIconBuilder::new()
                    .icon(app.default_window_icon().unwrap().clone())
                    .tooltip("悬浮待办")
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "toggle" => toggle_window(app),
                        "passthrough" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.emit("toggle-passthrough", ());
                            }
                        }
                        "desktop" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.emit("toggle-desktop-mode", ());
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            toggle_window(tray.app_handle());
                        }
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
