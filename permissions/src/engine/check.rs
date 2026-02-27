
use crate::schema::registry::SchemaRegistry;
use crate::storage::auth_store::AuthStore;
use crate::engine::context::Context;
use crate::engine::evaluator::evaluate_permission;
use crate::storage::tuple::Subject;


#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    Allow,
    Deny,
}

pub fn check(
    store: &AuthStore,
    schema: &SchemaRegistry,
    namespace: &str,
    entity_type: &str,
    entity_id: &str,
    permission: &str,
    subject: &Subject,
    context: &Context,
) -> CheckResult {

    let ns_schema = match schema.get_namespace(namespace) {
        Some(s) => s,
        None => return CheckResult::Deny,
    };

    let user_str = format!("{}:{}", subject.entity, subject.id);

    let allowed = evaluate_permission(
        ns_schema,
        entity_type,
        entity_id,
        permission,
        &user_str,
        store,
        context,
        0,
    );

    if allowed {
        CheckResult::Allow
    } else {
        CheckResult::Deny
    }
}

pub fn bulk_check(
    store: &AuthStore,
    schema: &SchemaRegistry,
    namespace: &str,
    checks: Vec<(&str, &str, &str)>, // (entity_type, entity_id, permission)
    subject: &Subject,
    context: &Context,
) -> Vec<CheckResult> {
    
    let mut results = Vec::new();

    for (entity_type, entity_id, permission) in checks {
        let result = check(
            store,
            schema,
            namespace,
            entity_type,
            entity_id,
            permission,
            subject,
            context,
        );
        results.push(result);
    }
    
    results
}
