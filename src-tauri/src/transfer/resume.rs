use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

use crate::core::AppResult;
use crate::network::protocol::FileMetadata;
use crate::transfer::file_chunk::FileChunker;
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
                let total_chunks =
                    FileChunker::chunk_count(metadata.file_size, chunk_size) as usize;

                let resume_info = FileResumeInfo {
                    metadata: metadata.clone(),
                    received_chunks: vec![0u8; (total_chunks + 7) / 8], // Bitmap: 1 bit per chunk
                    chunks_received: 0,
                    temp_path: download_path.join(format!(".{}-{}.tmp", task_id, file_id)),
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

    /// Reconcile persisted per-file state with the metadata announced by the sender.
    ///
    /// Only file id, name, size, mtime, relative path and checksum are compared;
    /// any mismatch discards the old bitmap and forces a full resend.
    pub fn reconcile_files(&mut self, files: &[FileMetadata], download_path: &std::path::Path) {
        for metadata in files {
            let matches = self
                .files
                .get(&metadata.file_id)
                .map(|existing| metadata_matches(&existing.metadata, metadata))
                .unwrap_or(false);

            if !matches {
                let chunk_size = FileChunker::optimal_chunk_size(metadata.file_size);
                let total_chunks =
                    FileChunker::chunk_count(metadata.file_size, chunk_size) as usize;
                self.files.insert(
                    metadata.file_id,
                    FileResumeInfo {
                        metadata: metadata.clone(),
                        received_chunks: vec![0u8; (total_chunks + 7) / 8],
                        chunks_received: 0,
                        temp_path: download_path
                            .join(format!(".{}-{}.tmp", self.task_id, metadata.file_id)),
                    },
                );
            }
        }

        // Drop files that are no longer part of the transfer.
        self.files
            .retain(|id, _| files.iter().any(|file| file.file_id == *id));
    }

    /// Mark a chunk as received
    pub fn mark_chunk_received(&mut self, file_id: u64, chunk_index: u64) {
        if let Some(file_info) = self.files.get_mut(&file_id) {
            let byte_idx = (chunk_index / 8) as usize;
            let bit_idx = (chunk_index % 8) as u8;

            if byte_idx < file_info.received_chunks.len() {
                let mask = 1 << bit_idx;
                if file_info.received_chunks[byte_idx] & mask == 0 {
                    file_info.received_chunks[byte_idx] |= mask;
                    file_info.chunks_received += 1;
                }
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

    /// Load resume info from database
    pub fn load_resume(&self, task_id: Uuid) -> AppResult<Option<ResumeInfo>> {
        self.storage.load_resume_info(task_id)
    }
}

/// Get current Unix timestamp
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Compare the metadata fields that determine whether a chunk bitmap is still valid.
fn metadata_matches(left: &FileMetadata, right: &FileMetadata) -> bool {
    left.file_name == right.file_name
        && left.file_size == right.file_size
        && left.modified_time == right.modified_time
        && left.relative_path == right.relative_path
        && left.checksum == right.checksum
}
