use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("IO错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("网络错误: {0}")]
    Network(String),

    #[error("序列化错误: {0}")]
    Serialization(#[from] bincode::Error),

    #[error("JSON序列化错误: {0}")]
    JsonSerialization(#[from] serde_json::Error),

    #[error("数据库错误: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("UUID错误: {0}")]
    Uuid(#[from] uuid::Error),

    #[error("配置错误: {0}")]
    Config(String),

    #[error("设备未找到: {0}")]
    DeviceNotFound(String),

    #[error("传输任务未找到: {0}")]
    TaskNotFound(String),

    #[error("权限错误: {0}")]
    Permission(String),

    #[error("其他错误: {0}")]
    Other(String),
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = std::result::Result<T, AppError>;
