//! glave's way into Azure OpenAI, for an organization whose models come through its Azure
//! contract.
//!
//! Azure OpenAI's v1 API speaks OpenAI's protocol, so `glave-provider-openai` talks to it; this
//! crate says where (a resource's `/openai/v1` endpoint, models named by deployment) and with
//! what: an `api-key`, or, keyless on AKS, a Microsoft Entra token from workload identity, the
//! cluster's federated token exchanged with Entra and refreshed before it expires.

mod endpoint;
mod workload;

pub use endpoint::{EndpointError, base_url};
pub use workload::{DEFAULT_SCOPE, WorkloadIdentity};
