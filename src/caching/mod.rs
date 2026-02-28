pub mod sqlite_state;
pub mod dataflow;

use crate::caching::sqlite_state::SqliteState;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use crate::storage::backend::Storage;

pub struct CacheManager {
    storage: Arc<Storage>,
    views: Mutex<HashMap<String, Arc<SqliteState>>>,
}

impl CacheManager {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            views: Mutex::new(HashMap::new()),
        }
    }

    pub fn create_view(&self, name: &str, _query: &str) -> Arc<SqliteState> {
        let mut views = self.views.lock().unwrap();
        if let Some(view) = views.get(name) {
            return view.clone();
        }
        
        // In a real system, we would parse the query and set up a dataflow here.
        let view = Arc::new(SqliteState::new(self.storage.clone(), name));
        views.insert(name.to_string(), view.clone());
        view
    }
}
