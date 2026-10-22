mod error;
mod events;
mod model;
mod registry;
mod scan;
mod slug;

pub use error::{CoreError, Result};
pub use events::{
    Event, EventReader, ROTATE_AT, append as append_event, events_dir, is_safe_id, log_path, role_injection, role_tag,
};
pub use model::{Platform, Repo};
pub use registry::{Registry, home_from};
pub use scan::{DEFAULT_DEPTH, scan};
pub use slug::{slugify, validate_name};
