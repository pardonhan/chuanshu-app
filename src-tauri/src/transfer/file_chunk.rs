use crc32fast::Hasher;

/// 计算 CRC32 校验和
pub fn calculate_crc32(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}
