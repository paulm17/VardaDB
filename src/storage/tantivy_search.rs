/// Tantivy-backed full-text search engine for VardaDB.
///
/// Replaces the SQLite FTS5 / manual BM25 subsystem (0x05/0x06 prefix keys).
/// Provides one Tantivy index per logical database, stored at
/// `{base_path}/{db_name}_tantivy/`.
///
/// ## Per-field deletion
///
/// Each document is keyed by a composite `doc_id = xxh3(uid || field_name)`.
/// This allows:
/// - `index_document` to atomically replace only the specific `(uid, field)` entry
///   without disturbing other indexed fields for the same uid.
/// - `remove_document` to remove only one field's entry (e.g. on update of that
///   field alone).
///
/// Full node deletion (delete_node) calls `remove_document` for each indexed field
/// in turn, which correctly clears all entries for that uid.
///
/// ## Tokenizer pipelines
///
/// Two pipelines mirror the existing `tokenizer.rs` strategies:
/// - `"term"`     → unicode tokenisation + lowercase (no stemming)
/// - `"fulltext"` → unicode tokenisation + lowercase + English Porter stemmer
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use parking_lot::Mutex;
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::{
    BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, PhraseQuery, Query, TermQuery,
};
use tantivy::schema::Value as TantivyValue;
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, FAST, INDEXED, STORED, STRING,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, Stemmer, TextAnalyzer};
use tantivy::{Index, IndexWriter, TantivyDocument, Term};

/// Field boost configuration for multi-field queries.
///
/// Each entry specifies a field name and an optional boost weight.
/// Boost values >1.0 increase the field's importance, <1.0 decreases it.
#[derive(Clone, Debug)]
pub struct FieldBoost {
    pub field: String,
    pub boost: f32,
}

/// Search result with snippet highlighting.
///
/// Contains the document uid, BM25 score, and optional snippet with
/// highlighted matching terms.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub uid: u64,
    pub score: f64,
    pub snippet: Option<String>,
    pub highlighted_terms: Vec<String>,
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Composite deletion key: `xxh3(uid_bytes || field_bytes)`.
///
/// Guarantees that updating/deleting a specific `(uid, field)` pair leaves all
/// other field documents for that uid untouched.
fn composite_doc_id(uid: u64, field: &str) -> u64 {
    let mut data = Vec::with_capacity(8 + field.len());
    data.extend_from_slice(&uid.to_le_bytes());
    data.extend_from_slice(field.as_bytes());
    xxhash_rust::xxh3::xxh3_64(&data)
}

// ---------------------------------------------------------------------------
// Per-database index handle
// ---------------------------------------------------------------------------

struct DbIndex {
    index: Index,
    writer: Mutex<IndexWriter>,
    /// Per-node uid — used by `remove_all_for_uid` and for search results.
    uid_field: Field,
    /// Composite `hash(uid, field)` — used for per-field deletion.
    doc_id_field: Field,
    field_name_field: Field,
    /// Indexed with `"term_tokenizer"` (no stemming).
    term_content_field: Field,
    /// Indexed with `"fulltext_tokenizer"` (Porter stemming).
    fulltext_content_field: Field,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Thread-safe, multi-database full-text search engine backed by Tantivy.
pub struct SearchEngine {
    base_path: PathBuf,
    indexes: DashMap<String, Arc<DbIndex>>,
    /// Tracks (db_name, uid, field) pairs that have already been indexed.
    /// Prevents the same (uid, field) from being added twice when a field
    /// has multiple search strategies (e.g. `[term, fulltext]`), which would
    /// create duplicate Tantivy documents.
    /// Cleared when `remove_document` is called for that (uid, field).
    indexed_this_batch: DashMap<(String, u64, String), ()>,
}

impl SearchEngine {
    pub fn new(base_path: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            base_path: base_path.to_path_buf(),
            indexes: DashMap::new(),
            indexed_this_batch: DashMap::new(),
        })
    }

    // -----------------------------------------------------------------------
    // Internal: open or create a Tantivy index for `db_name`
    // -----------------------------------------------------------------------

    fn get_or_create(&self, db_name: &str) -> anyhow::Result<Arc<DbIndex>> {
        if let Some(idx) = self.indexes.get(db_name) {
            return Ok(Arc::clone(idx.value()));
        }

        let index_path = self.base_path.join(format!("{}_tantivy", db_name));
        std::fs::create_dir_all(&index_path)?;

        // ---- Schema -------------------------------------------------------
        let mut sb = Schema::builder();
        let uid_field = sb.add_u64_field("uid", FAST | STORED);
        // doc_id needs to be INDEXED for delete_term to work, not just FAST
        let doc_id_field = sb.add_u64_field("doc_id", INDEXED | STORED);
        let field_name_field = sb.add_text_field("field_name", STRING | STORED);

        // "term" content: unicode + lowercase, no stemming
        let term_opts = TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("term_tokenizer")
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            )
            .set_stored();
        let term_content_field = sb.add_text_field("term_content", term_opts);

        // "fulltext" content: unicode + lowercase + English Porter stemmer
        let fulltext_opts = TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("fulltext_tokenizer")
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            )
            .set_stored();
        let fulltext_content_field = sb.add_text_field("fulltext_content", fulltext_opts);

        let schema = sb.build();

        // ---- Open or create -----------------------------------------------
        let dir = MmapDirectory::open(&index_path)?;
        let index = if index_path.join("meta.json").exists() {
            Index::open(dir)?
        } else {
            Index::create(dir, schema, Default::default())?
        };

        // ---- Register tokenizers ------------------------------------------
        index.tokenizers().register(
            "term_tokenizer",
            TextAnalyzer::builder(SimpleTokenizer::default())
                .filter(LowerCaser)
                .build(),
        );
        index.tokenizers().register(
            "fulltext_tokenizer",
            TextAnalyzer::builder(SimpleTokenizer::default())
                .filter(LowerCaser)
                .filter(Stemmer::new(tantivy::tokenizer::Language::English))
                .build(),
        );

        // 50 MB write buffer
        let writer = index.writer(50_000_000)?;

        let db_index = Arc::new(DbIndex {
            index,
            writer: Mutex::new(writer),
            uid_field,
            doc_id_field,
            field_name_field,
            term_content_field,
            fulltext_content_field,
        });

        self.indexes
            .insert(db_name.to_string(), Arc::clone(&db_index));
        Ok(db_index)
    }

    // -----------------------------------------------------------------------
    // Public: index / remove
    // -----------------------------------------------------------------------

    /// Index (or re-index) a document for `(uid, field_name)`.
    ///
    /// Uses the composite `doc_id = hash(uid, field)` to uniquely identify this
    /// (uid, field) pair. The `indexed_this_batch` map prevents duplicate
    /// Tantivy documents when a field has multiple search strategies.
    ///
    /// Commits immediately for durability - data is persisted before returning.
    pub fn index_document(
        &self,
        db_name: &str,
        uid: u64,
        field: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        // Deduplicate: if the same (db, uid, field) has already been indexed,
        // skip the add. This prevents duplicate Tantivy documents when a field
        // declares multiple search strategies (e.g. `@search(by: [term, fulltext])`),
        // because both strategies are already covered by a single document that
        // indexes into both content fields.
        let batch_key = (db_name.to_string(), uid, field.to_string());
        if self.indexed_this_batch.contains_key(&batch_key) {
            return Ok(());
        }
        self.indexed_this_batch.insert(batch_key.clone(), ());

        let idx = self.get_or_create(db_name)?;
        let cid = composite_doc_id(uid, field);

        // Note: We do NOT delete old documents here. If this is an update,
        // remove_document() should have been called first. Deleting here
        // would cause the newly added document to also be deleted because
        // they share the same doc_id_field value.
        //
        // The flow for an update is:
        // 1. remove_document() -> delete_term + commit
        // 2. index_document() -> add new doc + commit (this function)
        // If we also deleted here, the delete_term would apply to BOTH documents.

        let mut doc = TantivyDocument::default();
        doc.add_u64(idx.uid_field, uid);
        doc.add_u64(idx.doc_id_field, cid);
        doc.add_text(idx.field_name_field, field);
        doc.add_text(idx.term_content_field, text);
        doc.add_text(idx.fulltext_content_field, text);

        {
            let mut writer = idx.writer.lock();
            writer.add_document(doc)?;
            writer.commit()?;
        }
        Ok(())
    }

    /// Remove the indexed document for a specific `(uid, field)` pair.
    ///
    /// Called per-field during node updates and deletes so that only the
    /// relevant field entry is removed, leaving other field documents intact.
    ///
    /// IMPORTANT: This commits the Tantivy writer to ensure the delete is
    /// applied before any subsequent add_document for the same (uid, field).
    /// Without this, the delete_term would also delete the newly added document.
    pub fn remove_document(&self, db_name: &str, uid: u64, field: &str) -> anyhow::Result<()> {
        let batch_key = (db_name.to_string(), uid, field.to_string());
        self.indexed_this_batch.remove(&batch_key);

        let idx = self.get_or_create(db_name)?;
        let cid = composite_doc_id(uid, field);
        {
            let mut writer = idx.writer.lock();
            writer.delete_term(Term::from_field_u64(idx.doc_id_field, cid));
            writer.commit()?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Public: search
    // -----------------------------------------------------------------------

    /// BM25 ranked search.
    ///
    /// * `strategy` – `"term"` (no stemming) or `"fulltext"` (Porter).
    /// * `require_all` – `true` for AND semantics, `false` for OR semantics.
    /// * `fuzzy_distance` – Optional Levenshtein distance (0-2) for fuzzy matching.
    /// * `phrase_slop` – Optional slop for phrase queries (only used when query is quoted).
    ///
    /// If `query_text` is wrapped in double quotes, it is treated as a phrase query
    /// where terms must appear in exact order (with optional slop for proximity).
    ///
    /// Returns `(uid, bm25_score)` pairs sorted by descending relevance.
    pub fn search_bm25(
        &self,
        db_name: &str,
        query_text: &str,
        _field: &str,
        strategy: &str,
        k: usize,
        require_all: bool,
        fuzzy_distance: Option<u8>,
        phrase_slop: Option<u32>,
    ) -> Vec<(u64, f64)> {
        let idx = match self.get_or_create(db_name) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("SearchEngine::search_bm25: failed to open index: {}", e);
                return vec![];
            }
        };

        // Commit pending writes so the reader sees the latest data.
        {
            let mut writer = idx.writer.lock();
            if let Err(e) = writer.commit() {
                eprintln!("SearchEngine: auto-commit before search failed: {}", e);
            }
        }

        let reader = match idx.index.reader() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SearchEngine: failed to open reader: {}", e);
                return vec![];
            }
        };
        let searcher = reader.searcher();

        // Choose content field and tokenizer based on strategy.
        let (content_field, tokenizer_name) = if strategy == "fulltext" {
            (idx.fulltext_content_field, "fulltext_tokenizer")
        } else {
            (idx.term_content_field, "term_tokenizer")
        };

        let field_filter_term = Term::from_field_text(idx.field_name_field, _field);
        let field_filter: (Occur, Box<dyn Query>) = (
            Occur::Must,
            Box::new(TermQuery::new(field_filter_term, IndexRecordOption::Basic)),
        );

        let is_phrase =
            query_text.starts_with('"') && query_text.ends_with('"') && query_text.len() > 2;

        let content_query: Box<dyn Query> = if is_phrase {
            let phrase_text = &query_text[1..query_text.len() - 1];
            let phrase_terms = self.tokenize_with(&idx, tokenizer_name, phrase_text, content_field);
            if phrase_terms.is_empty() {
                return vec![];
            }
            let mut phrase_query = PhraseQuery::new(phrase_terms);
            if let Some(slop) = phrase_slop {
                phrase_query.set_slop(slop);
            }
            Box::new(phrase_query)
        } else {
            let terms = self.tokenize_with(&idx, tokenizer_name, query_text, content_field);
            if terms.is_empty() {
                return vec![];
            }
            if require_all {
                // AND — every term is individually required.
                let clauses: Vec<(Occur, Box<dyn Query>)> = terms
                    .into_iter()
                    .map(|t| {
                        if let Some(distance) = fuzzy_distance {
                            (
                                Occur::Must,
                                Box::new(FuzzyTermQuery::new(t, distance, true)) as Box<dyn Query>,
                            )
                        } else {
                            (
                                Occur::Must,
                                Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs))
                                    as Box<dyn Query>,
                            )
                        }
                    })
                    .collect();
                Box::new(BooleanQuery::new(clauses))
            } else {
                // OR — at least one term must match.  A BooleanQuery with only
                // SHOULD clauses requires at least one to match.
                let clauses: Vec<(Occur, Box<dyn Query>)> = terms
                    .into_iter()
                    .map(|t| {
                        if let Some(distance) = fuzzy_distance {
                            (
                                Occur::Should,
                                Box::new(FuzzyTermQuery::new(t, distance, true)) as Box<dyn Query>,
                            )
                        } else {
                            (
                                Occur::Should,
                                Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs))
                                    as Box<dyn Query>,
                            )
                        }
                    })
                    .collect();
                Box::new(BooleanQuery::new(clauses))
            }
        };

        let query = BooleanQuery::new(vec![field_filter, (Occur::Must, content_query)]);

        let top_docs = match searcher.search(&query, &TopDocs::with_limit(k)) {
            Ok(td) => td,
            Err(e) => {
                eprintln!("SearchEngine: search failed: {}", e);
                return vec![];
            }
        };

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_addr) in top_docs {
            if let Ok(doc) = searcher.doc::<TantivyDocument>(doc_addr) {
                if let Some(uid_val) = doc.get_first(idx.uid_field) {
                    if let Some(uid) = TantivyValue::as_u64(&uid_val) {
                        results.push((uid, score as f64));
                    }
                }
            }
        }
        results
    }

    /// Multi-field BM25 search with per-field boost weights.
    ///
    /// Searches across multiple fields simultaneously, applying boost weights
    /// to each field's contribution to the final score.
    ///
    /// * `fields` – Slice of `FieldBoost` entries specifying field name and boost.
    /// * `strategy` – `"term"` (no stemming) or `"fulltext"` (Porter).
    /// * `require_all` – `true` for AND semantics across terms, `false` for OR.
    /// * `fuzzy_distance` – Optional Levenshtein distance (0-2) for fuzzy matching.
    /// * `phrase_slop` – Optional slop for phrase queries.
    ///
    /// Returns `(uid, bm25_score)` pairs sorted by descending relevance.
    pub fn search_bm25_multi(
        &self,
        db_name: &str,
        query_text: &str,
        fields: &[FieldBoost],
        strategy: &str,
        k: usize,
        require_all: bool,
        fuzzy_distance: Option<u8>,
        phrase_slop: Option<u32>,
    ) -> Vec<(u64, f64)> {
        if fields.is_empty() {
            return vec![];
        }

        let idx = match self.get_or_create(db_name) {
            Ok(i) => i,
            Err(e) => {
                eprintln!(
                    "SearchEngine::search_bm25_multi: failed to open index: {}",
                    e
                );
                return vec![];
            }
        };

        {
            let mut writer = idx.writer.lock();
            if let Err(e) = writer.commit() {
                eprintln!("SearchEngine: auto-commit before search failed: {}", e);
            }
        }

        let reader = match idx.index.reader() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SearchEngine: failed to open reader: {}", e);
                return vec![];
            }
        };
        let searcher = reader.searcher();

        let (content_field, tokenizer_name) = if strategy == "fulltext" {
            (idx.fulltext_content_field, "fulltext_tokenizer")
        } else {
            (idx.term_content_field, "term_tokenizer")
        };

        let is_phrase =
            query_text.starts_with('"') && query_text.ends_with('"') && query_text.len() > 2;

        let mut field_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();

        for fb in fields {
            let field_filter_term = Term::from_field_text(idx.field_name_field, &fb.field);
            let field_filter: Box<dyn Query> =
                Box::new(TermQuery::new(field_filter_term, IndexRecordOption::Basic));

            let content_query: Box<dyn Query> = if is_phrase {
                let phrase_text = &query_text[1..query_text.len() - 1];
                let phrase_terms =
                    self.tokenize_with(&idx, tokenizer_name, phrase_text, content_field);
                if phrase_terms.is_empty() {
                    continue;
                }
                let mut phrase_query = PhraseQuery::new(phrase_terms);
                if let Some(slop) = phrase_slop {
                    phrase_query.set_slop(slop);
                }
                Box::new(phrase_query)
            } else {
                let terms = self.tokenize_with(&idx, tokenizer_name, query_text, content_field);
                if terms.is_empty() {
                    continue;
                }
                if require_all {
                    let clauses: Vec<(Occur, Box<dyn Query>)> = terms
                        .into_iter()
                        .map(|t| {
                            if let Some(distance) = fuzzy_distance {
                                (
                                    Occur::Must,
                                    Box::new(FuzzyTermQuery::new(t, distance, true))
                                        as Box<dyn Query>,
                                )
                            } else {
                                (
                                    Occur::Must,
                                    Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs))
                                        as Box<dyn Query>,
                                )
                            }
                        })
                        .collect();
                    Box::new(BooleanQuery::new(clauses))
                } else {
                    let clauses: Vec<(Occur, Box<dyn Query>)> = terms
                        .into_iter()
                        .map(|t| {
                            if let Some(distance) = fuzzy_distance {
                                (
                                    Occur::Should,
                                    Box::new(FuzzyTermQuery::new(t, distance, true))
                                        as Box<dyn Query>,
                                )
                            } else {
                                (
                                    Occur::Should,
                                    Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs))
                                        as Box<dyn Query>,
                                )
                            }
                        })
                        .collect();
                    Box::new(BooleanQuery::new(clauses))
                }
            };

            let field_query = BooleanQuery::new(vec![
                (Occur::Must, field_filter),
                (Occur::Must, content_query),
            ]);

            let boosted_query: Box<dyn Query> =
                Box::new(BoostQuery::new(Box::new(field_query), fb.boost));
            field_clauses.push((Occur::Should, boosted_query));
        }

        if field_clauses.is_empty() {
            return vec![];
        }

        let multi_query = BooleanQuery::new(field_clauses);

        let top_docs = match searcher.search(&multi_query, &TopDocs::with_limit(k)) {
            Ok(td) => td,
            Err(e) => {
                eprintln!("SearchEngine: multi-field search failed: {}", e);
                return vec![];
            }
        };

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_addr) in top_docs {
            if let Ok(doc) = searcher.doc::<TantivyDocument>(doc_addr) {
                if let Some(uid_val) = doc.get_first(idx.uid_field) {
                    if let Some(uid) = TantivyValue::as_u64(&uid_val) {
                        results.push((uid, score as f64));
                    }
                }
            }
        }
        results
    }

    /// BM25 ranked search with snippet generation.
    ///
    /// Similar to `search_bm25` but returns search results with highlighted snippets
    /// showing the matching terms in context.
    ///
    /// * `field` – The field name to search within.
    /// * `strategy` – `"term"` (no stemming) or `"fulltext"` (Porter).
    /// * `require_all` – `true` for AND semantics, `false` for OR semantics.
    /// * `fuzzy_distance` – Optional Levenshtein distance (0-2) for fuzzy matching.
    /// * `phrase_slop` – Optional slop for phrase queries.
    ///
    /// Returns `SearchResult` entries with uid, score, and snippet.
    pub fn search_bm25_with_snippets(
        &self,
        db_name: &str,
        query_text: &str,
        field: &str,
        strategy: &str,
        k: usize,
        require_all: bool,
        fuzzy_distance: Option<u8>,
        phrase_slop: Option<u32>,
    ) -> Vec<SearchResult> {
        let idx = match self.get_or_create(db_name) {
            Ok(i) => i,
            Err(e) => {
                eprintln!(
                    "SearchEngine::search_bm25_with_snippets: failed to open index: {}",
                    e
                );
                return vec![];
            }
        };

        {
            let mut writer = idx.writer.lock();
            if let Err(e) = writer.commit() {
                eprintln!("SearchEngine: auto-commit before search failed: {}", e);
            }
        }

        let reader = match idx.index.reader() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SearchEngine: failed to open reader: {}", e);
                return vec![];
            }
        };
        let searcher = reader.searcher();

        let (content_field, tokenizer_name) = if strategy == "fulltext" {
            (idx.fulltext_content_field, "fulltext_tokenizer")
        } else {
            (idx.term_content_field, "term_tokenizer")
        };

        let field_filter_term = Term::from_field_text(idx.field_name_field, field);
        let field_filter: (Occur, Box<dyn Query>) = (
            Occur::Must,
            Box::new(TermQuery::new(field_filter_term, IndexRecordOption::Basic)),
        );

        let is_phrase =
            query_text.starts_with('"') && query_text.ends_with('"') && query_text.len() > 2;

        let content_query: Box<dyn Query> = if is_phrase {
            let phrase_text = &query_text[1..query_text.len() - 1];
            let phrase_terms = self.tokenize_with(&idx, tokenizer_name, phrase_text, content_field);
            if phrase_terms.is_empty() {
                return vec![];
            }
            let mut phrase_query = PhraseQuery::new(phrase_terms);
            if let Some(slop) = phrase_slop {
                phrase_query.set_slop(slop);
            }
            Box::new(phrase_query)
        } else {
            let terms = self.tokenize_with(&idx, tokenizer_name, query_text, content_field);
            if terms.is_empty() {
                return vec![];
            }
            if require_all {
                let clauses: Vec<(Occur, Box<dyn Query>)> = terms
                    .into_iter()
                    .map(|t| {
                        if let Some(distance) = fuzzy_distance {
                            (
                                Occur::Must,
                                Box::new(FuzzyTermQuery::new(t, distance, true)) as Box<dyn Query>,
                            )
                        } else {
                            (
                                Occur::Must,
                                Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs))
                                    as Box<dyn Query>,
                            )
                        }
                    })
                    .collect();
                Box::new(BooleanQuery::new(clauses))
            } else {
                let clauses: Vec<(Occur, Box<dyn Query>)> = terms
                    .into_iter()
                    .map(|t| {
                        if let Some(distance) = fuzzy_distance {
                            (
                                Occur::Should,
                                Box::new(FuzzyTermQuery::new(t, distance, true)) as Box<dyn Query>,
                            )
                        } else {
                            (
                                Occur::Should,
                                Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs))
                                    as Box<dyn Query>,
                            )
                        }
                    })
                    .collect();
                Box::new(BooleanQuery::new(clauses))
            }
        };

        let query = BooleanQuery::new(vec![field_filter, (Occur::Must, content_query)]);

        let snippet_generator = match SnippetGenerator::create(&searcher, &query, content_field) {
            Ok(sg) => sg,
            Err(e) => {
                eprintln!("SearchEngine: failed to create snippet generator: {}", e);
                return vec![];
            }
        };

        let top_docs = match searcher.search(&query, &TopDocs::with_limit(k)) {
            Ok(td) => td,
            Err(e) => {
                eprintln!("SearchEngine: search failed: {}", e);
                return vec![];
            }
        };

        let mut results = Vec::with_capacity(top_docs.len());
        for (score, doc_addr) in top_docs {
            if let Ok(doc) = searcher.doc::<TantivyDocument>(doc_addr) {
                if let Some(uid_val) = doc.get_first(idx.uid_field) {
                    if let Some(uid) = TantivyValue::as_u64(&uid_val) {
                        let snippet = snippet_generator.snippet_from_doc(&doc);
                        let fragment_text = snippet.fragment();
                        let highlighted: Vec<String> = snippet
                            .highlighted()
                            .iter()
                            .filter_map(|range| {
                                fragment_text.get(range.clone()).map(|s| s.to_string())
                            })
                            .collect();
                        results.push(SearchResult {
                            uid,
                            score: score as f64,
                            snippet: Some(snippet.to_html()),
                            highlighted_terms: highlighted,
                        });
                    }
                }
            }
        }
        results
    }

    /// Commit pending writes for a single database.
    pub fn commit(&self, db_name: &str) -> anyhow::Result<()> {
        if let Some(idx) = self.indexes.get(db_name) {
            let mut writer = idx.writer.lock();
            writer.commit()?;
        }
        // Clear dedup tracking for this db so future writes are not blocked.
        self.indexed_this_batch
            .retain(|(db, _, _), _| db != db_name);
        Ok(())
    }

    /// Commit pending writes for all open indexes.
    pub fn commit_all(&self) -> anyhow::Result<()> {
        for entry in self.indexes.iter() {
            let mut writer = entry.value().writer.lock();
            writer.commit()?;
        }
        self.indexed_this_batch.clear();
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn tokenize_with(
        &self,
        idx: &DbIndex,
        tokenizer_name: &str,
        text: &str,
        field: Field,
    ) -> Vec<Term> {
        let mut tokenizer = match idx.index.tokenizers().get(tokenizer_name) {
            Some(t) => t,
            None => return vec![],
        };
        let mut token_stream = tokenizer.token_stream(text);
        let mut terms = Vec::new();
        while token_stream.advance() {
            let token = token_stream.token();
            terms.push(Term::from_field_text(field, &token.text));
        }
        terms
    }
}
