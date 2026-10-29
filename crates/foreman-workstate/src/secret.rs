#[cfg(windows)]
pub use foreman_core::secret::FileStore;
pub use foreman_core::secret::{LEGACY_PREFIX, MemoryStore, SecretStore, default_store};
