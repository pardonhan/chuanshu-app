use std::path::PathBuf;
use rusqlite::{Connection, params, OptionalExtension};
use uuid::Uuid;
use serde::Serialize;

use crate::core::AppResult;
use crate::ipc::Settings;
use crate::transfer::{resume::ResumeInfo, TransferType};

pub use crate::network::device::KnownDevice;

/// Storage manager for SQLite database operations
pub struct Storage {
    conn: Connection,
}

impl Storage {
    /// Create or open the database
    pub fn new(app_data_dir: &PathBuf) -> AppResult<Self> {
        // Ensure the app data directory exists
        std::fs::create_dir_all(app_data_dir)?;

        let db_path = app_data_dir.join("chuanshu.db");
        let conn = Connection::open(&db_path)?;

        let storage = Self { conn };
        storage.init_tables()?;

        log::info!("Database initialized at {:?}", db_path);
        Ok(storage)
    }

    /// Initialize database tables
    fn init_tables(&self) -> AppResult<()> {
        // Settings table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Transfer history table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS transfer_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT UNIQUE NOT NULL,
                peer_device_id TEXT NOT NULL,
                peer_device_name TEXT NOT NULL,
                transfer_type TEXT NOT NULL,
                status TEXT NOT NULL,
                total_size INTEGER NOT NULL,
                file_count INTEGER NOT NULL,
                file_names TEXT,
                error_message TEXT,
                created_at INTEGER DEFAULT (strftime('%s', 'now')),
                completed_at INTEGER
            )",
            [],
        )?;

        // Resume info table
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS resume_info (
                task_id TEXT PRIMARY KEY,
                peer_device_id TEXT NOT NULL,
                peer_device_name TEXT NOT NULL,
                transfer_type TEXT NOT NULL,
                total_size INTEGER NOT NULL,
                total_transferred INTEGER NOT NULL DEFAULT 0,
                files_json TEXT NOT NULL,
                received_chunks_json TEXT NOT NULL,
                save_path TEXT NOT NULL,
                created_at INTEGER DEFAULT (strftime('%s', 'now')),
                updated_at INTEGER DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Known devices table (for remembering connected devices)
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS known_devices (
                device_id TEXT PRIMARY KEY,
                device_name TEXT NOT NULL,
                os TEXT NOT NULL,
                ip_address TEXT NOT NULL,
                quic_port INTEGER NOT NULL,
                protocol_version TEXT,
                capabilities TEXT,
                last_seen INTEGER DEFAULT (strftime('%s', 'now')),
                last_connected INTEGER,
                is_online INTEGER DEFAULT 0,
                created_at INTEGER DEFAULT (strftime('%s', 'now'))
            )",
            [],
        )?;

        // Create indexes
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_history_status ON transfer_history(status)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_history_created ON transfer_history(created_at DESC)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_resume_updated ON resume_info(updated_at)",
            [],
        )?;
        self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_known_devices_last_connected ON known_devices(last_connected DESC)",
            [],
        )?;

        Ok(())
    }

    // ==================== Settings ====================

    /// Save a setting
    pub fn save_setting(&self, key: &str, value: &str) -> AppResult<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value, updated_at)
             VALUES (?1, ?2, strftime('%s', 'now'))
             ON CONFLICT(key) DO UPDATE SET
             value = excluded.value,
             updated_at = excluded.updated_at",
            params![key, value],
        )?;
        Ok(())
    }

    /// Get a setting
    pub fn get_setting(&self, key: &str) -> AppResult<Option<String>> {
        let value = self.conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [key],
            |row| row.get::<_, String>(0),
        ).optional()?;
        Ok(value)
    }

    /// Save all settings
    pub fn save_settings(&self, settings: &Settings) -> AppResult<()> {
        self.save_setting("device_name", &settings.device_name)?;
        self.save_setting("download_path", &settings.download_path)?;
        self.save_setting("auto_accept", &settings.auto_accept.to_string())?;
        self.save_setting("upload_limit", &settings.upload_limit.to_string())?;
        self.save_setting("download_limit", &settings.download_limit.to_string())?;
        self.save_setting("enable_notification", &settings.enable_notification.to_string())?;
        self.save_setting("theme", &settings.theme)?;
        Ok(())
    }

    /// Load all settings
    pub fn load_settings(&self) -> AppResult<Option<Settings>> {
        let device_name = self.get_setting("device_name")?;

        if device_name.is_none() {
            return Ok(None);
        }

        let parse_bool = |s: Option<String>| -> bool {
            s.map(|v| v.parse().unwrap_or(false)).unwrap_or(false)
        };

        let parse_u32 = |s: Option<String>| -> u32 {
            s.map(|v| v.parse().unwrap_or(0)).unwrap_or(0)
        };

        Ok(Some(Settings {
            device_name: device_name.unwrap(),
            download_path: self.get_setting("download_path")?.unwrap_or_else(|| "~/Downloads/传书".to_string()),
            auto_accept: parse_bool(self.get_setting("auto_accept")?),
            upload_limit: parse_u32(self.get_setting("upload_limit")?),
            download_limit: parse_u32(self.get_setting("download_limit")?),
            enable_notification: parse_bool(self.get_setting("enable_notification")?),
            theme: self.get_setting("theme")?.unwrap_or_else(|| "auto".to_string()),
        }))
    }

    // ==================== Transfer History ====================

    /// Add a transfer to history
    pub fn add_transfer_history(
        &self,
        task_id: Uuid,
        peer_device_id: Uuid,
        peer_device_name: &str,
        transfer_type: &str,
        status: &str,
        total_size: u64,
        file_count: u32,
        file_names: Option<&str>,
    ) -> AppResult<()> {
        self.conn.execute(
            "INSERT INTO transfer_history
             (task_id, peer_device_id, peer_device_name, transfer_type, status,
              total_size, file_count, file_names)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(task_id) DO UPDATE SET
             status = excluded.status,
             completed_at = CASE WHEN excluded.status IN ('completed', 'failed', 'canceled')
                                THEN strftime('%s', 'now') ELSE NULL END",
            params![
                task_id.to_string(),
                peer_device_id.to_string(),
                peer_device_name,
                transfer_type,
                status,
                total_size as i64,
                file_count as i64,
                file_names,
            ],
        )?;
        Ok(())
    }

    /// Update transfer status in history
    pub fn update_transfer_status(
        &self,
        task_id: Uuid,
        status: &str,
        error_message: Option<&str>,
    ) -> AppResult<()> {
        self.conn.execute(
            "UPDATE transfer_history SET
             status = ?2,
             error_message = ?3,
             completed_at = CASE WHEN ?2 IN ('completed', 'failed', 'canceled')
                                THEN strftime('%s', 'now') ELSE completed_at END
             WHERE task_id = ?1",
            params![task_id.to_string(), status, error_message],
        )?;
        Ok(())
    }

    /// Get transfer history with pagination
    pub fn get_transfer_history(&self, limit: i64, offset: i64) -> AppResult<Vec<TransferHistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT task_id, peer_device_name, transfer_type, status,
                    total_size, file_count, file_names, created_at, completed_at
             FROM transfer_history
             ORDER BY created_at DESC
             LIMIT ?1 OFFSET ?2"
        )?;

        let entries = stmt.query_map(params![limit, offset], |row| {
            Ok(TransferHistoryEntry {
                task_id: row.get(0)?,
                peer_device_name: row.get(1)?,
                transfer_type: row.get(2)?,
                status: row.get(3)?,
                total_size: row.get::<_, i64>(4)? as u64,
                file_count: row.get::<_, i64>(5)? as u32,
                file_names: row.get(6)?,
                created_at: row.get(7)?,
                completed_at: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    /// Clear transfer history
    pub fn clear_transfer_history(&self) -> AppResult<()> {
        self.conn.execute("DELETE FROM transfer_history", [])?;
        Ok(())
    }

    // ==================== Resume Info ====================

    /// Save resume info
    pub fn save_resume_info(&self, resume_info: &ResumeInfo) -> AppResult<()> {
        let files_json = serde_json::to_string(&resume_info.files)?;
        let received_chunks: std::collections::HashMap<u64, Vec<u8>> = resume_info.files
            .iter()
            .map(|(id, info)| (*id, info.received_chunks.clone()))
            .collect();
        let received_chunks_json = serde_json::to_string(&received_chunks)?;

        // Get first file info to extract common fields
        let first_file = resume_info.files.values().next()
            .ok_or_else(|| crate::core::AppError::Other("No files in resume info".to_string()))?;

        self.conn.execute(
            "INSERT INTO resume_info
             (task_id, peer_device_id, peer_device_name, transfer_type,
              total_size, total_transferred, files_json, received_chunks_json, save_path, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, strftime('%s', 'now'))
             ON CONFLICT(task_id) DO UPDATE SET
             total_transferred = excluded.total_transferred,
             received_chunks_json = excluded.received_chunks_json,
             updated_at = excluded.updated_at",
            params![
                resume_info.task_id.to_string(),
                resume_info.peer_device_id.to_string(),
                resume_info.peer_device_name,
                format!("{:?}", resume_info.transfer_type),
                first_file.metadata.file_size as i64,
                resume_info.total_transferred as i64,
                files_json,
                received_chunks_json,
                first_file.temp_path.to_string_lossy().to_string(),
            ],
        )?;

        Ok(())
    }

    /// Load resume info
    pub fn load_resume_info(&self, task_id: Uuid) -> AppResult<Option<ResumeInfo>> {
        let result = self.conn.query_row(
            "SELECT peer_device_id, peer_device_name, transfer_type, files_json, received_chunks_json, total_transferred
             FROM resume_info WHERE task_id = ?1",
            [task_id.to_string()],
            |row| {
                let peer_device_id: String = row.get(0)?;
                let peer_device_name: String = row.get(1)?;
                let transfer_type: String = row.get(2)?;
                let files_json: String = row.get(3)?;
                let received_chunks_json: String = row.get(4)?;
                let total_transferred: i64 = row.get(5)?;
                Ok((peer_device_id, peer_device_name, transfer_type, files_json, received_chunks_json, total_transferred))
            },
        ).optional()?;

        if let Some((peer_device_id_str, peer_device_name, transfer_type_str, files_json, received_chunks_json, total_transferred)) = result {
            let files: std::collections::HashMap<u64, crate::transfer::resume::FileResumeInfo> =
                serde_json::from_str(&files_json)?;
            let received_chunks: std::collections::HashMap<u64, Vec<u8>> =
                serde_json::from_str(&received_chunks_json)?;

            let peer_device_id = Uuid::parse_str(&peer_device_id_str)
                .map_err(|e| crate::core::AppError::Other(format!("Invalid peer device ID: {}", e)))?;

            let transfer_type = match transfer_type_str.as_str() {
                "Send" => TransferType::Send,
                "Receive" => TransferType::Receive,
                _ => TransferType::Receive, // Default to Receive
            };

            let mut resume_info = ResumeInfo {
                task_id,
                peer_device_id,
                peer_device_name,
                transfer_type,
                files,
                total_transferred: total_transferred as u64,
                last_updated: 0,
            };

            // Restore received chunks
            for (file_id, chunks) in received_chunks {
                if let Some(file_info) = resume_info.files.get_mut(&file_id) {
                    file_info.received_chunks = chunks;
                }
            }

            Ok(Some(resume_info))
        } else {
            Ok(None)
        }
    }

    /// Delete resume info
    pub fn delete_resume_info(&self, task_id: Uuid) -> AppResult<()> {
        self.conn.execute(
            "DELETE FROM resume_info WHERE task_id = ?1",
            [task_id.to_string()],
        )?;
        Ok(())
    }

    /// List all incomplete transfers
    pub fn list_incomplete_transfers(&self) -> AppResult<Vec<ResumeInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT task_id, peer_device_id, peer_device_name, transfer_type, files_json, received_chunks_json, total_transferred
             FROM resume_info ORDER BY updated_at DESC"
        )?;

        let entries: Vec<(String, String, String, String, String, String, i64)> = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

        let mut result = Vec::new();
        for (task_id_str, peer_device_id_str, peer_device_name, transfer_type_str, files_json, received_chunks_json, total_transferred) in entries {
            if let Ok(task_id) = Uuid::parse_str(&task_id_str) {
                let files: std::collections::HashMap<u64, crate::transfer::resume::FileResumeInfo> =
                    serde_json::from_str(&files_json)?;
                let received_chunks: std::collections::HashMap<u64, Vec<u8>> =
                    serde_json::from_str(&received_chunks_json)?;

                let peer_device_id = Uuid::parse_str(&peer_device_id_str)
                    .map_err(|e| crate::core::AppError::Other(format!("Invalid peer device ID: {}", e)))?;

                let transfer_type = match transfer_type_str.as_str() {
                    "Send" => TransferType::Send,
                    "Receive" => TransferType::Receive,
                    _ => TransferType::Receive,
                };

                let mut resume_info = ResumeInfo {
                    task_id,
                    peer_device_id,
                    peer_device_name,
                    transfer_type,
                    files,
                    total_transferred: total_transferred as u64,
                    last_updated: 0,
                };

                for (file_id, chunks) in received_chunks {
                    if let Some(file_info) = resume_info.files.get_mut(&file_id) {
                        file_info.received_chunks = chunks;
                    }
                }

                result.push(resume_info);
            }
        }

        Ok(result)
    }

    /// Clean up old resume entries
    pub fn cleanup_old_resume_entries(&self, max_age_days: u32) -> AppResult<u64> {
        let cutoff = std::time::SystemTime::now()
            - std::time::Duration::from_secs(max_age_days as u64 * 24 * 60 * 60);
        let cutoff_secs = cutoff
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let affected = self.conn.execute(
            "DELETE FROM resume_info WHERE updated_at < ?1",
            [cutoff_secs],
        )?;

        Ok(affected as u64)
    }

    // ==================== Known Devices ====================

    /// Save or update a known device
    pub fn save_known_device(&self, device: &crate::network::device::DeviceInfo) -> AppResult<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let capabilities: Vec<String> = device.capabilities.iter().map(|c| {
            match c {
                crate::network::device::Capability::FolderTransfer => "FolderTransfer".to_string(),
                crate::network::device::Capability::ResumeTransfer => "ResumeTransfer".to_string(),
                crate::network::device::Capability::MultiDeviceSend => "MultiDeviceSend".to_string(),
                crate::network::device::Capability::P2PTransfer => "P2PTransfer".to_string(),
            }
        }).collect();
        let capabilities_json = serde_json::to_string(&capabilities).unwrap_or_default();

        let os_str = match &device.os {
            crate::network::device::OperatingSystem::Windows => "Windows".to_string(),
            crate::network::device::OperatingSystem::MacOS => "MacOS".to_string(),
            crate::network::device::OperatingSystem::Linux => "Linux".to_string(),
            crate::network::device::OperatingSystem::Unknown => "Unknown".to_string(),
        };

        self.conn.execute(
            "INSERT INTO known_devices
             (device_id, device_name, os, ip_address, quic_port, protocol_version,
              capabilities, last_seen, is_online)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)
             ON CONFLICT(device_id) DO UPDATE SET
             device_name = excluded.device_name,
             os = excluded.os,
             ip_address = excluded.ip_address,
             quic_port = excluded.quic_port,
             protocol_version = excluded.protocol_version,
             capabilities = excluded.capabilities,
             last_seen = excluded.last_seen,
             is_online = 1",
            params![
                device.device_id.to_string(),
                device.device_name,
                os_str,
                device.ip_address.to_string(),
                device.quic_port as i64,
                device.protocol_version,
                capabilities_json,
                now,
            ],
        )?;

        Ok(())
    }

    /// Get all known devices
    pub fn get_known_devices(&self) -> AppResult<Vec<KnownDevice>> {
        let mut stmt = self.conn.prepare(
            "SELECT device_id, device_name, os, ip_address, quic_port,
                    protocol_version, capabilities, last_seen, last_connected,
                    is_online, created_at
             FROM known_devices
             ORDER BY last_connected DESC, last_seen DESC"
        )?;

        let devices = stmt.query_map([], |row| {
            Ok(KnownDevice {
                device_id: row.get(0)?,
                device_name: row.get(1)?,
                os: row.get(2)?,
                ip_address: row.get(3)?,
                quic_port: row.get(4)?,
                protocol_version: row.get(5)?,
                capabilities: row.get(6)?,
                last_seen: row.get(7)?,
                last_connected: row.get(8)?,
                is_online: row.get::<_, i64>(9)? != 0,
                created_at: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(devices)
    }

    /// Update device online status
    pub fn update_device_online_status(&self, device_id: Uuid, is_online: bool) -> AppResult<()> {
        self.conn.execute(
            "UPDATE known_devices SET is_online = ?1 WHERE device_id = ?2",
            params![if is_online { 1 } else { 0 }, device_id.to_string()],
        )?;
        Ok(())
    }

    /// Update last connected time
    pub fn update_last_connected(&self, device_id: Uuid) -> AppResult<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        self.conn.execute(
            "UPDATE known_devices SET last_connected = ?1 WHERE device_id = ?2",
            params![now, device_id.to_string()],
        )?;
        Ok(())
    }

    /// Mark device as offline
    pub fn mark_device_offline(&self, device_id: Uuid) -> AppResult<()> {
        self.conn.execute(
            "UPDATE known_devices SET is_online = 0 WHERE device_id = ?1",
            params![device_id.to_string()],
        )?;
        Ok(())
    }

    /// Mark all devices as offline (called on startup)
    pub fn mark_all_devices_offline(&self) -> AppResult<()> {
        self.conn.execute(
            "UPDATE known_devices SET is_online = 0 WHERE is_online = 1",
            params![],
        )?;
        Ok(())
    }

    /// Delete a known device
    pub fn delete_known_device(&self, device_id: Uuid) -> AppResult<()> {
        self.conn.execute(
            "DELETE FROM known_devices WHERE device_id = ?1",
            params![device_id.to_string()],
        )?;
        Ok(())
    }
}

/// Transfer history entry
#[derive(Debug, Clone, Serialize)]
pub struct TransferHistoryEntry {
    pub task_id: String,
    pub peer_device_name: String,
    pub transfer_type: String,
    pub status: String,
    pub total_size: u64,
    pub file_count: u32,
    pub file_names: Option<String>,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

// Global storage instance
static STORAGE: once_cell::sync::OnceCell<std::sync::Mutex<Storage>> = once_cell::sync::OnceCell::new();

/// Initialize the global storage
pub fn init_storage(app_data_dir: &PathBuf) -> AppResult<()> {
    let storage = Storage::new(app_data_dir)?;
    STORAGE.set(std::sync::Mutex::new(storage))
        .map_err(|_| crate::core::AppError::Other("Storage already initialized".to_string()))?;
    Ok(())
}

/// Get the global storage instance
pub fn get_storage() -> Option<std::sync::MutexGuard<'static, Storage>> {
    STORAGE.get().map(|s| s.lock().unwrap())
}
