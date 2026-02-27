use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Context {
    pub data: std::collections::HashMap<String, crate::storage::attribute::AttrValue>,
}

impl Context {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
        }
    }

    pub fn insert(&mut self, key: &str, val: crate::storage::attribute::AttrValue) {
        self.data.insert(key.to_string(), val);
    }
}
