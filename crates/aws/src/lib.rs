//! glave's way into Amazon Bedrock, for an organization whose models come through its AWS
//! contract.
//!
//! Bedrock serves OpenAI's Chat Completions, so `glave-provider-openai` talks to it, and Claude
//! through Anthropic's Messages API at its `/anthropic` route, so `glave-provider-anthropic`
//! does; this crate says where (`bedrock-runtime`, or `bedrock-mantle`) and with
//! what: a Bedrock API key (a bearer token, or `x-api-key` at `/anthropic`), or, keyless, a SigV4 signature over each request
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
