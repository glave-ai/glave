//! The certificates ledger's records (`specs/one-server.md`, "HTTPS of its own"): what glave
//! needs to serve HTTPS for its own name, in the personal project of the person whose server
//! it is. `acme.registered`: the account glave gets certificates with at an ACME directory,
//! its key sealed with the rest. `certificate.issued`: a certificate for a name, with its chain
//! and its private key; the latest for a name replaces the ones before it.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use crate::codec::{Payload, parse};
use crate::envelope::At;
use crate::secret::Secret;

pub(crate) const REGISTERED: &str = "acme.registered";
pub(crate) const ISSUED: &str = "certificate.issued";

/// An account registered with an ACME directory, as the ACME client keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcmeRegistered {
    /// The directory's URL, such as Let's Encrypt's.
    pub directory: String,
    /// The account's id and private key, as the client wrote them.
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub credentials: Secret,
}

/// A certificate issued for `name`, valid from `not_before` to `not_after`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateIssued {
    /// The name it is for, such as `glave.example.com`.
    pub name: String,
    /// The certificate and the chain to its authority, as PEM.
    pub chain: String,
    /// Its private key, as PEM.
    #[serde(serialize_with = "crate::secret::serialize_exposed")]
    pub key: Secret,
    pub not_before: At,
    pub not_after: At,
}

impl Payload for AcmeRegistered {
    const TYPE: &'static str = REGISTERED;
    const VERSION: u32 = 1;
}

impl Payload for CertificateIssued {
    const TYPE: &'static str = ISSUED;
    const VERSION: u32 = 1;
}

pub(crate) fn decode_registered(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<AcmeRegistered>> {
    (version == 1).then(|| parse(data))
}

pub(crate) fn decode_issued(
    version: u32,
    data: &RawValue,
) -> Option<serde_json::Result<CertificateIssued>> {
    (version == 1).then(|| parse(data))
}
