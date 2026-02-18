//! Million Todo Benchmark
//!
//! Benchmarks VardaDB insert performance with a simple Todo schema.
//! Run with: cargo bench --bench million_todos

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tempfile::TempDir;

// Schema for Todo type
const TODO_SCHEMA: &str = r#"
type Todo {
    id: ID!
    title: String!
    completed: Boolean!
    priority: Int!
    createdAt: String!
}
"#;

/// Generates a batch of Todo items as field maps
fn generate_todos(count: usize, start_id: usize) -> Vec<HashMap<String, serde_json::Value>> {
    (0..count)
        .map(|i| {
            let id = start_id + i;
            let mut fields = HashMap::new();
            fields.insert("title".to_string(), serde_json::json!(format!("Todo item {}", id)));
            fields.insert("completed".to_string(), serde_json::json!(id % 2 == 0));
            fields.insert("priority".to_string(), serde_json::json!((id % 5) as i64 + 1));
            fields.insert("createdAt".to_string(), serde_json::json!(format!("2024-01-{:02}T12:00:00Z", (id % 28) + 1)));
            fields
        })
        .collect()
}

/// Simple insert benchmark (no VardaDB, just measures generation + serialization overhead)
fn bench_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("todo_generation");
    
    for count in [1000, 10_000, 100_000].iter() {
        group.throughput(Throughput::Elements(*count as u64));
        group.bench_with_input(BenchmarkId::new("generate", count), count, |b, &count| {
            b.iter(|| {
                let todos = generate_todos(count, 0);
                assert_eq!(todos.len(), count);
            });
        });
    }
    
    group.finish();
}

/// Measures raw GraphQL serialization overhead
fn bench_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("todo_serialization");
    
    let todos_1k = generate_todos(1000, 0);
    
    group.throughput(Throughput::Elements(1000));
    group.bench_function("serialize_1k", |b| {
        b.iter(|| {
            for todo in &todos_1k {
                let _ = serde_json::to_vec(todo).unwrap();
            }
        });
    });
    
    group.finish();
}

/// Summary benchmark that prints stats to stdout (for quick testing)
fn bench_summary(c: &mut Criterion) {
    let mut group = c.benchmark_group("summary");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));
    
    group.bench_function("generate_and_serialize_10k", |b| {
        b.iter(|| {
            let todos = generate_todos(10_000, 0);
            let mut total_bytes = 0usize;
            for todo in &todos {
                total_bytes += serde_json::to_vec(todo).unwrap().len();
            }
            total_bytes
        });
    });
    
    group.finish();
}

/// Quick manual benchmark for timing large batches (not using criterion)
/// Run with: cargo bench --bench million_todos -- --nocapture
#[allow(dead_code)]
fn manual_million_test() {
    println!("\n=== Manual 1M Todo Generation Benchmark ===\n");
    
    let batch_sizes = [10_000, 100_000, 1_000_000];
    
    for count in batch_sizes {
        let start = Instant::now();
        let todos = generate_todos(count, 0);
        let gen_time = start.elapsed();
        
        let start = Instant::now();
        let mut total_bytes = 0usize;
        for todo in &todos {
            total_bytes += serde_json::to_vec(todo).unwrap().len();
        }
        let ser_time = start.elapsed();
        
        println!("{}:", count);
        println!("  Generation: {:?} ({:.0} items/sec)", gen_time, count as f64 / gen_time.as_secs_f64());
        println!("  Serialization: {:?} ({:.0} items/sec)", ser_time, count as f64 / ser_time.as_secs_f64());
        println!("  Total bytes: {} MB", total_bytes / 1_000_000);
        println!();
    }
}

criterion_group!(benches, bench_generation, bench_serialization, bench_summary);
criterion_main!(benches);

// Uncomment to run manual test:
// fn main() { manual_million_test(); }
