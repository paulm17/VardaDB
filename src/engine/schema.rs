use async_graphql::dynamic::{self};

// This is our "Engine" Schema, which currently wraps async-graphql
#[derive(Clone)]
struct TypeMetadata {
    #[allow(dead_code)]
    type_name: String,
    uniques: Vec<String>,
    inverses: Vec<crate::engine::resolver::InverseInfo>,
    search_fields: std::collections::HashMap<String, Vec<String>>,
    cascade_fields: Vec<(String, String)>,
    interface_implementations: Vec<String>, // Interfaces this type implements
    validate_fields: std::collections::HashMap<String, Vec<ValidationRule>>,
    relations: std::collections::HashMap<String, String>,
    kind: TypeKind,
}

#[derive(Clone, PartialEq, Debug)]
enum TypeKind {
    Object,
    Interface,
    Union(Vec<String>), // Possible types
}

#[derive(Clone, Debug)]
enum ValidationRule {
    Regex(String),
    Length { min: Option<i64>, max: Option<i64> },
    Range { min: Option<f64>, max: Option<f64> },
}

#[derive(Clone, Debug)]
pub struct GeoPointData {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Clone, Debug)]
pub struct GeoPolygonData {
    pub exterior: Vec<GeoPointData>,
    pub interiors: Vec<Vec<GeoPointData>>,
}

#[derive(Clone, Debug)]
pub struct GeoMultiPolygonData {
    pub polygons: Vec<GeoPolygonData>,
}

pub struct Schema {
    inner: async_graphql::dynamic::Schema,
}

impl Schema {

    pub async fn execute(&self, request: impl Into<async_graphql::Request>) -> async_graphql::Response {
        self.inner.execute(request).await
    }

    pub fn inner(&self) -> &async_graphql::dynamic::Schema {
        &self.inner
    }


    pub fn create_builder(sdl: &str) -> Result<dynamic::SchemaBuilder, String> {
        let system_sdl = "
            scalar DateTime
            scalar Int64
            input NearFilter {
                distance: Float!
                coordinate: PointInput!
            }
        ";
        let full_sdl = format!("{}\n{}", system_sdl, sdl);
        let doc = async_graphql_parser::parse_schema(&full_sdl).map_err(|e| e.to_string())?;

        let mut query_fields: Vec<dynamic::Field> = Vec::new();
        let mut mutation_fields: Vec<dynamic::Field> = Vec::new();
        let mut subscription_fields: Vec<dynamic::SubscriptionField> = Vec::new(); // Subscription Root
        let mut types: Vec<dynamic::Type> = Vec::new();
        // Register Scalars FIRST to ensure they are available for Input Objects
        crate::engine::scalars::register_scalars(&mut types);

        use async_graphql_parser::types::{TypeSystemDefinition, TypeKind as AstTypeKind, BaseType};

        // Pass 0: Pre-scan for field types (IsList map) for Inverse resolution
        let mut type_field_is_list: std::collections::HashMap<String, std::collections::HashMap<String, bool>> = std::collections::HashMap::new();
        for def in &doc.definitions {
             if let TypeSystemDefinition::Type(type_def) = def {
                if let AstTypeKind::Object(obj_def) = &type_def.node.kind {
                    let type_name = type_def.node.name.node.to_string();
                    let mut fields_map = std::collections::HashMap::new();
                    for field in &obj_def.fields {
                        let f_name = field.node.name.node.to_string();
                        let is_list = matches!(field.node.ty.node.base, BaseType::List(_));
                        fields_map.insert(f_name, is_list);
                    }
                    type_field_is_list.insert(type_name, fields_map);
                }
             }
        }

        // Pass 1: Collect Metadata for ALL types
        let mut metadata_map: std::collections::HashMap<String, TypeMetadata> = std::collections::HashMap::new();
        
        for def in &doc.definitions {
            if let TypeSystemDefinition::Type(type_def) = def {
                let type_name = type_def.node.name.node.to_string();
                if type_name == "Query" || type_name == "Mutation" || type_name == "Subscription" || type_name.starts_with("__") {
                    continue;
                }

                match &type_def.node.kind {
                    AstTypeKind::Object(obj_def) => {
                        let mut unique_fields: Vec<String> = Vec::new();
                        let mut inverses: Vec<crate::engine::resolver::InverseInfo> = Vec::new();
                        let mut type_search_fields: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
                        let mut cascade_fields: Vec<(String, String)> = Vec::new();
                        let mut relations: std::collections::HashMap<String, String> = std::collections::HashMap::new();

                        for field in &obj_def.fields {
                            let field_name = field.node.name.node.to_string();
                            
                            // Uniques
                            if field.node.directives.iter().any(|d| d.node.name.node == "unique") {
                                unique_fields.push(field_name.clone());
                            }
                            // Search
                            if let Some(directive) = field.node.directives.iter().find(|d| d.node.name.node == "search") {
                                let mut tokenizers = Vec::new();
                                for (name, value) in &directive.node.arguments {
                                    if name.node == "by" {
                                        match &value.node {
                                            async_graphql_value::ConstValue::List(items) => {
                                                for item in items {
                                                    match item {
                                                        async_graphql_value::ConstValue::Enum(n) => tokenizers.push(n.to_string()),
                                                        async_graphql_value::ConstValue::String(s) => tokenizers.push(s.clone()),
                                                        _ => {}
                                                    }
                                                }
                                            }
                                            async_graphql_value::ConstValue::Enum(n) => tokenizers.push(n.to_string()),
                                            async_graphql_value::ConstValue::String(s) => tokenizers.push(s.clone()),
                                            _ => {}
                                        }
                                        // println!("[Debug] Field '{}' search strategy: {:?}", field_name, tokenizers);
                                    }
                                }
                                if tokenizers.is_empty() {
                                    tokenizers.push("term".to_string());
                                }
                                type_search_fields.insert(field_name.clone(), tokenizers);
                            }
                            // Cascade
                            if field.node.directives.iter().any(|d| d.node.name.node == "cascade") {
                                 let field_type_name = match &field.node.ty.node.base {
                                    BaseType::Named(n) => n.to_string(),
                                    BaseType::List(inner) => match &inner.base {
                                        BaseType::Named(n) => n.to_string(),
                                        _ => "String".to_string()
                                    },
                                };
                                cascade_fields.push((field_name.clone(), field_type_name));
                            }

                            // Inverse
                            if let Some(dir) = field.node.directives.iter().find(|d| d.node.name.node == "hasInverse") {
                                if let Some((_, val)) = dir.node.arguments.iter().find(|(name, _)| name.node == "field") {
                                    if let async_graphql::Value::String(inverse_field_name) = &val.node {
                                         let field_type_name = match &field.node.ty.node.base {
                                            BaseType::Named(n) => n.to_string(),
                                            BaseType::List(inner) => match &inner.base {
                                                BaseType::Named(n) => n.to_string(),
                                                _ => "String".to_string()
                                            },
                                        };
                                        let inverse_type = field_type_name;
                                        let inverse_is_list = type_field_is_list.get(&inverse_type)
                                            .and_then(|f_map| f_map.get(inverse_field_name))
                                            .cloned()
                                            .unwrap_or(false);

                                        inverses.push(crate::engine::resolver::InverseInfo {
                                            field: field_name.clone(),
                                            inverse_type,
                                            inverse_field: inverse_field_name.clone(),
                                            inverse_is_list,
                                        });
                                    }
                                }
                            }
                        }
                        
                        // Parse Validations (Second Pass over fields to keep structure clean)
                         let mut validate_fields = std::collections::HashMap::new();
                         for field in &obj_def.fields {
                            let field_name = field.node.name.node.to_string();
                            let field_type_name = match &field.node.ty.node.base {
                                BaseType::Named(n) => n.to_string(),
                                BaseType::List(inner) => match &inner.base {
                                    BaseType::Named(n) => n.to_string(),
                                    _ => "String".to_string()
                                },
                            };
                            let is_scalar = matches!(field_type_name.as_str(), "String" | "Int" | "Boolean" | "ID" | "Float" | "Int64" | "DateTime" | "GeoPoint" | "Polygon" | "MultiPolygon");
                            if !is_scalar {
                                relations.insert(field_name.clone(), field_type_name.clone());
                            }

                            let mut rules = Vec::new();
                            
                            // @regex(pattern: "...")
                            if let Some(dir) = field.node.directives.iter().find(|d| d.node.name.node == "regex") {
                                if let Some((_, val)) = dir.node.arguments.iter().find(|(name, _)| name.node == "pattern") {
                                    if let async_graphql::Value::String(pattern) = &val.node {
                                        rules.push(ValidationRule::Regex(pattern.clone()));
                                    }
                                }
                            }
                            
                            // @length(min: Int, max: Int)
                            if let Some(dir) = field.node.directives.iter().find(|d| d.node.name.node == "length") {
                                let mut min = None;
                                let mut max = None;
                                if let Some((_, val)) = dir.node.arguments.iter().find(|(name, _)| name.node == "min") {
                                     if let async_graphql::Value::Number(n) = &val.node { min = n.as_i64(); }
                                }
                                if let Some((_, val)) = dir.node.arguments.iter().find(|(name, _)| name.node == "max") {
                                     if let async_graphql::Value::Number(n) = &val.node { max = n.as_i64(); }
                                }
                                if min.is_some() || max.is_some() {
                                    rules.push(ValidationRule::Length { min, max });
                                }
                            }

                            // @range(min: Float, max: Float)
                            if let Some(dir) = field.node.directives.iter().find(|d| d.node.name.node == "range") {
                                let mut min = None;
                                let mut max = None;
                                if let Some((_, val)) = dir.node.arguments.iter().find(|(name, _)| name.node == "min") {
                                     if let async_graphql::Value::Number(n) = &val.node { min = n.as_f64(); }
                                }
                                if let Some((_, val)) = dir.node.arguments.iter().find(|(name, _)| name.node == "max") {
                                     if let async_graphql::Value::Number(n) = &val.node { max = n.as_f64(); }
                                }
                                if min.is_some() || max.is_some() {
                                    rules.push(ValidationRule::Range { min, max });
                                }
                            }
                            
                            if !rules.is_empty() {
                                validate_fields.insert(field_name, rules);
                            }
                        }

                        let interfaces: Vec<String> = obj_def.implements.iter().map(|n| n.node.to_string()).collect();

                        metadata_map.insert(type_name.clone(), TypeMetadata {
                            type_name: type_name.clone(),
                            uniques: unique_fields,
                            inverses,
                            search_fields: type_search_fields,
                            cascade_fields,
                            interface_implementations: interfaces,
                            validate_fields,
                            relations,
                            kind: TypeKind::Object,
                        });
                    },
                    AstTypeKind::Interface(_int_def) => {
                         metadata_map.insert(type_name.clone(), TypeMetadata {
                            type_name: type_name.clone(),
                            uniques: vec![],
                            inverses: vec![],
                            search_fields: std::collections::HashMap::new(),
                            cascade_fields: vec![],
                            interface_implementations: vec![],
                            validate_fields: std::collections::HashMap::new(),
                            relations: std::collections::HashMap::new(),
                            kind: TypeKind::Interface,
                        });
                    },
                     AstTypeKind::Union(union_def) => {
                         let possible_types: Vec<String> = union_def.members.iter().map(|n| n.node.to_string()).collect();
                         metadata_map.insert(type_name.clone(), TypeMetadata {
                            type_name: type_name.clone(),
                            uniques: vec![],
                            inverses: vec![],
                            search_fields: std::collections::HashMap::new(),
                            cascade_fields: vec![],
                            interface_implementations: vec![],
                            validate_fields: std::collections::HashMap::new(),
                            relations: std::collections::HashMap::new(),
                            kind: TypeKind::Union(possible_types),
                        });
                    },
                    _ => {}
                }
            }
        }
        
        let metadata_arc = std::sync::Arc::new(metadata_map);

        // Pass 2: Generate Schema Artifacts
        for def in &doc.definitions {
            if let TypeSystemDefinition::Type(type_def) = def {
                let type_name = type_def.node.name.node.to_string();
                if type_name == "Query" || type_name == "Mutation" || type_name == "Subscription" || type_name.starts_with("__") {
                    continue;
                }
                
                // Handle Scalars immediately without metadata
                if let AstTypeKind::Scalar = &type_def.node.kind {
                    types.push(dynamic::Type::Scalar(dynamic::Scalar::new(type_name)));
                    continue;
                }
                
                match &type_def.node.kind {
                    AstTypeKind::Object(obj_def) => {
                        // Get Metadata
                        let meta = metadata_arc.get(&type_name).expect(&format!("Metadata missing for type {}", type_name)); 
                        let unique_fields = &meta.uniques;
                        let inverses = &meta.inverses;
                        let type_search_fields = &meta.search_fields;

                        let mut obj = dynamic::Object::new(type_name.clone());
                        if type_name != "GeoPoint" {
                            obj = obj.field(dynamic::Field::new("uid", dynamic::TypeRef::named_nn("ID"), |ctx| {
                                dynamic::FieldFuture::new(async move {
                                    let uid = ctx.parent_value.try_downcast_ref::<u64>()?;
                                    Ok(Some(dynamic::FieldValue::value(async_graphql::Value::String(uid.to_string()))))
                                })
                            }));
                        }
                        let mut input = dynamic::InputObject::new(format!("{}Input", type_name))
                            .field(dynamic::InputValue::new("uid", dynamic::TypeRef::named(dynamic::TypeRef::ID)));
                        let mut filter_input = dynamic::InputObject::new(format!("{}Filter", type_name));
                        
                        // Parse Fields
                        let mut scalar_fields_map: Vec<(String, String)> = Vec::new();
                        
                        // Implement Interfaces
                        for interface in &meta.interface_implementations {
                            obj = obj.implement(interface.clone());
                        }

                        for field in &obj_def.fields {
                            let field_name = field.node.name.node.to_string();
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

                            let is_scalar = matches!(field_type_name.as_str(), "String" | "Int" | "Boolean" | "ID" | "Float" | "Int64" | "DateTime" | "GeoPoint" | "Polygon" | "MultiPolygon")
                                            || crate::engine::scalars::is_scalar_type(&field_type_name);
                            let is_relation = !is_scalar;
                           
                            // Check if field type is polymorphic (Interface or Union)
                            // We need to check metadata map. If missing, assume scalar/standard object.
                            let is_polymorphic = if let Some(target_meta) = metadata_arc.get(&field_type_name) {
                                matches!(target_meta.kind, TypeKind::Interface | TypeKind::Union(_))
                            } else {
                                false
                            };

                            // Object Field
                             let ty_ref = match field_type_name.as_str() {
                                "ID" => dynamic::TypeRef::named_nn(dynamic::TypeRef::ID),
                                "String" => dynamic::TypeRef::named(dynamic::TypeRef::STRING),
                                "Int" => dynamic::TypeRef::named(dynamic::TypeRef::INT),
                                "Boolean" => dynamic::TypeRef::named(dynamic::TypeRef::BOOLEAN),
                                "Int64" => dynamic::TypeRef::named("Int64"),
                                "DateTime" => dynamic::TypeRef::named("DateTime"),
                                "GeoPoint" => dynamic::TypeRef::named("GeoPoint"),
                                "Float" => dynamic::TypeRef::named(dynamic::TypeRef::FLOAT),
                                _ => {
                                    if is_list { dynamic::TypeRef::named_list(field_type_name.clone()) } 
                                    else { dynamic::TypeRef::named(field_type_name.clone()) }
                                }
                            };
                            
                            let fname_clone = field_name.clone();
                            let type_name_clone = type_name.clone();
                            let field_type_name_clone = field_type_name.clone();
                            let is_rel = is_relation;
                            let is_poly = is_polymorphic;
                            
                            obj = obj.field(dynamic::Field::new(field_name.clone(), ty_ref, move |ctx| { 
                                let field_key = fname_clone.clone();
                                let t_name = type_name_clone.clone();
                                let f_type_name = field_type_name_clone.clone();
                                dynamic::FieldFuture::new(async move {
                                    // Special handling for GeoPoint (Embedded Object)
                                    if t_name == "GeoPoint" {
                                        let val = ctx.parent_value.try_downcast_ref::<async_graphql::Value>()?;
                                        if let async_graphql::Value::Object(map) = val {
                                             if let Some(v) = map.get(field_key.as_str()) {
                                                return Ok(Some(dynamic::FieldValue::value(v.clone())));
                                             }
                                        }
                                        return Ok(None);
                                    }

                                    let parent_uid_result = ctx.parent_value.try_downcast_ref::<u64>();
                                    if let Ok(uid) = parent_uid_result {
                                    // Standard Resolver (Scalar or Relation)
                                    use crate::engine::resolver::Resolver;
                                        let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();
                                        
                                        if let Some(val) = resolver.resolve(*uid, &field_key) {
                                            if is_rel {
                                                match val {
                                                    async_graphql::Value::List(items) => {
                                                        let mut fvs = Vec::new();
                                                        for item in items {
                                                            let uid_opt = match item {
                                                                async_graphql::Value::String(s) => s.parse::<u64>().ok(),
                                                                async_graphql::Value::Number(n) => n.as_u64(),
                                                                _ => None
                                                            };
                                                            if let Some(u) = uid_opt { 
                                                                // If polymorphic, need concrete type
                                                                if is_poly {
                                                                    if let Some(ctype) = resolver.get_node_type(u) {
                                                                        fvs.push(dynamic::FieldValue::with_type(dynamic::FieldValue::owned_any(u), ctype));
                                                                    } else {
                                                                        // Cannot resolve type? Skip or Error? Skippping safe
                                                                    }
                                                                } else {
                                                                    fvs.push(dynamic::FieldValue::owned_any(u)); 
                                                                }
                                                            }
                                                        }
                                                        Ok(Some(dynamic::FieldValue::list(fvs)))
                                                    },
                                                    async_graphql::Value::String(s) => {
                                                         if let Ok(u) = s.parse::<u64>() {
                                                             if is_poly {
                                                                 if let Some(ctype) = resolver.get_node_type(u) {
                                                                     Ok(Some(dynamic::FieldValue::with_type(dynamic::FieldValue::owned_any(u), ctype)))
                                                                 } else { Ok(None) }
                                                             } else {
                                                                 Ok(Some(dynamic::FieldValue::owned_any(u)))
                                                             }
                                                         } else { Ok(None) }
                                                    },
                                                    async_graphql::Value::Number(n) => {
                                                         if let Some(u) = n.as_u64() {
                                                             if is_poly {
                                                                 if let Some(ctype) = resolver.get_node_type(u) {
                                                                     Ok(Some(dynamic::FieldValue::with_type(dynamic::FieldValue::owned_any(u), ctype)))
                                                                 } else { Ok(None) }
                                                             } else {
                                                                 Ok(Some(dynamic::FieldValue::owned_any(u)))
                                                             }
                                                         } else { Ok(None) }
                                                    },
                                                    _ => Ok(None)
                                                }
                                            } else {
                                                // Handle Scalar / Embedded GeoPoint
                                                match val {
                                                    async_graphql::Value::Object(map) if f_type_name == "GeoPoint" => {
                                                        // Pass Object as Custom GeoPointData
                                                        let mut lat_v = 0.0;
                                                        let mut lon_v = 0.0;
                                                        if let Some(lat) = map.get("latitude") { 
                                                            if let async_graphql::Value::Number(n) = lat { lat_v = n.as_f64().unwrap_or(0.0); }
                                                        }
                                                        if let Some(lon) = map.get("longitude") { 
                                                            if let async_graphql::Value::Number(n) = lon { lon_v = n.as_f64().unwrap_or(0.0); }
                                                        }
                                                        Ok(Some(dynamic::FieldValue::owned_any(GeoPointData { latitude: lat_v, longitude: lon_v })))
                                                    },
                                                    async_graphql::Value::Object(map) if f_type_name == "Polygon" => {
                                                        // Helper to parse point
                                                        let parse_point = |v: &async_graphql::Value| -> Option<GeoPointData> {
                                                            if let async_graphql::Value::Object(m) = v {
                                                                let lat = m.get("latitude").and_then(|v| if let async_graphql::Value::Number(n) = v { n.as_f64() } else { None }).unwrap_or(0.0);
                                                                let lon = m.get("longitude").and_then(|v| if let async_graphql::Value::Number(n) = v { n.as_f64() } else { None }).unwrap_or(0.0);
                                                                Some(GeoPointData { latitude: lat, longitude: lon })
                                                            } else { None }
                                                        };

                                                        let mut exterior = vec![];
                                                        if let Some(async_graphql::Value::List(l)) = map.get("exterior") {
                                                            for item in l { if let Some(p) = parse_point(item) { exterior.push(p); } }
                                                        }

                                                        let mut interiors = vec![];
                                                        if let Some(async_graphql::Value::List(l)) = map.get("interiors") {
                                                            for ring_val in l {
                                                                if let async_graphql::Value::List(ring_list) = ring_val {
                                                                    let mut ring = vec![];
                                                                    for item in ring_list { if let Some(p) = parse_point(item) { ring.push(p); } }
                                                                    interiors.push(ring);
                                                                }
                                                            }
                                                        }
                                                        Ok(Some(dynamic::FieldValue::owned_any(GeoPolygonData { exterior, interiors })))
                                                    },
                                                    async_graphql::Value::Object(map) if f_type_name == "MultiPolygon" => {
                                                        // Helper to parse point
                                                        let parse_point = |v: &async_graphql::Value| -> Option<GeoPointData> {
                                                            if let async_graphql::Value::Object(m) = v {
                                                                let lat = m.get("latitude").and_then(|v| if let async_graphql::Value::Number(n) = v { n.as_f64() } else { None }).unwrap_or(0.0);
                                                                let lon = m.get("longitude").and_then(|v| if let async_graphql::Value::Number(n) = v { n.as_f64() } else { None }).unwrap_or(0.0);
                                                                Some(GeoPointData { latitude: lat, longitude: lon })
                                                            } else { None }
                                                        };
                                                        let parse_poly = |v: &async_graphql::Value| -> Option<GeoPolygonData> {
                                                            if let async_graphql::Value::Object(m) = v {
                                                                let mut exterior = vec![];
                                                                if let Some(async_graphql::Value::List(l)) = m.get("exterior") {
                                                                    for item in l { if let Some(p) = parse_point(item) { exterior.push(p); } }
                                                                }
                                                                let mut interiors = vec![];
                                                                if let Some(async_graphql::Value::List(l)) = m.get("interiors") {
                                                                    for ring_val in l {
                                                                        if let async_graphql::Value::List(ring_list) = ring_val {
                                                                            let mut ring = vec![];
                                                                            for item in ring_list { if let Some(p) = parse_point(item) { ring.push(p); } }
                                                                            interiors.push(ring);
                                                                        }
                                                                    }
                                                                }
                                                                Some(GeoPolygonData { exterior, interiors })
                                                            } else { None }
                                                        };

                                                        let mut polygons = vec![];
                                                        if let Some(async_graphql::Value::List(l)) = map.get("polygons") {
                                                            for item in l { if let Some(p) = parse_poly(item) { polygons.push(p); } }
                                                        }
                                                        Ok(Some(dynamic::FieldValue::owned_any(GeoMultiPolygonData { polygons })))
                                                    },
                                                    v => Ok(Some(dynamic::FieldValue::value(v))), 
                                                }
                                            }
                                        } else {
                                            Ok(None)
                                        }
                                    } else {
                                        Ok(None)
                                    }
                                })
                            }));

                            // Input fields
                             if is_scalar && field_type_name != "ID" {
                                let input_ty_ref = match field_type_name.as_str() {
                                    "String" => dynamic::TypeRef::named(dynamic::TypeRef::STRING),
                                    "Int" => dynamic::TypeRef::named(dynamic::TypeRef::INT),
                                    "Boolean" => dynamic::TypeRef::named(dynamic::TypeRef::BOOLEAN),
                                    "Float" => dynamic::TypeRef::named(dynamic::TypeRef::FLOAT),
                                    "Int64" => dynamic::TypeRef::named("Int64"),
                                    "DateTime" => dynamic::TypeRef::named("DateTime"),
                                    "GeoPoint" => dynamic::TypeRef::named("GeoPointInput"),
                                    _ => {
                                        if crate::engine::scalars::is_scalar_type(&field_type_name) {
                                            dynamic::TypeRef::named(field_type_name.clone())
                                        } else {
                                            dynamic::TypeRef::named(format!("{}Input", field_type_name))
                                        }
                                    },
                                };
                                input = input.field(dynamic::InputValue::new(field_name.clone(), input_ty_ref.clone()));
                                scalar_fields_map.push((field_name.clone(), field_type_name.clone()));
                                
                                let filter_ty_name = if crate::engine::scalars::is_scalar_type(&field_type_name) {
                                    crate::engine::scalars::get_scalar_filter_type(&field_type_name).to_string()
                                } else {
                                    format!("{}Filter", field_type_name)
                                };
                                
                                filter_input = filter_input.field(dynamic::InputValue::new(field_name, dynamic::TypeRef::named(filter_ty_name)));
                            } else if is_relation {
                                let rel_input_type = format!("{}Input", field_type_name);
                                if is_list {
                                    input = input.field(dynamic::InputValue::new(field_name.clone(), dynamic::TypeRef::named_list(rel_input_type)));
                                } else {
                                    input = input.field(dynamic::InputValue::new(field_name.clone(), dynamic::TypeRef::named(rel_input_type)));
                                }
                            }
                        }


                        // Add recursive logical connectors
                        let filter_ty = format!("{}Filter", type_name);
                        filter_input = filter_input
                            .field(dynamic::InputValue::new("and", dynamic::TypeRef::named_list(filter_ty.clone())))
                            .field(dynamic::InputValue::new("or", dynamic::TypeRef::named_list(filter_ty.clone())))
                            .field(dynamic::InputValue::new("not", dynamic::TypeRef::named(filter_ty.clone())));

                        types.push(dynamic::Type::Object(obj));
                        if type_name != "GeoPoint" {
                            types.push(dynamic::Type::InputObject(input));
                            types.push(dynamic::Type::InputObject(filter_input));
                        }
                        
                        // Sort Input
                        let mut sort_input = dynamic::InputObject::new(format!("{}Sort", type_name));
                         for (f_name, _) in &scalar_fields_map {
                             sort_input = sort_input.field(dynamic::InputValue::new(f_name.clone(), dynamic::TypeRef::named("SortDirection")));
                         }
                         types.push(dynamic::Type::InputObject(sort_input));

                        // --- ROOTS for OBJECTS ONLY ---
                        
                        // 1. Query List
                        let list_query_name = format!("query{}", type_name);
                        let type_name_for_list = type_name.clone();
                        let filter_type_name = format!("{}Filter", type_name);
                        
                        query_fields.push(dynamic::Field::new(list_query_name, dynamic::TypeRef::named_list(type_name_for_list.clone()), move |ctx| {
                            let t_name = type_name_for_list.clone();
                            dynamic::FieldFuture::new(async move {
                                use crate::engine::resolver::Resolver;
                                let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();
                                let mut filter_map = std::collections::HashMap::new();
                                if let Ok(filter_arg) = ctx.args.try_get("filter") { filter_map = filter_arg.deserialize()?; }
                                let mut sort_map = std::collections::HashMap::new();
                                if let Ok(sort_arg) = ctx.args.try_get("sort") { sort_map = sort_arg.deserialize()?; }
                                let mut first = None;
                                if let Ok(limit_arg) = ctx.args.try_get("first") { if let Ok(n) = limit_arg.u64() { first = Some(n as usize); } }
                                let mut after = None;
                                if let Ok(cursor_arg) = ctx.args.try_get("after") { if let Ok(s) = cursor_arg.string() { after = Some(s.to_string()); } }

                                let uids = resolver.scan_nodes(&t_name, filter_map, sort_map, first, after);
                                let result: Vec<dynamic::FieldValue> = uids.into_iter().map(|uid| dynamic::FieldValue::owned_any(uid)).collect();
                                Ok(Some(dynamic::FieldValue::list(result)))
                            })
                        }).argument(dynamic::InputValue::new("filter", dynamic::TypeRef::named(filter_type_name)))
                          .argument(dynamic::InputValue::new("sort", dynamic::TypeRef::named(format!("{}Sort", type_name))))
                          .argument(dynamic::InputValue::new("first", dynamic::TypeRef::named(dynamic::TypeRef::INT)))
                          .argument(dynamic::InputValue::new("after", dynamic::TypeRef::named(dynamic::TypeRef::STRING))));

                        // 2. Query Single
                        let query_single_name = format!("get{}", type_name);
                        let type_name_single = type_name.clone();
                        let uniques_single = unique_fields.clone();
                        let mut query_field = dynamic::Field::new(query_single_name, dynamic::TypeRef::named(type_name_single.clone()), move |ctx| {
                            let t_name = type_name_single.clone();
                            let u_fields = uniques_single.clone();
                            dynamic::FieldFuture::new(async move {
                                use crate::engine::resolver::Resolver;
                                let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();
                                if let Ok(id_arg) = ctx.args.try_get("id") {
                                    let id_str = id_arg.string()?.to_string();
                                    let uid = if id_str.starts_with("0x") { u64::from_str_radix(&id_str[2..], 16).unwrap_or(0) } else { id_str.parse::<u64>().unwrap_or(0) };
                                    if uid > 0 && resolver.node_exists(&t_name, uid) { return Ok(Some(dynamic::FieldValue::owned_any(uid))); }
                                }
                                for f in &u_fields {
                                    if let Ok(val_arg) = ctx.args.try_get(f) {
                                        let val_json: serde_json::Value = val_arg.deserialize().unwrap_or(serde_json::Value::Null);
                                        let val_json_str = serde_json::to_string(&val_json).unwrap_or_default();
                                        if let Some(uid) = resolver.find_uid(&format!("{}.{}", t_name, f), &val_json_str) { return Ok(Some(dynamic::FieldValue::owned_any(uid))); }
                                    }
                                }
                                Ok(None)
                            })
                        }).argument(dynamic::InputValue::new("id", dynamic::TypeRef::named(dynamic::TypeRef::ID)));
                        for f in unique_fields { query_field = query_field.argument(dynamic::InputValue::new(f.clone(), dynamic::TypeRef::named(dynamic::TypeRef::STRING))); }
                        query_fields.push(query_field);

                        // 3. Create
                        let create_name = format!("create{}", type_name);
                        let type_name_create = type_name.clone();
                        let uniques_create = unique_fields.clone();
                        let inverses_create = inverses.clone();
                        let search_fields_create = type_search_fields.clone();

                        let meta_arc_create = metadata_arc.clone();
                        mutation_fields.push(dynamic::Field::new(create_name, dynamic::TypeRef::named(type_name_create.clone()), move |ctx| {
                            let t_name = type_name_create.clone();
                            let _u_fields = uniques_create.clone();
                            let _inv_fields = inverses_create.clone();
                            let _s_fields = search_fields_create.clone();
                            let meta_arc = meta_arc_create.clone();
                            dynamic::FieldFuture::new(async move {
                                let input_arg = ctx.args.try_get("input")?;
                                let fields: std::collections::HashMap<String, async_graphql::Value> = input_arg.deserialize()?;
                                
                                // Validation
                                let _meta = meta_arc.get(&t_name).unwrap();
                                
                                use crate::engine::resolver::Resolver;
                                let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();

                                // Deep Creation
                                match deep_create_node(resolver, &meta_arc, &t_name, fields).await {
                                    Ok(uid) => Ok(Some(dynamic::FieldValue::owned_any(uid))),
                                    Err(e) => Err(e.into()),
                                }
                            })
                        }).argument(dynamic::InputValue::new("input", dynamic::TypeRef::named_nn(format!("{}Input", type_name)))));

                        // 4. Update
                        let update_name = format!("update{}", type_name);
                        let type_name_update = type_name.clone();
                        let uniques_update = unique_fields.clone();
                        let inverses_update = inverses.clone();
                        let search_fields_update = type_search_fields.clone();

                        let meta_arc_update = metadata_arc.clone();
                        mutation_fields.push(dynamic::Field::new(update_name, dynamic::TypeRef::named(dynamic::TypeRef::BOOLEAN), move |ctx| {
                             let t_name = type_name_update.clone();
                             let u_fields = uniques_update.clone();
                             let inv_fields = inverses_update.clone();
                             let s_fields = search_fields_update.clone();
                             let meta_arc = meta_arc_update.clone();
                             dynamic::FieldFuture::new(async move {
                                let id_arg = ctx.args.try_get("id")?;
                                let uid = id_arg.string()?.parse::<u64>().map_err(|_| "Invalid ID")?;
                                let input_arg = ctx.args.try_get("input")?;
                                let fields: std::collections::HashMap<String, async_graphql::Value> = input_arg.deserialize()?;

                                // Validation
                                let meta = meta_arc.get(&t_name).unwrap();
                                validate_input(&fields, &meta.validate_fields)?;

                                use crate::engine::resolver::Resolver;
                                let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();
                                match resolver.update_node(&t_name, uid, fields, &u_fields, &inv_fields, &s_fields) {
                                    Ok(_) => Ok(Some(dynamic::FieldValue::value(async_graphql::Value::Boolean(true)))),
                                    Err(e) => Err(e.into()),
                                }
                             })
                        }).argument(dynamic::InputValue::new("id", dynamic::TypeRef::named_nn(dynamic::TypeRef::ID)))
                          .argument(dynamic::InputValue::new("input", dynamic::TypeRef::named_nn(format!("{}Input", type_name)))));
                          
                        // 5. Delete (Recall: RECURSIVE DELETE LOGIC HERE)
                        let delete_name = format!("delete{}", type_name);
                        let type_name_delete = type_name.clone();
                        let meta_arc_delete = metadata_arc.clone();
                        
                        mutation_fields.push(dynamic::Field::new(delete_name, dynamic::TypeRef::named(dynamic::TypeRef::BOOLEAN), move |ctx| {
                            let t_name = type_name_delete.clone();
                            let meta_arc = meta_arc_delete.clone();
                            dynamic::FieldFuture::new(async move {
                                let id_arg = ctx.args.try_get("id")?;
                                let uid = id_arg.string()?.parse::<u64>().map_err(|_| "Invalid ID")?;
                                
                                use crate::engine::resolver::Resolver;
                                let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();

                                // Recursive Helper
                                fn recursive_delete<'a>(
                                    resolver: &'a Box<dyn Resolver + Send + Sync>,
                                    type_name: &'a str,
                                    uid: u64,
                                    meta_map: &'a std::collections::HashMap<String, TypeMetadata>
                                ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
                                    Box::pin(async move {
                                        if let Some(meta) = meta_map.get(type_name) {
                                            // 1. Process Cascades
                                            for (field, target_type) in &meta.cascade_fields {
                                                if let Some(val) = resolver.resolve(uid, field) {
                                                    let mut target_uids = Vec::new();
                                                    match val {
                                                        async_graphql::Value::List(items) => {
                                                            for item in items {
                                                                 let u_opt = match item {
                                                                    async_graphql::Value::String(s) => s.parse::<u64>().ok(),
                                                                    async_graphql::Value::Number(n) => n.as_u64(),
                                                                    _ => None
                                                                 };
                                                                 if let Some(u) = u_opt { target_uids.push(u); }
                                                            }
                                                        },
                                                        async_graphql::Value::String(s) => {
                                                            if let Ok(u) = s.parse::<u64>() { target_uids.push(u); }
                                                        }
                                                        _ => {}
                                                    }
                                                    for target_uid in target_uids {
                                                        recursive_delete(resolver, target_type, target_uid, meta_map).await?;
                                                    }
                                                }
                                            }
                                            // 2. Delete Self
                                            resolver.delete_node(type_name, uid, &meta.uniques, &meta.inverses, &meta.search_fields)?;
                                        }
                                        Ok(())
                                    })
                                }

                                match recursive_delete(resolver, &t_name, uid, &meta_arc).await {
                                    Ok(_) => Ok(Some(dynamic::FieldValue::value(async_graphql::Value::Boolean(true)))),
                                    Err(e) => Err(e.into()),
                                }
                            })
                        }).argument(dynamic::InputValue::new("id", dynamic::TypeRef::named_nn(dynamic::TypeRef::ID))));

                    },
                    AstTypeKind::Interface(int_def) => {
                         let mut interface = dynamic::Interface::new(type_name.clone());
                         // Interface fields are declarations
                         for field in &int_def.fields {
                            let field_name = field.node.name.node.to_string();
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
                            let ty_ref = match field_type_name.as_str() {
                                "ID" => dynamic::TypeRef::named_nn(dynamic::TypeRef::ID),
                                "String" => dynamic::TypeRef::named(dynamic::TypeRef::STRING),
                                "Int" => dynamic::TypeRef::named(dynamic::TypeRef::INT),
                                "Boolean" => dynamic::TypeRef::named(dynamic::TypeRef::BOOLEAN),
                                _ => {
                                    if is_list { dynamic::TypeRef::named_list(field_type_name.clone()) } 
                                    else { dynamic::TypeRef::named(field_type_name.clone()) }
                                }
                            };
                            interface = interface.field(dynamic::InterfaceField::new(field_name, ty_ref));
                         }
                         // Interface requires TypeResolver to map U64 -> Concrete Type
                         // But we handle this via FieldValue::with_type at the Object level (resolvers returning objects).
                         // However, to be safe, Interface should have a lookup?
                         // async-graphql dynamic interfaces need a resolve_type fn generally?
                         // Actually, FieldValue::with_type is enough.
                         // But we also register the Interface type.
                         // interface = interface.register(); // Add to registry
                         types.push(dynamic::Type::Interface(interface));
                         // Generate Input Object for Interface to support linking
                         let input = dynamic::InputObject::new(format!("{}Input", type_name))
                            .field(dynamic::InputValue::new("uid", dynamic::TypeRef::named(dynamic::TypeRef::ID)));
                         types.push(dynamic::Type::InputObject(input));
                    },
                    AstTypeKind::Union(union_def) => {
                        let mut union = dynamic::Union::new(type_name.clone());
                        for member in &union_def.members {
                            union = union.possible_type(member.node.to_string());
                        }
                        // union = union.register();
                        types.push(dynamic::Type::Union(union));
                        
                        // Generate Input Object for Union to support linking
                         let input = dynamic::InputObject::new(format!("{}Input", type_name))
                            .field(dynamic::InputValue::new("uid", dynamic::TypeRef::named(dynamic::TypeRef::ID)));
                         types.push(dynamic::Type::InputObject(input));
                    },
                    AstTypeKind::InputObject(input_def) => {
                         let mut input = dynamic::InputObject::new(type_name.clone());
                         for field in &input_def.fields {
                            let field_name = field.node.name.node.to_string();
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
                            
                            // Determine Base Name
                             let base_ty_name = match field_type_name.as_str() {
                                "ID" => dynamic::TypeRef::ID,
                                "String" => dynamic::TypeRef::STRING,
                                "Int" => dynamic::TypeRef::INT,
                                "Boolean" => dynamic::TypeRef::BOOLEAN,
                                "Float" => dynamic::TypeRef::FLOAT,
                                "Int64" => "Int64",
                                "DateTime" => "DateTime",
                                "GeoPoint" => "GeoPointInput",
                                "GeoPointInput" => "GeoPointInput",
                                _ => field_type_name.as_str(),
                            };

                            let mut ty_ref = if is_list {
                                dynamic::TypeRef::named_list(base_ty_name)
                            } else {
                                dynamic::TypeRef::named(base_ty_name)
                            };

                             if !field.node.ty.node.nullable {
                                 // To make it NonNull, we need to inspect structure or use constructors that wrap?
                                 // async-graphql::dynamic::TypeRef doesn't have "make_non_null".
                                 // We have to choose named_nn or named_list_nn at start.
                                 
                                 // Redo construction logic
                                 ty_ref = if is_list {
                                     dynamic::TypeRef::named_list_nn(base_ty_name)
                                 } else {
                                     dynamic::TypeRef::named_nn(base_ty_name)
                                 };
                             } else {
                                  // Nullable (default above was named/named_list which is nullable)
                             }

                            input = input.field(dynamic::InputValue::new(field_name, ty_ref));
                         }
                         types.push(dynamic::Type::InputObject(input));
                    },
                    AstTypeKind::Enum(enum_def) => {
                        let mut e = dynamic::Enum::new(type_name.clone());
                        for value in &enum_def.values {
                            e = e.item(dynamic::EnumItem::new(value.node.value.to_string()));
                        }
                        types.push(dynamic::Type::Enum(e));
                    },
                    _ => {}
                }
                }
            }


        // Define MutationType Enum
        let mutation_type_enum = dynamic::Enum::new("MutationType")
            .item(dynamic::EnumItem::new("CREATE"))
            .item(dynamic::EnumItem::new("UPDATE"))
            .item(dynamic::EnumItem::new("DELETE"));
        
        // Define MutationEvent Object
        let mutation_event_obj = dynamic::Object::new("MutationEvent")
            .field(dynamic::Field::new("type", dynamic::TypeRef::named_nn("String"), |ctx| {
                dynamic::FieldFuture::new(async move {
                     let event = ctx.parent_value.try_downcast_ref::<crate::realtime::bus::MutationEvent>()?;
                     Ok(Some(dynamic::FieldValue::value(async_graphql::Value::String(event.type_name.clone()))))
                })
            }))
            .field(dynamic::Field::new("uid", dynamic::TypeRef::named_nn("ID"), |ctx| {
                dynamic::FieldFuture::new(async move {
                    let event = ctx.parent_value.try_downcast_ref::<crate::realtime::bus::MutationEvent>()?;
                    Ok(Some(dynamic::FieldValue::value(async_graphql::Value::String(event.uid.to_string()))))
                })
             }))
            .field(dynamic::Field::new("mutation", dynamic::TypeRef::named_nn("MutationType"), |ctx| {
                dynamic::FieldFuture::new(async move {
                    let event = ctx.parent_value.try_downcast_ref::<crate::realtime::bus::MutationEvent>()?;
                    let s = match event.mutation_type {
                        crate::realtime::bus::MutationType::Create => "CREATE",
                        crate::realtime::bus::MutationType::Update => "UPDATE",
                        crate::realtime::bus::MutationType::Delete => "DELETE",
                    };
                    Ok(Some(dynamic::FieldValue::value(async_graphql::Value::Enum(async_graphql::Name::new(s)))))
                })
            }));

        // Generic Subscription Field: "subscribe(types: [String!])"
        subscription_fields.push(dynamic::SubscriptionField::new("event", dynamic::TypeRef::named_nn("MutationEvent"), |ctx| {
            dynamic::SubscriptionFieldFuture::new(async move {
                let types_arg = ctx.args.try_get("types")?;
                let types: Vec<String> = types_arg.list()?.iter().map(|v| v.string().unwrap_or("").to_string()).collect();
                
                use crate::engine::resolver::Resolver;
                let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();
                let bus = resolver.subscribe_events();
                let mut rx = bus.subscribe();

                let stream = async_stream::stream! {
                    loop {
                        if let Ok(event) = rx.recv().await {
                             if types.is_empty() || types.contains(&event.type_name) {
                                  yield Ok(dynamic::FieldValue::owned_any(event));
                             }
                        } else {
                            // Bus closed or lagged
                            break;
                        }
                    }
                };
                Ok(stream)
            })
        }).argument(dynamic::InputValue::new("types", dynamic::TypeRef::named_list(dynamic::TypeRef::STRING))));

        // Register Point Type
        let point_type = dynamic::Object::new("Point")
            .field(dynamic::Field::new("latitude", dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT), |ctx| {
                dynamic::FieldFuture::new(async move {
                    let p = ctx.parent_value.try_downcast_ref::<async_graphql::Value>()?;
                    if let async_graphql::Value::Object(map) = p {
                         if let Some(async_graphql::Value::Number(n)) = map.get("latitude") {
                             return Ok(Some(dynamic::FieldValue::value(async_graphql::Value::Number(n.clone()))));
                         }
                    }
                    Ok(None)
                })
            }))
             .field(dynamic::Field::new("longitude", dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT), |ctx| {
                dynamic::FieldFuture::new(async move {
                    let p = ctx.parent_value.try_downcast_ref::<async_graphql::Value>()?;
                    if let async_graphql::Value::Object(map) = p {
                         if let Some(async_graphql::Value::Number(n)) = map.get("longitude") {
                             return Ok(Some(dynamic::FieldValue::value(async_graphql::Value::Number(n.clone()))));
                         }
                    }
                    Ok(None)
                })
            }));
        types.push(dynamic::Type::Object(point_type));

        let point_input = dynamic::InputObject::new("PointInput")
            .field(dynamic::InputValue::new("latitude", dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT)))
            .field(dynamic::InputValue::new("longitude", dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT)));
        types.push(dynamic::Type::InputObject(point_input));
        
        // PointFilter Input
        let point_filter_input = dynamic::InputObject::new("PointFilter")
            .field(dynamic::InputValue::new("near", dynamic::TypeRef::named("NearFilter")));
        types.push(dynamic::Type::InputObject(point_filter_input));

        // NearFilter Input
        let near_filter_input = dynamic::InputObject::new("NearFilter")
            .field(dynamic::InputValue::new("distance", dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT)))
            .field(dynamic::InputValue::new("coordinate", dynamic::TypeRef::named_nn("PointInput")));
        types.push(dynamic::Type::InputObject(near_filter_input));

        // Build Schema
        let mut query_root = dynamic::Object::new("Query");
        for field in query_fields { query_root = query_root.field(field); }
        
        let mut mutation_root = dynamic::Object::new("Mutation");
        for field in mutation_fields { mutation_root = mutation_root.field(field); }

        let mut subscription_root = dynamic::Subscription::new("Subscription");
        for field in subscription_fields { subscription_root = subscription_root.field(field); }

        let mut schema_builder = dynamic::Schema::build("Query", Some("Mutation"), Some("Subscription"));
        schema_builder = schema_builder.register(query_root);
        schema_builder = schema_builder.register(mutation_root);
        schema_builder = schema_builder.register(subscription_root);
        schema_builder = schema_builder.register(mutation_type_enum);
        schema_builder = schema_builder.register(mutation_event_obj);

        types.push(dynamic::Type::Scalar(dynamic::Scalar::new("Int64")));
        types.push(dynamic::Type::Scalar(dynamic::Scalar::new("DateTime")));

        // Manual Injection of Geo Types to ensure custom resolvers are used
        let geo_point = dynamic::Object::new("GeoPoint")
            .field(dynamic::Field::new("latitude", dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT), |ctx| {
                dynamic::FieldFuture::new(async move {
                    let val = ctx.parent_value.try_downcast_ref::<GeoPointData>()?;
                    Ok(Some(dynamic::FieldValue::value(val.latitude)))
                })
            }))
            .field(dynamic::Field::new("longitude", dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT), |ctx| {
                dynamic::FieldFuture::new(async move {
                    let val = ctx.parent_value.try_downcast_ref::<GeoPointData>()?;
                    Ok(Some(dynamic::FieldValue::value(val.longitude)))
                })
            }));
        types.push(dynamic::Type::Object(geo_point));

        let geo_input = dynamic::InputObject::new("GeoPointInput")
             .field(dynamic::InputValue::new("latitude", dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT)))
             .field(dynamic::InputValue::new("longitude", dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT)));
        types.push(dynamic::Type::InputObject(geo_input));

        let near_filter = dynamic::InputObject::new("NearFilter")
             .field(dynamic::InputValue::new("distance", dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT)))
             .field(dynamic::InputValue::new("coordinate", dynamic::TypeRef::named_nn("GeoPointInput")));
        types.push(dynamic::Type::InputObject(near_filter));

        let geo_filter = dynamic::InputObject::new("GeoPointFilter")
             .field(dynamic::InputValue::new("near", dynamic::TypeRef::named("NearFilter")))
             .field(dynamic::InputValue::new("within", dynamic::TypeRef::named("PolygonInput")));
        types.push(dynamic::Type::InputObject(geo_filter));

        let polygon_filter = dynamic::InputObject::new("PolygonFilter")
             .field(dynamic::InputValue::new("intersects", dynamic::TypeRef::named("PolygonInput")))
             .field(dynamic::InputValue::new("within", dynamic::TypeRef::named("PolygonInput")));
        types.push(dynamic::Type::InputObject(polygon_filter));

        let multi_polygon_filter = dynamic::InputObject::new("MultiPolygonFilter")
             .field(dynamic::InputValue::new("intersects", dynamic::TypeRef::named("MultiPolygonInput")))
             .field(dynamic::InputValue::new("within", dynamic::TypeRef::named("PolygonInput")));
        types.push(dynamic::Type::InputObject(multi_polygon_filter));

        // Polygon
        let point_list_type = dynamic::TypeRef::named_nn_list("GeoPoint");
        let ring_list_type = dynamic::TypeRef::List(Box::new(dynamic::TypeRef::named_nn_list("GeoPoint")));

        let polygon = dynamic::Object::new("Polygon")
            .field(dynamic::Field::new("exterior", point_list_type.clone(), |ctx| {
                dynamic::FieldFuture::new(async move {
                    let val = ctx.parent_value.try_downcast_ref::<GeoPolygonData>()?;
                    let list: Vec<dynamic::FieldValue> = val.exterior.iter().map(|p| dynamic::FieldValue::owned_any(p.clone())).collect();
                    Ok(Some(dynamic::FieldValue::list(list)))
                })
            }))
            .field(dynamic::Field::new("interiors", ring_list_type.clone(), |ctx| {
                dynamic::FieldFuture::new(async move {
                    let val = ctx.parent_value.try_downcast_ref::<GeoPolygonData>()?;
                    let mut rings = vec![];
                    for ring in &val.interiors {
                         let ring_list: Vec<dynamic::FieldValue> = ring.iter().map(|p| dynamic::FieldValue::owned_any(p.clone())).collect();
                         rings.push(dynamic::FieldValue::list(ring_list));
                    }
                    Ok(Some(dynamic::FieldValue::list(rings)))
                })
            }));
        types.push(dynamic::Type::Object(polygon));

        let point_input_list_type = dynamic::TypeRef::named_nn_list("GeoPointInput");
        let ring_input_list_type = dynamic::TypeRef::List(Box::new(dynamic::TypeRef::named_nn_list("GeoPointInput")));
        
        let polygon_input = dynamic::InputObject::new("PolygonInput")
            .field(dynamic::InputValue::new("exterior", point_input_list_type))
            .field(dynamic::InputValue::new("interiors", ring_input_list_type));
        types.push(dynamic::Type::InputObject(polygon_input));

        // MultiPolygon
        let poly_list_type = dynamic::TypeRef::List(Box::new(dynamic::TypeRef::named_nn("Polygon")));
        let multi_polygon = dynamic::Object::new("MultiPolygon")
             .field(dynamic::Field::new("polygons", poly_list_type, |ctx| {
                 dynamic::FieldFuture::new(async move {
                     let val = ctx.parent_value.try_downcast_ref::<GeoMultiPolygonData>()?;
                     let list: Vec<dynamic::FieldValue> = val.polygons.iter().map(|p| dynamic::FieldValue::owned_any(p.clone())).collect();
                     Ok(Some(dynamic::FieldValue::list(list)))
                 })
             }));
        types.push(dynamic::Type::Object(multi_polygon));

         let poly_input_list_type = dynamic::TypeRef::List(Box::new(dynamic::TypeRef::named_nn("PolygonInput")));
         let multi_polygon_input = dynamic::InputObject::new("MultiPolygonInput")
            .field(dynamic::InputValue::new("polygons", poly_input_list_type));
        types.push(dynamic::Type::InputObject(multi_polygon_input));


        // Register Extended Scalars (graphql-scalars parity)
        // crate::engine::scalars::register_scalars(&mut types); // Moved to top

        for obj in types { schema_builder = schema_builder.register(obj); }
        
        // ... (Filters, etc) ...

        let string_filter = dynamic::InputObject::new("StringFilter")
            .field(dynamic::InputValue::new("eq", dynamic::TypeRef::named(dynamic::TypeRef::STRING)))
            .field(dynamic::InputValue::new("contains", dynamic::TypeRef::named(dynamic::TypeRef::STRING)))
            .field(dynamic::InputValue::new("allofterms", dynamic::TypeRef::named(dynamic::TypeRef::STRING)))
            .field(dynamic::InputValue::new("anyofterms", dynamic::TypeRef::named(dynamic::TypeRef::STRING)))
            .field(dynamic::InputValue::new("alloftext", dynamic::TypeRef::named(dynamic::TypeRef::STRING)))
            .field(dynamic::InputValue::new("anyoftext", dynamic::TypeRef::named(dynamic::TypeRef::STRING)))
            .field(dynamic::InputValue::new("lt", dynamic::TypeRef::named(dynamic::TypeRef::STRING)))
            .field(dynamic::InputValue::new("le", dynamic::TypeRef::named(dynamic::TypeRef::STRING)))
            .field(dynamic::InputValue::new("gt", dynamic::TypeRef::named(dynamic::TypeRef::STRING)))
            .field(dynamic::InputValue::new("ge", dynamic::TypeRef::named(dynamic::TypeRef::STRING)))
            .field(dynamic::InputValue::new("in", dynamic::TypeRef::named_list(dynamic::TypeRef::STRING)));
        
        let int_filter = dynamic::InputObject::new("IntFilter")
            .field(dynamic::InputValue::new("eq", dynamic::TypeRef::named(dynamic::TypeRef::INT)))
            .field(dynamic::InputValue::new("gt", dynamic::TypeRef::named(dynamic::TypeRef::INT)))
            .field(dynamic::InputValue::new("lt", dynamic::TypeRef::named(dynamic::TypeRef::INT)))
            .field(dynamic::InputValue::new("ge", dynamic::TypeRef::named(dynamic::TypeRef::INT)))
            .field(dynamic::InputValue::new("le", dynamic::TypeRef::named(dynamic::TypeRef::INT)))
            .field(dynamic::InputValue::new("between", dynamic::TypeRef::named_list(dynamic::TypeRef::INT)))
            .field(dynamic::InputValue::new("in", dynamic::TypeRef::named_list(dynamic::TypeRef::INT)));

        let float_filter = dynamic::InputObject::new("FloatFilter")
            .field(dynamic::InputValue::new("eq", dynamic::TypeRef::named(dynamic::TypeRef::FLOAT)))
            .field(dynamic::InputValue::new("gt", dynamic::TypeRef::named(dynamic::TypeRef::FLOAT)))
            .field(dynamic::InputValue::new("lt", dynamic::TypeRef::named(dynamic::TypeRef::FLOAT)))
            .field(dynamic::InputValue::new("ge", dynamic::TypeRef::named(dynamic::TypeRef::FLOAT)))
            .field(dynamic::InputValue::new("le", dynamic::TypeRef::named(dynamic::TypeRef::FLOAT)))
            .field(dynamic::InputValue::new("between", dynamic::TypeRef::named_list(dynamic::TypeRef::FLOAT)))
            .field(dynamic::InputValue::new("in", dynamic::TypeRef::named_list(dynamic::TypeRef::FLOAT)));

        let bool_filter = dynamic::InputObject::new("BooleanFilter")
             .field(dynamic::InputValue::new("eq", dynamic::TypeRef::named(dynamic::TypeRef::BOOLEAN)));
             
        let int64_filter = dynamic::InputObject::new("Int64Filter")
            .field(dynamic::InputValue::new("eq", dynamic::TypeRef::named("Int64")))
            .field(dynamic::InputValue::new("gt", dynamic::TypeRef::named("Int64")))
            .field(dynamic::InputValue::new("lt", dynamic::TypeRef::named("Int64")))
            .field(dynamic::InputValue::new("ge", dynamic::TypeRef::named("Int64")))
            .field(dynamic::InputValue::new("le", dynamic::TypeRef::named("Int64")))
            .field(dynamic::InputValue::new("in", dynamic::TypeRef::named_list("Int64")));

        let datetime_filter = dynamic::InputObject::new("DateTimeFilter")
            .field(dynamic::InputValue::new("eq", dynamic::TypeRef::named("DateTime")))
            .field(dynamic::InputValue::new("gt", dynamic::TypeRef::named("DateTime")))
            .field(dynamic::InputValue::new("lt", dynamic::TypeRef::named("DateTime")))
            .field(dynamic::InputValue::new("ge", dynamic::TypeRef::named("DateTime")))
            .field(dynamic::InputValue::new("le", dynamic::TypeRef::named("DateTime")))
            .field(dynamic::InputValue::new("in", dynamic::TypeRef::named_list("DateTime")));

        schema_builder = schema_builder.register(string_filter);
        schema_builder = schema_builder.register(int_filter);
        schema_builder = schema_builder.register(float_filter);
        schema_builder = schema_builder.register(bool_filter);
        schema_builder = schema_builder.register(int64_filter);
        schema_builder = schema_builder.register(datetime_filter);
             
        let sort_direction = dynamic::Enum::new("SortDirection")
            .item(dynamic::EnumItem::new("ASC"))
            .item(dynamic::EnumItem::new("DESC"));


        schema_builder = schema_builder.register(sort_direction);

        Ok(schema_builder)
    }

    pub async fn execute_with_resolver(&self, query: &str, resolver: Box<dyn crate::engine::resolver::Resolver + Send + Sync>) -> String {
        let req = async_graphql::Request::new(query)
            .data(resolver);
        let resp = self.inner.execute(req).await;
        serde_json::to_string(&resp).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn execute_stream_with_resolver(&self, query: &str, resolver: Box<dyn crate::engine::resolver::Resolver + Send + Sync>) -> impl futures_util::Stream<Item = async_graphql::Response> {
        let req = async_graphql::Request::new(query)
            .data(resolver);
        self.inner.execute_stream(req)
    }

    pub fn load_from_sdl(sdl: &str) -> Result<Schema, String> {
        let builder = Self::create_builder(sdl)?;
        let schema = builder.finish().map_err(|e| e.to_string())?;
        Ok(Self { inner: schema })
    }

    pub fn load_with_resolver<R: crate::engine::resolver::Resolver + Send + Sync + 'static>(sdl: &str, resolver: R) -> Result<Schema, String> {
        let builder = Self::create_builder(sdl)?;
        let schema = builder
            .data(Box::new(resolver) as Box<dyn crate::engine::resolver::Resolver + Send + Sync>)
            .finish()
            .map_err(|e| e.to_string())?;
        Ok(Self { inner: schema })
    }

    pub fn sdl(&self) -> String {
        self.inner.sdl()
    }
}


// Standalone Helper Functions

fn validate_input(
    fields: &std::collections::HashMap<String, async_graphql::Value>,
    rules: &std::collections::HashMap<String, Vec<ValidationRule>>
) -> Result<(), String> {
    for (field_name, field_rules) in rules {
        if let Some(val) = fields.get(field_name) {
             if matches!(val, async_graphql::Value::Null) { continue; }
             for rule in field_rules {
                 match rule {
                     ValidationRule::Regex(pattern) => {
                         if let async_graphql::Value::String(s) = val {
                              let re = regex::Regex::new(pattern).map_err(|_| format!("Invalid regex pattern on server for field {}", field_name))?;
                              if !re.is_match(s) {
                                  return Err(format!("Field '{}' must match pattern '{}'", field_name, pattern));
                              }
                         }
                     }
                      ValidationRule::Length { min, max } => {
                          if let async_graphql::Value::String(s) = val {
                              let len = s.len() as i64;
                              if let Some(m) = min { if len < *m { return Err(format!("Field '{}' length must be at least {}", field_name, m)); } }
                              if let Some(m) = max { if len > *m { return Err(format!("Field '{}' length must be at most {}", field_name, m)); } }
                          }
                      }
                      ValidationRule::Range { min, max } => {
                            if let async_graphql::Value::Number(n) = val {
                                if let Some(f_val) = n.as_f64() {
                                    if let Some(min_val) = min {
                                         if f_val < *min_val {
                                             return Err(format!("Field '{}' must be at least {}", field_name, min_val));
                                         }
                                    }
                                    if let Some(max_val) = max {
                                         if f_val > *max_val {
                                             return Err(format!("Field '{}' must be at most {}", field_name, max_val));
                                         }
                                    }
                                }
                            }
                        }
                 }
             }
        }
    }
    Ok(())
}

fn deep_create_node<'a>(
    resolver: &'a Box<dyn crate::engine::resolver::Resolver + Send + Sync>,
    meta_map: &'a std::collections::HashMap<String, TypeMetadata>,
    type_name: &'a str,
    mut fields: std::collections::HashMap<String, async_graphql::Value>
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u64, String>> + Send + 'a>> {
    Box::pin(async move {
        // Check if Linking via UID
        if let Some(uid_val) = fields.get("uid") {
            if let async_graphql::Value::String(s) = uid_val {
                 if let Ok(uid) = s.parse::<u64>() {
                     return Ok(uid);
                 }
            }
        }

        if let Some(meta) = meta_map.get(type_name) {
            // 1. Recursively create relations
            let mut fields_to_replace = Vec::new();
            
            for (field, target_type) in &meta.relations {
                if let Some(val) = fields.get(field) {
                    match val {
                        async_graphql::Value::Object(map) => {
                             let field_map: std::collections::HashMap<String, async_graphql::Value> = map.iter().map(|(k,v)| (k.to_string(), v.clone())).collect();
                             let uid = deep_create_node(resolver, meta_map, target_type, field_map).await?;
                             fields_to_replace.push((field.clone(), async_graphql::Value::String(uid.to_string())));
                        }
                        async_graphql::Value::List(list) => {
                             let mut new_uids = Vec::new();
                             for item in list {
                                 if let async_graphql::Value::Object(map) = item {
                                     let field_map: std::collections::HashMap<String, async_graphql::Value> = map.iter().map(|(k,v)| (k.to_string(), v.clone())).collect();
                                     let uid = deep_create_node(resolver, meta_map, target_type, field_map).await?;
                                     new_uids.push(async_graphql::Value::String(uid.to_string()));
                                 } else if let async_graphql::Value::String(_s) = item {
                                     // Already a UID? Keep it? Dgraph usually separates UID logic. 
                                     // But since we use same input field, we might get "uid": "..." inside object.
                                     // Or if mixed (not possible in GraphQL schema usually).
                                     // If we allowed UIDs in schema, we'd handle it here.
                                     // But now schema enforces Input Object. 
                                     // So item MUST be Object (with optional uid field).
                                     // Wait, if item has `uid` field, we should use it!
                                     // Currently deep_create_node just creates. It assumes NEW.
                                     // We need to check if schema allows linking. 
                                     // For parity, let's assume always Create for now, unless `uid` is passed?
                                     // TODO: Handle existing UID in input object.
                                 }
                             }
                             fields_to_replace.push((field.clone(), async_graphql::Value::List(new_uids)));
                        }
                        _ => {}
                    }
                }
            }
            
            // Link UIDs
            for (field, val) in fields_to_replace {
                fields.insert(field, val);
            }

            // 2. Validate
            validate_input(&fields, &meta.validate_fields)?;

            // 3. Create Self
            resolver.create_node(type_name, fields, &meta.uniques, &meta.inverses, &meta.search_fields)
        } else {
            Err(format!("Type {} not found", type_name))
        }
    })
}
