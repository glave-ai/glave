//! Sending through an SMTP server: one connection per message, which is how rarely glave sends
//! (an invite, a recovery link), each within [`SEND_WITHIN`].

use std::time::Duration;

use glave_event::Secret;
use glave_ports::{BoxFuture, Mail, MailError, Mailer};
use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport as _, Message, Tokio1Executor};
use thiserror::Error;

/// How long a message may take to hand to the mail server, from its name looked up to the
/// last reply: a few round trips and TLS.
const SEND_WITHIN: Duration = Duration::from_secs(30);

/// The port on which a mail server speaks TLS from the first byte (SMTPS); on any other,
/// glave asks for STARTTLS and refuses a server without it.
const SMTPS: u16 = 465;

/// Where and as whom glave sends: the deployment's `[email]` settings.
#[derive(Debug, Clone)]
pub struct SmtpSettings {
    /// The mail server's name.
    pub host: String,
    pub port: u16,
    /// Who messages come from, such as `glave <glave@example.com>`.
    pub from: String,
    /// The user and password it signs in with, when it asks for them.
    pub user: Option<String>,
    pub password: Option<Secret>,
}

/// Why the settings make no mailer.
#[derive(Debug, Error)]
pub enum SmtpError {
    #[error("{0:?} is not an address messages can come from")]
    From(String),
    #[error("could not set up sending through {host}: {reason}")]
    Server { host: String, reason: String },
    #[error(
        "[email] names a user to sign in as, but GLAVE_SMTP_PASSWORD is not set, or the other \
         way round: set both, or neither"
    )]
    Credentials,
}

/// Sends through one SMTP server.
pub struct Smtp {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl Smtp {
    /// A mailer for `settings`.
    pub fn new(settings: SmtpSettings) -> Result<Self, SmtpError> {
        let from: Mailbox = settings
            .from
            .parse()
            .map_err(|_| SmtpError::From(settings.from.clone()))?;
        let builder = if settings.port == SMTPS {
            AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.host)
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&settings.host)
        };
        let builder = builder
            .map_err(|error| SmtpError::Server {
                host: settings.host.clone(),
                reason: error.to_string(),
            })?
            .port(settings.port)
            .timeout(Some(SEND_WITHIN));
        let builder = match (settings.user, settings.password) {
            (Some(user), Some(password)) => {
                builder.credentials(Credentials::new(user, password.expose().to_owned()))
            }
            (None, None) => builder,
            _ => return Err(SmtpError::Credentials),
        };
        Ok(Self {
            transport: builder.build(),
            from,
        })
    }
}

impl Mailer for Smtp {
    fn reaches(&self, to: &str) -> bool {
        to.parse::<Mailbox>().is_ok()
    }

    fn send<'a>(&'a self, mail: &'a Mail) -> BoxFuture<'a, Result<(), MailError>> {
        Box::pin(async move {
            let to: Mailbox = mail
                .to
                .parse()
                .map_err(|_| MailError::Address(mail.to.clone()))?;
            let message = Message::builder()
                .from(self.from.clone())
                .to(to)
                .subject(mail.subject.as_str())
                .header(ContentType::TEXT_PLAIN)
                .body(mail.text.clone())
                .map_err(|error| MailError::Send(error.to_string()))?;
            // The transport's own limit is on connecting alone; this one is on all of it.
            let sent = tokio::time::timeout(SEND_WITHIN, self.transport.send(message)).await;
            let sent = sent.map_err(|_| {
                MailError::Send(format!(
                    "the mail server did not take the message within {}",
                    glave_ports::spoken(SEND_WITHIN)
                ))
            })?;
            sent.map(drop)
                .map_err(|error| MailError::Send(said(&error)))
        })
    }
}

/// What went wrong sending, without what the mail server said back, which may hold the address.
fn said(error: &lettre::transport::smtp::Error) -> String {
    match error.status() {
        Some(code) => format!("the mail server answered {code}"),
        None if error.is_timeout() => format!(
            "the mail server did not answer within {}",
            glave_ports::spoken(SEND_WITHIN)
        ),
        None if error.is_tls() => "TLS with the mail server failed".to_owned(),
        None => "the mail server could not be reached".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(from: &str, port: u16) -> SmtpSettings {
        SmtpSettings {
            host: "mail.example.com".into(),
            port,
            from: from.into(),
            user: None,
            password: None,
        }
    }

    #[test]
    fn a_mailer_needs_an_address_to_send_from() {
        assert!(matches!(
            Smtp::new(settings("not an address", 587)),
            Err(SmtpError::From(_))
        ));
        assert!(Smtp::new(settings("glave <glave@example.com>", 587)).is_ok());
        assert!(Smtp::new(settings("glave@example.com", SMTPS)).is_ok());
        let mut half = settings("glave@example.com", 587);
        half.user = Some("glave".into());
        assert!(matches!(Smtp::new(half), Err(SmtpError::Credentials)));
    }

    #[test]
    fn a_message_to_no_address_is_refused_before_anything_is_sent() {
        let smtp = Smtp::new(settings("glave@example.com", 587)).unwrap();
        let mail = Mail {
            to: "nobody".into(),
            subject: "Hi".into(),
            text: "Hello".into(),
        };
        let sent = glave_ports::block_on!(smtp.send(&mail));
        assert_eq!(sent, Err(MailError::Address("nobody".into())));
    }
}
