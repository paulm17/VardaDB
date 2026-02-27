use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AttrValue {
    Int(i32),
    Double(f64),
    Bool(bool),
    // Can be extended with string, string array, etc.
}

impl std::fmt::Display for AttrValue {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            AttrValue::Int(i) => write!(f, "{}", i),
            AttrValue::Double(d) => write!(f, "{}", d),
            AttrValue::Bool(b) => write!(f, "{}", b),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributeTuple {
    pub entity_type: String,
    pub entity_id: String,
    pub attribute: String,
    pub value: AttrValue,
}
