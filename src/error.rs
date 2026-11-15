use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("配置错误: {0}")]
    Config(String),

    #[error("网络错误: {0}")]
    Network(String),

    #[error("校验失败: {0}")]
    Verify(String),

    #[error("安装失败: {0}")]
    Install(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("下载已取消")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}
