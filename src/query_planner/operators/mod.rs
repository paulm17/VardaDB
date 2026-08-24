#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorKind {
    Scan,
    Filter,
    Sort,
    Limit,
    Project,
    Fetch,
    Union,
    Aggregate,
    Explain,
}

impl OperatorKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            OperatorKind::Scan => "scan",
            OperatorKind::Filter => "filter",
            OperatorKind::Sort => "sort",
            OperatorKind::Limit => "limit",
            OperatorKind::Project => "project",
            OperatorKind::Fetch => "fetch",
            OperatorKind::Union => "union",
            OperatorKind::Aggregate => "aggregate",
            OperatorKind::Explain => "explain",
        }
    }
}

/// Phase 2 landing zone: the streaming operator pipeline
/// (Scan -> Filter -> Split -> Aggregate -> Sort -> Limit -> Fetch -> Project)
/// will be built from these kinds, mirroring the upstream
/// `ExecOperator`/`ValueBatchStream` contract.
#[derive(Debug, Clone)]
pub struct PlannedOperator {
    pub kind: OperatorKind,
    pub detail: String,
}
