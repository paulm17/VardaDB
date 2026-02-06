use std::sync::Arc;
use crate::storage::backend::Storage;

pub struct FjallState {
    storage: Arc<Storage>,
    view_name: String,
}

impl FjallState {
    pub fn new(storage: Arc<Storage>, view_name: &str) -> Self {
        Self {
            storage,
            view_name: view_name.to_string(),
        }
    }

    // Simulate Readyset's `process_records`
    // In reality, this would take a batch of changes from the dataflow.
    pub fn process_update(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let view_key = format!("view:{}:{}", self.view_name, key);
        // TODO: Make DB configurable for Views?
        self.storage.insert("default", view_key.as_bytes(), value.as_bytes())?;
        Ok(())
    }
    
    pub fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        let view_key = format!("view:{}:{}", self.view_name, key);
        let val = self.storage.get("default", view_key.as_bytes())?;
        Ok(val.map(|v| String::from_utf8(v).unwrap()))
    }
}
