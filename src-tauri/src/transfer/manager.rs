use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tokio::time::interval;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::core::{AppError, AppResult, AppState};
use crate::ipc::SendFilesRequest;
use crate::network::protocol::{FileMeta, TransferRequest};
use crate::network::quic_client;
use crate::transfer::{TransferStatus, TransferTask, TransferTaskInfo, TransferType};

/// Transfer manager for handling file transfers
pub struct TransferManager {
    app_state: Arc<AppState>,
}

impl TransferManager {
    pub fn new(app_state: Arc<AppState>) -> Self {
        Self { app_state }
    }

    /// Send files to one or more devices（每台设备一个任务）
    pub async fn send_files(&self, request: SendFilesRequest) -> AppResult<Vec<Uuid>> {
        let files = Self::collect_files(&request.file_paths).await?;

        if files.is_empty() {
            return Err(AppError::Other("没有可传输的文件".to_string()));
        }

        let metas: Vec<FileMeta> = files.iter().map(|(_, meta)| meta.clone()).collect();
        let total_size: u64 = metas.iter().map(|f| f.file_size).sum();
        let file_count = metas.len() as u32;

        let mut task_ids = Vec::new();

        for device_id in request.device_ids {
            let device = match self.app_state.devices.get(&device_id) {
                Some(device) => device.value().clone(),
                None => {
                    log::warn!("Device {} not found, skip transfer", device_id);
                    continue;
                }
            };

            let task_id = Uuid::new_v4();
            let peer_addr = SocketAddr::new(device.ip_address, device.quic_port);
            let peer_name = device.device_name.clone();

            let transfer_request = TransferRequest::new(
                task_id,
                self.app_state.device_id,
                self.app_state.device_name(),
                total_size,
                metas.clone(),
            );

            let mut task = TransferTask::new(
                task_id,
                device_id,
                peer_name.clone(),
                TransferType::Send,
                total_size,
                file_count,
            );
            task.files = metas.clone();
            task.set_status(TransferStatus::Transferring);

            self.app_state
                .transfer_tasks
                .insert(task_id, Arc::new(Mutex::new(task)));

            task_ids.push(task_id);

            let app_state = self.app_state.clone();
            let files = files.clone();

            tokio::spawn(async move {
                let result =
                    quic_client::send_transfer(peer_addr, transfer_request, files, app_state.clone(), task_id)
                        .await;

                let Some(arc) = app_state
                    .transfer_tasks
                    .get(&task_id)
                    .map(|entry| entry.value().clone())
                else {
                    return;
                };

                let mut task = arc.lock().await;
                match result {
                    Ok(()) => {
                        task.set_status(TransferStatus::Completed);
                        record_history(&task, "completed", None);
                        drop(task);

                        if let Some(handle) = app_state.get_app_handle() {
                            let _ = crate::ipc::emit_transfer_completed(&handle, task_id, &peer_name);
                        }
                        log::info!("Transfer {} completed", task_id);
                    }
                    Err(e) => {
                        let message = e.to_string();
                        let canceled = task.status == TransferStatus::Canceled;

                        if canceled {
                            record_history(&task, "canceled", Some(&message));
                            drop(task);
                            if let Some(handle) = app_state.get_app_handle() {
                                let _ = crate::ipc::emit_transfer_failed(&handle, task_id, message);
                            }
                            log::info!("Transfer {} canceled", task_id);
                        } else {
                            task.set_error(message.clone());
                            record_history(&task, "failed", Some(&message));
                            drop(task);
                            if let Some(handle) = app_state.get_app_handle() {
                                let _ = crate::ipc::emit_transfer_failed(&handle, task_id, message);
                            }
                            log::error!("Transfer {} failed", task_id);
                        }
                    }
                }
            });
        }

        if task_ids.is_empty() {
            return Err(AppError::DeviceNotFound("目标设备不在线".to_string()));
        }

        Ok(task_ids)
    }

    /// 收集文件列表（保留目录结构）
    async fn collect_files(paths: &[String]) -> AppResult<Vec<(PathBuf, FileMeta)>> {
        let mut files = Vec::new();
        let mut file_id: u64 = 0;

        for path_str in paths {
            let path = PathBuf::from(path_str);

            if path.is_file() {
                files.push((path.clone(), build_meta(&path, Path::new(""), file_id).await?));
                file_id += 1;
            } else if path.is_dir() {
                let base_path = path.parent().unwrap_or(&path).to_path_buf();

                for entry in WalkDir::new(&path).follow_links(true) {
                    let entry = entry.map_err(|e| {
                        let io_err = e
                            .io_error()
                            .map(|err| std::io::Error::new(err.kind(), err.to_string()))
                            .unwrap_or_else(|| {
                                std::io::Error::new(std::io::ErrorKind::Other, "WalkDir error")
                            });
                        AppError::Io(io_err)
                    })?;

                    if entry.file_type().is_file() {
                        let relative = entry
                            .path()
                            .strip_prefix(&base_path)
                            .unwrap_or(entry.path());
                        let relative_dir = relative.parent().unwrap_or(Path::new(""));

                        files.push((
                            entry.path().to_path_buf(),
                            build_meta(entry.path(), relative_dir, file_id).await?,
                        ));
                        file_id += 1;
                    }
                }
            } else {
                log::warn!("Skip non-existent path: {}", path_str);
            }
        }

        Ok(files)
    }

    /// Cancel a transfer
    pub async fn cancel_transfer(&self, task_id: Uuid) -> AppResult<()> {
        if let Some(arc) = self
            .app_state
            .transfer_tasks
            .get(&task_id)
            .map(|entry| entry.value().clone())
        {
            let mut task = arc.lock().await;
            task.set_status(TransferStatus::Canceled);
            log::info!("Transfer {} canceled", task_id);
        }
        Ok(())
    }

    /// Pause a transfer
    pub async fn pause_transfer(&self, task_id: Uuid) -> AppResult<()> {
        if let Some(arc) = self
            .app_state
            .transfer_tasks
            .get(&task_id)
            .map(|entry| entry.value().clone())
        {
            let mut task = arc.lock().await;
            if task.status == TransferStatus::Transferring {
                task.set_status(TransferStatus::Paused);
                log::info!("Transfer {} paused", task_id);
            }
        }
        Ok(())
    }

    /// Resume a paused transfer（同一进程内从暂停点继续）
    pub async fn resume_transfer(&self, task_id: Uuid) -> AppResult<()> {
        if let Some(arc) = self
            .app_state
            .transfer_tasks
            .get(&task_id)
            .map(|entry| entry.value().clone())
        {
            let mut task = arc.lock().await;
            if task.status == TransferStatus::Paused {
                task.set_status(TransferStatus::Transferring);
                log::info!("Transfer {} resumed", task_id);
            }
        }
        Ok(())
    }
}

/// 由文件路径构造元数据
async fn build_meta(path: &Path, relative_path: &Path, file_id: u64) -> AppResult<FileMeta> {
    let metadata = tokio::fs::metadata(path).await?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(FileMeta {
        file_id,
        relative_path: relative_path.to_string_lossy().to_string(),
        file_name,
        file_size: metadata.len(),
    })
}

/// 写入传输历史
fn record_history(task: &TransferTask, status: &str, error: Option<&str>) {
    if let Some(storage) = crate::storage::get_storage() {
        let file_names = task
            .files
            .iter()
            .map(|f| f.file_name.clone())
            .collect::<Vec<_>>()
            .join(", ");

        let _ = storage.add_transfer_history(
            task.task_id,
            task.peer_device_id,
            &task.peer_device_name,
            "send",
            status,
            task.total_size,
            task.file_count,
            Some(&file_names),
        );

        if let Some(err) = error {
            let _ = storage.update_transfer_status(task.task_id, status, Some(err));
        }
    }
}

/// Background task for progress updates
pub async fn start_progress_updater(app_state: Arc<AppState>) {
    let mut ticker = interval(Duration::from_millis(500));
    let mut previous: HashMap<Uuid, (u64, Instant)> = HashMap::new();

    loop {
        ticker.tick().await;

        let tasks: Vec<(Uuid, Arc<Mutex<TransferTask>>)> = app_state
            .transfer_tasks
            .iter()
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect();

        for (task_id, arc) in tasks {
            let mut task = arc.lock().await;

            if task.status != TransferStatus::Transferring {
                previous.remove(&task_id);
                continue;
            }

            let now = Instant::now();
            let (prev_bytes, prev_time) = previous
                .get(&task_id)
                .copied()
                .unwrap_or((task.transferred_size, now));
            let elapsed = now.duration_since(prev_time).as_secs_f64();
            let delta = task.transferred_size.saturating_sub(prev_bytes);
            let speed = if elapsed > 0.0 {
                (delta as f64 / elapsed) as u64
            } else {
                0
            };

            let transferred = task.transferred_size;
            task.update_progress(transferred, speed);
            let info = TransferTaskInfo::from(&*task);
            drop(task);

            previous.insert(task_id, (info.transferred_size, now));

            let _ = app_state.emit_async("transfer-progress", &info).await;
        }
    }
}

/// Public function to send files
pub async fn send_files(
    request: SendFilesRequest,
    state: Arc<AppState>,
) -> AppResult<Vec<Uuid>> {
    let manager = TransferManager::new(state);
    manager.send_files(request).await
}

/// Public function to cancel transfer
pub async fn cancel_transfer(task_id: Uuid, state: Arc<AppState>) -> AppResult<()> {
    let manager = TransferManager::new(state);
    manager.cancel_transfer(task_id).await
}

/// Public function to pause transfer
pub async fn pause_transfer(task_id: Uuid, state: Arc<AppState>) -> AppResult<()> {
    let manager = TransferManager::new(state);
    manager.pause_transfer(task_id).await
}

/// Public function to resume transfer
pub async fn resume_transfer(task_id: Uuid, state: Arc<AppState>) -> AppResult<()> {
    let manager = TransferManager::new(state);
    manager.resume_transfer(task_id).await
}
