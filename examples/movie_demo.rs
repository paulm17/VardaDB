use clap::{Parser, Subcommand};
use reqwest::Client;
use serde_json::json;
use std::time::Instant;

#[derive(Parser)]
#[command(name = "varda-movie-demo")]
#[command(about = "CLI to demonstrate VardaDB Movie Graph capabilities")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long, default_value = "http://localhost:9000/graphql")]
    url: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Load the Schema and Generate Synthetic Data
    Load {
        #[arg(long, default_value = "1000")]
        movies: usize,
    },
    /// Benchmark Read Performance (Cold vs Cached)
    Benchmark,
    /// Run Fulltext Search on Movie Plots
    Search {
        query: String,
    },
    /// Run Geo-Spatial Query (Find movies near coordinates)
    Geo {
        #[arg(long)]
        lat: f64,
        #[arg(long)]
        lon: f64,
    },
    /// Run Complex Graph Traversal
    Graph,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let client = Client::new();

    match cli.command {
        Commands::Load { movies } => load_data(&client, &cli.url, movies).await?,
        Commands::Benchmark => run_benchmark(&client, &cli.url).await?,
        Commands::Search { query } => run_search(&client, &cli.url, &query).await?,
        Commands::Geo { lat, lon } => run_geo(&client, &cli.url, lat, lon).await?,
        Commands::Graph => run_graph(&client, &cli.url).await?,
    }

    Ok(())
}

async fn load_data(client: &Client, url: &str, count: usize) -> Result<(), Box<dyn std::error::Error>> {
    println!("Loading Movie Schema...");
    let schema = r#"
type Movie {
    title: String @search(by: [term, exact])
    plot: String @search(by: [fulltext])
    released: Int @search(by: [int])
    rating: Float @search(by: [float])
    genre: [String] @search(by: [term])
    director: Person @hasInverse(field: "directed")
    cast: [Person] @hasInverse(field: "actedIn")
    locations: [Location] @hasInverse(field: "movies")
    reviews: [Review] @hasInverse(field: "movie")
}
type Person {
    name: String @search(by: [term, exact])
    actedIn: [Movie]
    directed: [Movie]
}
type Location {
    name: String
    coordinates: GeoPoint @search(by: [geo])
    movies: [Movie]
}
type Review {
    text: String @search(by: [fulltext])
    rating: Int @search(by: [int])
    movie: Movie
}
    "#;

    // Use Admin API to update Schema
    let admin_url = url.replace("/graphql", "/admin/schema");
    let res = client.post(&admin_url)
        .body(schema)
        .send()
        .await?;
    
    if !res.status().is_success() {
        return Err(format!("Failed to update schema: {}", res.text().await?).into());
    }
    println!("Schema Updated Successfully.");

    println!("Generating {} movies...", count);
    
    // Batch create in chunks of 50
    let chunk_size = 50;
    for i in (0..count).step_by(chunk_size) {
        let mut mutations = String::new();
        for j in 0..chunk_size {
            if i + j >= count { break; }
            let idx = i + j;
            let title = format!("Movie {}", idx);
            let plot = if idx % 2 == 0 { "Alien invasion thriller" } else { "Romantic comedy in Paris" };
            mutations.push_str(&format!(r#"
                m{}: createMovie(input: {{ 
                    title: "{}", 
                    plot: "{}", 
                    released: {}, 
                    rating: {},
                    director: {{ name: "Director {}" }},
                    cast: [ {{ name: "Actor {}" }} ],
                    locations: [ {{ 
                        name: "Loc {}", 
                        coordinates: {{ latitude: 34.05, longitude: -118.25 }} 
                    }} ]
                }}) {{ uid }}
            "#, idx, title, plot, 1990 + (idx % 30), (idx % 10) as f64, idx, idx, idx));
        }

        let query = format!("mutation {{ {} }}", mutations);
        let res = client.post(url).json(&json!({ "query": query })).send().await?;
        if !res.status().is_success() {
             eprintln!("Batch failed: {}", res.text().await?);
        } else {
             let body: serde_json::Value = res.json().await?;
             if let Some(errs) = body.get("errors") {
                 eprintln!("GraphQL Errors: {}", serde_json::to_string_pretty(errs)?);
             } else {
                 print!(".");
             }
        }
    }
    println!("\nData Load Complete.");
    Ok(())
}

async fn run_benchmark(client: &Client, url: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n--- Query Caching Benchmark ---");
    
    // 1. Cold Query
    let query = r#"
        query Bench {
            queryMovie(filter: { title: { alloftext: "Movie" } }, first: 100) {
                title
                director { name }
                cast { name }
                locations { name }
            }
        }
    "#;

    let start = Instant::now();
    let res = client.post(url).json(&json!({ "query": query })).send().await?;
    let _json: serde_json::Value = res.json().await?;
    let duration = start.elapsed();
    println!("Cold Query Latency:  {:?}", duration);

    // 2. Hot Query
    let start = Instant::now();
    let res = client.post(url).json(&json!({ "query": query })).send().await?;
    let _json: serde_json::Value = res.json().await?;
    let duration = start.elapsed();
    println!("Cached Query Latency: {:?}", duration);

    if duration.as_micros() < 1000 {
        println!("✅ Cache HIT confirmed (< 1ms)");
    } else {
        println!("⚠️ Cache MISS (Latency > 1ms). Did caching work?");
    }

    // 3. Invalidation Test
    println!("\nPerforming Mutation to invalidate cache...");
    let mutation = r#"mutation { createMovie(input: { title: "Invalidator" }) { uid } }"#;
    client.post(url).json(&json!({ "query": mutation })).send().await?;

    println!("Re-running Query (Should be Cold)...");
    let start = Instant::now();
    let res = client.post(url).json(&json!({ "query": query })).send().await?;
    let _json: serde_json::Value = res.json().await?;
    let duration = start.elapsed();
    println!("New Cold Latency:    {:?}", duration);

    Ok(())
}

async fn run_search(client: &Client, url: &str, term: &str) -> Result<(), Box<dyn std::error::Error>> {
    let query = format!(r#"
        query {{
            queryMovie(filter: {{ plot: {{ alloftext: "{}" }} }}) {{
                title
                plot
            }}
        }}
    "#, term);

    let res = client.post(url).json(&json!({ "query": query })).send().await?;
    let json: serde_json::Value = res.json().await?;
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

async fn run_geo(client: &Client, url: &str, lat: f64, lon: f64) -> Result<(), Box<dyn std::error::Error>> {
    let query = format!(r#"
        query {{
            queryLocation(filter: {{ 
                coordinates: {{ near: {{ coordinate: {{ latitude: {}, longitude: {} }}, distance: 500000.0 }} }} 
            }}) {{
                name
                coordinates {{ latitude longitude }}
                movies {{ title }}
            }}
        }}
    "#, lat, lon);
    
    let res = client.post(url).json(&json!({ "query": query })).send().await?;
    let json: serde_json::Value = res.json().await?;
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}

async fn run_graph(client: &Client, url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let query = r#"
        query {
            queryPerson(first: 5) {
                name
                actedIn {
                    title
                    director {
                        name
                        directed {
                           title
                        }
                    }
                }
            }
        }
    "#;
    let res = client.post(url).json(&json!({ "query": query })).send().await?;
    let json: serde_json::Value = res.json().await?;
    println!("{}", serde_json::to_string_pretty(&json)?);
    Ok(())
}
