use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::Manager;
use tokio::runtime::Runtime;

mod core;
mod network;
mod transfer;
mod storage;
mod ipc;

use core::*;
use network::*;
use ipc::*;

/// 是否正在退出（托盘「退出」菜单触发）。为 true 时窗口关闭才会真正停止服务并退出进程。
static SHOULD_QUIT: AtomicBool = AtomicBool::new(false);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Create Tokio runtime
    let rt = Arc::new(Runtime::new().expect("Failed to create Tokio runtime"));
    let rt_for_cleanup = rt.clone();

    tauri::Builder::default()
        // Plugins
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_transfer_tasks,
            open_transfer_file,
            reveal_transfer_file,
            send_files,
            cancel_transfer,
            pause_transfer,
            resume_transfer,
            get_settings,
            save_settings,
            discover_device_by_ip,
            get_transfer_history,
            clear_transfer_history,
            get_known_devices,
            get_system_info,
            check_download_path,
            set_auto_launch,
            get_auto_launch,
            get_local_device_info,
        ])
        .setup(move |app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Debug)
                        .build(),
                )?;
            }

            // Initialize storage（支持用 CHUANSHU_DATA_DIR 覆盖，便于同机双开测试）
            let app_data_dir = match std::env::var_os("CHUANSHU_DATA_DIR") {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_data_dir()?,
            };
            if let Err(e) = storage::init_storage(&app_data_dir) {
                log::error!("Failed to initialize storage: {}", e);
            } else {
                log::info!("Storage initialized at {:?}", app_data_dir);
            }

            // 读取设置与设备身份
            let settings = storage::get_storage()
                .and_then(|s| s.load_settings().ok().flatten());
            let device_name = settings
                .as_ref()
                .map(|s| s.device_name.clone())
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(default_device_name);
            let device_id = storage::get_storage()
                .and_then(|s| s.get_or_create_device_id().ok())
                .unwrap_or_else(uuid::Uuid::new_v4);

            let state = Arc::new(AppState::new(device_id, device_name));
            state.set_app_handle(app.handle().clone());
            app.manage(state.clone());

            let handle = app.handle().clone();
            let rt_handle = rt.handle().clone();

            // 启动时探测接收目录写权限（macOS 首次访问「下载」目录会触发系统授权弹窗）
            {
                let download_path = settings
                    .as_ref()
                    .map(|s| s.download_path.clone())
                    .unwrap_or_default();
                rt_handle.spawn_blocking(move || {
                    let dir = if download_path.trim().is_empty() {
                        default_download_dir()
                    } else {
                        resolve_download_dir(&download_path)
                    };
                    match ensure_dir_writable(&dir) {
                        Ok(()) => log::info!("下载目录可写: {:?}", dir),
                        Err(e) => log::warn!("下载目录不可写 {:?}: {}", dir, e),
                    }
                });
            }

            // Start discovery service
            {
                let state = state.clone();
                let handle = handle.clone();
                rt_handle.spawn(async move {
                    if let Err(e) = start_discovery_service(state, handle).await {
                        log::error!("Discovery service failed to start: {}", e);
                    }
                });
            }

            // Start QUIC service
            {
                let state = state.clone();
                rt_handle.spawn(async move {
                    if let Err(e) = start_quic_server(state).await {
                        log::error!("QUIC service failed to start: {}", e);
                    }
                });
            }

            // Start progress updater
            {
                let state = state.clone();
                rt_handle.spawn(async move {
                    transfer::start_progress_updater(state).await;
                });
            }

            // 系统托盘：关闭窗口时最小化到托盘，通过托盘菜单显示主窗口或退出
            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::TrayIconBuilder;

                let rt_for_tray = rt.clone();
                let show_item = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
                let quit_item = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

                let icon = app
                    .default_window_icon()
                    .cloned()
                    .ok_or_else(|| AppError::Other("缺少应用图标".to_string()))?;

                let _tray = TrayIconBuilder::with_id("main-tray")
                    .icon(icon)
                    .tooltip("传书 - 局域网文件传输")
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(move |app, event| match event.id().as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "quit" => {
                            SHOULD_QUIT.store(true, Ordering::SeqCst);
                            network::stop_quic_server();
                            let _ = rt_for_tray.block_on(async {
                                network::stop_discovery_service().await
                            });
                            app.exit(0);
                        }
                        _ => {}
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let should_quit = SHOULD_QUIT.load(Ordering::SeqCst);

                // 读取「关闭窗口时最小化到托盘」设置，未设置时默认为 true
                let minimize_to_tray = storage::get_storage()
                    .and_then(|s| s.load_settings().ok().flatten())
                    .map(|s| s.minimize_to_tray)
                    .unwrap_or(true);

                if should_quit || !minimize_to_tray {
                    log::info!("Window closing, stopping services...");
                    rt_for_cleanup.block_on(async {
                        network::stop_discovery_service().await.ok();
                    });
                    network::stop_quic_server();
                } else {
                    api.prevent_close();
                    let _ = window.hide();
                    log::info!("Window hidden to system tray, services keep running");
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
