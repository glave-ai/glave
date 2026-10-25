//! What glave says of each model it offers, from its services' catalogs (`specs/gateway.md`,
//! "One entry per model"): one entry per model whichever services run it, its maker's own API
//! tried before an aggregator, before any other server; its name, context, output limit,
//! images, price and reasoning levels as the catalogs give them, the price from the service
//! tried first, and the level that service is sent unasked; and what the gateway checks a
//! request against, prices it at and sends it with.

use glave_event::{Catalog, CatalogModel, Currency, ProviderKind, Reasoning};
use glave_gateway::{Capabilities, Price, Target};
use glave_session::protocol::ModelInfo;

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
    let (efforts, default) = card.reasoning.as_ref().map_or_else(
        || (Vec::new(), None),
        |reasoning| (reasoning.efforts.clone(), reasoning.default.clone()),
    );
    let target = target
        .with_capabilities(capabilities(card))
        .with_efforts(efforts, default);
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

/// The entry for `id`, served by `providers` in order, as `cards` describe it, the first
/// first; named `name` when the settings name it. Its reasoning's default is `unasked`, the
/// level its first target is sent when a thread chooses none, if any.
pub(super) fn info(
    id: &str,
    cards: &[&CatalogModel],
    providers: Vec<String>,
    name: Option<&str>,
    unasked: Option<&str>,
) -> ModelInfo {
    let first = |pick: fn(&CatalogModel) -> Option<u64>| cards.iter().find_map(|card| pick(card));
    let listed = cards.iter().find_map(|card| card.name.as_deref());
    let (maker, own) = match listed.map(|listed| listed.split_once(": ")) {
        Some(Some((maker, own))) => (Some(maker.trim()), Some(own.trim())),
        Some(None) => (None, listed),
        None => (None, None),
    };
    ModelInfo {
        name: name.or(own).map(str::to_owned),
        maker: maker.filter(|maker| !maker.is_empty()).map(str::to_owned),
        providers,
        context: first(|card| card.context),
        max_output: first(|card| card.max_output),
        images: cards.iter().find_map(|card| card.images),
        // The price is the first service's: the one a call goes to.
        price: cards.first().and_then(|card| card.price),
        reasoning: cards
            .iter()
            .find_map(|card| card.reasoning.clone())
            .map(|reasoning| Reasoning {
                default: unasked.map(str::to_owned),
                ..reasoning
            }),
        ..ModelInfo::new(id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use glave_event::{At, TokenPrice};
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
    fn an_entry_takes_its_name_and_maker_from_the_listing_and_its_price_from_the_first_service() {
        let reference = card();
        let direct = CatalogModel::new("claude-haiku-5-5");
        let providers = vec!["Anthropic".to_owned(), "OpenRouter".to_owned()];
        let info = info(
            "claude-haiku-5-5",
            &[&direct, &reference],
            providers,
            None,
            None,
        );
        assert_eq!(info.name.as_deref(), Some("Claude Haiku 5.5"));
        assert_eq!(info.maker.as_deref(), Some("Anthropic"));
        assert_eq!(info.context, Some(1_000_000), "from the reference");
        assert_eq!(info.price, None, "the service tried first says no price");
        let reasoning = info.reasoning.unwrap();
        assert_eq!(reasoning.efforts.len(), 2);
        assert_eq!(
            reasoning.default, None,
            "the service tried first is sent none"
        );
        assert_eq!(info.providers, ["Anthropic", "OpenRouter"]);
        let named = super::info("m", &[&reference], Vec::new(), Some("Mine"), Some("high"));
        assert_eq!(
            named.name.as_deref(),
            Some("Mine"),
            "the settings' name wins"
        );
        assert_eq!(
            named.reasoning.and_then(|r| r.default).as_deref(),
            Some("high")
        );
    }

    #[test]
    fn a_target_takes_its_levels_and_their_default_from_its_own_service_alone() {
        let openrouter = Catalog {
            kind: ProviderKind::Openrouter,
            url: None,
            listed: At::from(jiff::Timestamp::UNIX_EPOCH),
            models: vec![card()],
        };
        let handle = || {
            let model = Arc::new(ScriptedModel::new(Vec::new()));
            Target::new(model, "openrouter", "anthropic/claude-haiku-5.5")
        };
        let own = described(handle(), Some((&card(), &openrouter)));
        assert_eq!(own.effort_for(None), Some("high"));
        assert_eq!(own.effort_for(Some("low")), Some("low"));
        let elsewhere = described(handle(), None);
        assert_eq!(
            elsewhere.effort_for(None),
            None,
            "another service's card says it"
        );
        assert_eq!(elsewhere.effort_for(Some("low")), None);
    }

    #[test]
    fn a_models_cards_are_its_services_own_then_another_s_for_the_same_model() {
        let catalog = |kind, models: Vec<CatalogModel>| Catalog {
            kind,
            url: None,
            listed: At::from(jiff::Timestamp::UNIX_EPOCH),
            models,
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
