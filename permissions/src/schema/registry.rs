use std::collections::HashMap;
use crate::schema::ast::Entity;
use crate::schema::parser::parse_schema;

#[derive(Debug, Clone)]
pub struct SchemaRegistry {
    pub namespaces: HashMap<String, HashMap<String, Entity>>,
}

impl SchemaRegistry {
    pub fn new() -> Self {
        Self {
            namespaces: HashMap::new(),
        }
    }

    pub fn load_namespace(&mut self, namespace: &str, schema_str: &str) {
        let parsed = parse_schema(schema_str);
        self.namespaces.insert(namespace.to_string(), parsed);
    }

    pub fn get_entity(&self, namespace: &str, entity_name: &str) -> Option<&Entity> {
        self.namespaces
            .get(namespace)
            .and_then(|ns| ns.get(entity_name))
    }

    pub fn get_namespace(&self, namespace: &str) -> Option<&HashMap<String, Entity>> {
        self.namespaces.get(namespace)
    }
}
