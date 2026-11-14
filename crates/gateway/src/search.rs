//! Web search through the gateway (`specs/gateway.md`, "Web search"): one service the server's
//! settings name, reached with a key only the server holds, each search at the service's flat
//! price for one. Where credits are kept, each search that says whose work it is holds that
//! price before it is sent and is charged after, as `search`: at the cost the service reports,
//! else at its price; one the service refused, at nothing, as a refused search is not billed;
//! one sent and never answered, at its hold.

use std::fmt;
use std::sync::Arc;

use glave_event::{CostKind, Currency, Money, Nanos, TokenCounts};
use glave_ports::{Failure, ModelRequest, ProviderError, SearchAsk, Searched};

use crate::credits::{Charging, Paying};
use crate::price::Price;
use crate::target::Model;

/// No price by the token: a search is priced whole.
static WHOLE: Price = Price {
    list: String::new(),
    currency: Currency::USD,
    input_per_mtok: Nanos::ZERO,
    cache_read_per_mtok: Nanos::ZERO,
    output_per_mtok: Nanos::ZERO,
};

/// The service a server searches the web through, and what one search costs there.
#[derive(Clone)]
pub struct WebSearch {
    /// What searches, holding the key.
    pub handle: Arc<dyn Model>,
    /// What one search costs, in dollars: what each holds where credits are kept.
    pub price: Money,
}

impl fmt::Debug for WebSearch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebSearch")
            .field("price", &self.price)
            .finish_non_exhaustive()
    }
}

impl WebSearch {
    /// Searches for `ask`, `request` saying whose work it is, paid from `credits` where they
    /// are kept and it says.
    pub(crate) async fn search(
        &self,
        credits: Option<&dyn Charging>,
        request: &ModelRequest,
        ask: &SearchAsk,
    ) -> Result<Searched, ProviderError> {
        let paying = match (credits, request.payer) {
            (Some(credits), Some(payer)) => {
                let whole = (&WHOLE, 0);
                Some(Paying::hold(credits, payer, self.price, whole, CostKind::Search).await?)
            }
            _ => None,
        };
        let found = self.handle.search(request, ask).await;
        if let Some(paying) = paying {
            match &found {
                Ok(found) => {
                    let cost = found.cost.or(Some(self.price));
                    paying.settle(cost, &TokenCounts::default()).await;
                }
                Err(error) => paying.broke(error).await,
            }
        }
        found
    }
}

/// Why a search went nowhere where the server searches nothing.
pub(crate) fn unserved() -> ProviderError {
    ProviderError::Unserved {
        failure: Failure::Rejected,
        reason: "this server has no web search set up".to_owned(),
    }
}
