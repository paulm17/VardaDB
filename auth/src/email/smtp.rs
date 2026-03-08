use crate::config::SmtpConfig;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, SmtpTransport, Transport};

#[derive(Debug, Clone)]
pub struct EmailParams {
    pub to: String,
    pub subject: String,
    pub html: String,
    pub text: String,
}

pub async fn send_email(config: &SmtpConfig, params: EmailParams) -> Result<(), String> {
    let email = Message::builder()
        .from(
            config
                .from
                .parse()
                .map_err(|e| format!("Invalid from address: {}", e))?,
        )
        .to(params
            .to
            .parse()
            .map_err(|e| format!("Invalid to address: {}", e))?)
        .subject(params.subject)
        .header(ContentType::TEXT_HTML)
        .body(params.html)
        .map_err(|e| format!("Failed to build email: {}", e))?;

    let credentials = Credentials::new(
        config.username.clone().unwrap_or_default(),
        config.password.clone().unwrap_or_default(),
    );

    let mailer: AsyncSmtpTransport<lettre::Tokio1Executor> =
        AsyncSmtpTransport::<lettre::Tokio1Executor>::relay(&config.server)
            .map_err(|e| format!("Failed to create relay: {}", e))?
            .port(config.port)
            .credentials(credentials)
            .build();

    mailer
        .send(email)
        .await
        .map_err(|e| format!("Failed to send email: {}", e))?;

    Ok(())
}

pub fn render_password_reset(confirmation_url: &str, code: &str) -> (String, String) {
    let html = format!(
        r#"
<table role="presentation" class="main">
  <tr>
    <td>
      <p>Hi,</p>
      <p>Follow this link to reset your password:</p>
      <a href="{}?code={}">Reset Password</a>
      <p>Good luck!</p>
    </td>
  </tr>
</table>"#,
        confirmation_url, code
    );

    let text = format!(
        "Follow this link to reset your password: {}?code={}",
        confirmation_url, code
    );

    (html, text)
}

pub fn render_magic_link(confirmation_url: &str, code: &str, redirect: &str) -> (String, String) {
    // Basic redirect URL appending
    let url = format!(
        "{}?code={}&redirect_to={}",
        confirmation_url,
        code,
        urlencoding::encode(redirect)
    );

    let html = format!(
        r#"
<table role="presentation" class="main">
  <tr>
    <td>
      <p>Hi,</p>
      <p>Click below to log in:</p>
      <a href="{}">Login with Magic Link</a>
      <p>Good luck!</p>
    </td>
  </tr>
</table>"#,
        url
    );

    let text = format!("Click here to log in: {}", url);

    (html, text)
}
