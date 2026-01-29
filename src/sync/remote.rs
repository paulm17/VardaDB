#[derive(Clone)]
pub struct Remote {
    pub url: Option<String>,
}

impl Remote {
    pub fn new(url: Option<String>) -> Self {
        Self { url }
    }

    pub async fn upload_sst(&self, _id: u64, _data: &[u8]) -> Result<(), String> {
        // Mock upload
        println!("Remote: Uploading SST to {:?}", self.url);
        Ok(())
    }
}
