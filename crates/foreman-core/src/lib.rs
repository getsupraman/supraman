mod error;
mod model;
mod registry;
mod scan;
mod slug;

pub use error::{CoreError, Result};
pub use model::{Platform, Repo};
pub use registry::{Registry, home_from};
pub use scan::{DEFAULT_DEPTH, scan};
pub use slug::{slugify, validate_name};
