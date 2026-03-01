use async_graphql_parser::types::{ExecutableDocument, Selection, SelectionSet, DocumentOperations};
use std::collections::{HashMap, HashSet};
use crate::engine::planner::error::PlannerError;

pub fn check_depth(document: &ExecutableDocument, max_depth: usize) -> Result<(), PlannerError> {
    // 1. Collect all fragment definitions
    let mut fragments = HashMap::new();
    for (name, frag) in document.fragments.iter() {
        fragments.insert(name.as_str(), &frag.node.selection_set.node);
    }
    
    // 2. Check depth for each operation
    match &document.operations {
        DocumentOperations::Single(op) => {
            let mut visited = HashSet::new();
            check_selection_set(&op.node.selection_set.node, 1, max_depth, &fragments, &mut visited)?;
        }
        DocumentOperations::Multiple(ops) => {
            for (_name, op) in ops.iter() {
                let mut visited = HashSet::new();
                check_selection_set(&op.node.selection_set.node, 1, max_depth, &fragments, &mut visited)?;
            }
        }
    }
    
    Ok(())
}

fn check_selection_set<'a>(
    selection_set: &'a SelectionSet,
    current_depth: usize,
    max_depth: usize,
    fragments: &HashMap<&'a str, &'a SelectionSet>,
    visited_fragments: &mut HashSet<&'a str>,
) -> Result<(), PlannerError> {
    if current_depth > max_depth {
        return Err(PlannerError::DepthLimitExceeded {
            limit: max_depth,
            found: current_depth,
        });
    }

    for selection in &selection_set.items {
        match &selection.node {
            Selection::Field(field) => {
                if !field.node.selection_set.node.items.is_empty() {
                    check_selection_set(
                        &field.node.selection_set.node,
                        current_depth + 1,
                        max_depth,
                        fragments,
                        visited_fragments,
                    )?;
                }
            }
            Selection::InlineFragment(inline_fragment) => {
                check_selection_set(
                    &inline_fragment.node.selection_set.node,
                    current_depth, // Inline fragments don't increase depth
                    max_depth,
                    fragments,
                    visited_fragments,
                )?;
            }
            Selection::FragmentSpread(fragment_spread) => {
                let frag_name = fragment_spread.node.fragment_name.node.as_str();
                
                // Cycle detection
                if !visited_fragments.insert(frag_name) {
                    return Err(PlannerError::CircularFragment);
                }
                
                if let Some(target_selection_set) = fragments.get(frag_name) {
                    check_selection_set(
                        target_selection_set,
                        current_depth, // Spreads don't typically increase depth relative to their inlined position
                        max_depth,
                        fragments,
                        visited_fragments,
                    )?;
                }
                
                visited_fragments.remove(frag_name); // Pop visit state
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_graphql_parser::parse_query;

    fn parse(query: &str) -> ExecutableDocument {
        parse_query(query).unwrap()
    }

    #[test]
    fn test_depth_ok() {
        let doc = parse("{ user { posts { comments { id } } } }");
        assert!(check_depth(&doc, 4).is_ok()); // Depth 4 (Query, user, posts, comments)
    }

    #[test]
    fn test_depth_exceeded() {
        let doc = parse("{ user { posts { comments { id } } } }");
        let result = check_depth(&doc, 3);
        assert!(matches!(result, Err(PlannerError::DepthLimitExceeded { found: 4, limit: 3 })));
    }

    #[test]
    fn test_fragment_depth_ok() {
        let doc = parse("
            fragment F1 on User { posts { comments { id } } }
            { user { ...F1 } }
        ");
        assert!(check_depth(&doc, 4).is_ok());
    }

    #[test]
    fn test_fragment_depth_exceeded() {
        let doc = parse("
            fragment F1 on User { posts { comments { id } } }
            { user { ...F1 } }
        ");
        let result = check_depth(&doc, 3);
        assert!(matches!(result, Err(PlannerError::DepthLimitExceeded { found: 4, limit: 3 })));
    }

    #[test]
    fn test_circular_fragment() {
        let doc = parse("
            fragment F1 on User { ...F2 }
            fragment F2 on User { ...F1 }
            { user { ...F1 } }
        ");
        let result = check_depth(&doc, 10);
        assert!(matches!(result, Err(PlannerError::CircularFragment)));
    }
}
