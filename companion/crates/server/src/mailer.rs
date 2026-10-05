use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use thiserror::Error;

pub trait Mailer: Send + Sync + 'static {
    fn delivery(&self) -> MailDelivery {
        MailDelivery::Email
    }

    fn send_sign_in_link(
        &self,
        to: &str,
        link: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + '_>>;
}

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MailDelivery {
    Email,
    ServerLog,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct MailError {
    message: String,
}

impl MailError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Default)]
pub struct LogMailer;

impl Mailer for LogMailer {
    fn delivery(&self) -> MailDelivery {
        MailDelivery::ServerLog
    }

    fn send_sign_in_link(
        &self,
        to: &str,
        link: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + '_>> {
        tracing::info!(target: "deskmate_server::mail",
            "\n==============================================\n  Deskmate sign-in link for {to}:\n  {link}\n=============================================="
        );
        Box::pin(std::future::ready(Ok(())))
    }
}

#[derive(Clone)]
pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpMailer {
    pub fn from_url(smtp_url: &str, from: &str) -> Result<Self, MailError> {
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(smtp_url)
            .map_err(|error| MailError::new(format!("invalid SMTP URL: {error}")))?
            .build();
        let from = from
            .parse()
            .map_err(|error| MailError::new(format!("invalid mail sender: {error}")))?;
        Ok(Self { transport, from })
    }
}

impl Mailer for SmtpMailer {
    fn send_sign_in_link(
        &self,
        to: &str,
        link: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + '_>> {
        let recipient = to.parse::<Mailbox>();
        let link = link.to_owned();
        Box::pin(async move {
            let recipient = recipient
                .map_err(|error| MailError::new(format!("invalid mail recipient: {error}")))?;
            let message = sign_in_message(self.from.clone(), recipient, &link)?;
            self.transport
                .send(message)
                .await
                .map_err(|error| MailError::new(format!("send sign-in email: {error}")))?;
            Ok(())
        })
    }
}

/// The sign-in email a person receives. It carries the public name, Deskboy;
/// "Deskmate" is the codename and stays out of anything a visitor reads.
fn sign_in_message(from: Mailbox, to: Mailbox, link: &str) -> Result<Message, MailError> {
    Message::builder()
        .from(from)
        .to(to)
        .subject("Sign in to Deskboy")
        .body(format!(
            "Use this link to sign in to Deskboy:\n\n{link}\n\nThis link expires in 15 minutes."
        ))
        .map_err(|error| MailError::new(format!("build sign-in email: {error}")))
}

/// A deterministic mail sink for consumers exercising the HTTP API.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct RecordingMailer {
    pub sent: Mutex<Vec<(String, String)>>,
}

impl Mailer for RecordingMailer {
    fn send_sign_in_link(
        &self,
        to: &str,
        link: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), MailError>> + Send + '_>> {
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((to.to_owned(), link.to_owned()));
        Box::pin(std::future::ready(Ok(())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smtp_mailer_validates_its_configuration_without_connecting() {
        assert!(SmtpMailer::from_url("not-an-smtp-url", "mail@example.com").is_err());
        assert!(SmtpMailer::from_url("smtps://smtp.example.com", "not-an-email").is_err());
        assert!(SmtpMailer::from_url("smtps://smtp.example.com", "mail@example.com").is_ok());
    }

    #[test]
    fn the_sign_in_email_uses_the_public_name() {
        let from: Mailbox = "Deskboy <hello@example.com>".parse().unwrap();
        let to: Mailbox = "person@example.com".parse().unwrap();
        let link = "https://desk.example.org/v1/app/auth/link#token";
        let message = sign_in_message(from, to, link).unwrap();
        let text = String::from_utf8(message.formatted()).unwrap();
        assert!(text.contains("Subject: Sign in to Deskboy\r\n"), "{text}");
        assert!(
            text.contains("Use this link to sign in to Deskboy:"),
            "{text}"
        );
        assert!(text.contains(link), "{text}");
        assert!(!text.contains("Deskmate"), "{text}");
    }
}
