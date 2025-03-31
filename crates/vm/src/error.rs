use thiserror::Error;
use tokio::task::JoinError;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Backend init error: {0}")]
    BackendInit(String),
    #[error("IO error: {0}")]
    IO(#[from] std::io::Error),
    #[error("Compile error: {0}")]
    Compile(String),
    #[error("Assembly error: {0}")]
    Assembly(String),
    #[error("Link error: {0}")]
    Link(String),
    #[error("Lib loading error: {0}")]
    LibLoading(#[from] libloading::Error),
    #[error("Get symbol error: {0}")]
    GetSymbol(String),
    #[error("Lock poison error: {0}")]
    LockPoison(String),
    #[error("Handle join error: {0}")]
    Join(#[from] JoinError),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Disable compiler")]
    DisableCompiler,
}
