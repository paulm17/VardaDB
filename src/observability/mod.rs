pub mod backend;
pub mod router;
pub mod ui;

use crate::storage::backend::Storage;
use std::sync::Arc;

pub fn init(storage: Arc<Storage>) {
    // Initialize the backend (Recorder + Subscriber)
    backend::init(storage);
}
