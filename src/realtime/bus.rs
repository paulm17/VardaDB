use tokio::sync::broadcast;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MutationType {
    Create,
    Update,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MutationSource {
    Local,
    Remote,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SchemaMetadata {
    pub uniques: Vec<String>,
    pub inverses: Vec<crate::engine::resolver::InverseInfo>,
    pub search_fields: std::collections::HashMap<String, Vec<String>>,
    pub facet_fields: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MutationEvent {
    pub type_name: String,
    pub uid: u64,
    pub mutation_type: MutationType,
    pub source: MutationSource,
    pub payload: Option<std::collections::HashMap<String, serde_json::Value>>,
    pub metadata: Option<SchemaMetadata>,
    pub timestamp: Option<crate::storage::timestamp::Timestamp>,
    #[serde(default)]
    pub node_id: u64,
}

#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<MutationEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1024);
        Self { sender }
    }

    pub fn publish(&self, event: MutationEvent) {
        // We ignore error if there are no subscribers
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<MutationEvent> {
        self.sender.subscribe()
    }
}
