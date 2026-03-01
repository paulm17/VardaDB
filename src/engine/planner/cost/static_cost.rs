use async_graphql_parser::types::{ExecutableDocument, Selection, SelectionSet, DocumentOperations};
use std::collections::{HashMap, HashSet};
use crate::engine::planner::error::PlannerError;
use crate::engine::planner::cost::schema_cache::DemandControlledSchema;
use crate::engine::planner::cost::CostEstimate;
use crate::config::PlannerConfig;
use async_graphql::Variables;

pub fn estimate_cost(
    document: &ExecutableDocument,
    schema: &DemandControlledSchema,
    config: &PlannerConfig,
    variables: &Variables,
) -> Result<CostEstimate, PlannerError> {
    
    // 1. Collect all fragment definitions
    let mut fragments = HashMap::new();
    for (name, frag) in document.fragments.iter() {
        fragments.insert(name.as_str(), &frag.node.selection_set.node);
    }
    
    let mut total_cost = 0.0;
    
    match &document.operations {
        DocumentOperations::Single(op) => {
            total_cost += score_selection_set(
                &op.node.selection_set.node,
                "Query", // default root (technically could be Mutation/Subscription)
                1.0,
                schema,
                &fragments,
                config,
                variables,
                &mut HashSet::new(),
            )?;
        }
        DocumentOperations::Multiple(ops) => {
            for (_name, op) in ops.iter() {
                // Determine root type based on operation type
                let root_type = match op.node.ty {
                    async_graphql_parser::types::OperationType::Query => "Query",
                    async_graphql_parser::types::OperationType::Mutation => "Mutation",
                    async_graphql_parser::types::OperationType::Subscription => "Subscription",
                };
            
                total_cost += score_selection_set(
                    &op.node.selection_set.node,
                    root_type,
                    1.0,
                    schema,
                    &fragments,
                    config,
                    variables,
                    &mut HashSet::new(),
                )?;
            }
        }
    }
    
    Ok(CostEstimate { total: total_cost })
}

fn score_selection_set<'a>(
    selection_set: &'a SelectionSet,
    parent_type: &str,
    list_multiplier: f64,
    schema: &DemandControlledSchema,
    fragments: &HashMap<&'a str, &'a SelectionSet>,
    config: &PlannerConfig,
    variables: &Variables,
    visited_fragments: &mut HashSet<&'a str>,
) -> Result<f64, PlannerError> {
    let mut total = 0.0;

    for selection in &selection_set.items {
        match &selection.node {
            Selection::Field(field) => {
                let field_name = field.node.name.node.as_str();
                
                // Introspection is free
                if field_name.starts_with("__") {
                    continue;
                }
                
                // Skip/Include Directives logic (simplified)
                let mut skipped = false;
                for dir in &field.node.directives {
                    let dir_name = dir.node.name.node.as_str();
                    if dir_name == "skip" {
                        if bool_from_dir_arg(dir, "if", variables).unwrap_or(false) {
                            skipped = true;
                            break;
                        }
                    } else if dir_name == "include" {
                        if !bool_from_dir_arg(dir, "if", variables).unwrap_or(true) {
                            skipped = true;
                            break;
                        }
                    }
                }
                
                if skipped {
                    continue;
                }
                
                // Field Cost
                let entry = schema.get(parent_type, field_name);
                let (type_cost, is_list, return_type) = if let Some(e) = entry {
                    let cost = e.cost_weight.unwrap_or(if e.is_scalar { 0.0 } else { 1.0 });
                    (cost, e.is_list, e.return_type.as_str())
                } else {
                    // Unknown field - fall back to 1.0
                    (1.0, false, "String")
                };
                
                // Determine instance multiplier
                let instance_cnt = if !is_list {
                    1.0
                } else {
                    let sz = extract_list_size(&field.node, entry, config, variables);
                    sz as f64 * list_multiplier
                };
                
                // Arguments Cost
                let mut arg_cost = 0.0;
                for (_name, arg) in &field.node.arguments {
                    arg_cost += score_argument_value(&arg.node, variables);
                }
                
                // Subselection
                let child_cost = if !field.node.selection_set.node.items.is_empty() {
                    score_selection_set(
                        &field.node.selection_set.node,
                        return_type,
                        instance_cnt,
                        schema,
                        fragments,
                        config,
                        variables,
                        visited_fragments,
                    )?
                } else {
                    0.0
                };
                
                total += (type_cost + arg_cost + child_cost) * instance_cnt;
            }
            Selection::InlineFragment(inline_fragment) => {
                let target_type = if let Some(cond) = &inline_fragment.node.type_condition {
                    cond.node.on.node.as_str()
                } else {
                    parent_type
                };
            
                total += score_selection_set(
                    &inline_fragment.node.selection_set.node,
                    target_type,
                    list_multiplier,
                    schema,
                    fragments,
                    config,
                    variables,
                    visited_fragments,
                )?;
            }
            Selection::FragmentSpread(fragment_spread) => {
                let frag_name = fragment_spread.node.fragment_name.node.as_str();
                
                if !visited_fragments.insert(frag_name) {
                    return Err(PlannerError::CircularFragment);
                }
                
                if let Some(target_selection_set) = fragments.get(frag_name) {
                    // Assume fragment applies to parent_type (or we'd need type context for fragments)
                    total += score_selection_set(
                        target_selection_set,
                        parent_type,
                        list_multiplier,
                        schema,
                        fragments,
                        config,
                        variables,
                        visited_fragments,
                    )?;
                }
                
                visited_fragments.remove(frag_name);
            }
        }
    }

    Ok(total)
}

fn score_argument_value(val: &async_graphql_value::Value, variables: &Variables) -> f64 {
    match val {
        async_graphql_value::Value::String(_) |
        async_graphql_value::Value::Number(_) |
        async_graphql_value::Value::Boolean(_) |
        async_graphql_value::Value::Enum(_) |
        async_graphql_value::Value::Binary(_) |
        async_graphql_value::Value::Null => 0.0,
        async_graphql_value::Value::List(items) => {
            let mut c = 0.0;
            for i in items { c += score_argument_value(i, variables); }
            c
        }
        async_graphql_value::Value::Object(map) => {
            let mut c = 1.0; // Base cost for object
            for (_, v) in map { c += score_argument_value(v, variables); }
            c
        }
        async_graphql_value::Value::Variable(name) => {
            // lookup in variables
            if let Some(v) = variables.get(name) {
                // For simplicity, value costs 0 if scalar, length if array, 1+ if object.
                // In actual async_graphql::Value returned from variables map it's ConstValue
                score_argument_value(&v.clone().into_value(), &Variables::default()) 
            } else {
                0.0
            }
        }
    }
}

fn bool_from_dir_arg(dir: &async_graphql_parser::Positioned<async_graphql_parser::types::Directive>, arg_name: &str, variables: &Variables) -> Option<bool> {
    for (name, val) in &dir.node.arguments {
        if name.node == arg_name {
            match &val.node {
                async_graphql_value::Value::Boolean(b) => return Some(*b),
                async_graphql_value::Value::Variable(vname) => {
                    if let Some(v) = variables.get(vname) {
                        if let async_graphql_value::ConstValue::Boolean(b) = v {
                            return Some(*b);
                        }
                    }
                },
                _ => {}
            }
        }
    }
    None
}

fn extract_list_size(field: &async_graphql_parser::types::Field, entry: Option<&crate::engine::planner::cost::schema_cache::FieldCostEntry>, config: &PlannerConfig, variables: &Variables) -> i32 {
    let mut default_size = config.default_list_size;
    
    if let Some(e) = entry {
        if let Some(ls) = &e.list_size {
            if let Some(assumed) = ls.assumed_size {
                default_size = assumed;
            }
            
            // Look for slicing arguments. First one wins
            for slice_arg_name in &ls.slicing_arguments {
                // simple arguments only for MVP (no "input.limit" paths)
                for (name, val) in &field.arguments {
                    if name.node.as_str() == slice_arg_name {
                        match &val.node {
                            async_graphql_value::Value::Number(n) => {
                                if let Some(i) = n.as_i64() {
                                    return i as i32;
                                }
                            }
                            async_graphql_value::Value::Variable(vname) => {
                                if let Some(v) = variables.get(vname) {
                                    if let async_graphql_value::ConstValue::Number(n) = v {
                                        if let Some(i) = n.as_i64() {
                                            return i as i32;
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    
    // As a fallback, try common argument names explicitly 
    for (name, val) in &field.arguments {
        let name_str = name.node.as_str();
        if name_str == "first" || name_str == "limit" || name_str == "count" {
             match &val.node {
                async_graphql_value::Value::Number(n) => { if let Some(i) = n.as_i64() { return i as i32; } }
                async_graphql_value::Value::Variable(vname) => {
                    if let Some(v) = variables.get(vname) {
                        if let async_graphql_value::ConstValue::Number(n) = v {
                            if let Some(i) = n.as_i64() { return i as i32; }
                        }
                    }
                }
                _ => {}
            }
        } else if name_str == "ids" || name_str == "uuids" {
             // Array size inference
             match &val.node {
                 async_graphql_value::Value::List(items) => { return items.len() as i32; },
                 async_graphql_value::Value::Variable(vname) => {
                     if let Some(v) = variables.get(vname) {
                        if let async_graphql_value::ConstValue::List(items) = v {
                            return items.len() as i32;
                        }
                     }
                 }
                 _ => {}
             }
        }
    }

    default_size
}
