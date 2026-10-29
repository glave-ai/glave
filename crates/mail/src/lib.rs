//! glave's email plug over SMTP (`specs/model.md`, "Identity"): plain messages to one address,
//! through the mail server a deployment's configuration names, over TLS always: STARTTLS on
//! any port but 465, TLS from the first byte on 465. Its password comes from the environment,
//! never a file.

mod smtp;

pub use smtp::{Smtp, SmtpError, SmtpSettings};
