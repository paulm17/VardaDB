use std::sync::Arc;
use tokio::time::{sleep, Duration};
use crate::config::AuthConfig;
use crate::state::AuthState;
use crate::email::smtp::{send_email, EmailParams, render_password_reset, render_magic_link};
use jobs::Queue;
use serde_json::Value;

pub async fn start_email_worker(auth_state: Arc<AuthState>, queue: Arc<Queue>) {
    tracing::info!("Auth Email Worker started for queue '{}'", "auth_email");

    loop {
        match queue.pop() {
            Ok(Some(job)) => {
                let payload_str = std::str::from_utf8(&job.payload).unwrap_or("");
                
                if let Ok(json) = serde_json::from_str::<Value>(payload_str) {
                    let job_type = json["type"].as_str().unwrap_or("");
                    let email = json["email"].as_str().unwrap_or("");
                    let code = json["code"].as_str().unwrap_or("");

                    if let Some(smtp_config) = &auth_state.config.smtp {
                        let server_url = &auth_state.config.server_url;
                        let result = match job_type {
                            "password_reset" => {
                                let confirmation_url = format!("{}/auth/verify_code", server_url);
                                let (html, text) = render_password_reset(&confirmation_url, code);
                                
                                let params = EmailParams {
                                    to: email.to_string(),
                                    subject: "Reset your password".to_string(),
                                    html,
                                    text,
                                };
                                
                                send_email(smtp_config, params).await
                            },
                            "magic_link" => {
                                let redirect_to = json["redirect_to"].as_str().unwrap_or("/");
                                let confirmation_url = format!("{}/auth/verify_magiclink_code", server_url);
                                let (html, text) = render_magic_link(&confirmation_url, code, redirect_to);
                                
                                let params = EmailParams {
                                    to: email.to_string(),
                                    subject: "Your Magic Link".to_string(),
                                    html,
                                    text,
                                };
                                
                                send_email(smtp_config, params).await
                            },
                            _ => {
                                tracing::warn!("Unknown email job type: {}", job_type);
                                Ok(())
                            }
                        };

                        match result {
                            Ok(_) => {
                                let _ = queue.ack(job.id);
                            },
                            Err(e) => {
                                tracing::error!("Failed to send email to {}: {}", email, e);
                                let _ = queue.fail_job(job, e);
                            }
                        }
                    } else {
                        // Email config missing, just ack and drop
                        tracing::warn!("Email job processed but SMTP configuration is missing.");
                        let _ = queue.ack(job.id);
                    }
                } else {
                    tracing::error!("Failed to parse email job payload");
                    let _ = queue.ack(job.id); // Invalid payload, discard
                }
            },
            Ok(None) => {
                sleep(Duration::from_millis(1000)).await;
            },
            Err(e) => {
                tracing::error!("Auth Email Worker Pop Error: {}", e);
                sleep(Duration::from_millis(1000)).await;
            }
        }
    }
}
