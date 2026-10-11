//! What can go wrong reading config, policy and trust: a file that cannot be read or written,
//! one that says something glave cannot take, and a choice the constraints do not allow.

use std::path::PathBuf;

/// A config, policy or trust error. Each says what failed and with what input.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A file could not be read, written or found.
    #[error("could not {action} {}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A file says something glave cannot take; the message names the file and the key.
    #[error("{0}")]
    Invalid(String),
    /// A choice outside the constraints; the message names the rule and where the choice
    /// came from.
    #[error("{0}")]
    Refused(String),
    /// Something failed in `what`, with the cause below it.
    #[error("{what}")]
    Within {
        what: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// A result whose error is [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// An I/O failure to `action` the file at `path`.
    pub(crate) fn io(
        action: &'static str,
        path: impl Into<PathBuf>,
    ) -> impl FnOnce(std::io::Error) -> Self {
        let path = path.into();
        move |source| Self::Io {
            action,
            path,
            source,
        }
    }

    /// `source`, failing in `what`.
    pub(crate) fn within(
        what: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Within {
            what: what.into(),
            source: Box::new(source),
        }
    }

    /// The message with every cause after it, as `{:#}` shows an error chain.
    pub fn chain(&self) -> String {
        let mut text = self.to_string();
        let mut next = std::error::Error::source(self);
        while let Some(cause) = next {
            text.push_str(": ");
            text.push_str(&cause.to_string());
            next = cause.source();
        }
        text
    }
}

/// Returns [`Error::Refused`] with a formatted message: a choice the constraints do not allow.
macro_rules! refuse {
    ($($arg:tt)*) => {
        return Err($crate::Error::Refused(format!($($arg)*)))
    };
}

/// Returns [`Error::Invalid`] with a formatted message: a file glave cannot take.
macro_rules! invalid {
    ($($arg:tt)*) => {
        return Err($crate::Error::Invalid(format!($($arg)*)))
    };
}

pub(crate) use {invalid, refuse};
