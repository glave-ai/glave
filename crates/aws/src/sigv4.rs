//! AWS Signature Version 4 over each request, by AWS's own signer, with keys from a [`Source`]
//! kept until five minutes before they expire.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use aws_credential_types::Credentials as AwsKeys;
use aws_sigv4::http_request::{SignableBody, SignableRequest, SigningSettings, sign};
use aws_sigv4::sign::v4;
use glave_ports::auth::{AuthError, Credentials, Header, Outgoing, Refreshing, Secret};
use glave_ports::{BoxFuture, Clock};
use jiff::{SignedDuration, Timestamp};

use crate::keys::{Keys, Source};
use crate::{Endpoint, Region};

/// How long before keys expire they are replaced, as AWS's own libraries do.
const EARLY: SignedDuration = SignedDuration::from_secs(300);

/// Requests to Bedrock signed with AWS keys.
pub struct SigV4 {
    source: Source,
    region: Region,
    service: &'static str,
    http: reqwest::Client,
    clock: Arc<dyn Clock>,
    keys: Refreshing<Keys>,
}

impl SigV4 {
    /// Signs for `endpoint` in `region`, with keys from `source`, at the time `clock` says.
    pub fn new(
        source: Source,
        region: Region,
        endpoint: Endpoint,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, AuthError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AuthError::unreachable(format!("could not set up HTTP: {e}")))?;
        Ok(Self {
            source,
            region,
            service: endpoint.signing_name(),
            http,
            keys: Refreshing::new(Arc::clone(&clock), EARLY),
            clock,
        })
    }
}

impl Credentials for SigV4 {
    fn authorize<'a>(
        &'a self,
        request: &'a Outgoing<'a>,
    ) -> BoxFuture<'a, Result<Vec<Header>, AuthError>> {
        Box::pin(async move {
            let keys = self.keys.get(|| self.source.fetch(&self.http)).await?;
            signed(
                &keys,
                self.region.as_str(),
                self.service,
                self.clock.now(),
                request,
            )
        })
    }

    fn describe(&self) -> String {
        format!(
            "AWS SigV4 for {} in {}, with {}",
            self.service,
            self.region.as_str(),
            self.source.describe()
        )
    }
}

/// The headers that sign `request` with `keys` for `service` in `region` at `time`:
/// `x-amz-date`, `x-amz-security-token` for temporary keys, and `authorization`.
fn signed(
    keys: &Keys,
    region: &str,
    service: &str,
    time: Timestamp,
    request: &Outgoing<'_>,
) -> Result<Vec<Header>, AuthError> {
    let identity = AwsKeys::new(
        keys.access_key_id.clone(),
        keys.secret_access_key.expose().to_owned(),
        keys.session_token
            .as_ref()
            .map(|token| token.expose().to_owned()),
        None,
        "glave",
    )
    .into();
    let params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name(service)
        .time(SystemTime::from(time))
        .settings(SigningSettings::default())
        .build()
        .map_err(|error| AuthError::refused(format!("could not sign the request: {error}")))?
        .into();
    let signable = SignableRequest::new(
        request.method,
        request.url,
        request.headers.iter().copied(),
        SignableBody::Bytes(request.body),
    )
    .map_err(|error| AuthError::refused(format!("could not sign the request: {error}")))?;
    let (instructions, _signature) = sign(signable, &params)
        .map_err(|error| AuthError::refused(format!("could not sign the request: {error}")))?
        .into_parts();
    Ok(instructions
        .headers()
        .map(|(name, value)| Header {
            name: name.to_ascii_lowercase(),
            value: Secret::new(value),
        })
        .collect())
}

#[cfg(test)]
mod tests;
