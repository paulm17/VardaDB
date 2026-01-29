use fjall::{Config, Keyspace, PartitionCreateOptions};
use std::path::Path;


pub struct Storage {
    pub keyspace: Keyspace,
    pub main_partition: fjall::Partition,
}

impl Storage {
    pub fn new(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let config = Config::new(path);
        let keyspace = config.open()?;
        
        // Open the default partition for storing our data
        let main_partition = keyspace.open_partition("main", PartitionCreateOptions::default())?;

        Ok(Self {
            keyspace,
            main_partition,
        })
    }

    pub fn get(&self, key: &[u8]) -> anyhow::Result<Option<Vec<u8>>> {
        let val = self.main_partition.get(key)?;
        Ok(val.map(|v| v.to_vec()))
    }

    pub fn insert(&self, key: &[u8], value: &[u8]) -> anyhow::Result<()> {
        self.main_partition.insert(key, value)?;
        Ok(())
    }

    pub fn remove(&self, key: &[u8]) -> anyhow::Result<()> {
        self.main_partition.remove(key)?;
        Ok(())
    }
    
    pub fn contains_key(&self, key: &[u8]) -> anyhow::Result<bool> {
        Ok(self.main_partition.contains_key(key)?)
    }

    // For transactions in the future
    pub fn flush(&self) -> anyhow::Result<()> {
        self.keyspace.persist(fjall::PersistMode::SyncAll)?;
        Ok(())
    }
}
