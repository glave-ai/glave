//! Routes by purpose: each purpose goes to its own route, or `main`'s when it has none, under
//! the target's model name, and every answer says which route and target served it.

#[cfg(test)]
#[path = "support/fake.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod routes {
    use glave_event::{Currency, Money};
    use glave_gateway::{Gateway, Price, RouteError, Routes};
    use glave_ports::{ModelProvider, Purpose, Served};

    use super::support::{Fake, block_on, request};

    #[test]
    fn a_purpose_without_a_route_uses_main() {
        let main = Fake::new("openrouter");
        let gateway = Gateway::new(Routes::new(vec![main.target("big")]).unwrap());
        for purpose in [
            Purpose::Main,
            Purpose::Judge,
            Purpose::Summary,
            Purpose::Subagent,
        ] {
            let answer = block_on!(gateway.complete(&request(purpose), &mut |_| {})).unwrap();
            assert_eq!(
                answer.served,
                Some(Served {
                    route: purpose.id().into(),
                    target: "openrouter/big".into(),
                    reasoning_stripped: false,
                })
            );
        }
        let models: Vec<String> = main.seen().into_iter().map(|r| r.model).collect();
        assert_eq!(
            models, ["big"; 4],
            "the target's model, not the one asked for"
        );
    }

    #[test]
    fn a_purpose_with_a_route_uses_its_own_targets() {
        let provider = Fake::new("openrouter");
        let routes = Routes::new(vec![provider.target("big")])
            .unwrap()
            .with_route(
                Purpose::Judge,
                vec![provider.target("small").named("cheap judge")],
            )
            .unwrap();
        let gateway = Gateway::new(routes);
        let judged = block_on!(gateway.complete(&request(Purpose::Judge), &mut |_| {})).unwrap();
        let main = block_on!(gateway.complete(&request(Purpose::Main), &mut |_| {})).unwrap();
        assert_eq!(judged.served.unwrap().target, "cheap judge");
        assert_eq!(main.served.unwrap().target, "openrouter/big");
        let models: Vec<String> = provider.seen().into_iter().map(|r| r.model).collect();
        assert_eq!(models, ["small", "big"]);
    }

    #[test]
    fn a_chosen_model_replaces_main_but_never_a_purpose_with_its_own_route() {
        let provider = Fake::new("openrouter");
        let routes = Routes::new(vec![provider.target("big"), provider.target("backup")])
            .unwrap()
            .with_route(Purpose::Judge, vec![provider.target("small")])
            .unwrap()
            .with_choices(vec![provider.target("other")]);
        let gateway = Gateway::new(routes);
        let asking = |purpose, model: &str| {
            let mut asked = request(purpose);
            asked.model = model.into();
            block_on!(gateway.complete(&asked, &mut |_| {}))
                .unwrap()
                .served
                .unwrap()
                .target
        };
        assert_eq!(asking(Purpose::Main, "other"), "openrouter/other");
        assert_eq!(
            asking(Purpose::Subagent, "openrouter/other"),
            "openrouter/other"
        );
        assert_eq!(asking(Purpose::Summary, "other"), "openrouter/other");
        assert_eq!(asking(Purpose::Judge, "other"), "openrouter/small");
        // The default, and a model that is not a choice, keep the route as it is.
        assert_eq!(asking(Purpose::Main, "big"), "openrouter/big");
        assert_eq!(asking(Purpose::Main, "unknown"), "openrouter/big");
        let models: Vec<String> = provider.seen().into_iter().map(|r| r.model).collect();
        assert_eq!(models, ["other", "other", "other", "small", "big", "big"]);
    }

    #[test]
    fn a_route_needs_a_target() {
        assert_eq!(
            Routes::new(Vec::new()).unwrap_err(),
            RouteError::Empty("main")
        );
        let main = Fake::new("p");
        let empty = Routes::new(vec![main.target("m")])
            .unwrap()
            .with_route(Purpose::Summary, Vec::new());
        assert_eq!(empty.unwrap_err(), RouteError::Empty("summary"));
    }

    #[test]
    fn a_configured_price_costs_an_answer_the_provider_did_not() {
        let provider = Fake::new("vllm");
        let price = Price {
            list: "ours-2026-10".into(),
            currency: Currency::USD,
            input_per_mtok: 1_000_000,
            cache_read_per_mtok: 0,
            output_per_mtok: 2_000_000,
        };
        let gateway =
            Gateway::new(Routes::new(vec![provider.target("hope").with_price(price)]).unwrap());
        let answer = block_on!(gateway.complete(&request(Purpose::Main), &mut |_| {})).unwrap();
        let cost = answer.cost.unwrap();
        // 100 in at $1 and 10 out at $2, per million.
        assert_eq!(
            cost.amount,
            Money {
                micros: 120,
                currency: Currency::USD
            }
        );
        assert_eq!(cost.price_list, "ours-2026-10");
    }
}
