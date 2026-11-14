//! glave's email plug over SMTP (`specs/model.md`, "Identity"): plain messages to one address,
//! through the mail server a deployment's configuration names, over TLS: STARTTLS on any port
//! but 465, TLS from the first byte on 465; to a mail server on this computer (`localhost` or a
//! loopback address), plainly, as nothing crosses a network. Its password comes from the
//! environment, never a file.

mod smtp;

pub use smtp::{Smtp, SmtpError, SmtpSettings};
