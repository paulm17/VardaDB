
#[cfg(test)]
mod tests {
    use super::super::store::*;
    use super::super::config::*;
    use crate::storage::sqlite_backend::{SqliteBackend, SqliteTable};
    use std::sync::Arc;

    #[test]
    fn test_vector_insert_and_search() -> anyhow::Result<()> {
        let path = tempfile::tempdir()?;
        let backend = Arc::new(SqliteBackend::new(path.path())?);
        backend.create_table("vectors")?;
        let table = SqliteTable::new("vectors".to_string(), backend);
        
        let config = HNSWConfig::new(Some(5), Some(16), Some(16), None);
        let store = VectorStore::new(table, config);

        // 1. Insert 3 vectors
        // A: [1.0, 0.0]
        // B: [0.9, 0.1]
        // C: [0.0, 1.0]
        
        // A and B are close. A and C are far.
        
        store.insert(1, vec![1.0, 0.0])?;
        store.insert(2, vec![0.9, 0.1])?;
        store.insert(3, vec![0.0, 1.0])?;

        // 2. Search for close to A
        let results = store.search(&[1.0, 0.05], 2)?;
        
        println!("Search Results (Close to A): {:?}", results);
        
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 1); // Should be A (closest)
        assert_eq!(results[1].0, 2); // Should be B (second closest)
        
        // 3. Search for close to C
        let results_c = store.search(&[0.1, 0.9], 1)?;
        println!("Search Results (Close to C): {:?}", results_c);
        assert_eq!(results_c[0].0, 3);

        Ok(())
    }
}
