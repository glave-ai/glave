//! A person's filters among a router's providers: each call that says whose work it is asks
//! the host for them, goes only to a router that keeps to them, carrying what fits there, and
//! is refused before it is sent when no provider fits, or when they could not be read.

use std::sync::{Arc, Mutex};

use glave_event::{CatalogProvider, Filters, PrincipalId, Sort};
use glave_gateway::{Filtering, Gateway, Router, Routes};
use glave_ports::{BoxFuture, Failure, ModelProvider, OrgId, Payer, ProviderError, Purpose, Route};
use jiff::Timestamp;

use crate::support::{Fake, block_on, request};

/// Filters that answer every call with `answer`, noting whose each was.
#[derive(Debug)]
struct Kept {
    answer: Result<Option<Filters>, ProviderError>,
    asked: Mutex<Vec<Payer>>,
}

impl Kept {
    fn answering(answer: Result<Option<Filters>, ProviderError>) -> Arc<Self> {
        Arc::new(Self {
            answer,
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl Filtering for Kept {
    fn filters(&self, payer: Payer) -> BoxFuture<'_, Result<Option<Filters>, ProviderError>> {
        self.asked.lock().unwrap().push(payer);
        let answer = self.answer.clone();
        Box::pin(async move { answer })
    }
}

fn payer() -> Payer {
    let person = PrincipalId::new(Timestamp::from_second(1).unwrap(), [1; 10]).unwrap();
    Payer {
        org: OrgId::from_uuid(person.as_uuid()),
        person: Some(person),
    }
}

fn us_only() -> Filters {
    Filters {
        based_in: vec!["US".into()],
        sort: Some(Sort::Price),
        ..Filters::default()
    }
}

/// Who the router serves the model from: one provider in China, one in the US.
fn router() -> Router {
    let provider = |slug: &str, home: &str| CatalogProvider {
        slug: slug.into(),
        name: slug.into(),
        headquarters: Some(home.into()),
        datacenters: None,
    };
    Router {
        providers: vec![provider("deepseek", "CN"), provider("groq", "US")].into(),
        endpoints: Vec::new(),
    }
}

/// A gateway trying `direct` first, then the router `routed`, with `filters`.
fn gateway(direct: &Arc<Fake>, routed: &Arc<Fake>, filters: Arc<Kept>) -> Gateway {
    let targets = vec![direct.target("big"), routed.target("big").routing(router())];
    Gateway::new(Routes::new(targets).unwrap()).with_filters(filters)
}

/// The main route's request, as `payer()`'s work.
fn theirs() -> glave_ports::ModelRequest {
    let mut asked = request(Purpose::Main);
    asked.payer = Some(payer());
    asked
}

#[test]
fn a_call_its_filters_narrow_goes_to_the_router_alone_with_the_providers_that_fit() {
    let (direct, routed) = (Fake::new("deepseek"), Fake::new("openrouter"));
    let filters = Kept::answering(Ok(Some(us_only())));
    let gateway = gateway(&direct, &routed, Arc::clone(&filters));
    let answer = block_on!(gateway.complete(&theirs(), &mut |_| {})).unwrap();
    assert_eq!(answer.served.unwrap().target, "openrouter/big");
    assert!(
        direct.seen().is_empty(),
        "a direct provider can't keep to them"
    );
    let route = Route {
        only: Some(vec!["groq".into()]),
        sort: Some(Sort::Price),
        ..Route::default()
    };
    let sent = routed.seen();
    assert_eq!(
        sent.first().and_then(|sent| sent.route.clone()),
        Some(route)
    );
    assert_eq!(*filters.asked.lock().unwrap(), [payer()]);
}

#[test]
fn only_the_order_asked_for_keeps_every_target_and_reaches_only_the_router() {
    let (direct, routed) = (Fake::new("deepseek"), Fake::new("openrouter"));
    let fastest = Filters {
        sort: Some(Sort::Speed),
        ..Filters::default()
    };
    let gateway = gateway(&direct, &routed, Kept::answering(Ok(Some(fastest))));
    let answer = block_on!(gateway.complete(&theirs(), &mut |_| {})).unwrap();
    assert_eq!(answer.served.unwrap().target, "deepseek/big");
    let sent = direct.seen();
    assert_eq!(sent.first().map(|sent| sent.route.clone()), Some(None));
}

#[test]
fn a_call_no_provider_fits_is_refused_before_it_is_sent() {
    let (direct, routed) = (Fake::new("deepseek"), Fake::new("openrouter"));
    let none = Filters {
        based_in: vec!["FR".into()],
        ..Filters::default()
    };
    let gateway = gateway(&direct, &routed, Kept::answering(Ok(Some(none))));
    let refused = block_on!(gateway.complete(&theirs(), &mut |_| {})).unwrap_err();
    let ProviderError::Unserved { failure, reason } = refused else {
        panic!("refused: {refused}");
    };
    assert_eq!(failure, Failure::Rejected);
    assert_eq!(
        reason,
        "deepseek/big can't keep to your filters among providers; no provider of asked-for at \
         openrouter/big fits your filters: loosen them to use it"
    );
    assert!(direct.seen().is_empty() && routed.seen().is_empty());
}

#[test]
fn filters_that_could_not_be_read_stop_the_call() {
    let (direct, routed) = (Fake::new("deepseek"), Fake::new("openrouter"));
    let unread = ProviderError::Unserved {
        failure: Failure::Unavailable,
        reason: "your filters could not be read".into(),
    };
    let gateway = gateway(&direct, &routed, Kept::answering(Err(unread.clone())));
    let refused = block_on!(gateway.complete(&theirs(), &mut |_| {})).unwrap_err();
    assert_eq!(refused, unread);
    assert!(direct.seen().is_empty() && routed.seen().is_empty());
}

#[test]
fn a_call_that_is_no_one_s_work_asks_for_no_filters() {
    let (direct, routed) = (Fake::new("deepseek"), Fake::new("openrouter"));
    let filters = Kept::answering(Ok(Some(us_only())));
    let gateway = gateway(&direct, &routed, Arc::clone(&filters));
    let answer = block_on!(gateway.complete(&request(Purpose::Main), &mut |_| {})).unwrap();
    assert_eq!(answer.served.unwrap().target, "deepseek/big");
    assert!(filters.asked.lock().unwrap().is_empty());
}

#[test]
fn where_no_router_serves_the_model_filters_leave_its_call_alone() {
    let direct = Fake::new("deepseek");
    let filters = Kept::answering(Ok(Some(us_only())));
    let gateway = Gateway::new(Routes::new(vec![direct.target("big")]).unwrap())
        .with_filters(Arc::clone(&filters) as Arc<dyn Filtering>);
    let answer = block_on!(gateway.complete(&theirs(), &mut |_| {})).unwrap();
    assert_eq!(answer.served.unwrap().target, "deepseek/big");
    assert!(
        filters.asked.lock().unwrap().is_empty(),
        "no router among its targets: nothing to ask"
    );
}
