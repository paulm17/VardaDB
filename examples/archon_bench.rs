//! ArchonDB absolute-timing benchmark.
//!
//! Runs the production read entry points (`scan_nodes`, `count_nodes`,
//! `resolve_list`) plus end-to-end GraphQL queries against a copy of the real
//! 34M-row ArchonDB database in `varda_db_data/`. Single-path measurement of
//! the current planner-first pipeline — there is deliberately no legacy/planner
//! toggle.
//!
//! Usage: `cargo run --release --example archon_bench`

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata, Resolver};
use vardadb::engine::schema::Schema;
use vardadb::storage::backend::Storage;

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
const DATA_DIR: &str = "varda_db_data";
const DB_NAME: &str = "archondb";
const SDL_PATH: &str =
    "/Volumes/Data/Users/paul/development/src/github/archon/db_data/archondb_schema.graphql";
const WARMUP_RUNS: usize = 2;
const TIMED_RUNS: usize = 10;

struct Row {
    group: &'static str,
    name: String,
    rows: usize,
    times_ms: Vec<f64>,
}

impl Row {
    fn stats(&self) -> (f64, f64, f64, f64) {
        let mut t = self.times_ms.clone();
        t.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = t.len();
        let mean = t.iter().sum::<f64>() / n as f64;
        (
            t[0],
            t[n / 2],
            mean,
            t[n - 1],
        )
    }
}

fn gql(
    rt: &tokio::runtime::Runtime,
    schema: &Schema,
    resolver: &SqliteResolver,
    query: &str,
) -> serde_json::Value {
    let res = rt.block_on(schema.execute_with_resolver(query, Box::new(resolver.clone())));
    let parsed: serde_json::Value = serde_json::from_str(&res)
        .unwrap_or_else(|e| panic!("unparseable response for {query}: {e} — {res}"));
    if let Some(errors) = parsed.get("errors") {
        panic!("GraphQL errors for {query}: {errors}");
    }
    parsed
}

fn main() {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    println!("== ArchonDB benchmark (warmup {WARMUP_RUNS} + {TIMED_RUNS} timed runs each) ==");

    // ------------------------------------------------------------------
    // Open storage and register the copied database namespace.
    // ------------------------------------------------------------------
    let data_dir = format!("{MANIFEST_DIR}/{DATA_DIR}");
    let db_path = format!("{data_dir}/{DB_NAME}.db");
    let storage = Arc::new(Storage::new(&data_dir, None).expect("open storage"));
    if let Err(e) = storage.create_database_with_path(DB_NAME, Some(db_path.clone())) {
        let msg = e.to_string();
        let already = msg.contains("exists") || msg.contains("Already");
        assert!(already, "register {DB_NAME}: {msg}");
        println!("namespace '{DB_NAME}' already registered");
    }
    let resolver = SqliteResolver::new(storage.clone(), DB_NAME);

    // ------------------------------------------------------------------
    // GraphQL schema for end-to-end workloads + value discovery.
    // ------------------------------------------------------------------
    let sdl = std::fs::read_to_string(SDL_PATH).expect("read archon schema sdl");
    let schema = Schema::load_from_sdl(&sdl).expect("load sdl");

    // Discover real values so every workload hits live data.
    let books = gql(&rt, &schema, &resolver, r#"{ queryBook { uid code nameEn } }"#);
    let book_arr = books["data"]["queryBook"].as_array().cloned().unwrap_or_default();
    assert!(!book_arr.is_empty(), "no Book rows found");
    let book_code = book_arr[0]["code"].as_str().unwrap().to_string();
    let book_name_word = book_arr[0]["nameEn"]
        .as_str()
        .unwrap()
        .split_whitespace()
        .find(|w| w.len() >= 4)
        .unwrap_or("name")
        .to_string();

    let langs = gql(&rt, &schema, &resolver, r#"{ queryLanguage { uid code } }"#);
    let lang_code = langs["data"]["queryLanguage"][0]["code"]
        .as_str()
        .unwrap()
        .to_string();

    let chapters = gql(
        &rt,
        &schema,
        &resolver,
        r#"{ queryChapter(first: 2, sort: { number: ASC }) { uid number } }"#,
    );
    let ch0 = chapters["data"]["queryChapter"][0]["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let ch0_num: i64 = chapters["data"]["queryChapter"][0]["number"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| {
            chapters["data"]["queryChapter"][0]["number"]
                .as_i64()
        })
        .expect("chapter number");

    let chunks = gql(&rt, &schema, &resolver, r#"{ queryChunk(first: 2) { text chunkType } }"#);
    let chunk_text = chunks["data"]["queryChunk"][0]["text"]
        .as_str()
        .unwrap_or_default();
    let chunk_term = chunk_text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .find(|w| w.chars().count() >= 5)
        .unwrap_or("light")
        .to_string();
    println!(
        "discovered: book_code={book_code:?} lang={lang_code:?} chapter={ch0} num={ch0_num} chunk_term={chunk_term:?}"
    );

    // First verse under the discovered chapter feeds the backref workload.
    let verses_of_ch0 = resolver
        .resolve_list(ch0.parse::<u64>().unwrap(), "verses", HashMap::new(), HashMap::new(), None, None, None, None)
        .expect("resolve chapter verses");
    let verse_uid = *verses_of_ch0.first().expect("chapter has verses");

    let meta = metadata_map();

    let mut rows: Vec<Row> = Vec::new();
    macro_rules! bench {
        ($group:literal, $name:expr, $f:expr) => {{
            let f = $f;
            // Warmup + timed.
            let mut last_rows = 0usize;
            for _ in 0..WARMUP_RUNS {
                last_rows = f();
            }
            let mut times = Vec::with_capacity(TIMED_RUNS);
            for _ in 0..TIMED_RUNS {
                let start = Instant::now();
                last_rows = f();
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            rows.push(Row {
                group: $group,
                name: $name.to_string(),
                rows: last_rows,
                times_ms: times,
            });
        }};
    }

    // A. Unique / point lookups ------------------------------------------
    bench!("A", "unique lookup Book.code", || {
        resolver
            .scan_nodes(
                "Book",
                gql_filter_eq("code", &book_code),
                HashMap::new(),
                None,
                None,
                None,
                &["code".to_string()],
                None,
                &meta,
            )
            .len()
    });
    bench!("A", "unique lookup Language.code", || {
        resolver
            .scan_nodes(
                "Language",
                gql_filter_eq("code", &lang_code),
                HashMap::new(),
                None,
                None,
                None,
                &["code".to_string()],
                None,
                &meta,
            )
            .len()
    });

    // B. Indexed scalar equality -----------------------------------------
    bench!("B", "filter Chapter.number eq", || {
        resolver
            .scan_nodes(
                "Chapter",
                gql_filter_num("number", "eq", ch0_num),
                HashMap::new(),
                None,
                None,
                None,
                &[],
                None,
                &meta,
            )
            .len()
    });

    // C. Counts -----------------------------------------------------------
    bench!("C", "count all Chapters (fast path)", || {
        resolver.count_nodes("Chapter", HashMap::new(), &[], None, &meta)
    });
    bench!("C", "count all Verses (fast path)", || {
        resolver.count_nodes("Verse", HashMap::new(), &[], None, &meta)
    });
    bench!("C", "count Verses filtered (pipeline)", || {
        resolver.count_nodes(
            "Verse",
            gql_filter_num("number", "eq", 1),
            &[],
            None,
            &meta,
        )
    });

    // D. Sort + pagination -------------------------------------------------
    bench!("D", "sort Books nameEn ASC first 3", || {
        let mut sort = HashMap::new();
        sort.insert("nameEn".to_string(), async_graphql::Value::String("ASC".into()));
        resolver
            .scan_nodes(
                "Book",
                HashMap::new(),
                sort,
                Some(3),
                None,
                None,
                &["code".to_string()],
                None,
                &meta,
            )
            .len()
    });
    bench!("D", "sort Chapters number DESC first 10", || {
        let mut sort = HashMap::new();
        sort.insert("number".to_string(), async_graphql::Value::String("DESC".into()));
        resolver
            .scan_nodes(
                "Chapter",
                HashMap::new(),
                sort,
                Some(10),
                None,
                None,
                &[],
                None,
                &meta,
            )
            .len()
    });

    // E. Text search --------------------------------------------------------
    bench!("E", "fulltext search Chunk.text alloftext", || {
        let mut inner = async_graphql::indexmap::IndexMap::new();
        inner.insert(
            async_graphql::Name::new("alloftext"),
            async_graphql::Value::String(chunk_term.clone()),
        );
        let cond = async_graphql::Value::Object(inner);
        let mut filter = HashMap::new();
        filter.insert("text".to_string(), cond);
        resolver
            .scan_nodes(
                "Chunk",
                filter,
                HashMap::new(),
                None,
                None,
                None,
                &[],
                None,
                &meta,
            )
            .len()
    });
    bench!("E", "term search Book.nameEn anyofterms", || {
        let mut inner = async_graphql::indexmap::IndexMap::new();
        inner.insert(
            async_graphql::Name::new("anyofterms"),
            async_graphql::Value::String(book_name_word.clone()),
        );
        let cond = async_graphql::Value::Object(inner);
        let mut filter = HashMap::new();
        filter.insert("nameEn".to_string(), cond);
        resolver
            .scan_nodes(
                "Book",
                filter,
                HashMap::new(),
                None,
                None,
                None,
                &["code".to_string()],
                None,
                &meta,
            )
            .len()
    });

    // F. Relation edges ------------------------------------------------------
    bench!("F", "edge fetch Chapter->verses", || {
        resolver
            .resolve_list(
                ch0.parse::<u64>().unwrap(),
                "verses",
                HashMap::new(),
                HashMap::new(),
                None,
                None,
                None,
                None,
            )
            .expect("verses")
            .len()
    });
    bench!("F", "edge backref Verse->chapter", || {
        resolver
            .resolve_list(verse_uid, "chapter", HashMap::new(), HashMap::new(), None, None, None, None)
            .expect("chapter")
            .len()
    });

    // G. End-to-end GraphQL (nested traversal) -------------------------------
    bench!("G", "graphql BookTranslation{chapters{verses}}", || {
        let res = gql(
            &rt,
            &schema,
            &resolver,
            r#"{ queryBookTranslation(first: 2) { chapters(first: 1) { verses(first: 10) { number } } } }"#,
        );
        res["data"]["queryBookTranslation"].as_array().map(|a| a.len()).unwrap_or(0)
    });
    bench!("G", "graphql Verse filter+first", || {
        let res = gql(
            &rt,
            &schema,
            &resolver,
            r#"{ queryVerse(filter: { number: { eq: 1 } }, first: 20) { number } }"#,
        );
        res["data"]["queryVerse"].as_array().map(|a| a.len()).unwrap_or(0)
    });

    // H. Aggregations ----------------------------------------------------------
    bench!("H", "count Chapters filtered gt", || {
        let mut inner = async_graphql::indexmap::IndexMap::new();
        inner.insert(async_graphql::Name::new("gt"), async_graphql::Value::Number(5.into()));
        let cond = async_graphql::Value::Object(inner);
        let mut filter = HashMap::new();
        filter.insert("number".to_string(), cond);
        resolver.count_nodes("Chapter", filter, &[], None, &meta)
    });

    // Vector probe (best-effort): usearch side-files are not namespaced, so
    // this exercises the default backend only. Reported, never fatal.
    let probe_dims = std::fs::read_to_string(format!("{data_dir}/{DB_NAME}_vectors.dims"))
        .ok()
        .and_then(|d| d.trim().parse::<usize>().ok());
    match probe_dims {
        Some(dims) => {
            let q = vec![0.0f64; dims];
            let start = Instant::now();
            let hits = resolver.search_vectors(&q, 5);
            println!(
                "vector probe: dims={dims} hits={} took {:.1} ms (default backend)",
                hits.len(),
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        None => println!("vector probe: skipped — no {DB_NAME}_vectors.dims readable in {data_dir}"),
    }

    print_table(&rows);
    write_markdown(&rows, &chunk_term);
}

fn gql_filter_eq(field: &str, value: &str) -> HashMap<String, async_graphql::Value> {
    let mut inner = async_graphql::indexmap::IndexMap::new();
    inner.insert(async_graphql::Name::new("eq"), async_graphql::Value::String(value.to_string()));
    let mut filter = HashMap::new();
    filter.insert(field.to_string(), async_graphql::Value::Object(inner));
    filter
}

fn gql_filter_num(field: &str, op: &str, value: i64) -> HashMap<String, async_graphql::Value> {
    let mut inner = async_graphql::indexmap::IndexMap::new();
    inner.insert(async_graphql::Name::new(op), async_graphql::Value::Number(value.into()));
    let mut filter = HashMap::new();
    filter.insert(field.to_string(), async_graphql::Value::Object(inner));
    filter
}

/// Minimal catalog covering the types the bench queries. Relation edges go
/// through `resolve_list`, which does not consult metadata; unique lists feed
/// candidate planning for the point lookups.
fn metadata_map() -> HashMap<String, QueryTypeMetadata> {
    let inv = |field: &str, itype: &str, ifield: &str| InverseInfo {
        field: field.to_string(),
        inverse_type: itype.to_string(),
        inverse_field: ifield.to_string(),
        inverse_is_list: true,
    };
    [
        ("Book", vec!["code".to_string()], vec![], HashMap::new()),
        ("Language", vec!["code".to_string()], vec![], HashMap::new()),
        ("Category", vec![], vec![], HashMap::new()),
        ("Translations", vec![], vec![inv("books", "Book", "translation")], HashMap::from([("books".to_string(), "Book".to_string())])),
        ("BookTranslation", vec![], vec![inv("chapters", "Chapter", "bookTranslation")], HashMap::from([("chapters".to_string(), "Chapter".to_string())])),
        ("Chapter", vec![], vec![inv("verses", "Verse", "chapter")], HashMap::from([("verses".to_string(), "Verse".to_string())])),
        ("Verse", vec![], vec![], HashMap::new()),
        ("Chunk", vec![], vec![], HashMap::new()),
    ]
    .into_iter()
    .map(|(name, uniques, inverses, relations)| {
        (
            name.to_string(),
            QueryTypeMetadata {
                uniques,
                inverses,
                relations,
            },
        )
    })
    .collect()
}

fn print_table(rows: &[Row]) {
    println!();
    println!(
        "{:<6} {:<44} {:>8} {:>10} {:>10} {:>10} {:>10}",
        "group", "workload", "rows", "min ms", "p50 ms", "mean ms", "max ms"
    );
    println!("{}", "-".repeat(102));
    for row in rows {
        let (min, p50, mean, max) = row.stats();
        println!(
            "{:<6} {:<44} {:>8} {:>10.2} {:>10.2} {:>10.2} {:>10.2}",
            row.group, row.name, row.rows, min, p50, mean, max
        );
    }
}

fn write_markdown(rows: &[Row], _chunk_term: &str) {
    let out_dir = format!("{MANIFEST_DIR}/docs");
    std::fs::create_dir_all(&out_dir).ok();
    let mut md = String::new();
    md.push_str("# ArchonDB Benchmark Results\n\n");
    md.push_str("Absolute timings of the planner-first pipeline against a copy of ");
    md.push_str("the production ArchonDB database (34.4M key-value rows; Verse=76k, Chapter=1.8k, ");
    md.push_str("Chunk=25k). Single-path measurement — no legacy/planner toggle.\n\n");
    md.push_str("## Methodology\n\n");
    md.push_str(&format!(
        "- Harness: `examples/archon_bench.rs` (`cargo run --release --example archon_bench`)\n\
         - Per workload: {WARMUP_RUNS} warmup runs, then {TIMED_RUNS} timed runs (`std::time::Instant`)\n\
         - Stats: min / p50 / mean / max in milliseconds; rows = result size of last timed run\n\
         - Database: repo-local copy at `{DATA_DIR}/{DB_NAME}.db` (SQLite KV store, FTS inside the .db)\n\
         - Values used in filters were discovered from the live database before timing\n\n"
    ));
    md.push_str("## Results\n\n");
    md.push_str("| group | workload | rows | min ms | p50 ms | mean ms | max ms |\n");
    md.push_str("|---|---|---:|---:|---:|---:|---:|\n");
    for row in rows {
        let (min, p50, mean, max) = row.stats();
        md.push_str(&format!(
            "| {} | {} | {} | {:.2} | {:.2} | {:.2} | {:.2} |\n",
            row.group, row.name, row.rows, min, p50, mean, max
        ));
    }
    md.push_str("\n## Notes\n\n");
    md.push_str("- Historical `log.txt` baselines (Chapter ~2590 ms / Verse ~7780 ms candidate_ms) predate the planner migration and ran on different hardware/software state — context only, not a same-machine comparison.\n");
    md.push_str("- The upstream `archondb_tantivy/` directory is not read by VardaDB (text search uses SQLite FTS tables inside the .db) and was therefore not copied.\n");
    md.push_str("- Text-search workloads (group E) return 0 rows on this dataset: the in-db FTS5 tables (`fts_data`, `fts_term_data`) are present but EMPTY — archon indexed text externally via tantivy. Timings therefore reflect the empty-index no-op path, not real text-search throughput.\n");
    md.push_str("- Vector KNN uses the non-namespaced default backend; side-file `.usearch` data was probed best-effort and excluded from the timed matrix unless wired.\n");

    let path = format!("{out_dir}/bench_results.md");
    std::fs::write(&path, md).expect("write bench_results.md");
    println!("\nwrote {path}");
}
