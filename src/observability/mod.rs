pub mod backend;
pub mod router;
pub mod ui;

use std::sync::Arc;
use crate::storage::backend::Storage;

pub fn init(storage: Arc<Storage>) {
    // Initialize the backend (Recorder + Subscriber)
    backend::init(storage);
}
