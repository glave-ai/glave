//! What a model is called when no catalog and no setting names it: written from its id as its
//! maker writes it (`anthropic/claude-haiku-5.5` is Anthropic's `Claude Haiku 5.5`), so a
//! client never names a model from its slug.

/// Makers by the prefix their model ids carry (`anthropic/…`), as they write their names.
const MAKERS: [(&str, &str); 17] = [
    ("ai21", "AI21"),
    ("amazon", "Amazon"),
    ("anthropic", "Anthropic"),
    ("cohere", "Cohere"),
    ("deepseek", "DeepSeek"),
    ("google", "Google"),
    ("meta-llama", "Meta"),
    ("microsoft", "Microsoft"),
    ("minimax", "MiniMax"),
    ("mistralai", "Mistral"),
    ("moonshotai", "Moonshot AI"),
    ("nvidia", "NVIDIA"),
    ("openai", "OpenAI"),
    ("perplexity", "Perplexity"),
    ("qwen", "Qwen"),
    ("x-ai", "xAI"),
    ("z-ai", "Z.ai"),
];

/// Words in model ids that are not written in title case.
const WORDS: [(&str, &str); 6] = [
    ("ai", "AI"),
    ("deepseek", "DeepSeek"),
    ("glm", "GLM"),
    ("gpt", "GPT"),
    ("oss", "OSS"),
    ("vl", "VL"),
];

/// A word of an id as its maker writes it: one of [`WORDS`], else in title case.
fn word(word: &str) -> String {
    let lower = word.to_lowercase();
    if let Some((_, written)) = WORDS.iter().find(|(known, _)| *known == lower) {
        return (*written).to_owned();
    }
    let mut chars = word.chars();
    chars
        .next()
        .map(char::to_uppercase)
        .into_iter()
        .flatten()
        .chain(chars)
        .collect()
}

/// A slug as people write it: `claude-haiku-5.5` is `Claude Haiku 5.5`, `gpt-6-luna` is
/// `GPT-6 Luna` (GPT joined to the number after it), and a tag after a colon goes in brackets
/// (`llama-4:free` is `Llama 4 (Free)`).
fn words(slug: &str) -> String {
    let (base, tag) = slug.split_once(':').unwrap_or((slug, ""));
    let tag = tag.split(':').next().unwrap_or_default();
    let mut name = String::new();
    let mut before: Option<String> = None;
    for each in base
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|each| !each.is_empty())
        .map(word)
    {
        match &before {
            None => {}
            Some(gpt) if gpt == "GPT" && each.starts_with(|c: char| c.is_ascii_digit()) => {
                name.push('-');
            }
            Some(_) => name.push(' '),
        }
        name.push_str(&each);
        before = Some(each);
    }
    if tag.is_empty() {
        name
    } else {
        format!("{name} ({})", word(tag))
    }
}

/// The name of the model `id` names, as its maker writes it, from its last part.
pub(crate) fn own_name(id: &str) -> String {
    let slug = id.rsplit('/').find(|part| !part.is_empty()).unwrap_or(id);
    let name = words(slug);
    if name.is_empty() { id.to_owned() } else { name }
}

/// Who made the model `id` names, from the part before its last: as the maker writes it, else
/// in title case; none for an id with one part.
pub(crate) fn maker_of(id: &str) -> Option<String> {
    let mut parts = id.rsplit('/').filter(|part| !part.is_empty());
    parts.next()?;
    let prefix = parts.next()?;
    let lower = prefix.to_lowercase();
    let maker = match MAKERS.iter().find(|(known, _)| *known == lower) {
        Some((_, written)) => (*written).to_owned(),
        None => words(prefix),
    };
    (!maker.is_empty()).then_some(maker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_is_named_from_its_id_as_its_maker_writes_it() {
        let named = |id: &str| (maker_of(id), own_name(id));
        let pair = |maker: &str, own: &str| (Some(maker.to_owned()), own.to_owned());
        assert_eq!(
            named("anthropic/claude-haiku-5.5"),
            pair("Anthropic", "Claude Haiku 5.5")
        );
        assert_eq!(named("openai/gpt-6-luna"), pair("OpenAI", "GPT-6 Luna"));
        assert_eq!(
            named("deepseek/deepseek-v4.1-flash"),
            pair("DeepSeek", "DeepSeek V4.1 Flash")
        );
        assert_eq!(
            named("acme-labs/rocket-2:free"),
            pair("Acme Labs", "Rocket 2 (Free)")
        );
        assert_eq!(
            named("openrouter/meta-llama/llama-4-scout"),
            pair("Meta", "Llama 4 Scout")
        );
        assert_eq!(named("MoonshotAI/kimi-k3"), pair("Moonshot AI", "Kimi K3"));
        assert_eq!(named("hope-12b"), (None, "Hope 12b".to_owned()));
        assert_eq!(own_name("gpt-oss-120b"), "GPT OSS 120b");
        assert_eq!(own_name("qwen3_vl"), "Qwen3 VL");
        assert_eq!(own_name("x-ai/grok-5"), "Grok 5");
        assert_eq!(maker_of("x-ai/grok-5").as_deref(), Some("xAI"));
        assert_eq!(
            own_name("---"),
            "---",
            "an id with no words is its own name"
        );
    }
}
