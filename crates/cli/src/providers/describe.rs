//! What glave says of each model it offers, from its services' catalogs (`specs/gateway.md`,
//! "One entry per model"): one entry per model whichever services run it, its maker's own API
//! tried before an aggregator, before any other server; its name, context, output limit,
//! images, price and reasoning levels as the catalogs give them, the price from the service
//! tried first, the levels too where that service has some, and the level it is sent unasked;
//! and what the gateway checks a request against, prices it at and sends it with.

use glave_event::{Catalog, CatalogModel, Currency, Precision, ProviderKind, Reasoning};
use glave_gateway::{Capabilities, Price, Target};
use glave_session::protocol::{EndpointInfo, ModelInfo};

/// The order a model's services are tried in: the maker's own API, then an aggregator, then
/// any other server. Cloud contracts come first of all, as the settings' model.
pub(super) fn rank(kind: ProviderKind) -> u8 {
    match kind {
        ProviderKind::Openai | ProviderKind::Anthropic => 0,
        ProviderKind::Openrouter => 1,
        ProviderKind::Compatible => 2,
    }
}

/// The name a model shares across the services that run it, as far as their ids tell: the id
/// without its maker's prefix, in lower case, with dots as hyphens and no date at its end, so
/// Anthropic's `claude-haiku-5-5-20261007` and OpenRouter's `anthropic/claude-haiku-5.5` are
/// one model. A model whose ids differ more than that shows once for each service.
pub(crate) fn identity(id: &str) -> String {
    let own = id.rsplit('/').next().unwrap_or(id).to_ascii_lowercase();
    let own = own.replace('.', "-");
    match own.rsplit_once('-') {
        Some((base, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => {
            base.to_owned()
        }
        _ => own,
    }
}

/// A service, in a person's words, by the short name its answers record (`openrouter`) or its
/// host.
pub(crate) fn service_name(provider: &str) -> String {
    match provider {
        "openrouter" => "OpenRouter",
        "openai" => "OpenAI",
        "anthropic" => "Anthropic",
        "azure" => "Azure OpenAI",
        "bedrock" => "Amazon Bedrock",
        "vertex" => "Google Vertex AI",
        "ollama" => "Ollama",
        "local" => "Local server",
        other => other,
    }
    .to_owned()
}

/// What the gateway may count on of the model `card` describes: what it does not say is
/// assumed possible.
pub(super) fn capabilities(card: &CatalogModel) -> Capabilities {
    Capabilities {
        images: card.images.unwrap_or(true),
        context_tokens: card.context,
        max_output_tokens: card
            .max_output
            .and_then(|tokens| u32::try_from(tokens).ok()),
        ..Capabilities::default()
    }
}

/// The price `catalog` lists for the model `card` describes, as the gateway prices a call.
pub(super) fn price(card: &CatalogModel, catalog: &Catalog) -> Option<Price> {
    let price = card.price?;
    let day = catalog.listed.timestamp().strftime("%Y-%m-%d");
    let kind = serde_json::to_value(catalog.kind).ok()?;
    Some(Price {
        list: format!("{} catalog {day}", kind.as_str()?),
        currency: Currency::USD,
        input_per_mtok: price.input,
        cache_read_per_mtok: price.cache_read.unwrap_or(price.input),
        output_per_mtok: price.output,
    })
}

/// A target with what `card`, its service's own description of it, says, if there is one,
/// known everywhere by its [`identity`]. Its reasoning levels and the one its model uses
/// unasked are its own service's alone: another service's names for them may not be what
/// this one takes.
pub(super) fn described(target: Target, card: Option<(&CatalogModel, &Catalog)>) -> Target {
    let same = identity(&target.model);
    let target = target.serving(same);
    let Some((card, catalog)) = card else {
        return target;
    };
    let target = target.with_capabilities(capabilities(card));
    let target = match &card.reasoning {
        Some(reasoning) => target.with_reasoning(reasoning),
        None => target,
    };
    match price(card, catalog) {
        Some(price) => target.with_price(price),
        None => target,
    }
}

/// The cards of the model `id` in `catalogs`: its own service's, then another service's for
/// the same model, as a reference for what the first does not say.
pub(super) fn cards_for<'a>(
    id: &str,
    own: Option<&'a Catalog>,
    catalogs: &'a [Catalog],
) -> Vec<&'a CatalogModel> {
    let exact = own.and_then(|catalog| catalog.models.iter().find(|model| model.id == id));
    let same = identity(id);
    let reference = catalogs
        .iter()
        .flat_map(|catalog| &catalog.models)
        .find(|model| {
            identity(&model.id) == same && exact.is_none_or(|own| !std::ptr::eq(own, *model))
        });
    exact.into_iter().chain(reference).collect()
}

/// Who serves a model through the router that serves it, `routed`: the router's catalog and
/// its name for the model. Its endpoints as that catalog lists them, each with where its
/// provider is based; none where no router serves it.
fn endpoints(routed: Option<(&Catalog, &str)>) -> Vec<EndpointInfo> {
    let found = routed.and_then(|(catalog, model)| {
        let card = catalog.models.iter().find(|card| card.id == model)?;
        Some((catalog, card))
    });
    let Some((catalog, card)) = found else {
        return Vec::new();
    };
    card.endpoints
        .iter()
        .map(|endpoint| {
            let provider = catalog
                .providers
                .iter()
                .find(|provider| provider.slug == endpoint.provider);
            EndpointInfo {
                provider: provider.map_or_else(|| endpoint.provider.clone(), |p| p.name.clone()),
                slug: endpoint.provider.clone(),
                based_in: provider.and_then(|provider| provider.headquarters.clone()),
                datacenters: provider.and_then(|provider| provider.datacenters.clone()),
                precision: Precision::of(endpoint.quantization.as_deref()),
                context: endpoint.context,
                max_output: endpoint.max_output,
                price: endpoint.price,
                keeps_nothing: endpoint.zdr,
                speed: endpoint.speed,
                hidden_by: Vec::new(),
            }
        })
        .collect()
}

/// The entry for `id`, served by `providers` in order, as `cards` describe it, the first
/// first; named `name` when the settings name it. Its reasoning is `target`'s, the one tried
/// first, where it has one (a cloud contract's Claude has one and no card), else the cards';
/// its default is the level `target` is sent when a thread chooses none, if any. Its
/// endpoints are those of the router that serves it, `routed` ([`endpoints`]).
pub(super) fn info(
    id: &str,
    cards: &[&CatalogModel],
    providers: Vec<String>,
    name: Option<&str>,
    target: Option<&Target>,
    routed: Option<(&Catalog, &str)>,
) -> ModelInfo {
    let unasked = target.and_then(|target| target.effort_for(None));
    let first = |pick: fn(&CatalogModel) -> Option<u64>| cards.iter().find_map(|card| pick(card));
    let listed = cards.iter().find_map(|card| card.name.as_deref());
    let info = ModelInfo {
        providers,
        context: first(|card| card.context),
        max_output: first(|card| card.max_output),
        images: cards.iter().find_map(|card| card.images),
        // The price is the first service's: the one a call goes to.
        price: cards.first().and_then(|card| card.price),
        reasoning: target
            .and_then(Target::reasoning)
            .or_else(|| cards.iter().find_map(|card| card.reasoning.as_ref()))
            .map(|reasoning| Reasoning {
                default: unasked.map(str::to_owned),
                ..reasoning.clone()
            }),
        endpoints: endpoints(routed),
        ..ModelInfo::new(id)
    };
    // The listing's name, then the settings' over it.
    [listed, name]
        .into_iter()
        .flatten()
        .fold(info, ModelInfo::named)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glave_event::{At, CatalogEndpoint, CatalogProvider, TokenPrice};
    use glave_ports::testing::ScriptedModel;

    use super::*;

    #[test]
    fn one_model_at_several_services_is_known_by_one_name() {
        assert_eq!(identity("anthropic/claude-haiku-5.5"), "claude-haiku-5-5");
        assert_eq!(identity("claude-haiku-5-5-20261007"), "claude-haiku-5-5");
        assert_eq!(identity("claude-haiku-5-5"), "claude-haiku-5-5");
        assert_eq!(identity("openai/gpt-6"), identity("gpt-6"));
        assert_ne!(identity("qwen3:8b"), identity("qwen/qwen3-8b"));
        assert_eq!(identity("model-2026"), "model-2026", "not a date");
    }

    fn card() -> CatalogModel {
        CatalogModel {
            name: Some("Anthropic: Claude Haiku 5.5".into()),
            context: Some(1_000_000),
            max_output: Some(128_000),
            images: Some(true),
            price: Some(TokenPrice {
                input: 100_000,
                output: 500_000,
                cache_read: Some(10_000),
            }),
            reasoning: Some(Reasoning {
                efforts: vec!["high".into(), "low".into()],
                default: Some("high".into()),
                mandatory: false,
            }),
            ..CatalogModel::new("anthropic/claude-haiku-5.5")
        }
    }

    #[test]
    fn a_model_s_endpoints_say_where_each_provider_is_based_as_far_as_the_router_says() {
        let endpoint = |provider: &str, quantization: Option<&str>, zdr: bool| CatalogEndpoint {
            provider: provider.into(),
            quantization: quantization.map(str::to_owned),
            context: Some(64_000),
            max_output: None,
            price: None,
            zdr,
            speed: Some(90),
        };
        let routed = CatalogModel {
            endpoints: vec![
                endpoint("deepseek", None, false),
                endpoint("deepinfra", Some("fp8"), true),
                endpoint("gone", Some("bf16"), false),
            ],
            ..CatalogModel::new("deepseek/deepseek-chat")
        };
        let provider = |slug: &str, name: &str, headquarters: &str| CatalogProvider {
            slug: slug.into(),
            name: name.into(),
            headquarters: Some(headquarters.into()),
            datacenters: (slug == "deepinfra").then(|| vec!["US".to_owned()]),
        };
        let openrouter = Catalog {
            kind: ProviderKind::Openrouter,
            url: None,
            listed: At::from(jiff::Timestamp::UNIX_EPOCH),
            models: vec![routed],
            providers: vec![
                provider("deepseek", "DeepSeek", "CN"),
                provider("deepinfra", "DeepInfra", "US"),
            ],
        };
        let found = endpoints(Some((&openrouter, "deepseek/deepseek-chat")));
        let facts: Vec<_> = found
            .iter()
            .map(|e| {
                (
                    e.provider.as_str(),
                    e.based_in.as_deref(),
                    e.precision,
                    e.keeps_nothing,
                )
            })
            .collect();
        assert_eq!(
            facts,
            [
                ("DeepSeek", Some("CN"), Precision::Unstated, false),
                ("DeepInfra", Some("US"), Precision::EightBit, true),
                ("gone", None, Precision::Full, false),
            ],
            "as the router that serves it lists them; a provider not listed is not placed"
        );
        assert_eq!(found[0].datacenters, None, "unknown, never guessed");
        assert_eq!(found[1].datacenters, Some(vec!["US".to_owned()]));
        assert_eq!(endpoints(None), Vec::new(), "no router serves it");
        assert_eq!(
            endpoints(Some((&openrouter, "other/model"))),
            Vec::new(),
            "a model the router does not list"
        );
    }

    /// A target described by `card`, its own service's, or by none.
    fn target(card: Option<&CatalogModel>) -> Target {
        let openrouter = Catalog {
            kind: ProviderKind::Openrouter,
            url: None,
            listed: At::from(jiff::Timestamp::UNIX_EPOCH),
            models: Vec::new(),
            providers: Vec::new(),
        };
        let model = Arc::new(ScriptedModel::new(Vec::new()));
        let target = Target::new(model, "openrouter", "anthropic/claude-haiku-5.5");
        described(target, card.map(|card| (card, &openrouter)))
    }

    #[test]
    fn an_entry_takes_its_name_and_maker_from_the_listing_and_its_price_from_the_first_service() {
        let reference = card();
        let direct = CatalogModel::new("claude-haiku-5-5");
        let providers = vec!["Anthropic".to_owned(), "OpenRouter".to_owned()];
        let cards = [&direct, &reference];
        let info = info(
            "claude-haiku-5-5",
            &cards,
            providers,
            None,
            Some(&target(None)),
            None,
        );
        assert_eq!(info.name, "Claude Haiku 5.5");
        assert_eq!(info.maker.as_deref(), Some("Anthropic"));
        assert_eq!(info.context, Some(1_000_000), "from the reference");
        assert_eq!(info.price, None, "the service tried first says no price");
        let reasoning = info.reasoning.unwrap();
        assert_eq!(reasoning.efforts.len(), 2);
        assert_eq!(reasoning.default, None, "the first is sent none");
        assert_eq!(info.providers, ["Anthropic", "OpenRouter"]);
        let own = target(Some(&reference));
        let named = super::info(
            "m",
            &[&reference],
            Vec::new(),
            Some("Mine"),
            Some(&own),
            None,
        );
        assert_eq!(named.name, "Mine", "the settings' name wins");
        assert_eq!(
            named.maker.as_deref(),
            Some("Anthropic"),
            "the listing's maker stays"
        );
        let unasked = named.reasoning.and_then(|reasoning| reasoning.default);
        assert_eq!(unasked.as_deref(), Some("high"), "what the first is sent");
    }

    #[test]
    fn claude_through_anthropic_offers_the_levels_its_listing_names_over_another_service_s() {
        let listing = serde_json::json!({"id": "claude-haiku-5-5", "capabilities": {
            "effort": {"supported": true, "max": {"supported": true}, "xhigh": {"supported": true},
                       "high": {"supported": true}, "medium": {"supported": true},
                       "low": {"supported": true}},
            "thinking": {"supported": true, "types": {"adaptive": {"supported": true}}},
        }});
        let own = super::super::card::described("claude-haiku-5-5", &listing);
        let anthropic = Catalog {
            kind: ProviderKind::Anthropic,
            url: None,
            listed: At::from(jiff::Timestamp::UNIX_EPOCH),
            models: vec![own.clone()],
            providers: Vec::new(),
        };
        let model = Arc::new(ScriptedModel::new(Vec::new()));
        let target = Target::new(model, "anthropic", "claude-haiku-5-5");
        let target = described(target, Some((&own, &anthropic)));
        let reference = card();
        let providers = vec!["Anthropic".to_owned(), "OpenRouter".to_owned()];
        let info = info(
            "claude-haiku-5-5",
            &[&own, &reference],
            providers,
            None,
            Some(&target),
            None,
        );
        let reasoning = info.reasoning.unwrap();
        assert_eq!(reasoning.efforts, ["max", "xhigh", "high", "medium", "low"]);
        assert_eq!(
            reasoning.default, None,
            "sent none: the picker says Default"
        );
    }

    #[test]
    fn a_target_with_no_card_offers_the_levels_it_is_sent_over_another_service_s() {
        let documented = Reasoning {
            efforts: vec!["max".into(), "low".into()],
            default: None,
            mandatory: false,
        };
        let cloud = target(None).with_reasoning(&documented);
        let reference = card();
        let info = info(
            "claude-haiku-5-5",
            &[&reference],
            Vec::new(),
            None,
            Some(&cloud),
            None,
        );
        assert_eq!(info.reasoning, Some(documented));
    }

    #[test]
    fn a_target_is_sent_the_levels_and_default_of_its_own_card_and_none_without_one() {
        let own = target(Some(&card()));
        assert_eq!(own.effort_for(None), Some("high"));
        assert_eq!(own.effort_for(Some("low")), Some("low"));
        let bare = target(None);
        assert_eq!(bare.effort_for(None), None);
        assert_eq!(bare.effort_for(Some("low")), None);
    }

    #[test]
    fn a_models_cards_are_its_services_own_then_another_s_for_the_same_model() {
        let catalog = |kind, models: Vec<CatalogModel>| Catalog {
            kind,
            url: None,
            listed: At::from(jiff::Timestamp::UNIX_EPOCH),
            models,
            providers: Vec::new(),
        };
        let catalogs = [
            catalog(
                ProviderKind::Anthropic,
                vec![CatalogModel::new("claude-haiku-5-5")],
            ),
            catalog(ProviderKind::Openrouter, vec![card()]),
        ];
        let both = cards_for("claude-haiku-5-5", catalogs.first(), &catalogs);
        let ids: Vec<&str> = both.iter().map(|card| card.id.as_str()).collect();
        assert_eq!(ids, ["claude-haiku-5-5", "anthropic/claude-haiku-5.5"]);
        let reference = cards_for("claude-haiku-5-5-20261007", None, &catalogs[1..]);
        assert_eq!(reference.len(), 1);
        let priced = price(&card(), &catalogs[1]).unwrap();
        assert_eq!(priced.list, "openrouter catalog 1970-01-01");
        assert_eq!(priced.cache_read_per_mtok, 10_000);
        let can = capabilities(&card());
        assert_eq!(can.context_tokens, Some(1_000_000));
        assert_eq!(can.max_output_tokens, Some(128_000));
        assert!(can.tools && can.images);
    }
}
