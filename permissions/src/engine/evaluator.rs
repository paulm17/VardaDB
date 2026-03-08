use rhai::{Dynamic, Engine, Scope};
use std::collections::HashMap;

use crate::engine::context::Context;
use crate::schema::ast::Entity;
use crate::storage::attribute::AttrValue;
use crate::storage::auth_store::AuthStore;
use crate::storage::tuple::Subject;

const MAX_DEPTH: usize = 100;

fn split_top_level<'a>(expr: &'a str, delimiter: &str) -> Vec<&'a str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let chars: Vec<char> = expr.chars().collect();
    let delim_chars: Vec<char> = delimiter.chars().collect();
    let delim_len = delim_chars.len();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            if depth > 0 {
                depth -= 1;
            }
        }
        if depth == 0 && i + delim_len <= chars.len() {
            let candidate: Vec<char> = chars[i..i + delim_len].to_vec();
            if candidate == delim_chars {
                tokens.push(expr[start..i].trim());
                i += delim_len;
                start = i;
                continue;
            }
        }
        i += 1;
    }
    tokens.push(expr[start..].trim());
    tokens
}

fn ensure_balanced(expr: &str) -> String {
    let mut count = 0;
    for c in expr.chars() {
        if c == '(' {
            count += 1;
        } else if c == ')' {
            if count > 0 {
                count -= 1;
            }
        }
    }
    let mut result = expr.trim().to_string();
    for _ in 0..count {
        result.push(')');
    }
    result
}

fn remove_outer_parens(expr: &str) -> String {
    let expr = ensure_balanced(expr);
    let chars: Vec<char> = expr.chars().collect();
    if chars.len() >= 2 && chars[0] == '(' && chars[chars.len() - 1] == ')' {
        let mut count = 0;
        for (i, &c) in chars.iter().enumerate() {
            if c == '(' {
                count += 1;
            } else if c == ')' {
                count -= 1;
            }
            if count == 0 {
                if i == chars.len() - 1 {
                    let inner = &expr[1..expr.len() - 1];
                    return inner.trim().to_string();
                } else {
                    break;
                }
            }
        }
    }
    expr
}

fn get_attr_as_double(store: &AuthStore, entity: &str, id: &str, attr: &str) -> f64 {
    if let Some(val) = store.get_attribute(entity, id, attr) {
        match val {
            AttrValue::Int(i) => i as f64,
            AttrValue::Double(d) => d,
            AttrValue::Bool(b) => {
                if b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    } else {
        0.0
    }
}

fn get_relation_subjects(
    store: &AuthStore,
    schema: &HashMap<String, Entity>,
    entity_name: &str,
    instance_id: &str,
    relation: &str,
) -> Vec<Subject> {
    let direct_subjects = store.get_subjects(entity_name, instance_id, relation);
    if !direct_subjects.is_empty() {
        if let Some(entity_def) = schema.get(entity_name) {
            if entity_def
                .relations
                .get(relation)
                .map_or(false, |t| t.contains('#'))
            {
                let mut subjects = Vec::new();
                for subject in direct_subjects {
                    if subject.id.contains('#') {
                        let parts: Vec<&str> = subject.id.splitn(2, '#').collect();
                        let base_id = parts[0];
                        let chained_relation = parts[1];
                        let resolved_subjects = get_relation_subjects(
                            store,
                            schema,
                            &subject.entity,
                            base_id,
                            chained_relation,
                        );
                        subjects.extend(resolved_subjects);
                    } else {
                        subjects.push(subject);
                    }
                }
                return subjects;
            }
        }
        return direct_subjects;
    } else {
        // Reverse relation logic for @type mentions where empty subjects implies lookup
        // Based on Zanzibar's implementation, sometimes we must reverse relation
        if let Some(entity_def) = schema.get(entity_name) {
            if let Some(rel_def) = entity_def.relations.get(relation) {
                let tokens: Vec<&str> = rel_def.split_whitespace().collect();
                if tokens.len() == 1 && tokens[0].starts_with('@') {
                    let target_type = tokens[0].trim_start_matches('@');
                    let results = store.get_all_for_target(target_type, entity_name, instance_id);
                    return results;
                }
            }
        }
    }
    direct_subjects
}

pub fn evaluate_expr(
    expr: &str,
    schema: &HashMap<String, Entity>,
    entity_name: &str,
    instance_id: &str,
    user: &str,
    store: &AuthStore,
    context: &Context,
    depth: usize,
) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }

    let expr = remove_outer_parens(expr);
    let or_tokens = split_top_level(&expr, " or ");

    if or_tokens.len() > 1 {
        for token in or_tokens {
            if evaluate_expr(
                token,
                schema,
                entity_name,
                instance_id,
                user,
                store,
                context,
                depth + 1,
            ) {
                return true;
            }
        }
        return false;
    }

    let and_tokens = split_top_level(&expr, " and ");

    if and_tokens.len() > 1 {
        for token in and_tokens {
            if !evaluate_expr(
                token,
                schema,
                entity_name,
                instance_id,
                user,
                store,
                context,
                depth + 1,
            ) {
                return false;
            }
        }
        return true;
    }

    evaluate_token(
        &expr,
        schema,
        entity_name,
        instance_id,
        user,
        store,
        context,
        depth + 1,
    )
}

fn evaluate_token(
    token: &str,
    schema: &HashMap<String, Entity>,
    entity_name: &str,
    instance_id: &str,
    user: &str,
    store: &AuthStore,
    context: &Context,
    depth: usize,
) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    let token = token.trim();

    if token.contains('(') && token.ends_with(')') {
        let open_paren = token.find('(').unwrap();
        let func_name = token[..open_paren].trim();
        let args_str = &token[open_paren + 1..token.len() - 1];
        let args: Vec<&str> = args_str.split(',').map(|s| s.trim()).collect();
        return evaluate_function(
            func_name,
            &args,
            schema,
            entity_name,
            instance_id,
            store,
            context,
            user,
            depth + 1,
        );
    }

    if token.starts_with("not ") {
        let subtoken = token[4..].trim();
        return !evaluate_expr(
            subtoken,
            schema,
            entity_name,
            instance_id,
            user,
            store,
            context,
            depth + 1,
        );
    }

    if token.contains(" not ") {
        let parts: Vec<&str> = token.splitn(2, " not ").collect();
        let left = parts[0].trim();
        let right = parts[1].trim();
        return evaluate_expr(
            left,
            schema,
            entity_name,
            instance_id,
            user,
            store,
            context,
            depth + 1,
        ) && !evaluate_expr(
            right,
            schema,
            entity_name,
            instance_id,
            user,
            store,
            context,
            depth + 1,
        );
    }

    if token.contains('.') {
        let parts: Vec<&str> = token.splitn(2, '.').collect();
        if parts.len() == 2 {
            let rel_name = parts[0].trim();
            let nested_perm = parts[1].trim();

            let subjects = get_relation_subjects(store, schema, entity_name, instance_id, rel_name);

            for subj in subjects {
                let res = evaluate_permission(
                    schema,
                    &subj.entity,
                    &subj.id,
                    nested_perm,
                    user,
                    store,
                    context,
                    depth + 1,
                );
                if res {
                    return true;
                }
            }
            return false;
        }
    }

    let subjects = get_relation_subjects(store, schema, entity_name, instance_id, token);

    if !subjects.is_empty() {
        if subjects
            .iter()
            .any(|s| format!("{}:{}", s.entity, s.id) == user)
        {
            return true;
        }
        return false;
    }

    if let Some(val) = store.get_attribute(entity_name, instance_id, token) {
        return match val {
            AttrValue::Int(i) => i != 0,
            AttrValue::Double(d) => d != 0.0,
            AttrValue::Bool(b) => b,
        };
    }

    false
}

fn evaluate_function(
    func_name: &str,
    args: &[&str],
    schema: &HashMap<String, Entity>,
    entity_name: &str,
    instance_id: &str,
    store: &AuthStore,
    context: &Context,
    user: &str,
    depth: usize,
) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    // New built-in function for tuple-to-userset rewriting.
    if func_name == "tuple_to_userset" {
        if args.len() == 2 {
            return evaluate_tuple_to_userset(
                args[0],
                args[1],
                schema,
                entity_name,
                instance_id,
                user,
                store,
                context,
                depth + 1,
            );
        } else {
            return false;
        }
    }
    if let Some((param_names, rule_body)) = get_rule_body(schema, func_name) {
        if param_names.len() != args.len() {
            return false;
        }
        let engine = Engine::new();
        let mut scope = Scope::new();
        for (i, param) in param_names.iter().enumerate() {
            let arg = args[i];
            let val = get_attr_as_double(store, entity_name, instance_id, arg);
            scope.push_dynamic(param.to_string(), Dynamic::from_float(val));
        }
        // Minimal rhai context simulation, need mapping Context -> Map
        // scope.push("context", Dynamic::from(Map::new()));
        match engine.eval_with_scope::<bool>(&mut scope, &rule_body) {
            Ok(result) => result,
            Err(_) => false,
        }
    } else {
        false
    }
}

fn evaluate_tuple_to_userset(
    tupleset_relation: &str,
    computed_relation: &str,
    schema: &HashMap<String, Entity>,
    entity_name: &str,
    instance_id: &str,
    user: &str,
    store: &AuthStore,
    context: &Context,
    depth: usize,
) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }

    let subjects =
        get_relation_subjects(store, schema, entity_name, instance_id, tupleset_relation);

    for subj in subjects {
        let res = evaluate_permission(
            schema,
            &subj.entity,
            &subj.id,
            computed_relation,
            user,
            store,
            context,
            depth + 1,
        );
        if res {
            return true;
        }
    }
    false
}

fn get_rule_body(
    schema: &HashMap<String, Entity>,
    rule_name: &str,
) -> Option<(Vec<String>, String)> {
    for entity in schema.values() {
        if let Some((params, body)) = entity.rules.get(rule_name) {
            return Some((params.clone(), body.clone()));
        }
    }
    None
}

pub fn evaluate_permission(
    schema: &HashMap<String, Entity>,
    entity_name: &str,
    instance_id: &str,
    perm_name: &str,
    user: &str,
    store: &AuthStore,
    context: &Context,
    depth: usize,
) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }

    if let Some(entity) = schema.get(entity_name) {
        if let Some(expr) = entity.permissions.get(perm_name) {
            return evaluate_expr(
                expr,
                schema,
                entity_name,
                instance_id,
                user,
                store,
                context,
                depth + 1,
            );
        }
    }

    let direct_subjects = store.get_subjects(entity_name, instance_id, perm_name);
    let resolved_subjects =
        get_relation_subjects(store, schema, entity_name, instance_id, perm_name);

    direct_subjects
        .into_iter()
        .chain(resolved_subjects.into_iter())
        .any(|s| format!("{}:{}", s.entity, s.id) == user)
}
