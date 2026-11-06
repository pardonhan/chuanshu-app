use std::io::SeekFrom;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use quinn::{Connection, Endpoint};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::core::{
    AppError, AppResult, AppState, DEFAULT_CHUNK_SIZE, QUIC_DEFAULT_PORT,
    TRANSFER_PROTOCOL_VERSION, default_download_dir, resolve_download_dir,
};
use crate::network::protocol::{
    read_serialized, write_serialized, FileHeader, TransferRequest, TransferResponse,
    MAX_CONTROL_FRAME_SIZE, TAG_DATA, TAG_FILE_END, TAG_FILE_HEADER, TAG_TRANSFER_END,
};
use crate::network::tls;
use crate::transfer::file_chunk::calculate_crc32;
use crate::transfer::rate_limiter::SharedRateLimiter;
use crate::transfer::{TransferStatus, TransferTask, TransferType};

/// QUIC server handle
pub struct QuicServer {
    endpoint: Endpoint,
    port: u16,
}

impl QuicServer {
    pub async fn new(port: u16) -> AppResult<Self> {
        let server_config = tls::server_config()?;
        let addr: SocketAddr = format!("0.0.0.0:{}", port)
            .parse()
            .map_err(|e| AppError::Other(format!("Invalid address: {}", e)))?;

        let endpoint = Endpoint::server(server_config, addr)?;
        let actual_port = endpoint.local_addr()?.port();
        log::info!("QUIC server bound to port {}", actual_port);

        Ok(Self {
            endpoint,
            port: actual_port,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub async fn run(&self, app_state: Arc<AppState>) -> AppResult<()> {
        log::info!("QUIC server running on port {}", self.port);

        while let Some(incoming) = self.endpoint.accept().await {
            let app_state = app_state.clone();
            tokio::spawn(async move {
                match incoming.await {
                    Ok(connection) => {
                        if let Err(e) = handle_connection(connection, app_state).await {
                            log::debug!("Transfer connection error: {}", e);
                        }
                    }
                    Err(e) => log::error!("Failed to accept QUIC connection: {}", e),
                }
            });
        }

        Ok(())
    }

    pub fn stop(&self) {
        self.endpoint.close(0u32.into(), b"server shutdown");
        log::info!("QUIC server stopped");
    }
}

/// 一个连接只处理一条双向流：请求 -> 响应 -> 数据 -> 确认
async fn handle_connection(connection: Connection, app_state: Arc<AppState>) -> AppResult<()> {
    let remote_addr = connection.remote_address();
    log::info!("New QUIC connection from {}", remote_addr);

    let (send, recv) = tokio::time::timeout(Duration::from_secs(10), connection.accept_bi())
        .await
        .map_err(|_| AppError::Other("等待传输流超时".to_string()))?
        .map_err(|e| AppError::Network(format!("接受传输流失败: {}", e)))?;

    handle_transfer(send, recv, app_state).await
}

async fn handle_transfer(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    app_state: Arc<AppState>,
) -> AppResult<()> {
    let request: TransferRequest = read_serialized(&mut recv, MAX_CONTROL_FRAME_SIZE).await?;

    log::info!(
        "Transfer request from {} ({}): {} files, {} bytes",
        request.sender_name,
        request.sender_id,
        request.files.len(),
        request.total_size
    );

    if request.protocol_version != TRANSFER_PROTOCOL_VERSION {
        let reason = format!(
            "协议版本不匹配（本机 {}，对端 {}），请将两端升级到同一版本",
            TRANSFER_PROTOCOL_VERSION, request.protocol_version
        );
        let response = TransferResponse {
            task_id: request.task_id,
            accepted: false,
            reason: Some(reason),
            save_path: None,
        };
        let _ = write_serialized(&mut send, &response).await;
        let _ = send.finish().await;
        return Ok(());
    }

    // 读取接收设置
    let settings = crate::storage::get_storage()
        .and_then(|s| s.load_settings().ok().flatten());
    let auto_accept = settings.as_ref().map(|s| s.auto_accept).unwrap_or(true);
    let download_path = settings
        .as_ref()
        .map(|s| s.download_path.clone())
        .unwrap_or_default();
    let download_limit = settings.as_ref().map(|s| s.download_limit).unwrap_or(0);

    let configured_dir = if download_path.trim().is_empty() {
        default_download_dir()
    } else {
        resolve_download_dir(&download_path)
    };

    if !auto_accept {
        let response = TransferResponse {
            task_id: request.task_id,
            accepted: false,
            reason: Some("接收端已关闭自动接收".to_string()),
            save_path: None,
        };
        let _ = write_serialized(&mut send, &response).await;
        let _ = send.finish().await;
        return Ok(());
    }

    // 确保接收目录可用：配置目录不可写时依次回退到默认下载目录与应用数据目录
    let mut candidates = vec![configured_dir.clone()];
    let default_dir = default_download_dir();
    if !candidates.contains(&default_dir) {
        candidates.push(default_dir);
    }
    if let Some(fallback) = crate::core::app_data_download_dir(&app_state) {
        if !candidates.contains(&fallback) {
            candidates.push(fallback);
        }
    }

    let download_dir = match crate::core::first_writable_dir(&candidates) {
        Some(dir) => dir,
        None => {
            let reason = format!(
                "接收端没有可写的保存目录（已尝试 {:?}）。{}",
                candidates,
                crate::core::permission_hint()
            );
            log::error!("{}", reason);

            let response = TransferResponse {
                task_id: request.task_id,
                accepted: false,
                reason: Some(reason),
                save_path: None,
            };
            let _ = write_serialized(&mut send, &response).await;
            let _ = send.finish().await;
            return Ok(());
        }
    };

    if download_dir != configured_dir {
        log::warn!(
            "配置的下载目录 {:?} 不可写，本次改存到 {:?}",
            configured_dir,
            download_dir
        );
    }

    let response = TransferResponse {
        task_id: request.task_id,
        accepted: true,
        reason: None,
        save_path: Some(download_dir.to_string_lossy().to_string()),
    };
    write_serialized(&mut send, &response).await?;

    // 创建接收任务
    let mut task = TransferTask::new(
        request.task_id,
        request.sender_id,
        request.sender_name.clone(),
        TransferType::Receive,
        request.total_size,
        request.files.len() as u32,
    );
    task.files = request.files.clone();
    task.set_status(TransferStatus::Transferring);
    app_state
        .transfer_tasks
        .insert(request.task_id, Arc::new(Mutex::new(task)));

    let result = receive_files(&mut recv, &request, &app_state, &download_dir, download_limit).await;

    match result {
        Ok(()) => {
            let _ = send.write_all(&[0x00u8]).await;
            let _ = send.finish().await;

            if let Some(arc) = app_state.transfer_tasks.get(&request.task_id).map(|e| e.value().clone()) {
                let mut task = arc.lock().await;
                task.set_status(TransferStatus::Completed);
                save_history(&task, "completed", None);
                if let Some(handle) = app_state.get_app_handle() {
                    let _ = crate::ipc::emit_transfer_completed(
                        &handle,
                        request.task_id,
                        &request.sender_name,
                    );
                }
            }

            log::info!("Transfer {} received successfully", request.task_id);
            Ok(())
        }
        Err(e) => {
            let message = e.to_string();
            log::error!("Transfer {} failed: {}", request.task_id, message);

            send_error_ack(&mut send, &message).await;

            if let Some(arc) = app_state.transfer_tasks.get(&request.task_id).map(|e| e.value().clone()) {
                let mut task = arc.lock().await;
                let canceled = task.status == TransferStatus::Canceled;
                if canceled {
                    task.set_status(TransferStatus::Canceled);
                    save_history(&task, "canceled", Some(&message));
                } else {
                    task.set_error(message.clone());
                    save_history(&task, "failed", Some(&message));
                }
                if let Some(handle) = app_state.get_app_handle() {
                    let _ = crate::ipc::emit_transfer_failed(&handle, request.task_id, message);
                }
            }

            Ok(())
        }
    }
}

fn save_history(task: &TransferTask, status: &str, error: Option<&str>) {
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
            "receive",
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

async fn send_error_ack(send: &mut quinn::SendStream, message: &str) {
    let bytes = message.as_bytes();
    let _ = send.write_all(&[0x01u8]).await;
    let _ = send.write_all(&(bytes.len() as u32).to_be_bytes()).await;
    let _ = send.write_all(bytes).await;
    let _ = send.finish().await;
}

struct IncomingFile {
    temp_path: PathBuf,
    final_path: PathBuf,
    file: tokio::fs::File,
    hasher: Sha256,
    declared_size: u64,
    received: u64,
}

/// 接收文件，失败时清理未完成的 .part
async fn receive_files(
    recv: &mut quinn::RecvStream,
    request: &TransferRequest,
    app_state: &Arc<AppState>,
    download_dir: &Path,
    download_limit: u32,
) -> AppResult<()> {
    let mut temp_files: Vec<PathBuf> = Vec::new();
    let result = receive_files_inner(
        recv,
        request,
        app_state,
        download_dir,
        download_limit,
        &mut temp_files,
    )
    .await;

    if result.is_err() {
        for path in temp_files {
            let _ = tokio::fs::remove_file(&path).await;
        }
    }

    result
}

async fn receive_files_inner(
    recv: &mut quinn::RecvStream,
    request: &TransferRequest,
    app_state: &Arc<AppState>,
    download_dir: &Path,
    download_limit: u32,
    temp_files: &mut Vec<PathBuf>,
) -> AppResult<()> {
    let limiter = SharedRateLimiter::new(download_limit);
    let mut current: Option<IncomingFile> = None;
    let mut transferred: u64 = 0;
    let mut finished_files: usize = 0;
    let mut received_ids: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let total_files = request.files.len();

    loop {
        wait_if_paused(app_state, request.task_id).await?;

        let mut tag = [0u8; 1];
        recv.read_exact(&mut tag)
            .await
            .map_err(|e| AppError::Network(format!("读取帧标签失败: {}", e)))?;

        match tag[0] {
            TAG_TRANSFER_END => break,

            TAG_FILE_HEADER => {
                if current.is_some() {
                    return Err(AppError::Other("上一个文件尚未结束".to_string()));
                }

                let header: FileHeader = read_serialized(recv, MAX_CONTROL_FRAME_SIZE).await?;

                if !received_ids.insert(header.file_id) {
                    return Err(AppError::Other(format!("重复的文件 ID: {}", header.file_id)));
                }

                let meta = request
                    .files
                    .iter()
                    .find(|f| f.file_id == header.file_id)
                    .ok_or_else(|| AppError::Other(format!("未知文件 ID: {}", header.file_id)))?;
                if meta.file_size != header.file_size {
                    return Err(AppError::Other(format!(
                        "文件大小不一致: {} (声明 {}, 实际 {})",
                        header.file_name, meta.file_size, header.file_size
                    )));
                }

                let final_path = build_safe_target(download_dir, &header)?;
                if let Some(parent) = final_path.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }

                let base = download_dir
                    .canonicalize()
                    .unwrap_or_else(|_| download_dir.to_path_buf());
                let parent = final_path
                    .parent()
                    .ok_or_else(|| AppError::Other("非法路径".to_string()))?
                    .canonicalize()
                    .map_err(|e| AppError::Other(format!("路径校验失败: {}", e)))?;
                if !parent.starts_with(&base) {
                    return Err(AppError::Other("非法的相对路径".to_string()));
                }

                let final_path = unique_path(&final_path);
                let temp_path = download_dir.join(format!(
                    ".{}.{}.part",
                    request.task_id, header.file_id
                ));

                let file = tokio::fs::File::create(&temp_path).await?;
                temp_files.push(temp_path.clone());

                if let Some(arc) = app_state.transfer_tasks.get(&request.task_id).map(|e| e.value().clone()) {
                    let mut task = arc.lock().await;
                    task.set_current_file(&header.file_name, finished_files as u32);
                }

                current = Some(IncomingFile {
                    temp_path,
                    final_path,
                    file,
                    hasher: Sha256::new(),
                    declared_size: header.file_size,
                    received: 0,
                });
            }

            TAG_DATA => {
                let mut meta_buf = [0u8; 16];
                recv.read_exact(&mut meta_buf)
                    .await
                    .map_err(|e| AppError::Network(format!("读取分片头失败: {}", e)))?;

                let offset = u64::from_be_bytes(meta_buf[0..8].try_into().unwrap());
                let data_len = u32::from_be_bytes(meta_buf[8..12].try_into().unwrap()) as usize;
                let crc = u32::from_be_bytes(meta_buf[12..16].try_into().unwrap());

                if data_len > DEFAULT_CHUNK_SIZE as usize {
                    return Err(AppError::Other("分片过大".to_string()));
                }

                let mut buf = vec![0u8; data_len];
                recv.read_exact(&mut buf)
                    .await
                    .map_err(|e| AppError::Network(format!("读取分片数据失败: {}", e)))?;

                if calculate_crc32(&buf) != crc {
                    return Err(AppError::Other("分片校验失败".to_string()));
                }

                let file = current
                    .as_mut()
                    .ok_or_else(|| AppError::Other("收到数据但缺少文件头".to_string()))?;

                file.file.seek(SeekFrom::Start(offset)).await?;
                file.file.write_all(&buf).await?;
                file.hasher.update(&buf);
                file.received += data_len as u64;
                transferred += data_len as u64;

                limiter.consume_and_wait(data_len as u64).await;

                if let Some(arc) = app_state.transfer_tasks.get(&request.task_id).map(|e| e.value().clone()) {
                    let mut task = arc.lock().await;
                    task.update_progress(transferred, 0);
                }
            }

            TAG_FILE_END => {
                let mut has_sha = [0u8; 1];
                recv.read_exact(&mut has_sha)
                    .await
                    .map_err(|e| AppError::Network(format!("读取文件结尾失败: {}", e)))?;

                let mut file = current
                    .take()
                    .ok_or_else(|| AppError::Other("收到文件结尾但缺少文件头".to_string()))?;

                if has_sha[0] == 1 {
                    let mut expected = [0u8; 32];
                    recv.read_exact(&mut expected)
                        .await
                        .map_err(|e| AppError::Network(format!("读取文件摘要失败: {}", e)))?;
                    let actual = file.hasher.finalize();
                    if actual.as_slice() != &expected[..] {
                        return Err(AppError::Other(format!(
                            "文件校验失败: {}",
                            file.final_path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default()
                        )));
                    }
                }

                file.file.flush().await?;
                file.file.sync_all().await?;

                if file.received != file.declared_size {
                    return Err(AppError::Other(format!(
                        "文件大小校验失败：期望 {} 字节，实际 {} 字节",
                        file.declared_size, file.received
                    )));
                }

                drop(file.file);
                tokio::fs::rename(&file.temp_path, &file.final_path).await?;
                temp_files.retain(|p| p != &file.temp_path);

                finished_files += 1;
                log::info!("Received file -> {:?}", file.final_path);
            }

            other => {
                return Err(AppError::Other(format!("未知帧标签: {}", other)));
            }
        }
    }

    if let Some(file) = current.take() {
        return Err(AppError::Other(format!(
            "文件未完成: {}",
            file.final_path.to_string_lossy()
        )));
    }

    if finished_files != total_files {
        return Err(AppError::Other(format!(
            "文件数量不一致：期望 {}，实际 {}",
            total_files, finished_files
        )));
    }

    if transferred != request.total_size {
        return Err(AppError::Other(format!(
            "传输总大小不一致：期望 {}，实际 {}",
            request.total_size, transferred
        )));
    }

    Ok(())
}

/// 暂停时等待，取消时返回错误
async fn wait_if_paused(app_state: &Arc<AppState>, task_id: uuid::Uuid) -> AppResult<()> {
    loop {
        let arc = app_state
            .transfer_tasks
            .get(&task_id)
            .map(|e| e.value().clone());

        let Some(arc) = arc else { return Ok(()) };
        let task = arc.lock().await;

        match task.status {
            TransferStatus::Canceled => return Err(AppError::Other("传输已取消".to_string())),
            TransferStatus::Paused => {
                drop(task);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            _ => return Ok(()),
        }
    }
}

/// 拼接相对路径，拒绝 `..`、绝对路径与盘符
fn build_safe_target(base: &Path, header: &FileHeader) -> AppResult<PathBuf> {
    let mut target = base.to_path_buf();

    for part in header
        .relative_path
        .split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != ".")
    {
        if part == ".." || part.contains(':') {
            return Err(AppError::Other("非法的相对路径".to_string()));
        }
        target.push(part);
    }

    let name = header.file_name.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains(':')
    {
        return Err(AppError::Other("非法的文件名".to_string()));
    }
    target.push(name);

    Ok(target)
}

/// 重名时追加序号，避免覆盖已有文件
fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }

    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let ext = path.extension().map(|e| e.to_string_lossy().to_string());

    for index in 1..10000 {
        let name = match &ext {
            Some(ext) => format!("{} ({}).{}", stem, index, ext),
            None => format!("{} ({})", stem, index),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    path.to_path_buf()
}

/// Global QUIC server instance
static QUIC_SERVER: tokio::sync::OnceCell<QuicServer> = tokio::sync::OnceCell::const_new();

/// Start the QUIC server
pub async fn start_quic_server(app_state: Arc<AppState>) -> AppResult<u16> {
    if QUIC_SERVER.get().is_none() {
        let server = QuicServer::new(QUIC_DEFAULT_PORT).await?;
        let _ = QUIC_SERVER.set(server);
    }

    let server = QUIC_SERVER
        .get()
        .ok_or_else(|| AppError::Other("QUIC 服务端初始化失败".to_string()))?;
    let port = server.port();

    tokio::spawn({
        let app_state = app_state.clone();
        async move {
            if let Err(e) = server.run(app_state).await {
                log::error!("QUIC server error: {}", e);
            }
        }
    });

    log::info!("QUIC server started on port {}", port);
    Ok(port)
}

/// Stop the QUIC server
pub fn stop_quic_server() {
    if let Some(server) = QUIC_SERVER.get() {
        server.stop();
    }
}
