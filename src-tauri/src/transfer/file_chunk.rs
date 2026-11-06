use crc32fast::Hasher;
use sha2::{Digest, Sha256};

use crate::core::{AppResult, SMALL_FILE_THRESHOLD};

/// Utility functions for splitting files into transfer chunks.
pub struct FileChunker;

impl FileChunker {
    /// Calculate a chunk size appropriate for the given file size.
    pub fn optimal_chunk_size(file_size: u64) -> u32 {
        if file_size < SMALL_FILE_THRESHOLD {
            (file_size as u32).max(64 * 1024) // Min 64KB
        } else if file_size < 100 * 1024 * 1024 {
            1024 * 1024 // 1MB chunks
        } else if file_size < 1024 * 1024 * 1024 {
            4 * 1024 * 1024 // 4MB chunks
        } else {
            8 * 1024 * 1024 // 8MB chunks
        }
    }

    /// Calculate number of chunks for a file.
    pub fn chunk_count(file_size: u64, chunk_size: u32) -> u64 {
        ((file_size + chunk_size as u64 - 1) / chunk_size as u64).max(1)
    }
}

/// Calculate CRC32 checksum.
pub fn calculate_crc32(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// Calculate SHA-256 hash of a file.
pub async fn calculate_file_hash(file_path: &std::path::Path) -> AppResult<String> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(file_path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];

    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}
