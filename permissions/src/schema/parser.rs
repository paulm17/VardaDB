use std::collections::HashMap;
use regex::Regex;
use crate::schema::ast::Entity;

pub fn parse_schema(schema_str: &str) -> HashMap<String, Entity> {
    let rule_re = Regex::new(r"rule\s+(\w+)\s*\(([^)]*)\)\s*\{([^}]*)\}").unwrap();
    let mut global_rules: HashMap<String, (Vec<String>, String)> = HashMap::new();
    for cap in rule_re.captures_iter(schema_str) {
        let rule_name = cap[1].to_string();
        let params_str = cap[2].trim();
        let body = cap[3].trim().to_string();
        let params: Vec<String> = if params_str.is_empty() {
            Vec::new()
        } else {
            params_str.split(',')
                .map(|s| s.trim().split_whitespace().next().unwrap_or("").to_string())
                .collect()
        };
        global_rules.insert(rule_name, (params, body));
    }

    let mut entities = HashMap::new();
    let entity_re = Regex::new(r"entity\s+(\w+)\s*\{([^}]*)\}").unwrap();
    for cap in entity_re.captures_iter(schema_str) {
        let entity_name = cap[1].to_string();
        let block_content = cap[2].trim();
        let mut relations = HashMap::new();
        let mut permissions = HashMap::new();
        let mut rules = HashMap::new();
        for line in block_content.lines() {
            let line = line.trim();
            if line.starts_with("//") || line.is_empty() {
                continue;
            }
            if line.starts_with("relation") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let rel_name = parts[1];
                    let targets: Vec<String> = parts
                        .iter()
                        .filter(|&&s| s.starts_with('@'))
                        .map(|s| s.to_string())
                        .collect();
                    relations.insert(rel_name.to_string(), targets.join(" "));
                }
            } else if line.starts_with("permission") || line.starts_with("action") {
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() >= 2 {
                    let left = parts[0].trim();
                    let tokens: Vec<&str> = left.split_whitespace().collect();
                    if tokens.len() >= 2 {
                        let perm_name = tokens[1];
                        let expr = parts[1].trim();
                        permissions.insert(perm_name.to_string(), expr.to_string());
                    }
                }
            } else if line.starts_with("rule") {
                if let Some(cap) = rule_re.captures(line) {
                    let rule_name = cap[1].to_string();
                    let params_str = cap[2].trim();
                    let body = cap[3].trim().to_string();
                    let params: Vec<String> = if params_str.is_empty() {
                        Vec::new()
                    } else {
                        params_str.split(',')
                            .map(|s| s.trim().split_whitespace().next().unwrap_or("").to_string())
                            .collect()
                    };
                    rules.insert(rule_name, (params, body));
                }
            }
        }
        let entity = Entity {
            name: entity_name.clone(),
            relations,
            permissions,
            rules,
        };
        entities.insert(entity_name, entity);
    }
    if !global_rules.is_empty() {
        entities.insert(
            "__global__".to_string(),
            Entity {
                name: "__global__".to_string(),
                relations: HashMap::new(),
                permissions: HashMap::new(),
                rules: global_rules,
            },
        );
    }
    entities
}
