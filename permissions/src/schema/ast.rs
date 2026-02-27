use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub name: String,
    pub relations: HashMap<String, String>,
    pub permissions: HashMap<String, String>,
    // rule name -> (parameter names, rule body)
    pub rules: HashMap<String, (Vec<String>, String)>,
}
