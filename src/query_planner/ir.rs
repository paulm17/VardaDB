use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntityId {
    pub type_name: Option<String>,
    pub uid: u64,
}

impl EntityId {
    pub fn new(uid: u64) -> Self {
        Self {
            type_name: None,
            uid,
        }
    }

    pub fn typed(type_name: impl Into<String>, uid: u64) -> Self {
        Self {
            type_name: Some(type_name.into()),
            uid,
        }
    }

    pub fn raw(&self) -> String {
        self.uid.to_string()
    }
}

impl From<u64> for EntityId {
    fn from(uid: u64) -> Self {
        EntityId::new(uid)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Enum(String),
    List(Vec<QueryValue>),
    Object(BTreeMap<String, QueryValue>),
    EntityId(EntityId),
}

impl QueryValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            QueryValue::String(s) => Some(s),
            _ => None,
        }
    }
}

impl From<&async_graphql::Value> for QueryValue {
    fn from(v: &async_graphql::Value) -> Self {
        use async_graphql::Value;
        match v {
            Value::Null => QueryValue::Null,
            Value::Number(n) => match n.as_i64() {
                Some(i) => QueryValue::Int(i),
                None => QueryValue::Float(n.as_f64().unwrap_or(0.0)),
            },
            Value::String(s) => QueryValue::String(s.clone()),
            Value::Boolean(b) => QueryValue::Bool(*b),
            Value::Binary(_) => QueryValue::Null,
            Value::Enum(name) => QueryValue::Enum(name.to_string()),
            Value::List(items) => QueryValue::List(items.iter().map(Into::into).collect()),
            Value::Object(map) => {
                let mut out = BTreeMap::new();
                for (k, v) in map {
                    out.insert(k.to_string(), QueryValue::from(v));
                }
                QueryValue::Object(out)
            }
        }
    }
}

impl From<async_graphql::Value> for QueryValue {
    fn from(v: async_graphql::Value) -> Self {
        QueryValue::from(&v)
    }
}

impl From<&QueryValue> for async_graphql::Value {
    fn from(v: &QueryValue) -> Self {
        use async_graphql::Value;
        match v {
            QueryValue::Null => Value::Null,
            QueryValue::Bool(b) => Value::Boolean(*b),
            QueryValue::Int(i) => Value::Number((*i).into()),
            QueryValue::Float(f) => async_graphql::Number::from_f64(*f)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            QueryValue::String(s) => Value::String(s.clone()),
            QueryValue::Enum(e) => Value::Enum(async_graphql::Name::new(e)),
            QueryValue::List(items) => {
                Value::List(items.iter().map(<async_graphql::Value as From<_>>::from).collect())
            }
            QueryValue::Object(map) => {
                let mut out = async_graphql::indexmap::IndexMap::new();
                for (k, v) in map {
                    out.insert(async_graphql::Name::new(k), async_graphql::Value::from(v));
                }
                Value::Object(out)
            }
            QueryValue::EntityId(e) => Value::String(e.raw()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldPath {
    pub segments: Vec<FieldSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FieldSegment {
    Field(String),
    Index(usize),
}

impl FieldPath {
    pub fn field(name: impl Into<String>) -> Self {
        Self {
            segments: vec![FieldSegment::Field(name.into())],
        }
    }

    pub fn single(&self) -> Option<&str> {
        match self.segments.as_slice() {
            [FieldSegment::Field(name)] => Some(name),
            _ => None,
        }
    }
}

impl std::fmt::Display for FieldPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut first = true;
        for seg in &self.segments {
            if !first {
                write!(f, ".")?;
            }
            first = false;
            match seg {
                FieldSegment::Field(name) => write!(f, "{}", name)?,
                FieldSegment::Index(i) => write!(f, "{}", i)?,
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    In,
    Contains,
    AllOfTerms,
    AnyOfTerms,
    AllOfText,
    AnyOfText,
    NearVector,
    Within,
    Intersects,
}

impl FilterOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            FilterOp::Eq => "eq",
            FilterOp::Ne => "ne",
            FilterOp::Gt => "gt",
            FilterOp::Ge => "ge",
            FilterOp::Lt => "lt",
            FilterOp::Le => "le",
            FilterOp::In => "in",
            FilterOp::Contains => "contains",
            FilterOp::AllOfTerms => "allofterms",
            FilterOp::AnyOfTerms => "anyofterms",
            FilterOp::AllOfText => "alloftext",
            FilterOp::AnyOfText => "anyoftext",
            FilterOp::NearVector => "nearVector",
            FilterOp::Within => "within",
            FilterOp::Intersects => "intersects",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilterPredicate {
    pub path: FieldPath,
    pub op: FilterOp,
    pub value: QueryValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogicalFilter {
    And(Vec<LogicalFilter>),
    Or(Vec<LogicalFilter>),
    Not(Box<LogicalFilter>),
    Predicate(FilterPredicate),
    Relation {
        field: String,
        target_type: String,
        filter: Box<LogicalFilter>,
    },
}

impl LogicalFilter {
    pub fn top_level_predicates(&self) -> Vec<&FilterPredicate> {
        match self {
            LogicalFilter::Predicate(p) => vec![p],
            LogicalFilter::And(parts) => parts
                .iter()
                .flat_map(|p| p.top_level_predicates())
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn is_empty_conjunction(&self) -> bool {
        matches!(self, LogicalFilter::And(parts) if parts.is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderKey {
    pub path: FieldPath,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregateFunction {
    Count,
    Sum,
    Mean,
    Min,
    Max,
}

#[derive(Debug, Clone)]
pub struct AggregateSpec {
    pub function: AggregateFunction,
    pub expr: Option<crate::query_planner::ir::LogicalExpr>,
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CursorValue {
    Entity(EntityId),
    Scalar(QueryValue),
    Compound(Vec<QueryValue>),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pagination {
    pub first: Option<usize>,
    pub offset: Option<usize>,
    pub after: Option<CursorValue>,
}

#[derive(Debug, Clone)]
pub enum ProjectField {
    Scalar { name: String },
    Computed { alias: String, expr: crate::query_planner::ir::LogicalExpr },
    Relation { name: String, plan: Box<LogicalQuery> },
}

#[derive(Debug, Clone, Default)]
pub struct Projection {
    pub fields: Vec<ProjectField>,
}

#[derive(Debug, Clone)]
pub struct RelationPlan {
    pub field: String,
    pub query: Box<LogicalQuery>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplainMode {
    None,
    Text,
    Json,
}

#[derive(Debug, Clone)]
pub enum QueryRoot {
    TypeScan { type_name: String },
    UniqueLookup { type_name: String, field: String, value: QueryValue },
    IdLookup { type_name: String, id: EntityId },
    RelationScan {
        parent_type: String,
        parent_id: Option<EntityId>,
        field: String,
    },
    CandidateSet {
        type_name: String,
        source: crate::query_planner::plan::CandidateSource,
    },
}

impl QueryRoot {
    pub fn type_name(&self) -> &str {
        match self {
            QueryRoot::TypeScan { type_name }
            | QueryRoot::UniqueLookup { type_name, .. }
            | QueryRoot::IdLookup { type_name, .. }
            | QueryRoot::RelationScan { parent_type: type_name, .. }
            | QueryRoot::CandidateSet { type_name, .. } => type_name,
        }
    }
}

#[derive(Debug, Clone)]
pub enum LogicalExpr {
    Value(QueryValue),
    Field(FieldPath),
    Binary {
        left: Box<LogicalExpr>,
        op: BinaryOp,
        right: Box<LogicalExpr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<LogicalExpr>,
    },
    Function {
        name: String,
        args: Vec<LogicalExpr>,
    },
    Subquery(Box<LogicalQuery>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
    In,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone)]
pub struct LogicalQuery {
    pub root: QueryRoot,
    pub filter: Option<LogicalFilter>,
    pub order_by: Vec<OrderKey>,
    pub pagination: Pagination,
    pub projection: Projection,
    pub relations: Vec<RelationPlan>,
    pub aggregates: Vec<AggregateSpec>,
    pub explain: ExplainMode,
}

impl LogicalQuery {
    pub fn scan(type_name: impl Into<String>) -> Self {
        Self {
            root: QueryRoot::TypeScan {
                type_name: type_name.into(),
            },
            filter: None,
            order_by: Vec::new(),
            pagination: Pagination::default(),
            projection: Projection::default(),
            relations: Vec::new(),
            aggregates: Vec::new(),
            explain: ExplainMode::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryRecord {
    pub id: EntityId,
    pub fields: BTreeMap<String, QueryValue>,
}
