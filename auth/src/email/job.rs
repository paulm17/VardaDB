// Email worker — requires concrete Queue type.
// TODO: After SQLite migration stabilizes, refactor this to accept
// Arc<dyn JobProcessor> trait that exposes pop/ack/fail_job methods.
// For now, this module is only compiled when auth-email feature is enabled,
// and uses a placeholder that logs a warning.

use crate::state::AuthState;
use std::sync::Arc;

#[cfg(feature = "auth-email")]
pub async fn start_email_worker(_auth_state: Arc<AuthState>, _queue: Arc<dyn jobs::JobEnqueuer>) {
    tracing::warn!(
        "Email worker is currently disabled during SQLite migration. Emails will not be sent."
    );
    // TODO: Restore full email worker with pop/ack/fail_job once Queue trait is expanded
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
    }
}

#[cfg(not(feature = "auth-email"))]
pub async fn start_email_worker(_auth_state: Arc<AuthState>, _queue: Arc<dyn jobs::JobEnqueuer>) {
    // No-op when auth-email feature is disabled
}
