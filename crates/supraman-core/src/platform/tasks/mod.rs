//! Task Scheduler behind [`Registrar`].

#[cfg(not(windows))]
use crate::schedule::{Listing, Registrar};
#[cfg(not(windows))]
use crate::{CoreError, Result};

#[cfg(windows)]
mod com;

/// Task Scheduler through COM `ITaskService`. Each call runs on a thread of its own in the MTA, never
/// the caller's: an STA call pumps the UI thread's messages.
#[derive(Debug, Default)]
pub struct WindowsRegistrar;

#[cfg(not(windows))]
fn refused() -> CoreError {
    CoreError::Tasks(super::refused("Task Scheduler"))
}

/// Every call refused: no schedule is registered here.
#[cfg(not(windows))]
impl Registrar for WindowsRegistrar {
    fn user(&self) -> Result<String> {
        Err(refused())
    }

    fn tasks(&self, _: &str) -> Result<Vec<(String, String)>> {
        Err(refused())
    }

    fn register(&self, _: &str, _: &str, _: &str) -> Result<()> {
        Err(refused())
    }

    fn delete(&self, _: &str, _: &str) -> Result<()> {
        Err(refused())
    }

    fn delete_empty_folder(&self, _: &str) -> Result<bool> {
        Err(refused())
    }

    fn all_tasks(&self, _: &(dyn Fn(&str) -> bool + Sync)) -> Result<Listing> {
        Err(refused())
    }
}
