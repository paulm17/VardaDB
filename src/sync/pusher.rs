use crate::sync::remote::Remote;

pub struct Pusher {
    remote: Remote,
}

impl Pusher {
    pub fn new(remote: Remote) -> Self {
        Self { remote }
    }

    pub async fn start(&self) {
        // simplified loop
        // In real impl, this listens to Fjall events
        println!("Pusher: Started. (Stub)");
        
        // Simulate one upload
        let _ = self.remote.upload_sst(1, b"sst-data").await;
    }
}
