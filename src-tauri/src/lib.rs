use std::sync::Arc;
use tauri::Manager;
use tokio::runtime::Runtime;

mod core;
mod ipc;
mod network;
mod storage;
mod transfer;

use core::*;
use ipc::*;
use network::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Create Tokio runtime
    let rt = Arc::new(Runtime::new().expect("Failed to create Tokio runtime"));

    let rt_clone = rt.clone();
    let rt_for_cleanup = rt.clone();

    tauri::Builder::default()
        // Plugins
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(rt.handle().clone())
        .invoke_handler(tauri::generate_handler![
            get_device_list,
            get_transfer_tasks,
            send_files,
            cancel_transfer,
            pause_transfer,
            resume_transfer,
            accept_transfer_request,
            reject_transfer_request,
            get_settings,
            save_settings,
            discover_device_by_ip,
            get_transfer_history,
            clear_transfer_history,
            get_known_devices,
            delete_known_device,
            get_system_info,
            set_auto_launch,
            get_auto_launch,
            open_file_location,
            resend_files,
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

            // Load settings before creating the shared state so the configured
            // device name is the one actually announced on the network.
            let settings = storage::get_storage()
                .and_then(|storage| storage.load_settings().ok().flatten())
                .unwrap_or_default();

            let app_state = Arc::new(AppState::new(settings.device_name.clone()));
            app.manage(app_state.clone());

            let handle = app.handle().clone();
            let rt_handle = rt_clone.handle().clone();
            app_state.set_app_handle(handle.clone());

            // Start QUIC first: discovery announces the server certificate
            // fingerprint, so the listener must exist before broadcasting.
            let state = app_state.clone();
            if let Err(e) = rt_handle.block_on(async { start_quic_server(state).await }) {
                log::error!("QUIC service failed to start: {}", e);
            }

            // Start discovery service
            let state = app_state.clone();
            let handle_clone = handle.clone();
            rt_handle.spawn(async move {
                if let Err(e) = start_discovery_service(state, handle_clone).await {
                    log::error!("Discovery service failed to start: {}", e);
                }
            });

            // Background task maintenance
            let state = app_state.clone();
            rt_handle.spawn(async move {
                transfer::start_task_maintenance(state).await;
            });

            Ok(())
        })
        .on_window_event(move |_window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                log::info!("Window closing, stopping services...");
                // Stop services gracefully
                rt_for_cleanup.block_on(async {
                    network::stop_discovery_service().await.ok();
                });
                network::stop_quic_server();
                network::quic_client::close_quic_client();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
