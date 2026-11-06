use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use quinn::Endpoint;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::core::{AppError, AppResult, AppState, DEFAULT_CHUNK_SIZE};
use crate::network::protocol::{
    read_serialized, write_serialized, FileHeader, FileMeta, TransferRequest, TransferResponse,
    MAX_CONTROL_FRAME_SIZE, TAG_DATA, TAG_FILE_END, TAG_FILE_HEADER, TAG_TRANSFER_END,
};
use crate::network::tls;
use crate::transfer::file_chunk::calculate_crc32;
use crate::transfer::rate_limiter::SharedRateLimiter;
use crate::transfer::{TransferStatus, TransferTask};

/// 建立连接并完成整次传输（请求 -> 响应 -> 数据 -> 确认）
pub async fn send_transfer(
    peer: SocketAddr,
    request: TransferRequest,
    files: Vec<(PathBuf, FileMeta)>,
    app_state: Arc<AppState>,
    task_id: Uuid,
) -> AppResult<()> {
    let mut endpoint = Endpoint::client(
        "0.0.0.0:0"
            .parse()
            .map_err(|e| AppError::Other(format!("本地地址非法: {}", e)))?,
    )?;
    endpoint.set_default_client_config(tls::client_config());

    let connecting = endpoint
        .connect(peer, "chuanshu.local")
        .map_err(|e| AppError::Network(format!("发起连接失败: {}", e)))?;

    let connection = tokio::time::timeout(Duration::from_secs(10), connecting)
        .await
        .map_err(|_| AppError::Network(format!("连接 {} 超时", peer)))?
        .map_err(|e| AppError::Network(format!("连接 {} 失败: {}", peer, e)))?;

    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| AppError::Network(format!("打开传输流失败: {}", e)))?;

    // 发送请求并等待接收端确认
    write_serialized(&mut send, &request).await?;

    let response: TransferResponse = tokio::time::timeout(
        Duration::from_secs(15),
        read_serialized(&mut recv, MAX_CONTROL_FRAME_SIZE),
    )
    .await
    .map_err(|_| AppError::Network("等待接收端响应超时".to_string()))?
    .map_err(|e| AppError::Network(format!("接收端未响应，连接已断开（{}）", e)))?;

    if !response.accepted {
        let reason = response.reason.unwrap_or_else(|| "接收端拒绝了本次传输".to_string());
        connection.close(0u32.into(), b"rejected");
        update_task(&app_state, task_id, |task| task.set_error(reason.clone())).await;
        return Err(AppError::Permission(reason));
    }

    // 标记为传输中（取消状态不被覆盖）
    if let Some(arc) = app_state.transfer_tasks.get(&task_id).map(|e| e.value().clone()) {
        let mut task = arc.lock().await;
        if task.status == TransferStatus::Canceled {
            drop(task);
            connection.close(0u32.into(), b"canceled");
            return Err(AppError::Other("传输已取消".to_string()));
        }
        task.files = request.files.clone();
        task.set_status(TransferStatus::Transferring);
    }

    let upload_limit = crate::storage::get_storage()
        .and_then(|s| s.load_settings().ok().flatten())
        .map(|s| s.upload_limit)
        .unwrap_or(0);
    let limiter = SharedRateLimiter::new(upload_limit);

    let mut buffer = vec![0u8; DEFAULT_CHUNK_SIZE as usize];
    let mut transferred: u64 = 0;

    for (index, (path, meta)) in files.iter().enumerate() {
        wait_if_paused(&app_state, task_id).await?;

        if let Some(arc) = app_state.transfer_tasks.get(&task_id).map(|e| e.value().clone()) {
            let mut task = arc.lock().await;
            task.set_current_file(&meta.file_name, index as u32);
        }

        let header = FileHeader {
            file_id: meta.file_id,
            file_name: meta.file_name.clone(),
            relative_path: meta.relative_path.clone(),
            file_size: meta.file_size,
        };

        send.write_all(&[TAG_FILE_HEADER])
            .await
            .map_err(|e| AppError::Network(format!("发送文件头失败: {}", e)))?;
        write_serialized(&mut send, &header).await?;

        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| AppError::Other(format!("无法读取文件 {}: {}", path.display(), e)))?;

        let mut hasher = Sha256::new();
        let mut offset: u64 = 0;

        loop {
            wait_if_paused(&app_state, task_id).await?;

            let n = file
                .read(&mut buffer)
                .await
                .map_err(|e| AppError::Other(format!("读取文件失败 {}: {}", path.display(), e)))?;
            if n == 0 {
                break;
            }

            let data = &buffer[..n];
            hasher.update(data);
            let crc = calculate_crc32(data);

            send.write_all(&[TAG_DATA])
                .await
                .map_err(|e| AppError::Network(format!("发送数据失败: {}", e)))?;
            send.write_all(&offset.to_be_bytes())
                .await
                .map_err(|e| AppError::Network(format!("发送数据失败: {}", e)))?;
            send.write_all(&(n as u32).to_be_bytes())
                .await
                .map_err(|e| AppError::Network(format!("发送数据失败: {}", e)))?;
            send.write_all(&crc.to_be_bytes())
                .await
                .map_err(|e| AppError::Network(format!("发送数据失败: {}", e)))?;
            send.write_all(data)
                .await
                .map_err(|e| AppError::Network(format!("发送数据失败: {}", e)))?;

            limiter.consume_and_wait(n as u64).await;

            offset += n as u64;
            transferred += n as u64;

            if let Some(arc) = app_state.transfer_tasks.get(&task_id).map(|e| e.value().clone()) {
                let mut task = arc.lock().await;
                task.update_progress(transferred, 0);
            }
        }

        if offset != meta.file_size {
            return Err(AppError::Other(format!(
                "文件大小发生变化: {}（声明 {} 字节，实际 {} 字节）",
                meta.file_name, meta.file_size, offset
            )));
        }

        let digest = hasher.finalize();
        send.write_all(&[TAG_FILE_END, 0x01])
            .await
            .map_err(|e| AppError::Network(format!("发送文件结尾失败: {}", e)))?;
        send.write_all(&digest)
            .await
            .map_err(|e| AppError::Network(format!("发送文件摘要失败: {}", e)))?;
    }

    wait_if_paused(&app_state, task_id).await?;

    send.write_all(&[TAG_TRANSFER_END])
        .await
        .map_err(|e| AppError::Network(format!("发送结束帧失败: {}", e)))?;
    send.finish()
        .await
        .map_err(|e| AppError::Network(format!("关闭发送流失败: {}", e)))?;

    // 等待接收端最终确认
    let mut ack = [0u8; 1];
    tokio::time::timeout(Duration::from_secs(60), recv.read_exact(&mut ack))
        .await
        .map_err(|_| AppError::Network("等待接收端确认超时".to_string()))?
        .map_err(|e| AppError::Network(format!("读取接收端确认失败: {}", e)))?;

    if ack[0] != 0x00 {
        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf)
            .await
            .map_err(|e| AppError::Network(format!("读取错误信息失败: {}", e)))?;
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut message = vec![0u8; len.min(64 * 1024)];
        recv.read_exact(&mut message)
            .await
            .map_err(|e| AppError::Network(format!("读取错误信息失败: {}", e)))?;
        return Err(AppError::Other(format!(
            "接收端处理失败: {}",
            String::from_utf8_lossy(&message)
        )));
    }

    connection.close(0u32.into(), b"done");
    let _ = tokio::time::timeout(Duration::from_secs(5), endpoint.wait_idle()).await;

    Ok(())
}

async fn update_task<F>(app_state: &Arc<AppState>, task_id: Uuid, f: F)
where
    F: FnOnce(&mut TransferTask),
{
    if let Some(arc) = app_state.transfer_tasks.get(&task_id).map(|e| e.value().clone()) {
        let mut task = arc.lock().await;
        f(&mut task);
    }
}

async fn wait_if_paused(app_state: &Arc<AppState>, task_id: Uuid) -> AppResult<()> {
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
