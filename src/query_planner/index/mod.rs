#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexCapability {
    UniqueKey,
    OrderedIndex,
    TermIndex,
    FulltextIndex,
    VectorIndex,
}

impl IndexCapability {
    pub fn as_str(&self) -> &'static str {
        match self {
            IndexCapability::UniqueKey => "unique_key",
            IndexCapability::OrderedIndex => "ordered_index",
            IndexCapability::TermIndex => "term_index",
            IndexCapability::FulltextIndex => "fulltext_index",
            IndexCapability::VectorIndex => "vector_index",
        }
    }
}
