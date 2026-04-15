use dashmap::DashMap;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::{BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, PhraseQuery, TermQuery};
use tantivy::schema::{OwnedValue, *};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, Stemmer, TextAnalyzer, TokenizerManager};
use tantivy::{doc, Index, IndexWriter, ReloadPolicy, TantivyDocument};
use xxhash_rust::xxh3::xxh3_64;

fn term_tokenizer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build()
}

fn fulltext_tokenizer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .filter(Stemmer::default())
        .build()
}

#[derive(Clone, Debug)]
pub struct FieldBoost {
    pub field: String,
    pub boost: f32,
}

struct DbIndex {
    index: Index,
    writer: Mutex<IndexWriter>,
    uid_field: Field,
    doc_id_field: Field,
    field_name_field: Field,
    term_content_field: Field,
    fulltext_content_field: Field,
    indexed_this_batch: Mutex<std::collections::HashSet<u64>>,
}

pub struct SearchEngine {
    indexes: DashMap<String, Arc<DbIndex>>,
    base_path: PathBuf,
}

fn build_schema() -> (Schema, Field, Field, Field, Field, Field) {
    let mut schema_builder = Schema::builder();
    let uid_field = schema_builder.add_u64_field("uid", FAST | STORED);
    let doc_id_field = schema_builder.add_u64_field("doc_id", INDEXED | STORED);
    let field_name_field = schema_builder.add_text_field("field_name", STRING | STORED);
    let term_content_field = schema_builder.add_text_field(
        "term_content",
        TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("term_tokenizer")
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            )
            .set_stored(),
    );
    let fulltext_content_field = schema_builder.add_text_field(
        "fulltext_content",
        TextOptions::default()
            .set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("fulltext_tokenizer")
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            )
            .set_stored(),
    );
    let schema = schema_builder.build();
    (
        schema,
        uid_field,
        doc_id_field,
        field_name_field,
        term_content_field,
        fulltext_content_field,
    )
}

fn composite_doc_id(uid: u64, field: &str) -> u64 {
    let field_hash = xxh3_64(field.as_bytes());
    uid.wrapping_add(field_hash)
}

impl DbIndex {
    fn open(base_path: &Path, db_name: &str) -> anyhow::Result<Self> {
        let (
            schema,
            uid_field,
            doc_id_field,
            field_name_field,
            term_content_field,
            fulltext_content_field,
        ) = build_schema();

        let index_path = base_path.join(format!("{}_tantivy", db_name));
        std::fs::create_dir_all(&index_path)?;

        let tokenizer_manager = TokenizerManager::default();
        tokenizer_manager.register("term_tokenizer", term_tokenizer());
        tokenizer_manager.register("fulltext_tokenizer", fulltext_tokenizer());

        let dir = MmapDirectory::open(&index_path)?;
        let mut index = Index::open_or_create(dir, schema.clone())?;
        index.set_tokenizers(tokenizer_manager);

        let writer = index.writer_with_num_threads(1, 15_000_000)?;

        Ok(Self {
            index,
            writer: Mutex::new(writer),
            uid_field,
            doc_id_field,
            field_name_field,
            term_content_field,
            fulltext_content_field,
            indexed_this_batch: Mutex::new(std::collections::HashSet::new()),
        })
    }
}

impl SearchEngine {
    pub fn new(base_path: impl AsRef<Path>) -> Self {
        Self {
            indexes: DashMap::new(),
            base_path: base_path.as_ref().to_path_buf(),
        }
    }

    fn get_or_create_index(&self, db_name: &str) -> anyhow::Result<Arc<DbIndex>> {
        if let Some(entry) = self.indexes.get(db_name) {
            return Ok(entry.value().clone());
        }
        let db_index = DbIndex::open(&self.base_path, db_name)?;
        let arc = Arc::new(db_index);
        self.indexes.insert(db_name.to_string(), arc.clone());
        Ok(arc)
    }

    pub fn index_document(
        &self,
        db_name: &str,
        uid: u64,
        field: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        let db_index = self.get_or_create_index(db_name)?;
        let doc_id = composite_doc_id(uid, field);

        {
            let mut batch = db_index.indexed_this_batch.lock();
            batch.insert(doc_id);
        }

        {
            let mut writer = db_index.writer.lock();
            let term_query = TermQuery::new(
                tantivy::Term::from_field_u64(db_index.doc_id_field, doc_id),
                IndexRecordOption::Basic,
            );
            writer.delete_query(Box::new(term_query))?;

            let tant_doc = doc!(
                db_index.uid_field => uid,
                db_index.doc_id_field => doc_id,
                db_index.field_name_field => field,
                db_index.term_content_field => text,
                db_index.fulltext_content_field => text,
            );
            writer.add_document(tant_doc)?;
            writer.commit()?;
        }
        Ok(())
    }

    pub fn remove_document(&self, db_name: &str, uid: u64, field: &str) -> anyhow::Result<()> {
        let db_index = self.get_or_create_index(db_name)?;
        let doc_id = composite_doc_id(uid, field);
        let mut writer = db_index.writer.lock();
        let term_query = TermQuery::new(
            tantivy::Term::from_field_u64(db_index.doc_id_field, doc_id),
            IndexRecordOption::Basic,
        );
        writer.delete_query(Box::new(term_query))?;
        writer.commit()?;
        Ok(())
    }

    fn tokenize_with(
        &self,
        idx: &DbIndex,
        tokenizer_name: &str,
        text: &str,
        field: Field,
    ) -> Vec<tantivy::Term> {
        let mut tokenizer = match idx.index.tokenizers().get(tokenizer_name) {
            Some(t) => t,
            None => return vec![],
        };
        let mut token_stream = tokenizer.token_stream(text);
        let mut terms = Vec::new();
        while token_stream.advance() {
            let token = token_stream.token();
            terms.push(tantivy::Term::from_field_text(field, &token.text));
        }
        terms
    }

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
        let db_index = match self.get_or_create_index(db_name) {
            Ok(idx) => idx,
            Err(_) => return vec![],
        };

        let content_field = if strategy == "term" {
            db_index.term_content_field
        } else {
            db_index.fulltext_content_field
        };
        let tokenizer_name = if strategy == "term" {
            "term_tokenizer"
        } else {
            "fulltext_tokenizer"
        };

        let field_name_term = tantivy::Term::from_field_text(db_index.field_name_field, _field);
        let field_query = TermQuery::new(field_name_term, IndexRecordOption::Basic);

        let is_phrase =
            query_text.starts_with('"') && query_text.ends_with('"') && query_text.len() > 2;

        let content_query: Box<dyn tantivy::query::Query> = if is_phrase {
            let phrase_text = &query_text[1..query_text.len() - 1];
            let phrase_terms =
                self.tokenize_with(&db_index, tokenizer_name, phrase_text, content_field);
            if phrase_terms.is_empty() {
                return vec![];
            }
            if phrase_terms.len() == 1 {
                Box::new(TermQuery::new(
                    phrase_terms.into_iter().next().unwrap(),
                    IndexRecordOption::WithFreqs,
                ))
            } else {
                let mut phrase_query = PhraseQuery::new(phrase_terms);
                if let Some(slop) = phrase_slop {
                    phrase_query.set_slop(slop);
                }
                Box::new(phrase_query)
            }
        } else {
            let terms = self.tokenize_with(&db_index, tokenizer_name, query_text, content_field);
            if terms.is_empty() {
                return vec![];
            }
            if require_all {
                let clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = terms
                    .into_iter()
                    .map(|t| {
                        if let Some(distance) = fuzzy_distance {
                            (
                                Occur::Must,
                                Box::new(FuzzyTermQuery::new(t, distance, true))
                                    as Box<dyn tantivy::query::Query>,
                            )
                        } else {
                            (
                                Occur::Must,
                                Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs))
                                    as Box<dyn tantivy::query::Query>,
                            )
                        }
                    })
                    .collect();
                Box::new(BooleanQuery::new(clauses))
            } else {
                let clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = terms
                    .into_iter()
                    .map(|t| {
                        if let Some(distance) = fuzzy_distance {
                            (
                                Occur::Should,
                                Box::new(FuzzyTermQuery::new(t, distance, true))
                                    as Box<dyn tantivy::query::Query>,
                            )
                        } else {
                            (
                                Occur::Should,
                                Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs))
                                    as Box<dyn tantivy::query::Query>,
                            )
                        }
                    })
                    .collect();
                Box::new(BooleanQuery::new(clauses))
            }
        };

        let combined = BooleanQuery::new(vec![
            (
                Occur::Must,
                Box::new(field_query) as Box<dyn tantivy::query::Query>,
            ),
            (Occur::Must, content_query),
        ]);

        let reader = match db_index
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
        {
            Ok(r) => r,
            Err(_) => return vec![],
        };

        let searcher = reader.searcher();

        let collector = TopDocs::with_limit(k);
        let top_docs = match searcher.search(&combined, &collector) {
            Ok(docs) => docs,
            Err(_) => return vec![],
        };

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            if let Ok(doc) = searcher.doc::<TantivyDocument>(doc_address) {
                if let Some(uid_val) = doc.get_first(db_index.uid_field) {
                    match uid_val {
                        OwnedValue::U64(uid) => {
                            results.push((*uid, score as f64));
                        }
                        _ => {}
                    }
                }
            }
        }
        results
    }

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

        let db_index = match self.get_or_create_index(db_name) {
            Ok(idx) => idx,
            Err(_) => return vec![],
        };

        {
            let mut writer = db_index.writer.lock();
            let _ = writer.commit();
        }

        let reader = match db_index
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
        {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        let searcher = reader.searcher();

        let (content_field, tokenizer_name) = if strategy == "fulltext" {
            (db_index.fulltext_content_field, "fulltext_tokenizer")
        } else {
            (db_index.term_content_field, "term_tokenizer")
        };

        let is_phrase =
            query_text.starts_with('"') && query_text.ends_with('"') && query_text.len() > 2;

        let mut field_clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = Vec::new();

        for fb in fields {
            let field_filter_term =
                tantivy::Term::from_field_text(db_index.field_name_field, &fb.field);
            let field_filter: Box<dyn tantivy::query::Query> =
                Box::new(TermQuery::new(field_filter_term, IndexRecordOption::Basic));

            let content_query: Box<dyn tantivy::query::Query> = if is_phrase {
                let phrase_text = &query_text[1..query_text.len() - 1];
                let phrase_terms =
                    self.tokenize_with(&db_index, tokenizer_name, phrase_text, content_field);
                if phrase_terms.is_empty() {
                    continue;
                }
                let mut phrase_query = PhraseQuery::new(phrase_terms);
                if let Some(slop) = phrase_slop {
                    phrase_query.set_slop(slop);
                }
                Box::new(phrase_query)
            } else {
                let terms =
                    self.tokenize_with(&db_index, tokenizer_name, query_text, content_field);
                if terms.is_empty() {
                    continue;
                }
                if require_all {
                    let clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = terms
                        .into_iter()
                        .map(|t| {
                            if let Some(distance) = fuzzy_distance {
                                (
                                    Occur::Must,
                                    Box::new(FuzzyTermQuery::new(t, distance, true))
                                        as Box<dyn tantivy::query::Query>,
                                )
                            } else {
                                (
                                    Occur::Must,
                                    Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs))
                                        as Box<dyn tantivy::query::Query>,
                                )
                            }
                        })
                        .collect();
                    Box::new(BooleanQuery::new(clauses))
                } else {
                    let clauses: Vec<(Occur, Box<dyn tantivy::query::Query>)> = terms
                        .into_iter()
                        .map(|t| {
                            if let Some(distance) = fuzzy_distance {
                                (
                                    Occur::Should,
                                    Box::new(FuzzyTermQuery::new(t, distance, true))
                                        as Box<dyn tantivy::query::Query>,
                                )
                            } else {
                                (
                                    Occur::Should,
                                    Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs))
                                        as Box<dyn tantivy::query::Query>,
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

            let boosted_query: Box<dyn tantivy::query::Query> =
                Box::new(BoostQuery::new(Box::new(field_query), fb.boost));
            field_clauses.push((Occur::Should, boosted_query));
        }

        if field_clauses.is_empty() {
            return vec![];
        }

        let multi_query = BooleanQuery::new(field_clauses);

        let top_docs = match searcher.search(&multi_query, &TopDocs::with_limit(k)) {
            Ok(docs) => docs,
            Err(_) => return vec![],
        };

        let mut results = Vec::new();
        for (score, doc_address) in top_docs {
            if let Ok(doc) = searcher.doc::<TantivyDocument>(doc_address) {
                if let Some(uid_val) = doc.get_first(db_index.uid_field) {
                    match uid_val {
                        OwnedValue::U64(uid) => {
                            results.push((*uid, score as f64));
                        }
                        _ => {}
                    }
                }
            }
        }
        results
    }

    pub fn commit(&self, db_name: &str) -> anyhow::Result<()> {
        if let Some(entry) = self.indexes.get(db_name) {
            let mut writer = entry.value().writer.lock();
            writer.commit()?;
            let mut batch = entry.value().indexed_this_batch.lock();
            batch.clear();
        }
        Ok(())
    }

    pub fn commit_all(&self) -> anyhow::Result<()> {
        for entry in self.indexes.iter() {
            let mut writer = entry.value().writer.lock();
            writer.commit()?;
            let mut batch = entry.value().indexed_this_batch.lock();
            batch.clear();
        }
        Ok(())
    }
}
