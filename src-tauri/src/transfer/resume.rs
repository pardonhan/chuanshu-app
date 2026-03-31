use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

use crate::core::AppResult;
use crate::transfer::file_chunk::FileChunker;
use crate::network::protocol::FileMetadata;
use crate::transfer::TransferType;

/// Resume information for a transfer task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeInfo {
    /// Transfer task ID
    pub task_id: Uuid,
    /// Peer device ID
    pub peer_device_id: Uuid,
    /// Peer device name
    pub peer_device_name: String,
    /// Transfer type (send/receive)
    pub transfer_type: TransferType,
    /// File resume info indexed by file_id
    pub files: HashMap<u64, FileResumeInfo>,
    /// Total bytes transferred
    pub total_transferred: u64,
    /// Last updated timestamp
    pub last_updated: u64,
}

/// Resume information for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResumeInfo {
    /// File metadata
    pub metadata: FileMetadata,
    /// Received chunks bitmap (as bytes for serialization)
    pub received_chunks: Vec<u8>,
    /// Number of chunks received
    pub chunks_received: u64,
    /// Temporary file path
    pub temp_path: PathBuf,
}

impl ResumeInfo {
    /// Create new resume info for a transfer
    pub fn new(
        task_id: Uuid,
        peer_device_id: Uuid,
        peer_device_name: String,
        transfer_type: TransferType,
        files: Vec<FileMetadata>,
        download_path: PathBuf,
    ) -> Self {
        let files_map: HashMap<u64, FileResumeInfo> = files
            .into_iter()
            .map(|metadata| {
                let file_id = metadata.file_id;
                let chunk_size = FileChunker::optimal_chunk_size(metadata.file_size);
                let total_chunks = FileChunker::chunk_count(metadata.file_size, chunk_size) as usize;

                let resume_info = FileResumeInfo {
                    metadata: metadata.clone(),
                    received_chunks: vec![0u8; (total_chunks + 7) / 8], // Bitmap: 1 bit per chunk
                    chunks_received: 0,
                    temp_path: download_path.join(format!(".{}.tmp", file_id)),
                };

                (file_id, resume_info)
            })
            .collect();

        Self {
            task_id,
            peer_device_id,
            peer_device_name,
            transfer_type,
            files: files_map,
            total_transferred: 0,
            last_updated: current_timestamp(),
        }
    }

    /// Mark a chunk as received
    pub fn mark_chunk_received(&mut self, file_id: u64, chunk_index: u64) {
        if let Some(file_info) = self.files.get_mut(&file_id) {
            let byte_idx = (chunk_index / 8) as usize;
            let bit_idx = (chunk_index % 8) as u8;

            if byte_idx < file_info.received_chunks.len() {
                file_info.received_chunks[byte_idx] |= 1 << bit_idx;
                file_info.chunks_received += 1;
            }
        }
        self.last_updated = current_timestamp();
    }

    /// Check if a chunk has been received
    pub fn is_chunk_received(&self, file_id: u64, chunk_index: u64) -> bool {
        if let Some(file_info) = self.files.get(&file_id) {
            let byte_idx = (chunk_index / 8) as usize;
            let bit_idx = (chunk_index % 8) as u8;

            if byte_idx < file_info.received_chunks.len() {
                return (file_info.received_chunks[byte_idx] >> bit_idx) & 1 == 1;
            }
        }
        false
    }

    /// Get missing chunks for a file
    pub fn get_missing_chunks(&self, file_id: u64) -> Vec<u64> {
        let mut missing = Vec::new();

        if let Some(file_info) = self.files.get(&file_id) {
            let chunk_size = FileChunker::optimal_chunk_size(file_info.metadata.file_size);
            let total_chunks = FileChunker::chunk_count(file_info.metadata.file_size, chunk_size);

            for chunk_idx in 0..total_chunks {
                if !self.is_chunk_received(file_id, chunk_idx) {
                    missing.push(chunk_idx);
                }
            }
        }

        missing
    }

    /// Calculate total progress (0.0 to 1.0)
    pub fn progress(&self) -> f64 {
        let total_chunks: u64 = self.files.values()
            .map(|f| {
                let chunk_size = FileChunker::optimal_chunk_size(f.metadata.file_size);
                FileChunker::chunk_count(f.metadata.file_size, chunk_size)
            })
            .sum();

        let received_chunks: u64 = self.files.values()
            .map(|f| f.chunks_received)
            .sum();

        if total_chunks == 0 {
            0.0
        } else {
            received_chunks as f64 / total_chunks as f64
        }
    }

    /// Get list of completed files
    pub fn completed_files(&self) -> Vec<u64> {
        self.files
            .iter()
            .filter(|(_, info)| {
                let chunk_size = FileChunker::optimal_chunk_size(info.metadata.file_size);
                let total_chunks = FileChunker::chunk_count(info.metadata.file_size, chunk_size);
                info.chunks_received >= total_chunks
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// Remove completed files from resume info
    pub fn remove_completed_files(&mut self) -> Vec<(FileMetadata, PathBuf)> {
        let completed: Vec<u64> = self.completed_files();
        let mut result = Vec::new();

        for file_id in completed {
            if let Some(info) = self.files.remove(&file_id) {
                result.push((info.metadata, info.temp_path));
            }
        }

        result
    }
}

/// Resume manager for persisting and loading resume info
pub struct ResumeManager<'a> {
    storage: &'a crate::storage::Storage,
}

impl<'a> ResumeManager<'a> {
    /// Create a new resume manager
    pub fn new(storage: &'a crate::storage::Storage) -> Self {
        Self { storage }
    }

    /// Save resume info to database
    pub fn save_resume(&self, resume_info: &ResumeInfo) -> AppResult<()> {
        self.storage.save_resume_info(resume_info)
    }

    /// Load resume info from database
    pub fn load_resume(&self, task_id: Uuid) -> AppResult<Option<ResumeInfo>> {
        self.storage.load_resume_info(task_id)
    }

    /// Delete resume info
    pub fn delete_resume(&self, task_id: Uuid) -> AppResult<()> {
        self.storage.delete_resume_info(task_id)
    }

    /// List all incomplete transfers
    pub fn list_incomplete_transfers(&self) -> AppResult<Vec<ResumeInfo>> {
        self.storage.list_incomplete_transfers()
    }

    /// Clean up old resume entries
    pub fn cleanup_old_entries(&self, max_age_days: u32) -> AppResult<u64> {
        self.storage.cleanup_old_resume_entries(max_age_days)
    }
}

/// Get current Unix timestamp
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Convert bitmap to list of received chunk indices
pub fn bitmap_to_indices(bitmap: &[u8], total_chunks: u64) -> Vec<u64> {
    let mut indices = Vec::new();

    for (byte_idx, byte) in bitmap.iter().enumerate() {
        for bit_idx in 0..8 {
            if byte & (1 << bit_idx) != 0 {
                let chunk_idx = (byte_idx * 8 + bit_idx) as u64;
                if chunk_idx < total_chunks {
                    indices.push(chunk_idx);
                }
            }
        }
    }

    indices
}

/// Convert list of indices to bitmap
pub fn indices_to_bitmap(indices: &[u64], total_chunks: u64) -> Vec<u8> {
    let bitmap_size = ((total_chunks + 7) / 8) as usize;
    let mut bitmap = vec![0u8; bitmap_size];

    for idx in indices {
        let byte_idx = (idx / 8) as usize;
        let bit_idx = (idx % 8) as u8;
        if byte_idx < bitmap.len() {
            bitmap[byte_idx] |= 1 << bit_idx;
        }
    }

    bitmap
}
