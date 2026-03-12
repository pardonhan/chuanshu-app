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

    // Initialize application state
    let app_state = Arc::new(AppState::new("我的设备".to_string()));

    let rt_clone = rt.clone();
    let rt_for_cleanup = rt.clone();

    tauri::Builder::default()
        // Plugins
        .plugin(tauri_plugin_fs::init())
        .manage(app_state.clone())
        .manage(rt.handle().clone())
        .invoke_handler(tauri::generate_handler![
            get_device_list,
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
        ])
        .setup(move |app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Debug)
                        .build(),
                )?;
            }

            // Initialize storage
            let app_data_dir = app.path().app_data_dir()?;
            if let Err(e) = storage::init_storage(&app_data_dir) {
                log::error!("Failed to initialize storage: {}", e);
            } else {
                log::info!("Storage initialized at {:?}", app_data_dir);
            }

            // Load settings
            let _device_name = if let Some(storage) = storage::get_storage() {
                storage.load_settings()?.map(|s| s.device_name).unwrap_or_else(|| "我的设备".to_string())
            } else {
                "我的设备".to_string()
            };

            let handle = app.handle().clone();
            let state = app_state.clone();
            let rt_handle = rt_clone.handle().clone();

            // Start discovery service
            rt_handle.spawn(async move {
                if let Err(e) = start_discovery_service(state, handle).await {
                    log::error!("Discovery service failed to start: {}", e);
                }
            });

            // Start QUIC service
            let state = app_state.clone();
            rt_handle.spawn(async move {
                if let Err(e) = start_quic_server(state).await {
                    log::error!("QUIC service failed to start: {}", e);
                }
            });

            // Start progress updater
            let state = app_state.clone();
            rt_handle.spawn(async move {
                transfer::start_progress_updater(state).await;
            });

            Ok(())
        })
        .on_window_event(move |_window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { .. } => {
                    log::info!("Window closing, stopping services...");
                    // Stop services gracefully
                    rt_for_cleanup.block_on(async {
                        network::stop_discovery_service().await.ok();
                    });
                    network::stop_quic_server();
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
