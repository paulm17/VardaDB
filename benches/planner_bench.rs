//! Planner regression benchmark suite (spec §Benchmark And Regression Plan).
//!
//! Criterion scenarios over a copy of the production ArchonDB database,
//! mapping the ten baseline workload families defined in
//! `query_planner.md`. Baselines are captured and compared with:
//!
//!   cargo bench --bench planner_bench -- --save-baseline main
//!   cargo bench --bench planner_bench -- --baseline main
//!
//! Regression rules enforced by review (see docs/bench_baselines.md):
//! - any scenario regressing >30% vs baseline fails the gate
//! - cold-path scenarios (first-touch caches) allow up to 2x jitter
//!
//! The suite skips gracefully when `varda_db_data/archondb.db` or the Archon
//! schema SDL is absent so CI machines stay green.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use criterion::{criterion_group, criterion_main, Criterion};
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::engine::resolver::{InverseInfo, QueryTypeMetadata, Resolver};
use vardadb::engine::schema::Schema;
use vardadb::query_planner::function::aggregate::default_aggregate_registry;
use vardadb::query_planner::ir::{FieldPath, FieldSegment, LogicalExpr};
use vardadb::query_planner::operators::aggregate::{AggregateSpec, HashAggregateOperator};
use vardadb::query_planner::operators::{ExecContext, ExecOperator, FlowResult, FullTypeScan};
use vardadb::query_planner::physical_expr::compile_arc;
use vardadb::query_planner::runtime_for;
use vardadb::storage::backend::Storage;

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");
const DATA_DIR: &str = "varda_db_data";
const DB_NAME: &str = "archondb";
const SDL_PATH: &str =
    "/Volumes/Data/Users/paul/development/src/github/archon/db_data/archondb_schema.graphql";

struct Fixture {
    rt: tokio::runtime::Runtime,
    schema: Schema,
    resolver: SqliteResolver,
    #[allow(dead_code)]
    metadata: HashMap<String, QueryTypeMetadata>,
    book_code: String,
    lang_code: String,
    ch0: String,
    ch0_num: i64,
    chunk_term: String,
}

static FIXTURE: OnceLock<Option<Arc<Fixture>>> = OnceLock::new();

fn fixture() -> Option<&'static Arc<Fixture>> {
    FIXTURE.get_or_init(|| open_fixture().map(Arc::new)).as_ref()
}

fn open_fixture() -> Option<Fixture> {
    let data_dir = format!("{MANIFEST_DIR}/{DATA_DIR}");
    let db_path = format!("{data_dir}/{DB_NAME}.db");
    let sdl_path = SDL_PATH.to_string();
    if !std::path::Path::new(&db_path).exists() || !std::path::Path::new(&sdl_path).exists() {
        return None;
    }

    let storage = Arc::new(Storage::new(&data_dir, None).expect("open storage"));
    if let Err(e) = storage.create_database_with_path(DB_NAME, Some(db_path.clone())) {
        let msg = e.to_string();
        assert!(
            msg.contains("exists") || msg.contains("Already"),
            "register {DB_NAME}: {msg}"
        );
    }
    let resolver = SqliteResolver::new(storage.clone(), DB_NAME);
    let sdl = std::fs::read_to_string(&sdl_path).ok()?;
    let schema = Schema::load_from_sdl(&sdl).expect("load sdl");
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let gql = |rt: &tokio::runtime::Runtime,
               schema: &Schema,
               resolver: &SqliteResolver,
               query: &str|
     -> serde_json::Value {
        let res = rt.block_on(schema.execute_with_resolver(query, Box::new(resolver.clone())));
        serde_json::from_str(&res)
            .unwrap_or_else(|e| panic!("unparseable response for {query}: {e} — {res}"))
    };

    let books = gql(&rt, &schema, &resolver, r#"{ queryBook(first: 1) { code } }"#);
    let book_code = books["data"]["queryBook"][0]["code"]
        .as_str()
        .expect("book code")
        .to_string();
    let langs = gql(&rt, &schema, &resolver, r#"{ queryLanguage(first: 1) { code } }"#);
    let lang_code = langs["data"]["queryLanguage"][0]["code"]
        .as_str()
        .expect("language code")
        .to_string();
    let chapters = gql(
        &rt,
        &schema,
        &resolver,
        r#"{ queryChapter(first: 1, sort: { number: ASC }) { uid number } }"#,
    );
    let ch0 = chapters["data"]["queryChapter"][0]["uid"]
        .as_str()
        .expect("chapter uid")
        .to_string();
    let ch0_num = chapters["data"]["queryChapter"][0]["number"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| chapters["data"]["queryChapter"][0]["number"].as_i64())
        .expect("chapter number");
    let chunks = gql(&rt, &schema, &resolver, r#"{ queryChunk(first: 1) { text } }"#);
    let chunk_term = chunks["data"]["queryChunk"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .find(|w| w.chars().count() >= 5)
        .unwrap_or("light")
        .to_string();

    Some(Fixture {
        rt,
        schema,
        resolver,
        metadata: metadata_map(),
        book_code,
        lang_code,
        ch0,
        ch0_num,
        chunk_term,
    })
}

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
        (
            "Translations",
            vec![],
            vec![inv("books", "Book", "translation")],
            HashMap::from([("books".to_string(), "Book".to_string())]),
        ),
        (
            "BookTranslation",
            vec![],
            vec![inv("chapters", "Chapter", "bookTranslation")],
            HashMap::from([("chapters".to_string(), "Chapter".to_string())]),
        ),
        (
            "Chapter",
            vec![],
            vec![inv("verses", "Verse", "chapter")],
            HashMap::from([("verses".to_string(), "Verse".to_string())]),
        ),
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

fn gql_filter_eq(field: &str, value: &str) -> HashMap<String, async_graphql::Value> {
    let mut inner = async_graphql::indexmap::IndexMap::new();
    inner.insert(
        async_graphql::Name::new("eq"),
        async_graphql::Value::String(value.to_string()),
    );
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

fn gql_nested_num(rel: &str, field: &str, op: &str, value: i64) -> HashMap<String, async_graphql::Value> {
    let mut inner = async_graphql::indexmap::IndexMap::new();
    inner.insert(async_graphql::Name::new(op), async_graphql::Value::Number(value.into()));
    let mut nested = HashMap::new();
    nested.insert(field.to_string(), async_graphql::Value::Object(inner));
    let mut rel_obj = async_graphql::indexmap::IndexMap::new();
    for (k, v) in nested {
        rel_obj.insert(async_graphql::Name::new(k.as_str()), v);
    }
    let mut filter = HashMap::new();
    filter.insert(rel.to_string(), async_graphql::Value::Object(rel_obj));
    filter
}

fn criterion_benchmark(c: &mut Criterion) {
    let Some(fx) = fixture() else {
        eprintln!(
            "planner_bench: {DATA_DIR}/{DB_NAME}.db or Archon SDL missing — skipping suite"
        );
        return;
    };

    let resolver = &fx.resolver;

    // W1/W2: full scans ---------------------------------------------------
    let mut g = c.benchmark_group("01_full_scan");
    g.throughput(criterion::Throughput::Elements(76_373));
    g.bench_function("verse_unfiltered", |b| {
        b.iter(|| resolver.scan_nodes("Verse", HashMap::new(), HashMap::new(), None, None, None, &[], None, &fx.metadata).len())
    });
    g.finish();

    let mut g = c.benchmark_group("02_unique_get");
    g.bench_function("book_by_code", |b| {
        b.iter(|| {
            resolver.scan_nodes("Book", gql_filter_eq("code", &fx.book_code), HashMap::new(), None, None, None, &["code".to_string()], None, &fx.metadata).len()
        })
    });
    g.bench_function("language_by_code", |b| {
        b.iter(|| {
            resolver.scan_nodes("Language", gql_filter_eq("code", &fx.lang_code), HashMap::new(), None, None, None, &["code".to_string()], None, &fx.metadata).len()
        })
    });
    g.finish();

    // W3: ordered scan + first (ordered-index probe path) -----------------
    let mut g = c.benchmark_group("03_ordered_scan_first");
    g.bench_function("chapter_number_asc_first10", |b| {
        b.iter(|| {
            let mut sort = HashMap::new();
            sort.insert("number".to_string(), async_graphql::Value::String("ASC".into()));
            resolver.scan_nodes("Chapter", HashMap::new(), sort, Some(10), None, None, &[], None, &fx.metadata).len()
        })
    });
    g.finish();

    // W4/W5: nested relation filters ---------------------------------------
    let mut g = c.benchmark_group("04_nested_relation_low");
    g.bench_function("translation_chapters_number_eq", |b| {
        b.iter(|| {
            resolver.scan_nodes("BookTranslation", gql_nested_num("chapters", "number", "eq", fx.ch0_num), HashMap::new(), None, None, None, &[], None, &fx.metadata).len()
        })
    });
    g.finish();

    let mut g = c.benchmark_group("05_nested_relation_wide");
    g.bench_function("chapter_verses_number_eq", |b| {
        b.iter(|| {
            resolver.scan_nodes("Chapter", gql_nested_num("verses", "number", "eq", 1), HashMap::new(), None, None, None, &[], None, &fx.metadata).len()
        })
    });
    g.finish();

    // W6: text search (FTS tables are EMPTY in this dataset — measures the
    // empty-index no-op path; kept so the scenario shape stays covered) ----
    let mut g = c.benchmark_group("06_text_search");
    g.bench_function("chunk_text_alloftext", |b| {
        b.iter(|| {
            let mut inner = async_graphql::indexmap::IndexMap::new();
            inner.insert(
                async_graphql::Name::new("alloftext"),
                async_graphql::Value::String(fx.chunk_term.clone()),
            );
            let mut filter = HashMap::new();
            filter.insert("text".to_string(), async_graphql::Value::Object(inner));
            resolver.scan_nodes("Chunk", filter, HashMap::new(), None, None, None, &[], None, &fx.metadata).len()
        })
    });
    g.finish();

    // W7: vector search — skipped by design: Storage::search_vectors uses
    // the non-namespaced "default" backend; archon vectors live in
    // non-readable side-files. Documented in docs/bench_baselines.md.

    // W8: relation expansion fanout ----------------------------------------
    let mut g = c.benchmark_group("08_edge_fetch");
    g.bench_function("chapter_to_verses", |b| {
        b.iter(|| {
            resolver.resolve_list(fx.ch0.parse::<u64>().unwrap(), "verses", HashMap::new(), HashMap::new(), None, None, None, None).unwrap_or_default().len()
        })
    });
    g.finish();

    // W9: counts -------------------------------------------------------------
    let mut g = c.benchmark_group("09_count");
    g.bench_function("chapters_fast_path", |b| {
        b.iter(|| resolver.count_nodes("Chapter", HashMap::new(), &[], None, &fx.metadata))
    });
    g.bench_function("verses_fast_path", |b| {
        b.iter(|| resolver.count_nodes("Verse", HashMap::new(), &[], None, &fx.metadata))
    });
    // NOTE: chapter numbers repeat per translation (~1..20 each), so
    // thresholds must stay small to match anything in this dataset.
    g.bench_function("chapters_filtered_pipeline", |b| {
        b.iter(|| {
            resolver.count_nodes(
                "Chapter",
                gql_filter_num("number", "ge", 15),
                &[],
                None,
                &fx.metadata,
            )
        })
    });
    g.finish();

    // W10: aggregation (operator level) -------------------------------------
    let mut g = c.benchmark_group("10_aggregate");
    g.bench_function("sum_chapter_number", |b| {
        b.iter(|| {
            let rt = runtime_for(resolver, &fx.metadata);
            let mut ctx = ExecContext::new(&rt, DB_NAME);
            let spec = AggregateSpec {
                func: default_aggregate_registry().get("math::sum").expect("sum fn"),
                arg: Some(compile_arc(&LogicalExpr::Field(FieldPath {
                    segments: vec![FieldSegment::Field("number".to_string())],
                }))
                .unwrap_or_else(|e| panic!("compile sum arg: {e}"))),
                alias: "total".to_string(),
            };
            let op = HashAggregateOperator::new(Box::new(FullTypeScan::new("Chapter")), vec![spec], Vec::new());
            match op.execute(&mut ctx) {
                FlowResult::Rows(_) => op.first_count().unwrap_or_default(),
                _ => panic!("aggregate failed"),
            }
        })
    });
    g.finish();

    // End-to-end GraphQL nested fetch --------------------------------------
    let mut g = c.benchmark_group("11_graphql_e2e");
    g.bench_function("nested_translation_chapters_verses", |b| {
        b.iter(|| {
            let res = fx.rt.block_on(fx.schema.execute_with_resolver(
                r#"{ queryBookTranslation(first: 2) { id chapters(first: 1) { id } } }"#,
                Box::new(resolver.clone()),
            ));
            res.contains("errors") == false
        })
    });
    g.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
