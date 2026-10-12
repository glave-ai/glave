//! glave's way into Amazon Bedrock, for an organization whose models come through its AWS
//! contract.
//!
//! Bedrock serves OpenAI's Chat Completions, so `glave-provider-openai` talks to it; this crate
//! says where (`bedrock-runtime`, or `bedrock-mantle` for the models only it serves) and with
//! what: a Bedrock API key as a bearer token, or, keyless, a SigV4 signature over each request
//! with keys from the environment, from IRSA (STS `AssumeRoleWithWebIdentity`) or from EKS Pod
//! Identity, refreshed before they expire.

mod container;
mod endpoint;
mod keys;
mod sigv4;
mod sts;

pub use endpoint::{Endpoint, Region, RegionError};
pub use keys::Source;
pub use sigv4::SigV4;
