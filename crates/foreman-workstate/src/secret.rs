#[cfg(windows)]
pub use foreman_core::secret::FileStore;
pub use foreman_core::secret::{MemoryStore, SecretStore, default_store};
