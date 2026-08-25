//! Graph recursion over relation edges (Stage 3.3).
//!
//! Ported from upstream `exec/operators/recursion/*` with one structural
//! deviation: upstream recurses along *expression paths* evaluated against
//! arbitrary values (async, concurrency-bounded). VardaDB graphs are relation
//! edges (`PlannerRelations::related_ids`), so the whole strategy family
//! collapses into one iterative BFS over a single hop, with the goal
//! ([`RecurseGoal`]) selecting what the traversal materializes:
//!
//! - `Terminal` (upstream `default.rs`): the deepest reached frontier.
//! - `CollectAll` (upstream `collect.rs`): every reachable node, deduplicated.
//! - `Levels` (upstream `repeat.rs` phase 1): one [`RowBatch`] per depth —
//!   path materialization without the backward assembly (VardaDB has no `@`
//!   projection marker yet).
//! - `ShortestPath { target }` (upstream `shortest.rs`): BFS parent chain
//!   from the first root that reaches `target`, ordered root → target.
//!
//! Cycle handling: a per-execution `visited` set makes cyclic and
//! self-referential graphs terminate deterministically (upstream relies on
//! value equality plus the idiom recursion limit). Depth bounds clamp at
//! [`RECURSION_LIMIT`].

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::query_planner::ir::EntityId;

use super::{
    CardinalityHint, ExecContext, ExecOperator, FlowResult, OperatorKind, OperatorStat,
    OutputOrdering, PlannerError, RowBatch,
};

/// Hard upper bound on traversal hops (upstream `idiom_recursion_limit`
/// analog). Constructor clamps `max_depth` to this.
pub const RECURSION_LIMIT: u32 = 128;

/// What a finished traversal materializes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecurseGoal {
    /// Nodes at the deepest reached depth (upstream default strategy).
    Terminal,
    /// Every node reached at depth >= `min_depth`, ascending uid order.
    CollectAll,
    /// One output batch per depth level (depth `min_depth` first).
    Levels,
    /// Root → target chain through BFS parent links; empty batch when
    /// unreachable.
    ShortestPath { target: u64 },
}

impl RecurseGoal {
    fn label(&self) -> &'static str {
        match self {
            RecurseGoal::Terminal => "terminal",
            RecurseGoal::CollectAll => "collect",
            RecurseGoal::Levels => "levels",
            RecurseGoal::ShortestPath { .. } => "shortest",
        }
    }
}

/// Follow one relation field repeatedly from each input root.
pub struct RecurseOperator {
    input: Box<dyn ExecOperator>,
    /// Forward relation field traversed each hop (e.g. `children`).
    field: String,
    min_depth: u32,
    max_depth: u32,
    goal: RecurseGoal,
}

impl RecurseOperator {
    pub fn new(
        input: Box<dyn ExecOperator>,
        field: impl Into<String>,
        min_depth: u32,
        max_depth: u32,
        goal: RecurseGoal,
    ) -> Self {
        RecurseOperator {
            input,
            field: field.into(),
            min_depth,
            max_depth: max_depth.min(RECURSION_LIMIT),
            goal,
        }
    }

    pub fn boxed(
        input: Box<dyn ExecOperator>,
        field: impl Into<String>,
        min_depth: u32,
        max_depth: u32,
        goal: RecurseGoal,
    ) -> Box<dyn ExecOperator> {
        Box::new(Self::new(input, field, min_depth, max_depth, goal))
    }

    /// Run the BFS once input ids are known. Returns one batch per depth
    /// level starting at depth 0 (the roots), plus first-discovery parent
    /// links for shortest-path reconstruction.
    fn traverse(
        &self,
        ctx: &mut ExecContext,
        roots: Vec<u64>,
    ) -> Result<(Vec<Vec<u64>>, HashMap<u64, u64>), PlannerError> {
        let mut levels: Vec<Vec<u64>> = vec![roots.clone()];
        let mut visited: HashSet<u64> = roots.iter().copied().collect();
        // Parent links for shortest-path reconstruction (first discovery wins,
        // which is exactly BFS shortestness for unweighted graphs).
        let mut parents: HashMap<u64, u64> = HashMap::new();

        let mut frontier = roots;
        let mut depth = 0u32;
        while depth < self.max_depth {
            let mut next: BTreeSet<u64> = BTreeSet::new();
            for uid in &frontier {
                let found = ctx
                    .runtime
                    .related_ids(&EntityId::new(*uid), &self.field, None, None)
                    .map_err(|e| PlannerError::Storage(e.to_string()))?;
                for id in found {
                    let child = id.uid;
                    if visited.insert(child) {
                        parents.insert(child, *uid);
                        next.insert(child);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            depth += 1;
            let level: Vec<u64> = next.into_iter().collect();
            frontier = level.clone();
            levels.push(level);
        }
        Ok((levels, parents))
    }
}

impl ExecOperator for RecurseOperator {
    fn kind(&self) -> OperatorKind {
        OperatorKind::Traverse
    }

    fn detail(&self) -> String {
        format!(
            "recurse {} depth {}..{} goal={}",
            self.field,
            self.min_depth,
            self.max_depth,
            self.goal.label()
        )
    }

    fn cardinality(&self) -> CardinalityHint {
        CardinalityHint::Unbounded
    }

    fn output_ordering(&self) -> OutputOrdering {
        OutputOrdering::Unordered
    }

    fn children(&self) -> Vec<&dyn ExecOperator> {
        vec![self.input.as_ref()]
    }

    fn execute(&self, ctx: &mut ExecContext) -> FlowResult<Vec<RowBatch>> {
        let start = std::time::Instant::now();
        let mut roots: Vec<u64> = Vec::new();
        let flow = match self.input.execute(ctx) {
            FlowResult::Rows(batches) => {
                for batch in batches {
                    for id in batch.0 {
                        roots.push(id.uid);
                    }
                }
                FlowResult::Rows(Vec::new())
            }
            FlowResult::Break | FlowResult::Continue => FlowResult::Rows(Vec::new()),
            flow @ FlowResult::Error(_) => flow,
        };
        let _ = flow;
        roots.sort_unstable();
        roots.dedup();

        let (levels, parents) = match self.traverse(ctx, roots) {
            Ok(result) => result,
            Err(err) => return FlowResult::Error(err),
        };

        let out: Vec<RowBatch> = match &self.goal {
            RecurseGoal::Terminal => {
                let reached = levels.len().saturating_sub(1) as u32;
                if reached >= self.min_depth {
                    let batch = levels
                        .last()
                        .map(|level| level.iter().map(|uid| EntityId::new(*uid)).collect())
                        .unwrap_or_default();
                    vec![RowBatch(batch)]
                } else {
                    vec![RowBatch(Vec::new())]
                }
            }
            RecurseGoal::CollectAll => {
                let mut all: BTreeSet<u64> = BTreeSet::new();
                for (depth, level) in levels.iter().enumerate() {
                    if depth as u32 >= self.min_depth {
                        all.extend(level.iter().copied());
                    }
                }
                vec![RowBatch(all.into_iter().map(EntityId::new).collect())]
            }
            RecurseGoal::Levels => levels
                .iter()
                .skip(self.min_depth as usize)
                .map(|level| RowBatch(level.iter().map(|uid| EntityId::new(*uid)).collect()))
                .collect(),
            RecurseGoal::ShortestPath { target } => {
                let flat: HashSet<u64> = levels.iter().flatten().copied().collect();
                if flat.contains(target) {
                    let mut chain = vec![*target];
                    let mut cursor = *target;
                    while let Some(parent) = parents.get(&cursor) {
                        chain.push(*parent);
                        cursor = *parent;
                    }
                    chain.reverse();
                    vec![RowBatch(chain.into_iter().map(EntityId::new).collect())]
                } else {
                    vec![RowBatch(Vec::new())]
                }
            }
        };

        let rows_out: usize = out.iter().map(|b| b.len()).sum();
        ctx.explain.record(OperatorStat {
            kind: "traverse".to_string(),
            detail: self.detail(),
            rows_in: levels.first().map_or(0, Vec::len),
            rows_out,
            elapsed_us: start.elapsed().as_micros() as u64,
            notes: vec![format!(
                "{} levels deep, goal={}",
                levels.len(),
                self.goal.label()
            )],
        });

        FlowResult::Rows(out)
    }
}
