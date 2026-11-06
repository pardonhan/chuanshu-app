use std::path::PathBuf;
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
                .unwrap_or_else(|| "我的设备".to_string());
            let device_id = storage::get_storage()
                .and_then(|s| s.get_or_create_device_id().ok())
                .unwrap_or_else(uuid::Uuid::new_v4);

            let state = Arc::new(AppState::new(device_id, device_name));
            state.set_app_handle(app.handle().clone());
            app.manage(state.clone());

            let handle = app.handle().clone();
            let rt_handle = rt.handle().clone();

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

            Ok(())
        })
        .on_window_event(move |_window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                log::info!("Window closing, stopping services...");
                rt_for_cleanup.block_on(async {
                    network::stop_discovery_service().await.ok();
                });
                network::stop_quic_server();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
