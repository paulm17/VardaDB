use serde_json::Value;
use std::fmt::Write;

pub enum OperationType {
    Query,
    Mutation,
}

impl OperationType {
    fn as_str(&self) -> &'static str {
        match self {
            OperationType::Query => "query",
            OperationType::Mutation => "mutation",
        }
    }
}

pub struct GraphqlBuilder {
    op_type: OperationType,
    root_field: String,
    args: Vec<(String, Value)>,
    return_fields: Vec<String>,
}

impl GraphqlBuilder {
    pub fn new_query(root_field: &str) -> Self {
        Self {
            op_type: OperationType::Query,
            root_field: root_field.to_string(),
            args: Vec::new(),
            return_fields: Vec::new(),
        }
    }

    pub fn new_mutation(root_field: &str) -> Self {
        Self {
            op_type: OperationType::Mutation,
            root_field: root_field.to_string(),
            args: Vec::new(),
            return_fields: Vec::new(),
        }
    }

    /// Add an argument to the root field.
    /// Example: .arg("input", json!({ ... }))
    pub fn arg(mut self, name: &str, value: Value) -> Self {
        self.args.push((name.to_string(), value));
        self
    }

    /// Specify fields to return.
    /// Supports nested fields via raw string syntax if needed, e.g. "language { id }"
    pub fn return_fields(mut self, fields: &[&str]) -> Self {
        for f in fields {
            self.return_fields.push(f.to_string());
        }
        self
    }

    /// Build the full GraphQL query string and variables.
    /// Returns (query_string, variables).
    /// Currently variables are always null as arguments are inlined.
    pub fn build(self) -> (String, Value) {
        let mut query = String::new();
        
        write!(query, "{} {{ {} ", self.op_type.as_str(), self.root_field).unwrap();

        if !self.args.is_empty() {
            write!(query, "(").unwrap();
            for (i, (key, val)) in self.args.iter().enumerate() {
                if i > 0 {
                    write!(query, ", ").unwrap();
                }
                // Serialize the value to JSON string which is valid GraphQL value format
                let val_str = serde_json::to_string(val).unwrap();
                write!(query, "{}: {}", key, val_str).unwrap();
            }
            write!(query, ")").unwrap();
        }

        if !self.return_fields.is_empty() {
            write!(query, " {{ ").unwrap();
            for f in &self.return_fields {
                write!(query, "{} ", f).unwrap();
            }
            write!(query, "}}").unwrap();
        }

        write!(query, " }}").unwrap();

        (query, Value::Null)
    }
}
