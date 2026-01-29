use tokio::sync::broadcast;


#[derive(Clone, Debug, PartialEq)]
pub enum MutationType {
    Create,
    Update,
    Delete,
}

#[derive(Clone, Debug)]
pub struct MutationEvent {
    pub type_name: String,
    pub uid: u64,
    pub mutation_type: MutationType,
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
