use async_graphql::dynamic::{self};
use tokio::sync::Semaphore;

static MUTATION_SEMAPHORE: Semaphore = Semaphore::const_new(64);

// This is our "Engine" Schema, which currently wraps async-graphql
#[derive(Clone, Debug)]
pub struct TypeMetadata {
    #[allow(dead_code)]
    pub type_name: String,
    pub uniques: Vec<String>,
    pub inverses: Vec<crate::engine::resolver::InverseInfo>,
    pub search_fields: std::collections::HashMap<String, Vec<String>>,
    pub cascade_fields: Vec<(String, String)>,
    pub interface_implementations: Vec<String>, // Interfaces this type implements
    pub validate_fields: std::collections::HashMap<String, Vec<ValidationRule>>,
    pub relations: std::collections::HashMap<String, String>,

    pub vector_config: Option<crate::engine::resolver::VectorConfig>,
    pub kind: TypeKind,
}

#[derive(Clone, PartialEq, Debug)]
pub enum TypeKind {
    Object,
    Interface,
    Union(Vec<String>), // Possible types
}

#[derive(Clone, Debug)]
pub enum ValidationRule {
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

#[derive(Clone)]
pub struct Schema {
    inner: async_graphql::dynamic::Schema,
    #[allow(dead_code)]
    sdl: String,
    pub type_metadata: std::collections::HashMap<String, TypeMetadata>,
}

impl Schema {
    pub async fn execute(
        &self,
        request: impl Into<async_graphql::Request>,
    ) -> async_graphql::Response {
        self.inner.execute(request).await
    }

    pub fn inner(&self) -> &async_graphql::dynamic::Schema {
        &self.inner
    }

    pub fn create_builder(
        sdl: &str,
    ) -> Result<
        (
            dynamic::SchemaBuilder,
            std::collections::HashMap<String, TypeMetadata>,
        ),
        String,
    > {
        let system_sdl = "
            scalar DateTime
            scalar Int64
            input NearFilter {
                distance: Float!
                coordinate: PointInput!
            }

            type FileRef {
                id: ID! @unique
                storageKey: String! @search(by: [exact])
                fileName: String!
                mimeType: String!
                size: Int64!
                contentHash: String @search(by: [exact])
                status: String! @search(by: [exact]) # 'STAGED', 'COMMITTED', 'ARCHIVED'
                createdAt: DateTime!
                metadata: String # JSON string
            }

            type IncompleteUpload {
                id: ID! @unique
                tusId: String! @unique
                offset: Int64!
                length: Int64
                createdAt: DateTime!
                updatedAt: DateTime!
            }

            type UploadQueueEntry {
                id: ID! @unique
                fileRefId: String! @search(by: [exact])
                status: String! @search(by: [exact]) # 'PENDING', 'UPLOADING', 'COMPLETED', 'FAILED'
                retryCount: Int
                nextRetryAt: DateTime
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

        use async_graphql_parser::types::{
            BaseType, TypeKind as AstTypeKind, TypeSystemDefinition,
        };

        // Pass 0: Pre-scan for field types to assist Inverse resolution
        struct FieldInfo {
            type_name: String,
            is_list: bool,
        }
        let mut type_field_info: std::collections::HashMap<
            String,
            std::collections::HashMap<String, FieldInfo>,
        > = std::collections::HashMap::new();

        for def in &doc.definitions {
            if let TypeSystemDefinition::Type(type_def) = def {
                if let AstTypeKind::Object(obj_def) = &type_def.node.kind {
                    let type_name = type_def.node.name.node.to_string();
                    let mut fields_map = std::collections::HashMap::new();
                    for field in &obj_def.fields {
                        let f_name = field.node.name.node.to_string();
                        let f_type_name = match &field.node.ty.node.base {
                            BaseType::Named(n) => n.to_string(),
                            BaseType::List(inner) => match &inner.base {
                                BaseType::Named(n) => n.to_string(),
                                _ => "String".to_string(),
                            },
                        };
                        let is_list = matches!(field.node.ty.node.base, BaseType::List(_));
                        fields_map.insert(
                            f_name,
                            FieldInfo {
                                type_name: f_type_name,
                                is_list,
                            },
                        );
                    }
                    type_field_info.insert(type_name, fields_map);
                }
            }
        }

        // Pass 1: Collect Metadata for ALL types
        let mut metadata_map: std::collections::HashMap<String, TypeMetadata> =
            std::collections::HashMap::new();
        let mut enum_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        for def in &doc.definitions {
            if let TypeSystemDefinition::Type(type_def) = def {
                let type_name = type_def.node.name.node.to_string();
                if type_name == "Query"
                    || type_name == "Mutation"
                    || type_name == "Subscription"
                    || type_name.starts_with("__")
                {
                    continue;
                }

                match &type_def.node.kind {
                    AstTypeKind::Enum(_) => {
                        enum_names.insert(type_name.clone());
                    }
                    AstTypeKind::Object(obj_def) => {
                        let mut unique_fields: Vec<String> = Vec::new();
                        let mut inverses: Vec<crate::engine::resolver::InverseInfo> = Vec::new();
                        let mut type_search_fields: std::collections::HashMap<String, Vec<String>> =
                            std::collections::HashMap::new();
                        let mut cascade_fields: Vec<(String, String)> = Vec::new();

                        let mut vector_config: Option<crate::engine::resolver::VectorConfig> = None;
                        let mut relations: std::collections::HashMap<String, String> =
                            std::collections::HashMap::new();

                        for field in &obj_def.fields {
                            let field_name = field.node.name.node.to_string();

                            // Uniques
                            if field
                                .node
                                .directives
                                .iter()
                                .any(|d| d.node.name.node == "unique")
                            {
                                unique_fields.push(field_name.clone());
                            }
                            // Search
                            if let Some(directive) = field
                                .node
                                .directives
                                .iter()
                                .find(|d| d.node.name.node == "search")
                            {
                                let mut tokenizers = Vec::new();
                                for (name, value) in &directive.node.arguments {
                                    if name.node == "by" {
                                        match &value.node {
                                            async_graphql_value::ConstValue::List(items) => {
                                                for item in items {
                                                    match item {
                                                        async_graphql_value::ConstValue::Enum(
                                                            n,
                                                        ) => tokenizers.push(n.to_string()),
                                                        async_graphql_value::ConstValue::String(
                                                            s,
                                                        ) => tokenizers.push(s.clone()),
                                                        _ => {}
                                                    }
                                                }
                                            }
                                            async_graphql_value::ConstValue::Enum(n) => {
                                                tokenizers.push(n.to_string())
                                            }
                                            async_graphql_value::ConstValue::String(s) => {
                                                tokenizers.push(s.clone())
                                            }
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
                            if field
                                .node
                                .directives
                                .iter()
                                .any(|d| d.node.name.node == "cascade")
                            {
                                let field_type_name = match &field.node.ty.node.base {
                                    BaseType::Named(n) => n.to_string(),
                                    BaseType::List(inner) => match &inner.base {
                                        BaseType::Named(n) => n.to_string(),
                                        _ => "String".to_string(),
                                    },
                                };
                                cascade_fields.push((field_name.clone(), field_type_name));
                            }
                            // Vector
                            if let Some(dir) = field
                                .node
                                .directives
                                .iter()
                                .find(|d| d.node.name.node == "vector")
                            {
                                let mut source = "".to_string();
                                if let Some((_, val)) =
                                    dir.node.arguments.iter().find(|(n, _)| n.node == "from")
                                {
                                    if let async_graphql::Value::String(s) = &val.node {
                                        source = s.clone();
                                    }
                                }
                                // Default source to parsed "text" if empty? Or required?
                                // For now, we assume user provides it, or if missing we can default to "text"?
                                // But better to be explicit. If empty, maybe just field name? No, that's recursion.

                                // If source is empty, it implies manual vector input (no auto-generation from text).
                                // We still need VectorConfig to trigger indexing.
                                vector_config = Some(crate::engine::resolver::VectorConfig {
                                    field: field_name.clone(),
                                    source,
                                });
                            }

                            // Inverse
                            if let Some(dir) = field
                                .node
                                .directives
                                .iter()
                                .find(|d| d.node.name.node == "hasInverse")
                            {
                                if let Some((_, val)) = dir
                                    .node
                                    .arguments
                                    .iter()
                                    .find(|(name, _)| name.node == "field")
                                {
                                    if let async_graphql::Value::String(inverse_field_name) =
                                        &val.node
                                    {
                                        let field_type_name = match &field.node.ty.node.base {
                                            BaseType::Named(n) => n.to_string(),
                                            BaseType::List(inner) => match &inner.base {
                                                BaseType::Named(n) => n.to_string(),
                                                _ => "String".to_string(),
                                            },
                                        };
                                        let inverse_type = field_type_name;
                                        // Use new map
                                        let inverse_is_list = type_field_info
                                            .get(&inverse_type)
                                            .and_then(|f_map| f_map.get(inverse_field_name))
                                            .map(|info| info.is_list)
                                            .unwrap_or(false);

                                        inverses.push(crate::engine::resolver::InverseInfo {
                                            field: field_name.clone(),
                                            inverse_type,
                                            inverse_field: inverse_field_name.clone(),
                                            inverse_is_list,
                                        });
                                    }
                                }
                            } else {
                                // Implicit Inverse Detection
                                let target_type_name = match &field.node.ty.node.base {
                                    BaseType::Named(n) => n.to_string(),
                                    BaseType::List(inner) => match &inner.base {
                                        BaseType::Named(n) => n.to_string(),
                                        _ => "String".to_string(),
                                    },
                                };

                                // Check if target type has a field pointing back to us (of our type)
                                if let Some(target_fields) = type_field_info.get(&target_type_name)
                                {
                                    // Iterate fields of target type to find one with type == our type_name
                                    // Heuristic: First match? Or exact match on name?
                                    // Dgraph rule: If A.b -> B, checks B.a -> A. Field names are arbitrary but types matter?
                                    // Actually usually it's explicit. But here we want implicit based on relation existence?
                                    // Better heuristic: Check if target type has a field that is of type `type_name`.
                                    // If multiple, maybe we can't decide (or check names?).
                                    // Let's try matching field name to type name first (common convention: author: Author).
                                    // Or simply: ANY field in B that points to A is an inverse candidate.

                                    // Let's look for field in B which has type A.
                                    let mut candidates = Vec::new();
                                    for (t_field_name, t_field_info) in target_fields {
                                        if t_field_info.type_name == type_name {
                                            candidates.push(t_field_name.clone());
                                        }
                                    }

                                    if candidates.len() == 1 {
                                        let inverse_field_name = &candidates[0];
                                        let inverse_is_list =
                                            target_fields.get(inverse_field_name).unwrap().is_list;
                                        if !inverses.iter().any(|i| i.field == field_name) {
                                            inverses.push(crate::engine::resolver::InverseInfo {
                                                field: field_name.clone(),
                                                inverse_type: target_type_name.clone(),
                                                inverse_field: inverse_field_name.clone(),
                                                inverse_is_list,
                                            });
                                        }
                                    }
                                    // If multiple candidates, usually ambiguous without @hasInverse.
                                    // We'll skip for safety to avoid wrong linking.
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
                                    _ => "String".to_string(),
                                },
                            };
                            let is_scalar = matches!(
                                field_type_name.as_str(),
                                "String"
                                    | "Int"
                                    | "Boolean"
                                    | "ID"
                                    | "Float"
                                    | "Int64"
                                    | "DateTime"
                                    | "GeoPoint"
                                    | "Polygon"
                                    | "MultiPolygon"
                            );
                            if !is_scalar {
                                relations.insert(field_name.clone(), field_type_name.clone());
                            }

                            let mut rules = Vec::new();

                            // @regex(pattern: "...")
                            if let Some(dir) = field
                                .node
                                .directives
                                .iter()
                                .find(|d| d.node.name.node == "regex")
                            {
                                if let Some((_, val)) = dir
                                    .node
                                    .arguments
                                    .iter()
                                    .find(|(name, _)| name.node == "pattern")
                                {
                                    if let async_graphql::Value::String(pattern) = &val.node {
                                        rules.push(ValidationRule::Regex(pattern.clone()));
                                    }
                                }
                            }

                            // @length(min: Int, max: Int)
                            if let Some(dir) = field
                                .node
                                .directives
                                .iter()
                                .find(|d| d.node.name.node == "length")
                            {
                                let mut min = None;
                                let mut max = None;
                                if let Some((_, val)) = dir
                                    .node
                                    .arguments
                                    .iter()
                                    .find(|(name, _)| name.node == "min")
                                {
                                    if let async_graphql::Value::Number(n) = &val.node {
                                        min = n.as_i64();
                                    }
                                }
                                if let Some((_, val)) = dir
                                    .node
                                    .arguments
                                    .iter()
                                    .find(|(name, _)| name.node == "max")
                                {
                                    if let async_graphql::Value::Number(n) = &val.node {
                                        max = n.as_i64();
                                    }
                                }
                                if min.is_some() || max.is_some() {
                                    rules.push(ValidationRule::Length { min, max });
                                }
                            }

                            // @range(min: Float, max: Float)
                            if let Some(dir) = field
                                .node
                                .directives
                                .iter()
                                .find(|d| d.node.name.node == "range")
                            {
                                let mut min = None;
                                let mut max = None;
                                if let Some((_, val)) = dir
                                    .node
                                    .arguments
                                    .iter()
                                    .find(|(name, _)| name.node == "min")
                                {
                                    if let async_graphql::Value::Number(n) = &val.node {
                                        min = n.as_f64();
                                    }
                                }
                                if let Some((_, val)) = dir
                                    .node
                                    .arguments
                                    .iter()
                                    .find(|(name, _)| name.node == "max")
                                {
                                    if let async_graphql::Value::Number(n) = &val.node {
                                        max = n.as_f64();
                                    }
                                }
                                if min.is_some() || max.is_some() {
                                    rules.push(ValidationRule::Range { min, max });
                                }
                            }

                            if !rules.is_empty() {
                                validate_fields.insert(field_name, rules);
                            }
                        }

                        let interfaces: Vec<String> = obj_def
                            .implements
                            .iter()
                            .map(|n| n.node.to_string())
                            .collect();

                        metadata_map.insert(
                            type_name.clone(),
                            TypeMetadata {
                                type_name: type_name.clone(),
                                uniques: unique_fields,
                                inverses,
                                search_fields: type_search_fields,
                                cascade_fields,
                                interface_implementations: interfaces,
                                validate_fields,
                                relations,
                                vector_config,
                                kind: TypeKind::Object,
                            },
                        );
                    }
                    AstTypeKind::Interface(_int_def) => {
                        metadata_map.insert(
                            type_name.clone(),
                            TypeMetadata {
                                type_name: type_name.clone(),
                                uniques: vec![],
                                inverses: vec![],
                                search_fields: std::collections::HashMap::new(),
                                cascade_fields: vec![],
                                interface_implementations: vec![],
                                validate_fields: std::collections::HashMap::new(),
                                relations: std::collections::HashMap::new(),
                                vector_config: None,
                                kind: TypeKind::Interface,
                            },
                        );
                    }
                    AstTypeKind::Union(union_def) => {
                        let possible_types: Vec<String> = union_def
                            .members
                            .iter()
                            .map(|n| n.node.to_string())
                            .collect();
                        metadata_map.insert(
                            type_name.clone(),
                            TypeMetadata {
                                type_name: type_name.clone(),
                                uniques: vec![],
                                inverses: vec![],
                                search_fields: std::collections::HashMap::new(),
                                cascade_fields: vec![],
                                interface_implementations: vec![],
                                validate_fields: std::collections::HashMap::new(),
                                relations: std::collections::HashMap::new(),
                                vector_config: None,
                                kind: TypeKind::Union(possible_types),
                            },
                        );
                    }
                    _ => {}
                }
            }
        }

        let metadata_arc = std::sync::Arc::new(metadata_map.clone());

        // Pass 2: Generate Schema Artifacts
        for def in &doc.definitions {
            if let TypeSystemDefinition::Type(type_def) = def {
                let type_name = type_def.node.name.node.to_string();
                if type_name == "Query"
                    || type_name == "Mutation"
                    || type_name == "Subscription"
                    || type_name.starts_with("__")
                {
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
                        let meta = metadata_arc
                            .get(&type_name)
                            .expect(&format!("Metadata missing for type {}", type_name));
                        let unique_fields = &meta.uniques;
                        let inverses = &meta.inverses;
                        let type_search_fields = &meta.search_fields;

                        let mut obj = dynamic::Object::new(type_name.clone());
                        if type_name != "GeoPoint" {
                            obj = obj.field(dynamic::Field::new(
                                "uid",
                                dynamic::TypeRef::named_nn("ID"),
                                |ctx| {
                                    dynamic::FieldFuture::new(async move {
                                        let uid = ctx.parent_value.try_downcast_ref::<u64>()?;
                                        Ok(Some(dynamic::FieldValue::value(
                                            async_graphql::Value::String(uid.to_string()),
                                        )))
                                    })
                                },
                            ));
                        }
                        let mut input = dynamic::InputObject::new(format!("{}Input", type_name))
                            .field(dynamic::InputValue::new(
                                "uid",
                                dynamic::TypeRef::named(dynamic::TypeRef::ID),
                            ))
                            .field(dynamic::InputValue::new(
                                "id",
                                dynamic::TypeRef::named(dynamic::TypeRef::ID),
                            ));
                        let mut filter_input =
                            dynamic::InputObject::new(format!("{}Filter", type_name));

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
                                BaseType::Named(n) => {
                                    field_type_name = n.to_string();
                                }
                                BaseType::List(inner) => {
                                    is_list = true;
                                    match &inner.base {
                                        BaseType::Named(n) => {
                                            field_type_name = n.to_string();
                                        }
                                        _ => {}
                                    }
                                }
                            }

                            let is_enum = enum_names.contains(&field_type_name);
                            let is_scalar = matches!(
                                field_type_name.as_str(),
                                "String"
                                    | "Int"
                                    | "Boolean"
                                    | "ID"
                                    | "Float"
                                    | "Int64"
                                    | "DateTime"
                                    | "GeoPoint"
                                    | "Polygon"
                                    | "MultiPolygon"
                            ) || crate::engine::scalars::is_scalar_type(
                                &field_type_name,
                            ) || is_enum;
                            let is_relation = !is_scalar;

                            // Check if field type is polymorphic (Interface or Union)
                            // We need to check metadata map. If missing, assume scalar/standard object.
                            let is_polymorphic = if let Some(target_meta) =
                                metadata_arc.get(&field_type_name)
                            {
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
                                    if is_list {
                                        dynamic::TypeRef::named_list(field_type_name.clone())
                                    } else {
                                        dynamic::TypeRef::named(field_type_name.clone())
                                    }
                                }
                            };

                            let fname_clone = field_name.clone();
                            let type_name_clone = type_name.clone();
                            let field_type_name_clone = field_type_name.clone();
                            let is_rel = is_relation;
                            let is_poly = is_polymorphic;
                            let is_list = is_list;

                            let mut dynamic_field = dynamic::Field::new(
                                field_name.clone(),
                                ty_ref,
                                move |ctx| {
                                    let field_key = fname_clone.clone();
                                    let t_name = type_name_clone.clone();
                                    let _f_type_name = field_type_name_clone.clone();
                                    dynamic::FieldFuture::new(async move {
                                        // Special handling for GeoPoint (Embedded Object)
                                        if t_name == "GeoPoint" {
                                            let val = ctx
                                                .parent_value
                                                .try_downcast_ref::<async_graphql::Value>()?;
                                            if let async_graphql::Value::Object(map) = val {
                                                if let Some(v) = map.get(field_key.as_str()) {
                                                    return Ok(Some(dynamic::FieldValue::value(
                                                        v.clone(),
                                                    )));
                                                }
                                            }
                                            return Ok(None);
                                        }

                                        let parent_uid_result =
                                            ctx.parent_value.try_downcast_ref::<u64>();
                                        if let Ok(uid) = parent_uid_result {
                                            // Standard Resolver (Scalar or Relation)
                                            use crate::engine::resolver::Resolver;
                                            let resolver = ctx
                                                .data::<Box<dyn Resolver + Send + Sync>>()
                                                .unwrap();

                                            if is_rel {
                                                // 1. Parse Arguments for Relation
                                                let mut filter_map =
                                                    std::collections::HashMap::new();
                                                if let Ok(filter_arg) = ctx.args.try_get("filter") {
                                                    filter_map = filter_arg.deserialize()?;
                                                }
                                                let mut sort_map = std::collections::HashMap::new();
                                                if let Ok(sort_arg) = ctx.args.try_get("sort") {
                                                    sort_map = sort_arg.deserialize()?;
                                                }
                                                let mut first = None;
                                                if let Ok(limit_arg) = ctx.args.try_get("first") {
                                                    if let Ok(n) = limit_arg.u64() {
                                                        first = Some(n as usize);
                                                    }
                                                }
                                                let mut after = None;
                                                if let Ok(cursor_arg) = ctx.args.try_get("after") {
                                                    if let Ok(s) = cursor_arg.string() {
                                                        after = Some(s.to_string());
                                                    }
                                                }
                                                let mut offset = None;
                                                if let Ok(offset_arg) = ctx.args.try_get("offset") {
                                                    if let Ok(n) = offset_arg.u64() {
                                                        offset = Some(n as usize);
                                                    }
                                                }

                                                let mut near_vector = None;
                                                if let Ok(nv_arg) = ctx.args.try_get("nearVector") {
                                                    if let Ok(list) = nv_arg.list() {
                                                        let vec: Vec<f64> = list
                                                            .iter()
                                                            .filter_map(|v| v.f64().ok())
                                                            .collect();
                                                        if !vec.is_empty() {
                                                            near_vector = Some(vec);
                                                        }
                                                    }
                                                }

                                                // 2. Call resolve_list
                                                match resolver.resolve_list(
                                                    *uid,
                                                    &field_key,
                                                    filter_map,
                                                    sort_map,
                                                    first,
                                                    after,
                                                    offset,
                                                    near_vector,
                                                ) {
                                                    Ok(uids) => {
                                                        let mut fvs = Vec::new();
                                                        for u in uids {
                                                            // If polymorphic, need concrete type
                                                            if is_poly {
                                                                if let Some(ctype) =
                                                                    resolver.get_node_type(u)
                                                                {
                                                                    fvs.push(dynamic::FieldValue::with_type(dynamic::FieldValue::owned_any(u), ctype));
                                                                }
                                                            } else {
                                                                fvs.push(
                                                                    dynamic::FieldValue::owned_any(
                                                                        u,
                                                                    ),
                                                                );
                                                            }
                                                        }
                                                        if is_list {
                                                            Ok(Some(dynamic::FieldValue::list(fvs)))
                                                        } else {
                                                            if let Some(first) =
                                                                fvs.into_iter().next()
                                                            {
                                                                Ok(Some(first))
                                                            } else {
                                                                Ok(None)
                                                            }
                                                        }
                                                    }
                                                    Err(_) => Ok(None),
                                                }
                                            } else {
                                                // Scalar logic
                                                if let Some(val) =
                                                    resolver.resolve(*uid, &field_key)
                                                {
                                                    if _f_type_name == "GeoPoint"
                                                        || _f_type_name == "Polygon"
                                                    {
                                                        Ok(Some(dynamic::FieldValue::owned_any(
                                                            val,
                                                        )))
                                                    } else {
                                                        match val {
                                                            async_graphql::Value::List(items) => {
                                                                // Should not happen for Scalars usually, unless scalar list?
                                                                // Existing logic for scalar list:
                                                                let mut fvs = Vec::new();
                                                                for item in items {
                                                                    fvs.push(
                                                                        dynamic::FieldValue::value(
                                                                            item,
                                                                        ),
                                                                    );
                                                                }
                                                                Ok(Some(dynamic::FieldValue::list(
                                                                    fvs,
                                                                )))
                                                            }
                                                            async_graphql::Value::String(s) => Ok(
                                                                Some(dynamic::FieldValue::value(
                                                                    async_graphql::Value::String(s),
                                                                )),
                                                            ),
                                                            async_graphql::Value::Number(n) => Ok(
                                                                Some(dynamic::FieldValue::value(
                                                                    async_graphql::Value::Number(n),
                                                                )),
                                                            ),
                                                            _ => {
                                                                // Fallthrough to complex handling
                                                                Ok(Some(
                                                                    dynamic::FieldValue::value(val),
                                                                ))
                                                            }
                                                        }
                                                    }
                                                } else {
                                                    Ok(None)
                                                }
                                            }
                                        } else {
                                            Ok(None)
                                        }
                                    })
                                },
                            );

                            if is_relation {
                                dynamic_field = dynamic_field
                                    .argument(dynamic::InputValue::new(
                                        "filter",
                                        dynamic::TypeRef::named(format!(
                                            "{}Filter",
                                            field_type_name
                                        )),
                                    ))
                                    .argument(dynamic::InputValue::new(
                                        "sort",
                                        dynamic::TypeRef::named(format!("{}Sort", field_type_name)),
                                    ))
                                    .argument(dynamic::InputValue::new(
                                        "first",
                                        dynamic::TypeRef::named(dynamic::TypeRef::INT),
                                    ))
                                    .argument(dynamic::InputValue::new(
                                        "offset",
                                        dynamic::TypeRef::named(dynamic::TypeRef::INT),
                                    ))
                                    .argument(dynamic::InputValue::new(
                                        "after",
                                        dynamic::TypeRef::named(dynamic::TypeRef::STRING),
                                    ));

                                // Check if target type has vector field
                                if let Some(target_meta) = metadata_arc.get(&field_type_name) {
                                    if target_meta.vector_config.is_some() {
                                        dynamic_field =
                                            dynamic_field.argument(dynamic::InputValue::new(
                                                "nearVector",
                                                dynamic::TypeRef::named_list(
                                                    dynamic::TypeRef::FLOAT,
                                                ),
                                            ));
                                    }
                                }
                            }
                            obj = obj.field(dynamic_field);

                            // Input fields
                            if is_scalar && field_type_name != "ID" {
                                let input_ty_ref = match field_type_name.as_str() {
                                    "String" => {
                                        if is_list {
                                            dynamic::TypeRef::named_list(dynamic::TypeRef::STRING)
                                        } else {
                                            dynamic::TypeRef::named(dynamic::TypeRef::STRING)
                                        }
                                    }
                                    "Int" => {
                                        if is_list {
                                            dynamic::TypeRef::named_list(dynamic::TypeRef::INT)
                                        } else {
                                            dynamic::TypeRef::named(dynamic::TypeRef::INT)
                                        }
                                    }
                                    "Boolean" => {
                                        if is_list {
                                            dynamic::TypeRef::named_list(dynamic::TypeRef::BOOLEAN)
                                        } else {
                                            dynamic::TypeRef::named(dynamic::TypeRef::BOOLEAN)
                                        }
                                    }
                                    "Float" => {
                                        if is_list {
                                            dynamic::TypeRef::named_list(dynamic::TypeRef::FLOAT)
                                        } else {
                                            dynamic::TypeRef::named(dynamic::TypeRef::FLOAT)
                                        }
                                    }
                                    "Int64" => {
                                        if is_list {
                                            dynamic::TypeRef::named_list("Int64")
                                        } else {
                                            dynamic::TypeRef::named("Int64")
                                        }
                                    }
                                    "DateTime" => {
                                        if is_list {
                                            dynamic::TypeRef::named_list("DateTime")
                                        } else {
                                            dynamic::TypeRef::named("DateTime")
                                        }
                                    }
                                    "GeoPoint" => {
                                        if is_list {
                                            dynamic::TypeRef::named_list("GeoPointInput")
                                        } else {
                                            dynamic::TypeRef::named("GeoPointInput")
                                        }
                                    }
                                    _ => {
                                        let base_name = if crate::engine::scalars::is_scalar_type(
                                            &field_type_name,
                                        ) || is_enum
                                        {
                                            field_type_name.clone()
                                        } else {
                                            format!("{}Input", field_type_name)
                                        };
                                        if is_list {
                                            dynamic::TypeRef::named_list(base_name)
                                        } else {
                                            dynamic::TypeRef::named(base_name)
                                        }
                                    }
                                };
                                input = input.field(dynamic::InputValue::new(
                                    field_name.clone(),
                                    input_ty_ref.clone(),
                                ));
                                scalar_fields_map
                                    .push((field_name.clone(), field_type_name.clone()));

                                let filter_ty_name =
                                    if crate::engine::scalars::is_scalar_type(&field_type_name) {
                                        crate::engine::scalars::get_scalar_filter_type(
                                            &field_type_name,
                                        )
                                        .to_string()
                                    } else if is_enum {
                                        "StringFilter".to_string()
                                    } else {
                                        format!("{}Filter", field_type_name)
                                    };

                                filter_input = filter_input.field(dynamic::InputValue::new(
                                    field_name,
                                    dynamic::TypeRef::named(filter_ty_name),
                                ));
                            } else if is_relation {
                                let rel_input_type = format!("{}Input", field_type_name);
                                if is_list {
                                    input = input.field(dynamic::InputValue::new(
                                        field_name.clone(),
                                        dynamic::TypeRef::named_list(rel_input_type),
                                    ));
                                } else {
                                    input = input.field(dynamic::InputValue::new(
                                        field_name.clone(),
                                        dynamic::TypeRef::named(rel_input_type),
                                    ));
                                }

                                // RECURSIVE FILTER INPUT GENERATION
                                let rel_filter_type = format!("{}Filter", field_type_name);
                                // For 1:M (List), standard is usually "some: Filter". For simplicty/Dgraph-parity, we map field -> Filter.
                                // If list, it implies "Any Match".
                                filter_input = filter_input.field(dynamic::InputValue::new(
                                    field_name.clone(),
                                    dynamic::TypeRef::named(rel_filter_type),
                                ));
                            }
                        }

                        // Add recursive logical connectors
                        let filter_ty = format!("{}Filter", type_name);
                        filter_input = filter_input
                            .field(dynamic::InputValue::new(
                                "and",
                                dynamic::TypeRef::named_list(filter_ty.clone()),
                            ))
                            .field(dynamic::InputValue::new(
                                "or",
                                dynamic::TypeRef::named_list(filter_ty.clone()),
                            ))
                            .field(dynamic::InputValue::new(
                                "not",
                                dynamic::TypeRef::named(filter_ty.clone()),
                            ));

                        types.push(dynamic::Type::Object(obj));
                        if type_name != "GeoPoint" {
                            types.push(dynamic::Type::InputObject(input));
                            types.push(dynamic::Type::InputObject(filter_input));
                        }

                        // Sort Input
                        let mut sort_input =
                            dynamic::InputObject::new(format!("{}Sort", type_name));
                        for (f_name, _) in &scalar_fields_map {
                            sort_input = sort_input.field(dynamic::InputValue::new(
                                f_name.clone(),
                                dynamic::TypeRef::named("SortDirection"),
                            ));
                        }
                        types.push(dynamic::Type::InputObject(sort_input));

                        // --- ROOTS for OBJECTS ONLY ---

                        // 1. Query List
                        let list_query_name = format!("query{}", type_name);
                        let type_name_for_list = type_name.clone();
                        let filter_type_name = format!("{}Filter", type_name);

                        let uniques = meta.uniques.clone();
                        let has_vector = meta.vector_config.is_some();
                        let mut list_field = dynamic::Field::new(
                            list_query_name,
                            dynamic::TypeRef::named_list(type_name_for_list.clone()),
                            move |ctx| {
                                let t_name = type_name_for_list.clone();
                                let uniques = uniques.clone();
                                dynamic::FieldFuture::new(async move {
                                    use crate::engine::resolver::Resolver;
                                    let resolver =
                                        ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();
                                    let mut filter_map = std::collections::HashMap::new();
                                    if let Ok(filter_arg) = ctx.args.try_get("filter") {
                                        filter_map = filter_arg.deserialize()?;
                                    }
                                    let mut sort_map = std::collections::HashMap::new();
                                    if let Ok(sort_arg) = ctx.args.try_get("sort") {
                                        sort_map = sort_arg.deserialize()?;
                                    }
                                    let mut first = None;
                                    if let Ok(limit_arg) = ctx.args.try_get("first") {
                                        if let Ok(n) = limit_arg.u64() {
                                            first = Some(n as usize);
                                        }
                                    }
                                    let mut after = None;
                                    if let Ok(cursor_arg) = ctx.args.try_get("after") {
                                        if let Ok(s) = cursor_arg.string() {
                                            after = Some(s.to_string());
                                        }
                                    }
                                    let mut offset = None;
                                    if let Ok(offset_arg) = ctx.args.try_get("offset") {
                                        if let Ok(n) = offset_arg.u64() {
                                            offset = Some(n as usize);
                                        }
                                    }

                                    let mut near_vector = None;
                                    if let Ok(nv_arg) = ctx.args.try_get("nearVector") {
                                        if let Ok(list) = nv_arg.list() {
                                            let vec: Vec<f64> =
                                                list.iter().filter_map(|v| v.f64().ok()).collect();
                                            if !vec.is_empty() {
                                                near_vector = Some(vec);
                                            }
                                        }
                                    }

                                    let uids = resolver.scan_nodes(
                                        &t_name,
                                        filter_map,
                                        sort_map,
                                        first,
                                        after,
                                        offset,
                                        &uniques,
                                        near_vector,
                                    );
                                    let result: Vec<dynamic::FieldValue> = uids
                                        .into_iter()
                                        .map(|uid| dynamic::FieldValue::owned_any(uid))
                                        .collect();
                                    Ok(Some(dynamic::FieldValue::list(result)))
                                })
                            },
                        )
                        .argument(dynamic::InputValue::new(
                            "filter",
                            dynamic::TypeRef::named(filter_type_name),
                        ))
                        .argument(dynamic::InputValue::new(
                            "sort",
                            dynamic::TypeRef::named(format!("{}Sort", type_name)),
                        ))
                        .argument(dynamic::InputValue::new(
                            "first",
                            dynamic::TypeRef::named(dynamic::TypeRef::INT),
                        ))
                        .argument(dynamic::InputValue::new(
                            "offset",
                            dynamic::TypeRef::named(dynamic::TypeRef::INT),
                        ))
                        .argument(dynamic::InputValue::new(
                            "after",
                            dynamic::TypeRef::named(dynamic::TypeRef::STRING),
                        ));

                        if has_vector {
                            list_field = list_field.argument(dynamic::InputValue::new(
                                "nearVector",
                                dynamic::TypeRef::named_list(dynamic::TypeRef::FLOAT),
                            ));
                        }

                        query_fields.push(list_field);

                        // Define Relation Filter Type *Explicitly* here if needed or rely on dynamic
                        // The loop below handles fields.

                        // 2. Query Single
                        let query_single_name = format!("get{}", type_name);
                        let type_name_single = type_name.clone();
                        let uniques_single = unique_fields.clone();
                        let mut query_field = dynamic::Field::new(
                            query_single_name,
                            dynamic::TypeRef::named(type_name_single.clone()),
                            move |ctx| {
                                let t_name = type_name_single.clone();
                                let u_fields = uniques_single.clone();
                                dynamic::FieldFuture::new(async move {
                                    use crate::engine::resolver::Resolver;
                                    let resolver =
                                        ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();
                                    let id_arg = if let Ok(arg) = ctx.args.try_get("uid") {
                                        Some(arg)
                                    } else {
                                        ctx.args.try_get("id").ok()
                                    };
                                    if let Some(id_arg) = id_arg {
                                        let id_str = id_arg.string()?.to_string();
                                        let uid = if id_str.starts_with("0x") {
                                            u64::from_str_radix(&id_str[2..], 16).unwrap_or(0)
                                        } else {
                                            id_str.parse::<u64>().unwrap_or(0)
                                        };
                                        if uid > 0 && resolver.node_exists(&t_name, uid) {
                                            return Ok(Some(dynamic::FieldValue::owned_any(uid)));
                                        }
                                    }
                                    for f in &u_fields {
                                        if let Ok(val_arg) = ctx.args.try_get(f) {
                                            let val_json: serde_json::Value = val_arg
                                                .deserialize()
                                                .unwrap_or(serde_json::Value::Null);
                                            let val_json_str = serde_json::to_string(&val_json)
                                                .unwrap_or_default();
                                            if let Some(uid) = resolver.find_uid(
                                                &format!("{}.{}", t_name, f),
                                                &val_json_str,
                                            ) {
                                                return Ok(Some(dynamic::FieldValue::owned_any(
                                                    uid,
                                                )));
                                            }
                                        }
                                    }
                                    Ok(None)
                                })
                            },
                        )
                        .argument(dynamic::InputValue::new(
                            "uid",
                            dynamic::TypeRef::named(dynamic::TypeRef::ID),
                        ))
                        .argument(dynamic::InputValue::new(
                            "id",
                            dynamic::TypeRef::named(dynamic::TypeRef::ID),
                        ));
                        for f in unique_fields {
                            query_field = query_field.argument(dynamic::InputValue::new(
                                f.clone(),
                                dynamic::TypeRef::named(dynamic::TypeRef::STRING),
                            ));
                        }
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
                                let mut_start = std::time::Instant::now();
                                let input_arg = ctx.args.try_get("input")?;
                                let fields: std::collections::HashMap<String, async_graphql::Value> = input_arg.deserialize()?;
                                let deser_time = mut_start.elapsed();
                                
                                // Validation
                                let _meta = meta_arc.get(&t_name).unwrap();
                                
                                use crate::engine::resolver::Resolver;
                                let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();

                                // Acquire Semaphore Permit
                                let sem_start = std::time::Instant::now();
                                let _permit = MUTATION_SEMAPHORE.acquire().await.map_err(|e| e.to_string())?;
                                let sem_time = sem_start.elapsed();

                                // Deep Creation
                                let create_start = std::time::Instant::now();
                                let result = deep_create_node(resolver, &meta_arc, &t_name, fields).await;
                                let create_time = create_start.elapsed();
                                
                                let total = mut_start.elapsed();
                                if crate::debug_logging() && total.as_millis() > 5 {
                                    eprintln!("[SERVER] create{} | deser={:?} sem_wait={:?} create={:?} total={:?}",
                                             t_name, deser_time, sem_time, create_time, total);
                                }
                                
                                match result {
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
                                let id_arg = ctx.args.try_get("uid")?;
                                let uid = id_arg.string()?.parse::<u64>().map_err(|_| "Invalid ID")?;
                                let input_arg = ctx.args.try_get("input")?;
                                let mut fields: std::collections::HashMap<String, async_graphql::Value> = input_arg.deserialize()?;
                                
                                // Normalize fields: If value is Object with uid/id, flatten to String(uid)
                                for (_, value) in fields.iter_mut() {
                                    if let async_graphql::Value::Object(map) = value {
                                        let uid_val = map.get("uid").or(map.get("id"));
                                        if let Some(u) = uid_val {
                                            match u {
                                                async_graphql::Value::String(s) => *value = async_graphql::Value::String(s.clone()),
                                                async_graphql::Value::Number(n) => *value = async_graphql::Value::String(n.to_string()),
                                                _ => {}
                                            }
                                        }
                                    } else if let async_graphql::Value::List(list) = value {
                                        // Handle List of Objects
                                        for item in list.iter_mut() {
                                            if let async_graphql::Value::Object(map) = item {
                                                let uid_val = map.get("uid").or(map.get("id"));
                                                if let Some(u) = uid_val {
                                                     match u {
                                                        async_graphql::Value::String(s) => *item = async_graphql::Value::String(s.clone()),
                                                        async_graphql::Value::Number(n) => *item = async_graphql::Value::String(n.to_string()),
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

                                // Validation
                                let meta = meta_arc.get(&t_name).unwrap();
                                validate_input(&fields, &meta.validate_fields)?;

                                use crate::engine::resolver::Resolver;
                                let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();
                                let result = tokio::task::block_in_place(|| {
                                    resolver.update_node(&t_name, uid, fields, &u_fields, &inv_fields, &s_fields, meta.vector_config.as_ref())
                                });
                                match result {
                                    Ok(_) => Ok(Some(dynamic::FieldValue::value(async_graphql::Value::Boolean(true)))),
                                    Err(e) => Err(e.into()),
                                }
                             })
                        }).argument(dynamic::InputValue::new("uid", dynamic::TypeRef::named_nn(dynamic::TypeRef::ID)))
                          .argument(dynamic::InputValue::new("input", dynamic::TypeRef::named_nn(format!("{}Input", type_name)))));

                        // 5. Delete (Recall: RECURSIVE DELETE LOGIC HERE)
                        let delete_name = format!("delete{}", type_name);
                        let type_name_delete = type_name.clone();
                        let meta_arc_delete = metadata_arc.clone();

                        mutation_fields.push(dynamic::Field::new(delete_name, dynamic::TypeRef::named(dynamic::TypeRef::BOOLEAN), move |ctx| {
                            let t_name = type_name_delete.clone();
                            let meta_arc = meta_arc_delete.clone();
                            dynamic::FieldFuture::new(async move {
                                let id_arg = ctx.args.try_get("uid")?;
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
                                            // println!("Recursive Delete: Type={}, UID={}, CascadeFields={:?}", type_name, uid, meta.cascade_fields);
                                            // 1. Process Cascades
                                            for (field, target_type) in &meta.cascade_fields {
                                                if let Some(val) = resolver.resolve(uid, field) {
                                                    // println!("  Field: {}, Resolved: {:?}", field, val);
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
                                                    // println!("  Target UIDs to cascade: {:?}", target_uids);
                                                    for target_uid in target_uids {
                                                        recursive_delete(resolver, target_type, target_uid, meta_map).await?;
                                                    }
                                                } else {
                                                    // println!("  Field: {} resolved to None", field);
                                                }
                                            }
                                            // 2. Delete Self
                                            tokio::task::block_in_place(|| {
                                                resolver.delete_node(type_name, uid, &meta.uniques, &meta.inverses, &meta.search_fields)
                                            })?;
                                        }
                                        Ok(())
                                    })
                                }

                                match recursive_delete(resolver, &t_name, uid, &meta_arc).await {
                                    Ok(_) => Ok(Some(dynamic::FieldValue::value(async_graphql::Value::Boolean(true)))),
                                    Err(e) => Err(e.into()),
                                }
                            })
                        }).argument(dynamic::InputValue::new("uid", dynamic::TypeRef::named_nn(dynamic::TypeRef::ID))));
                    }
                    AstTypeKind::Interface(int_def) => {
                        let mut interface = dynamic::Interface::new(type_name.clone());
                        // Interface fields are declarations
                        for field in &int_def.fields {
                            let field_name = field.node.name.node.to_string();
                            let mut field_type_name = "String".to_string();
                            let mut is_list = false;
                            match &field.node.ty.node.base {
                                BaseType::Named(n) => {
                                    field_type_name = n.to_string();
                                }
                                BaseType::List(inner) => {
                                    is_list = true;
                                    match &inner.base {
                                        BaseType::Named(n) => {
                                            field_type_name = n.to_string();
                                        }
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
                                    if is_list {
                                        dynamic::TypeRef::named_list(field_type_name.clone())
                                    } else {
                                        dynamic::TypeRef::named(field_type_name.clone())
                                    }
                                }
                            };
                            interface =
                                interface.field(dynamic::InterfaceField::new(field_name, ty_ref));
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
                        let input = dynamic::InputObject::new(format!("{}Input", type_name)).field(
                            dynamic::InputValue::new(
                                "uid",
                                dynamic::TypeRef::named(dynamic::TypeRef::ID),
                            ),
                        );
                        types.push(dynamic::Type::InputObject(input));

                        // Generate Filter/Sort for Interface (Minimal implementation)
                        let filter = dynamic::InputObject::new(format!("{}Filter", type_name));
                        types.push(dynamic::Type::InputObject(filter));
                        let sort = dynamic::InputObject::new(format!("{}Sort", type_name));
                        types.push(dynamic::Type::InputObject(sort));
                    }
                    AstTypeKind::Union(union_def) => {
                        let mut union = dynamic::Union::new(type_name.clone());
                        for member in &union_def.members {
                            union = union.possible_type(member.node.to_string());
                        }
                        // union = union.register();
                        types.push(dynamic::Type::Union(union));

                        // Generate Input Object for Union to support linking
                        let input = dynamic::InputObject::new(format!("{}Input", type_name)).field(
                            dynamic::InputValue::new(
                                "uid",
                                dynamic::TypeRef::named(dynamic::TypeRef::ID),
                            ),
                        );
                        types.push(dynamic::Type::InputObject(input));
                    }
                    AstTypeKind::InputObject(input_def) => {
                        let mut input = dynamic::InputObject::new(type_name.clone());
                        for field in &input_def.fields {
                            let field_name = field.node.name.node.to_string();
                            let mut field_type_name = "String".to_string();
                            let mut is_list = false;
                            match &field.node.ty.node.base {
                                BaseType::Named(n) => {
                                    field_type_name = n.to_string();
                                }
                                BaseType::List(inner) => {
                                    is_list = true;
                                    match &inner.base {
                                        BaseType::Named(n) => {
                                            field_type_name = n.to_string();
                                        }
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
                    }
                    AstTypeKind::Enum(enum_def) => {
                        let mut e = dynamic::Enum::new(type_name.clone());
                        for value in &enum_def.values {
                            e = e.item(dynamic::EnumItem::new(value.node.value.to_string()));
                        }
                        types.push(dynamic::Type::Enum(e));
                    }
                    _ => {}
                }
            }
        }

        // Define MutationType Enum
        // 4.5 Flush (System) - Added outside the loop to be a single root checking mutation
        mutation_fields.push(dynamic::Field::new(
            "flushDatabase",
            dynamic::TypeRef::named(dynamic::TypeRef::BOOLEAN),
            |ctx| {
                dynamic::FieldFuture::new(async move {
                    use crate::engine::resolver::Resolver;
                    let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();
                    match resolver.flush() {
                        Ok(_) => Ok(Some(dynamic::FieldValue::value(
                            async_graphql::Value::Boolean(true),
                        ))),
                        Err(e) => Err(e.into()),
                    }
                })
            },
        ));

        // 4.6 Compact (System) - Trigger explicit compaction (blocking)
        mutation_fields.push(dynamic::Field::new(
            "compactDatabase",
            dynamic::TypeRef::named(dynamic::TypeRef::INT),
            |ctx| {
                dynamic::FieldFuture::new(async move {
                    use crate::engine::resolver::Resolver;
                    let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();
                    match resolver.compact() {
                        Ok(duration_ms) => Ok(Some(dynamic::FieldValue::value(
                            async_graphql::Value::Number((duration_ms as i64).into()),
                        ))),
                        Err(e) => Err(e.into()),
                    }
                })
            },
        ));

        // Define MutationType Enum
        let mutation_type_enum = dynamic::Enum::new("MutationType")
            .item(dynamic::EnumItem::new("CREATE"))
            .item(dynamic::EnumItem::new("UPDATE"))
            .item(dynamic::EnumItem::new("DELETE"));

        // Define MutationEvent Object
        let mutation_event_obj = dynamic::Object::new("MutationEvent")
            .field(dynamic::Field::new(
                "type",
                dynamic::TypeRef::named_nn("String"),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let event = ctx
                            .parent_value
                            .try_downcast_ref::<crate::realtime::bus::MutationEvent>()?;
                        Ok(Some(dynamic::FieldValue::value(
                            async_graphql::Value::String(event.type_name.clone()),
                        )))
                    })
                },
            ))
            .field(dynamic::Field::new(
                "uid",
                dynamic::TypeRef::named_nn("ID"),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let event = ctx
                            .parent_value
                            .try_downcast_ref::<crate::realtime::bus::MutationEvent>()?;
                        Ok(Some(dynamic::FieldValue::value(
                            async_graphql::Value::String(event.uid.to_string()),
                        )))
                    })
                },
            ))
            .field(dynamic::Field::new(
                "mutation",
                dynamic::TypeRef::named_nn("MutationType"),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let event = ctx
                            .parent_value
                            .try_downcast_ref::<crate::realtime::bus::MutationEvent>()?;
                        let s = match event.mutation_type {
                            crate::realtime::bus::MutationType::Create => "CREATE",
                            crate::realtime::bus::MutationType::Update => "UPDATE",
                            crate::realtime::bus::MutationType::Delete => "DELETE",
                        };
                        Ok(Some(dynamic::FieldValue::value(
                            async_graphql::Value::Enum(async_graphql::Name::new(s)),
                        )))
                    })
                },
            ))
            .field(dynamic::Field::new(
                "payload",
                dynamic::TypeRef::named("JSON"),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let event = ctx
                            .parent_value
                            .try_downcast_ref::<crate::realtime::bus::MutationEvent>()?;
                        if let Some(payload) = &event.payload {
                            let val = serde_json::to_value(payload).map_err(|e| e.to_string())?;
                            let g_val =
                                async_graphql::Value::from_json(val).map_err(|e| e.to_string())?;
                            Ok(Some(dynamic::FieldValue::value(g_val)))
                        } else {
                            Ok(None)
                        }
                    })
                },
            ));

        // Define SearchResult Object for Vector Search
        let search_result_obj = dynamic::Object::new("SearchResult")
            .field(dynamic::Field::new(
                "uid",
                dynamic::TypeRef::named_nn("ID"),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let val = ctx.parent_value.try_downcast_ref::<(u64, f64)>()?;
                        Ok(Some(dynamic::FieldValue::value(
                            async_graphql::Value::String(val.0.to_string()),
                        )))
                    })
                },
            ))
            .field(dynamic::Field::new(
                "distance",
                dynamic::TypeRef::named_nn("Float"),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let val = ctx.parent_value.try_downcast_ref::<(u64, f64)>()?;
                        Ok(Some(dynamic::FieldValue::value(
                            async_graphql::Value::Number(
                                async_graphql::Number::from_f64(val.1).unwrap(),
                            ),
                        )))
                    })
                },
            ));

        // Generic Subscription Field: "subscribe(types: [String!])"
        subscription_fields.push(
            dynamic::SubscriptionField::new(
                "event",
                dynamic::TypeRef::named_nn("MutationEvent"),
                |ctx| {
                    dynamic::SubscriptionFieldFuture::new(async move {
                        let types_arg = ctx.args.try_get("types")?;
                        let types: Vec<String> = types_arg
                            .list()?
                            .iter()
                            .map(|v| v.string().unwrap_or("").to_string())
                            .collect();

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
                },
            )
            .argument(dynamic::InputValue::new(
                "types",
                dynamic::TypeRef::named_list(dynamic::TypeRef::STRING),
            )),
        );

        // Register Point Type
        let point_type = dynamic::Object::new("Point")
            .field(dynamic::Field::new(
                "latitude",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let p = ctx
                            .parent_value
                            .try_downcast_ref::<async_graphql::Value>()?;
                        if let async_graphql::Value::Object(map) = p {
                            if let Some(async_graphql::Value::Number(n)) = map.get("latitude") {
                                return Ok(Some(dynamic::FieldValue::value(
                                    async_graphql::Value::Number(n.clone()),
                                )));
                            }
                        }
                        Ok(None)
                    })
                },
            ))
            .field(dynamic::Field::new(
                "longitude",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let p = ctx
                            .parent_value
                            .try_downcast_ref::<async_graphql::Value>()?;
                        if let async_graphql::Value::Object(map) = p {
                            if let Some(async_graphql::Value::Number(n)) = map.get("longitude") {
                                return Ok(Some(dynamic::FieldValue::value(
                                    async_graphql::Value::Number(n.clone()),
                                )));
                            }
                        }
                        Ok(None)
                    })
                },
            ));
        types.push(dynamic::Type::Object(point_type));

        let point_input = dynamic::InputObject::new("PointInput")
            .field(dynamic::InputValue::new(
                "latitude",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT),
            ))
            .field(dynamic::InputValue::new(
                "longitude",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT),
            ));
        types.push(dynamic::Type::InputObject(point_input));

        // PointFilter Input
        let point_filter_input = dynamic::InputObject::new("PointFilter").field(
            dynamic::InputValue::new("near", dynamic::TypeRef::named("NearFilter")),
        );
        types.push(dynamic::Type::InputObject(point_filter_input));

        // NearFilter Input
        let near_filter_input = dynamic::InputObject::new("NearFilter")
            .field(dynamic::InputValue::new(
                "distance",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT),
            ))
            .field(dynamic::InputValue::new(
                "coordinate",
                dynamic::TypeRef::named_nn("PointInput"),
            ));
        types.push(dynamic::Type::InputObject(near_filter_input));

        // Build Schema
        let mut query_root = dynamic::Object::new("Query");
        for field in query_fields {
            query_root = query_root.field(field);
        }

        // Inject Vector Search Query
        query_root = query_root.field(
            dynamic::Field::new(
                "search",
                dynamic::TypeRef::named_nn_list("SearchResult"),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let vector: Vec<f64> = ctx
                            .args
                            .try_get("vector")?
                            .list()?
                            .iter()
                            .map(|v| v.f64().unwrap_or(0.0))
                            .collect();

                        let k = ctx
                            .args
                            .try_get("k")
                            .ok()
                            .and_then(|v| v.u64().ok())
                            .unwrap_or(10) as usize;

                        use crate::engine::resolver::Resolver;
                        let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();

                        let results = resolver.search_vectors(&vector, k);
                        let list: Vec<dynamic::FieldValue> = results
                            .into_iter()
                            .map(|r| dynamic::FieldValue::owned_any(r))
                            .collect();

                        Ok(Some(dynamic::FieldValue::list(list)))
                    })
                },
            )
            .argument(dynamic::InputValue::new(
                "vector",
                dynamic::TypeRef::named_nn_list(dynamic::TypeRef::FLOAT),
            ))
            .argument(dynamic::InputValue::new(
                "k",
                dynamic::TypeRef::named(dynamic::TypeRef::INT),
            )),
        );

        // Inject Hybrid Search Query
        query_root = query_root.field(
            dynamic::Field::new(
                "hybridSearch",
                dynamic::TypeRef::named_nn_list("SearchResult"),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let vector: Vec<f64> = ctx
                            .args
                            .try_get("vector")?
                            .list()?
                            .iter()
                            .map(|v| v.f64().unwrap_or(0.0))
                            .collect();
                        let text = ctx.args.try_get("text")?.string()?.to_string();
                        let field = ctx.args.try_get("field")?.string()?.to_string();
                        let k = ctx
                            .args
                            .try_get("k")
                            .ok()
                            .and_then(|v| v.u64().ok())
                            .unwrap_or(10) as usize;

                        use crate::engine::resolver::Resolver;
                        let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();

                        let results = resolver.search_hybrid(&text, &field, &vector, k);
                        let list: Vec<dynamic::FieldValue> = results
                            .into_iter()
                            .map(|r| dynamic::FieldValue::owned_any(r))
                            .collect();

                        Ok(Some(dynamic::FieldValue::list(list)))
                    })
                },
            )
            .argument(dynamic::InputValue::new(
                "vector",
                dynamic::TypeRef::named_nn_list(dynamic::TypeRef::FLOAT),
            ))
            .argument(dynamic::InputValue::new(
                "text",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::STRING),
            ))
            .argument(dynamic::InputValue::new(
                "field",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::STRING),
            ))
            .argument(dynamic::InputValue::new(
                "k",
                dynamic::TypeRef::named(dynamic::TypeRef::INT),
            )),
        );
        // Define AuthZ types
        let check_permission_input = dynamic::InputObject::new("CheckPermissionInput")
            .field(dynamic::InputValue::new(
                "entityType",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "entityId",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "permission",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::STRING),
            ));

        let check_permission_result = dynamic::Object::new("CheckPermissionResult")
            .field(dynamic::Field::new(
                "entityType",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::STRING),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let val = ctx
                            .parent_value
                            .try_downcast_ref::<async_graphql::Value>()?;
                        if let async_graphql::Value::Object(map) = val {
                            if let Some(v) = map.get("entityType") {
                                return Ok(Some(dynamic::FieldValue::value(v.clone())));
                            }
                        }
                        Ok(None)
                    })
                },
            ))
            .field(dynamic::Field::new(
                "entityId",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::STRING),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let val = ctx
                            .parent_value
                            .try_downcast_ref::<async_graphql::Value>()?;
                        if let async_graphql::Value::Object(map) = val {
                            if let Some(v) = map.get("entityId") {
                                return Ok(Some(dynamic::FieldValue::value(v.clone())));
                            }
                        }
                        Ok(None)
                    })
                },
            ))
            .field(dynamic::Field::new(
                "permission",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::STRING),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let val = ctx
                            .parent_value
                            .try_downcast_ref::<async_graphql::Value>()?;
                        if let async_graphql::Value::Object(map) = val {
                            if let Some(v) = map.get("permission") {
                                return Ok(Some(dynamic::FieldValue::value(v.clone())));
                            }
                        }
                        Ok(None)
                    })
                },
            ))
            .field(dynamic::Field::new(
                "allowed",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::BOOLEAN),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let val = ctx
                            .parent_value
                            .try_downcast_ref::<async_graphql::Value>()?;
                        if let async_graphql::Value::Object(map) = val {
                            if let Some(v) = map.get("allowed") {
                                return Ok(Some(dynamic::FieldValue::value(v.clone())));
                            }
                        }
                        Ok(None)
                    })
                },
            ));

        query_root = query_root.field(
            dynamic::Field::new(
                "bulkCheckPermission",
                dynamic::TypeRef::named_nn_list("CheckPermissionResult"),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        use crate::engine::resolver::Resolver;
                        let resolver = ctx.data::<Box<dyn Resolver + Send + Sync>>().unwrap();

                        let checks_arg = ctx.args.try_get("checks")?;
                        let mut checks = Vec::new();
                        if let Ok(list) = checks_arg.list() {
                            for item in list.iter() {
                                if let Ok(obj) = item.object() {
                                    let entity_type = obj
                                        .get("entityType")
                                        .and_then(|v| v.string().ok())
                                        .unwrap_or("")
                                        .to_string();
                                    let entity_id = obj
                                        .get("entityId")
                                        .and_then(|v| v.string().ok())
                                        .unwrap_or("")
                                        .to_string();
                                    let permission = obj
                                        .get("permission")
                                        .and_then(|v| v.string().ok())
                                        .unwrap_or("")
                                        .to_string();
                                    checks.push((entity_type, entity_id, permission));
                                }
                            }
                        }

                        let results = resolver.bulk_check_permission(&ctx, checks)?;

                        let mut list = Vec::new();
                        for res in results {
                            let mut map = async_graphql::indexmap::IndexMap::new();
                            map.insert(
                                async_graphql::Name::new("entityType"),
                                async_graphql::Value::String(res.0),
                            );
                            map.insert(
                                async_graphql::Name::new("entityId"),
                                async_graphql::Value::String(res.1),
                            );
                            map.insert(
                                async_graphql::Name::new("permission"),
                                async_graphql::Value::String(res.2),
                            );
                            map.insert(
                                async_graphql::Name::new("allowed"),
                                async_graphql::Value::Boolean(res.3),
                            );
                            list.push(dynamic::FieldValue::owned_any(
                                async_graphql::Value::Object(map),
                            ));
                        }

                        Ok(Some(dynamic::FieldValue::list(list)))
                    })
                },
            )
            .argument(dynamic::InputValue::new(
                "checks",
                dynamic::TypeRef::named_nn_list("CheckPermissionInput"),
            )),
        );

        let mut mutation_root = dynamic::Object::new("Mutation");
        for field in mutation_fields {
            mutation_root = mutation_root.field(field);
        }

        let mut subscription_root = dynamic::Subscription::new("Subscription");
        for field in subscription_fields {
            subscription_root = subscription_root.field(field);
        }

        let mut schema_builder =
            dynamic::Schema::build("Query", Some("Mutation"), Some("Subscription"));
        schema_builder = schema_builder.register(query_root);
        schema_builder = schema_builder.register(mutation_root);
        schema_builder = schema_builder.register(subscription_root);
        schema_builder = schema_builder.register(mutation_type_enum);
        schema_builder = schema_builder.register(mutation_event_obj);
        schema_builder = schema_builder.register(search_result_obj);
        schema_builder = schema_builder.register(check_permission_input);
        schema_builder = schema_builder.register(check_permission_result);

        types.push(dynamic::Type::Scalar(dynamic::Scalar::new("Int64")));
        types.push(dynamic::Type::Scalar(dynamic::Scalar::new("DateTime")));

        let geo_point = dynamic::Object::new("GeoPoint")
            .field(dynamic::Field::new(
                "latitude",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let val = ctx
                            .parent_value
                            .try_downcast_ref::<async_graphql::Value>()?;
                        if let async_graphql::Value::Object(map) = val {
                            if let Some(async_graphql::Value::Number(n)) = map.get("latitude") {
                                return Ok(Some(dynamic::FieldValue::value(
                                    async_graphql::Value::Number(n.clone()),
                                )));
                            }
                        }
                        Ok(None)
                    })
                },
            ))
            .field(dynamic::Field::new(
                "longitude",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let val = ctx
                            .parent_value
                            .try_downcast_ref::<async_graphql::Value>()?;
                        if let async_graphql::Value::Object(map) = val {
                            if let Some(async_graphql::Value::Number(n)) = map.get("longitude") {
                                return Ok(Some(dynamic::FieldValue::value(
                                    async_graphql::Value::Number(n.clone()),
                                )));
                            }
                        }
                        Ok(None)
                    })
                },
            ));
        types.push(dynamic::Type::Object(geo_point));

        let geo_input = dynamic::InputObject::new("GeoPointInput")
            .field(dynamic::InputValue::new(
                "latitude",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT),
            ))
            .field(dynamic::InputValue::new(
                "longitude",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT),
            ));
        types.push(dynamic::Type::InputObject(geo_input));

        let near_filter = dynamic::InputObject::new("NearFilter")
            .field(dynamic::InputValue::new(
                "distance",
                dynamic::TypeRef::named_nn(dynamic::TypeRef::FLOAT),
            ))
            .field(dynamic::InputValue::new(
                "coordinate",
                dynamic::TypeRef::named_nn("GeoPointInput"),
            ));
        types.push(dynamic::Type::InputObject(near_filter));

        let geo_filter = dynamic::InputObject::new("GeoPointFilter")
            .field(dynamic::InputValue::new(
                "near",
                dynamic::TypeRef::named("NearFilter"),
            ))
            .field(dynamic::InputValue::new(
                "within",
                dynamic::TypeRef::named("PolygonInput"),
            ));
        types.push(dynamic::Type::InputObject(geo_filter));

        let polygon_filter = dynamic::InputObject::new("PolygonFilter")
            .field(dynamic::InputValue::new(
                "intersects",
                dynamic::TypeRef::named("PolygonInput"),
            ))
            .field(dynamic::InputValue::new(
                "within",
                dynamic::TypeRef::named("PolygonInput"),
            ));
        types.push(dynamic::Type::InputObject(polygon_filter));

        let multi_polygon_filter = dynamic::InputObject::new("MultiPolygonFilter")
            .field(dynamic::InputValue::new(
                "intersects",
                dynamic::TypeRef::named("MultiPolygonInput"),
            ))
            .field(dynamic::InputValue::new(
                "within",
                dynamic::TypeRef::named("PolygonInput"),
            ));
        types.push(dynamic::Type::InputObject(multi_polygon_filter));

        // Polygon
        let point_list_type = dynamic::TypeRef::named_nn_list("GeoPoint");
        let ring_list_type =
            dynamic::TypeRef::List(Box::new(dynamic::TypeRef::named_nn_list("GeoPoint")));

        let polygon = dynamic::Object::new("Polygon")
            .field(dynamic::Field::new(
                "exterior",
                point_list_type.clone(),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let val = ctx
                            .parent_value
                            .try_downcast_ref::<async_graphql::Value>()?;
                        if let async_graphql::Value::Object(map) = val {
                            if let Some(async_graphql::Value::List(list)) = map.get("exterior") {
                                let mapped: Vec<dynamic::FieldValue> = list
                                    .iter()
                                    .map(|v| dynamic::FieldValue::owned_any(v.clone()))
                                    .collect();
                                return Ok(Some(dynamic::FieldValue::list(mapped)));
                            }
                        }
                        Ok(None)
                    })
                },
            ))
            .field(dynamic::Field::new(
                "interiors",
                ring_list_type.clone(),
                |ctx| {
                    dynamic::FieldFuture::new(async move {
                        let val = ctx
                            .parent_value
                            .try_downcast_ref::<async_graphql::Value>()?;
                        if let async_graphql::Value::Object(map) = val {
                            if let Some(async_graphql::Value::List(rings)) = map.get("interiors") {
                                let mut mapped_rings = Vec::new();
                                for ring in rings {
                                    if let async_graphql::Value::List(points) = ring {
                                        let mapped_points: Vec<dynamic::FieldValue> = points
                                            .iter()
                                            .map(|v| dynamic::FieldValue::owned_any(v.clone()))
                                            .collect();
                                        mapped_rings.push(dynamic::FieldValue::list(mapped_points));
                                    }
                                }
                                return Ok(Some(dynamic::FieldValue::list(mapped_rings)));
                            }
                        }
                        Ok(None)
                    })
                },
            ));
        types.push(dynamic::Type::Object(polygon));

        let point_input_list_type = dynamic::TypeRef::named_nn_list("GeoPointInput");
        let ring_input_list_type =
            dynamic::TypeRef::List(Box::new(dynamic::TypeRef::named_nn_list("GeoPointInput")));

        let polygon_input = dynamic::InputObject::new("PolygonInput")
            .field(dynamic::InputValue::new("exterior", point_input_list_type))
            .field(dynamic::InputValue::new("interiors", ring_input_list_type));
        types.push(dynamic::Type::InputObject(polygon_input));

        // MultiPolygon
        let poly_list_type =
            dynamic::TypeRef::List(Box::new(dynamic::TypeRef::named_nn("Polygon")));
        let multi_polygon = dynamic::Object::new("MultiPolygon").field(dynamic::Field::new(
            "polygons",
            poly_list_type,
            |ctx| {
                dynamic::FieldFuture::new(async move {
                    let val = ctx.parent_value.try_downcast_ref::<GeoMultiPolygonData>()?;
                    let list: Vec<dynamic::FieldValue> = val
                        .polygons
                        .iter()
                        .map(|p| dynamic::FieldValue::owned_any(p.clone()))
                        .collect();
                    Ok(Some(dynamic::FieldValue::list(list)))
                })
            },
        ));
        types.push(dynamic::Type::Object(multi_polygon));

        let poly_input_list_type =
            dynamic::TypeRef::List(Box::new(dynamic::TypeRef::named_nn("PolygonInput")));
        let multi_polygon_input = dynamic::InputObject::new("MultiPolygonInput")
            .field(dynamic::InputValue::new("polygons", poly_input_list_type));
        types.push(dynamic::Type::InputObject(multi_polygon_input));

        // Register Extended Scalars (graphql-scalars parity)
        // crate::engine::scalars::register_scalars(&mut types); // Moved to top

        for obj in types {
            schema_builder = schema_builder.register(obj);
        }

        // ... (Filters, etc) ...

        let string_filter = dynamic::InputObject::new("StringFilter")
            .field(dynamic::InputValue::new(
                "eq",
                dynamic::TypeRef::named(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "contains",
                dynamic::TypeRef::named(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "allofterms",
                dynamic::TypeRef::named(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "anyofterms",
                dynamic::TypeRef::named(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "alloftext",
                dynamic::TypeRef::named(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "anyoftext",
                dynamic::TypeRef::named(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "lt",
                dynamic::TypeRef::named(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "le",
                dynamic::TypeRef::named(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "gt",
                dynamic::TypeRef::named(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "ge",
                dynamic::TypeRef::named(dynamic::TypeRef::STRING),
            ))
            .field(dynamic::InputValue::new(
                "in",
                dynamic::TypeRef::named_list(dynamic::TypeRef::STRING),
            ));

        let int_filter = dynamic::InputObject::new("IntFilter")
            .field(dynamic::InputValue::new(
                "eq",
                dynamic::TypeRef::named(dynamic::TypeRef::INT),
            ))
            .field(dynamic::InputValue::new(
                "gt",
                dynamic::TypeRef::named(dynamic::TypeRef::INT),
            ))
            .field(dynamic::InputValue::new(
                "lt",
                dynamic::TypeRef::named(dynamic::TypeRef::INT),
            ))
            .field(dynamic::InputValue::new(
                "ge",
                dynamic::TypeRef::named(dynamic::TypeRef::INT),
            ))
            .field(dynamic::InputValue::new(
                "le",
                dynamic::TypeRef::named(dynamic::TypeRef::INT),
            ))
            .field(dynamic::InputValue::new(
                "between",
                dynamic::TypeRef::named_list(dynamic::TypeRef::INT),
            ))
            .field(dynamic::InputValue::new(
                "in",
                dynamic::TypeRef::named_list(dynamic::TypeRef::INT),
            ));

        let float_filter = dynamic::InputObject::new("FloatFilter")
            .field(dynamic::InputValue::new(
                "eq",
                dynamic::TypeRef::named(dynamic::TypeRef::FLOAT),
            ))
            .field(dynamic::InputValue::new(
                "gt",
                dynamic::TypeRef::named(dynamic::TypeRef::FLOAT),
            ))
            .field(dynamic::InputValue::new(
                "lt",
                dynamic::TypeRef::named(dynamic::TypeRef::FLOAT),
            ))
            .field(dynamic::InputValue::new(
                "ge",
                dynamic::TypeRef::named(dynamic::TypeRef::FLOAT),
            ))
            .field(dynamic::InputValue::new(
                "le",
                dynamic::TypeRef::named(dynamic::TypeRef::FLOAT),
            ))
            .field(dynamic::InputValue::new(
                "between",
                dynamic::TypeRef::named_list(dynamic::TypeRef::FLOAT),
            ))
            .field(dynamic::InputValue::new(
                "in",
                dynamic::TypeRef::named_list(dynamic::TypeRef::FLOAT),
            ));

        let bool_filter = dynamic::InputObject::new("BooleanFilter").field(
            dynamic::InputValue::new("eq", dynamic::TypeRef::named(dynamic::TypeRef::BOOLEAN)),
        );

        let int64_filter = dynamic::InputObject::new("Int64Filter")
            .field(dynamic::InputValue::new(
                "eq",
                dynamic::TypeRef::named("Int64"),
            ))
            .field(dynamic::InputValue::new(
                "gt",
                dynamic::TypeRef::named("Int64"),
            ))
            .field(dynamic::InputValue::new(
                "lt",
                dynamic::TypeRef::named("Int64"),
            ))
            .field(dynamic::InputValue::new(
                "ge",
                dynamic::TypeRef::named("Int64"),
            ))
            .field(dynamic::InputValue::new(
                "le",
                dynamic::TypeRef::named("Int64"),
            ))
            .field(dynamic::InputValue::new(
                "in",
                dynamic::TypeRef::named_list("Int64"),
            ));

        let datetime_filter = dynamic::InputObject::new("DateTimeFilter")
            .field(dynamic::InputValue::new(
                "eq",
                dynamic::TypeRef::named("DateTime"),
            ))
            .field(dynamic::InputValue::new(
                "gt",
                dynamic::TypeRef::named("DateTime"),
            ))
            .field(dynamic::InputValue::new(
                "lt",
                dynamic::TypeRef::named("DateTime"),
            ))
            .field(dynamic::InputValue::new(
                "ge",
                dynamic::TypeRef::named("DateTime"),
            ))
            .field(dynamic::InputValue::new(
                "le",
                dynamic::TypeRef::named("DateTime"),
            ))
            .field(dynamic::InputValue::new(
                "in",
                dynamic::TypeRef::named_list("DateTime"),
            ));

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

        Ok((schema_builder, metadata_map))
    }

    pub async fn execute_with_resolver(
        &self,
        query: &str,
        resolver: Box<dyn crate::engine::resolver::Resolver + Send + Sync>,
    ) -> String {
        let req = async_graphql::Request::new(query).data(resolver);
        let resp = self.inner.execute(req).await;
        serde_json::to_string(&resp).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn execute_stream_with_resolver(
        &self,
        query: &str,
        resolver: Box<dyn crate::engine::resolver::Resolver + Send + Sync>,
    ) -> impl futures_util::Stream<Item = async_graphql::Response> {
        let req = async_graphql::Request::new(query).data(resolver);
        self.inner.execute_stream(req)
    }

    pub fn load_from_sdl(sdl: &str) -> Result<Schema, String> {
        let (builder, type_metadata) = Self::create_builder(sdl)?;
        let schema = builder.finish().map_err(|e| e.to_string())?;
        Ok(Self {
            inner: schema,
            sdl: sdl.to_string(),
            type_metadata,
        })
    }

    pub fn load_with_resolver<R: crate::engine::resolver::Resolver + Send + Sync + 'static>(
        sdl: &str,
        resolver: R,
    ) -> Result<Schema, String> {
        let (builder, type_metadata) = Self::create_builder(sdl)?;
        let schema = builder
            .data(Box::new(resolver) as Box<dyn crate::engine::resolver::Resolver + Send + Sync>)
            .finish()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            inner: schema,
            sdl: sdl.to_string(),
            type_metadata,
        })
    }

    /// Returns the generated SDL from async-graphql (includes all generated types)
    /// Use this for export-schema command
    pub fn sdl(&self) -> String {
        self.inner.sdl()
    }

    /// Returns the original source SDL (user-provided schema)
    /// Use this for schema sync between instances
    pub fn source_sdl(&self) -> String {
        self.sdl.clone()
    }
}

// Standalone Helper Functions

fn validate_input(
    fields: &std::collections::HashMap<String, async_graphql::Value>,
    rules: &std::collections::HashMap<String, Vec<ValidationRule>>,
) -> Result<(), String> {
    for (field_name, field_rules) in rules {
        if let Some(val) = fields.get(field_name) {
            if matches!(val, async_graphql::Value::Null) {
                continue;
            }
            for rule in field_rules {
                match rule {
                    ValidationRule::Regex(pattern) => {
                        if let async_graphql::Value::String(s) = val {
                            let re = regex::Regex::new(pattern).map_err(|_| {
                                format!("Invalid regex pattern on server for field {}", field_name)
                            })?;
                            if !re.is_match(s) {
                                return Err(format!(
                                    "Field '{}' must match pattern '{}'",
                                    field_name, pattern
                                ));
                            }
                        }
                    }
                    ValidationRule::Length { min, max } => {
                        if let async_graphql::Value::String(s) = val {
                            let len = s.len() as i64;
                            if let Some(m) = min {
                                if len < *m {
                                    return Err(format!(
                                        "Field '{}' length must be at least {}",
                                        field_name, m
                                    ));
                                }
                            }
                            if let Some(m) = max {
                                if len > *m {
                                    return Err(format!(
                                        "Field '{}' length must be at most {}",
                                        field_name, m
                                    ));
                                }
                            }
                        }
                    }
                    ValidationRule::Range { min, max } => {
                        if let async_graphql::Value::Number(n) = val {
                            if let Some(f_val) = n.as_f64() {
                                if let Some(min_val) = min {
                                    if f_val < *min_val {
                                        return Err(format!(
                                            "Field '{}' must be at least {}",
                                            field_name, min_val
                                        ));
                                    }
                                }
                                if let Some(max_val) = max {
                                    if f_val > *max_val {
                                        return Err(format!(
                                            "Field '{}' must be at most {}",
                                            field_name, max_val
                                        ));
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
    mut fields: std::collections::HashMap<String, async_graphql::Value>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u64, String>> + Send + 'a>> {
    Box::pin(async move {
        // Check if Linking via UID or ID
        let uid_val = fields.get("uid").or(fields.get("id"));
        if let Some(uid_val) = uid_val {
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
                            let field_map: std::collections::HashMap<String, async_graphql::Value> =
                                map.iter()
                                    .map(|(k, v)| (k.to_string(), v.clone()))
                                    .collect();
                            let uid = deep_create_node(resolver, meta_map, target_type, field_map)
                                .await?;
                            fields_to_replace.push((
                                field.clone(),
                                async_graphql::Value::String(uid.to_string()),
                            ));
                        }
                        async_graphql::Value::List(list) => {
                            let mut new_uids = Vec::new();
                            for item in list {
                                if let async_graphql::Value::Object(map) = item {
                                    let field_map: std::collections::HashMap<
                                        String,
                                        async_graphql::Value,
                                    > = map
                                        .iter()
                                        .map(|(k, v)| (k.to_string(), v.clone()))
                                        .collect();
                                    let uid = deep_create_node(
                                        resolver,
                                        meta_map,
                                        target_type,
                                        field_map,
                                    )
                                    .await?;
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
                            fields_to_replace
                                .push((field.clone(), async_graphql::Value::List(new_uids)));
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
            // 2. Validate
            validate_input(&fields, &meta.validate_fields)?;

            // 3. Create Self — run on blocking thread to avoid Fjall I/O stalling the async runtime
            tokio::task::block_in_place(|| {
                resolver.create_node(
                    type_name,
                    fields,
                    &meta.uniques,
                    &meta.inverses,
                    &meta.search_fields,
                    meta.vector_config.as_ref(),
                )
            })
        } else {
            Err(format!("Type {} not found", type_name))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enum_input_generation_repro() {
        let sdl = "
            enum VideoStatus {
                NEW
                PUBLISHED
            }
            type Video {
                status: VideoStatus
            }
        ";
        let builder_res = Schema::create_builder(sdl);
        assert!(
            builder_res.is_ok(),
            "Builder creation failed: {:?}",
            builder_res.err()
        );
        let (builder, _) = builder_res.unwrap();
        let schema_res = builder.finish();

        // This is expected to fail before the fix because VideoStatus is treated as object/relation
        // and it looks for VideoStatusInput, which doesn't exist.
        assert!(
            schema_res.is_ok(),
            "Schema finish failed (likely missing Input type): {:?}",
            schema_res.err()
        );
    }
}
