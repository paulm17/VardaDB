
use vardadb::storage::backend::Storage;

#[test]
fn test_storage_vector_integration() -> anyhow::Result<()> {
    let path = tempfile::tempdir()?;
    let storage = Storage::new(path.path(), Some(1))?;

    // 1. Insert 3 Nodes with Vectors
    // Node 1: [1.0, 0.0]
    // Node 2: [0.9, 0.1]
    // Node 3: [0.0, 1.0]
    
    storage.put_vector(1, vec![1.0, 0.0])?;
    storage.put_vector(2, vec![0.9, 0.1])?;
    storage.put_vector(3, vec![0.0, 1.0])?;

    // 2. Search for close to Node 1
    let results = storage.search_vectors(&[1.0, 0.05], 2)?;
    
    println!("Integration Search Results: {:?}", results);
    
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, 1);
    assert_eq!(results[1].0, 2);

    Ok(())
}
