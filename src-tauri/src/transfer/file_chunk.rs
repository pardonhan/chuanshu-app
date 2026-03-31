use std::path::Path;
use std::fs::File;
use std::io::{Read, Write, Seek, SeekFrom};
use crc32fast::Hasher;
use memmap2::Mmap;

use crate::core::{AppResult, DEFAULT_CHUNK_SIZE, SMALL_FILE_THRESHOLD};
use crate::network::protocol::{ChunkMetadata, FileMetadata};

/// File chunker for splitting files into chunks
pub struct FileChunker {
    chunk_size: u32,
}

impl FileChunker {
    /// Create a new file chunker with default chunk size
    pub fn new() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
        }
    }

    /// Create a new file chunker with custom chunk size
    pub fn with_chunk_size(chunk_size: u32) -> Self {
        Self { chunk_size }
    }

    /// Calculate optimal chunk size based on file size
    pub fn optimal_chunk_size(file_size: u64) -> u32 {
        if file_size < SMALL_FILE_THRESHOLD {
            // Small files: use smaller chunks
            (file_size as u32).max(64 * 1024) // Min 64KB
        } else if file_size < 100 * 1024 * 1024 {
            // Medium files (< 100MB): 1MB chunks
            1024 * 1024
        } else if file_size < 1024 * 1024 * 1024 {
            // Large files (< 1GB): 4MB chunks
            4 * 1024 * 1024
        } else {
            // Very large files (>= 1GB): 8MB chunks
            8 * 1024 * 1024
        }
    }

    /// Calculate number of chunks for a file
    pub fn chunk_count(file_size: u64, chunk_size: u32) -> u64 {
        ((file_size + chunk_size as u64 - 1) / chunk_size as u64).max(1)
    }

    /// Read a chunk from a file
    pub fn read_chunk(
        &self,
        file_path: &Path,
        chunk_index: u64,
        total_chunks: u64,
    ) -> AppResult<(ChunkMetadata, Vec<u8>)> {
        let file_size = std::fs::metadata(file_path)?.len();
        let chunk_size = self.chunk_size as u64;
        let offset = chunk_index * chunk_size;
        let remaining = file_size.saturating_sub(offset);
        let actual_size = remaining.min(chunk_size) as u32;

        // Use memory mapping for large files
        let data = if file_size > 10 * 1024 * 1024 {
            self.read_chunk_mmap(file_path, offset, actual_size)?
        } else {
            self.read_chunk_direct(file_path, offset, actual_size)?
        };

        // Calculate checksum
        let checksum = calculate_crc32(&data);

        // Create metadata
        let metadata = ChunkMetadata {
            task_id: uuid::Uuid::nil(), // Will be filled by caller
            file_id: 0, // Will be filled by caller
            chunk_index,
            total_chunks,
            chunk_size: actual_size,
            offset,
            checksum,
        };

        Ok((metadata, data))
    }

    /// Read chunk using memory mapping
    fn read_chunk_mmap(
        &self,
        file_path: &Path,
        offset: u64,
        size: u32,
    ) -> AppResult<Vec<u8>> {
        let file = File::open(file_path)?;
        let mmap = unsafe { Mmap::map(&file)? };

        let start = offset as usize;
        let end = start + size as usize;

        if end > mmap.len() {
            return Err(crate::core::AppError::Other(
                "Chunk extends beyond file bounds".to_string()
            ));
        }

        Ok(mmap[start..end].to_vec())
    }

    /// Read chunk using direct file I/O
    fn read_chunk_direct(
        &self,
        file_path: &Path,
        offset: u64,
        size: u32,
    ) -> AppResult<Vec<u8>> {
        let mut file = File::open(file_path)?;
        file.seek(SeekFrom::Start(offset))?;

        let mut buffer = vec![0u8; size as usize];
        file.read_exact(&mut buffer)?;

        Ok(buffer)
    }
}

impl Default for FileChunker {
    fn default() -> Self {
        Self::new()
    }
}

/// File assembler for reconstructing files from chunks
pub struct FileAssembler {
    file_path: std::path::PathBuf,
    file_size: u64,
    file: Option<File>,
    received_chunks: bitvec::vec::BitVec,
    total_chunks: u64,
}

impl FileAssembler {
    /// Create a new file assembler
    pub fn new(file_path: &Path, file_size: u64, chunk_size: u32) -> AppResult<Self> {
        let total_chunks = FileChunker::chunk_count(file_size, chunk_size);

        // Ensure parent directory exists
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Create/truncate file and pre-allocate space
        let file = File::create(file_path)?;

        // Pre-allocate file space
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            file.set_len(file_size)?;
        }

        Ok(Self {
            file_path: file_path.to_path_buf(),
            file_size,
            file: Some(file),
            received_chunks: bitvec::bitvec![0; total_chunks as usize],
            total_chunks,
        })
    }

    /// Write a chunk to the file
    pub fn write_chunk(&mut self, chunk: &ChunkMetadata, data: &[u8]) -> AppResult<bool> {
        // Verify checksum
        let actual_checksum = calculate_crc32(data);
        if actual_checksum != chunk.checksum {
            return Err(crate::core::AppError::Other(
                format!("Chunk checksum mismatch: expected {}, got {}", chunk.checksum, actual_checksum)
            ));
        }

        // Write chunk at offset
        let file = self.file.as_mut().unwrap();
        file.seek(SeekFrom::Start(chunk.offset))?;
        file.write_all(data)?;

        // Mark chunk as received
        self.received_chunks.set(chunk.chunk_index as usize, true);

        // Check if file is complete
        Ok(self.is_complete())
    }

    /// Check if all chunks have been received
    pub fn is_complete(&self) -> bool {
        self.received_chunks.all()
    }

    /// Get progress as a fraction (0.0 to 1.0)
    pub fn progress(&self) -> f64 {
        let received = self.received_chunks.count_ones() as f64;
        received / self.total_chunks as f64
    }

    /// Get the received chunks bitmap
    pub fn received_chunks(&self) -> &bitvec::vec::BitVec {
        &self.received_chunks
    }

    /// Finalize the file (close and verify)
    pub fn finalize(mut self) -> AppResult<std::path::PathBuf> {
        if let Some(mut file) = self.file.take() {
            file.flush()?;
            file.sync_all()?;
        }
        Ok(self.file_path)
    }

    /// Get missing chunk indices
    pub fn missing_chunks(&self) -> Vec<u64> {
        self.received_chunks
            .iter()
            .enumerate()
            .filter(|(_, received)| !**received)
            .map(|(idx, _)| idx as u64)
            .collect()
    }
}

/// Calculate CRC32 checksum
pub fn calculate_crc32(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// Calculate SHA-256 hash of a file
pub async fn calculate_file_hash(file_path: &Path) -> AppResult<String> {
    use sha2::{Sha256, Digest};
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(file_path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024]; // 64KB buffer

    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

/// Batch small files for efficient transfer
pub struct BatchBuilder {
    files: Vec<FileMetadata>,
    total_size: u64,
    max_batch_size: u64,
    max_file_count: usize,
}

impl BatchBuilder {
    /// Create a new batch builder
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            total_size: 0,
            max_batch_size: crate::core::MAX_BATCH_SIZE,
            max_file_count: crate::core::MAX_BATCH_FILES,
        }
    }

    /// Add a file to the batch
    pub fn add(&mut self, metadata: FileMetadata) -> bool {
        // Check if adding this file would exceed limits
        if self.files.len() >= self.max_file_count {
            return false;
        }
        if self.total_size + metadata.file_size > self.max_batch_size {
            return false;
        }

        self.total_size += metadata.file_size;
        self.files.push(metadata);
        true
    }

    /// Check if batch can accept more files
    pub fn can_add(&self, file_size: u64) -> bool {
        if self.files.len() >= self.max_file_count {
            return false;
        }
        if self.total_size + file_size > self.max_batch_size {
            return false;
        }
        true
    }

    /// Build the batch
    pub fn build(self) -> (Vec<FileMetadata>, u64) {
        (self.files, self.total_size)
    }

    /// Get current batch size
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Check if batch is empty
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

impl Default for BatchBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Split files into batches for efficient transfer
pub fn batch_files(files: Vec<FileMetadata>) -> Vec<Vec<FileMetadata>> {
    let mut batches: Vec<Vec<FileMetadata>> = Vec::new();
    let mut current_batch = BatchBuilder::new();

    for file in files {
        // Large files go in their own batch
        if file.file_size > SMALL_FILE_THRESHOLD {
            if !current_batch.is_empty() {
                batches.push(current_batch.build().0);
                current_batch = BatchBuilder::new();
            }
            batches.push(vec![file]);
            continue;
        }

        // Try to add to current batch
        if !current_batch.add(file.clone()) {
            // Batch is full, start a new one
            batches.push(current_batch.build().0);
            current_batch = BatchBuilder::new();
            current_batch.add(file);
        }
    }

    // Don't forget the last batch
    if !current_batch.is_empty() {
        batches.push(current_batch.build().0);
    }

    batches
}
