use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, SupervisorError>;

#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    #[error(transparent)]
    Core(#[from] foreman_core::CoreError),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("cannot serialize the desired state of '{platform}': {source}")]
    Serialize {
        platform: String,
        source: toml::ser::Error,
    },
}
