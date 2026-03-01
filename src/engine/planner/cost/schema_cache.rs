use std::collections::HashMap;
use async_graphql_parser::types::{TypeSystemDefinition, TypeKind as AstTypeKind, BaseType};
use crate::engine::planner::directives::{CostDirective, ListSizeDirective};

#[derive(Clone, Debug)]
pub struct FieldCostEntry {
    pub cost_weight: Option<f64>,
    pub list_size: Option<ListSizeDirective>,
    pub is_scalar: bool,
    pub is_list: bool,
    pub return_type: String,
}

#[derive(Clone, Debug)]
pub struct DemandControlledSchema {
    /// Mapping from `TypeName.fieldName` -> `FieldCostEntry`
    pub fields: HashMap<String, FieldCostEntry>,
}

impl DemandControlledSchema {
    pub fn new(sdl: &str) -> Result<Self, String> {
        let doc = async_graphql_parser::parse_schema(sdl).map_err(|e| e.to_string())?;
        let mut fields_map = HashMap::new();

        // Pass 1: Collect Enum names to treat them as scalars
        let mut enum_names = std::collections::HashSet::new();
        for def in &doc.definitions {
            if let TypeSystemDefinition::Type(type_def) = def {
                if matches!(type_def.node.kind, AstTypeKind::Enum(_)) {
                    enum_names.insert(type_def.node.name.node.to_string());
                }
            }
        }

        // Pass 2: Extract field metadata
        for def in &doc.definitions {
            if let TypeSystemDefinition::Type(type_def) = def {
                let parent_type_name = type_def.node.name.node.to_string();

                match &type_def.node.kind {
                    AstTypeKind::Object(obj_def) => {
                        for field in &obj_def.fields {
                            let field_name = field.node.name.node.to_string();
                            let key = format!("{}.{}", parent_type_name, field_name);

                            let mut field_type_name = "String".to_string();
                            let mut is_list = false;

                            match &field.node.ty.node.base {
                                BaseType::Named(n) => { field_type_name = n.to_string(); }
                                BaseType::List(inner) => {
                                    is_list = true;
                                    match &inner.base {
                                        BaseType::Named(n) => { field_type_name = n.to_string(); }
                                        _ => {}
                                    }
                                }
                            }

                            let is_enum = enum_names.contains(&field_type_name);
                            let is_scalar = matches!(
                                field_type_name.as_str(),
                                "String" | "Int" | "Boolean" | "ID" | "Float" | "Int64" | "DateTime" | "GeoPoint" | "Polygon" | "MultiPolygon"
                            ) || crate::engine::scalars::is_scalar_type(&field_type_name) || is_enum;

                            let mut cost_weight = None;
                            let mut list_size = None;

                            for dir in &field.node.directives {
                                if dir.node.name.node == "cost" {
                                    if let Ok(Some(cost_dir)) = CostDirective::from_directive(dir) {
                                        cost_weight = Some(cost_dir.weight);
                                    }
                                } else if dir.node.name.node == "listSize" {
                                    if let Ok(Some(ls_dir)) = ListSizeDirective::from_directive(dir) {
                                        list_size = Some(ls_dir);
                                    }
                                }
                            }

                            fields_map.insert(key, FieldCostEntry {
                                cost_weight,
                                list_size,
                                is_scalar,
                                is_list,
                                return_type: field_type_name,
                            });
                        }
                    },
                    AstTypeKind::Interface(int_def) => {
                        for field in &int_def.fields {
                            let field_name = field.node.name.node.to_string();
                            let key = format!("{}.{}", parent_type_name, field_name);

                            let mut field_type_name = "String".to_string();
                            let mut is_list = false;

                            match &field.node.ty.node.base {
                                BaseType::Named(n) => { field_type_name = n.to_string(); }
                                BaseType::List(inner) => {
                                    is_list = true;
                                    match &inner.base {
                                        BaseType::Named(n) => { field_type_name = n.to_string(); }
                                        _ => {}
                                    }
                                }
                            }

                            let is_enum = enum_names.contains(&field_type_name);
                            let is_scalar = matches!(
                                field_type_name.as_str(),
                                "String" | "Int" | "Boolean" | "ID" | "Float" | "Int64" | "DateTime" | "GeoPoint" | "Polygon" | "MultiPolygon"
                            ) || crate::engine::scalars::is_scalar_type(&field_type_name) || is_enum;

                            let mut cost_weight = None;
                            let mut list_size = None;

                            for dir in &field.node.directives {
                                if dir.node.name.node == "cost" {
                                    if let Ok(Some(cost_dir)) = CostDirective::from_directive(dir) {
                                        cost_weight = Some(cost_dir.weight);
                                    }
                                } else if dir.node.name.node == "listSize" {
                                    if let Ok(Some(ls_dir)) = ListSizeDirective::from_directive(dir) {
                                        list_size = Some(ls_dir);
                                    }
                                }
                            }

                            fields_map.insert(key, FieldCostEntry {
                                cost_weight,
                                list_size,
                                is_scalar,
                                is_list,
                                return_type: field_type_name,
                            });
                        }
                    },
                    _ => {}
                }
            }
        }

        // Pass 3: Manually register dynamic query/mutation root fields
        // queryUser -> returns [User]
        for def in &doc.definitions {
            if let TypeSystemDefinition::Type(type_def) = def {
                let type_name = type_def.node.name.node.to_string();
                if matches!(type_def.node.kind, AstTypeKind::Object(_)) && type_name != "Query" && type_name != "Mutation" && type_name != "Subscription" && !type_name.starts_with("__") {
                    
                    // Register dynamic Query fields
                    fields_map.insert(format!("Query.get{}", type_name), FieldCostEntry {
                        cost_weight: Some(1.0),
                        list_size: None,
                        is_scalar: false,
                        is_list: false,
                        return_type: type_name.clone(),
                    });
                    
                    // We assume a default list size of 10 if not specified on queryUser
                    fields_map.insert(format!("Query.query{}", type_name), FieldCostEntry {
                        cost_weight: Some(1.0),
                        list_size: Some(ListSizeDirective { assumed_size: Some(10), slicing_arguments: vec![], require_one_slicing_argument: false }),
                        is_scalar: false,
                        is_list: true,
                        return_type: type_name.clone(),
                    });
                    
                    fields_map.insert(format!("Query.aggregate{}", type_name), FieldCostEntry {
                        cost_weight: Some(1.0),
                        list_size: None,
                        is_scalar: false,
                        is_list: false,
                        return_type: format!("{}AggregateResult", type_name),
                    });
                }
            }
        }

        Ok(Self { fields: fields_map })
    }

    pub fn get(&self, type_name: &str, field_name: &str) -> Option<&FieldCostEntry> {
        self.fields.get(&format!("{}.{}", type_name, field_name))
    }
}
