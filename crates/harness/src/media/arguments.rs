//! What the media tools are called and take (`specs/gateway.md`, "Media tools"): how the
//! model is told of each, a call in a line, and a call's arguments read into what the harness
//! asks the provider. The one place their shapes live.

use glave_event::Arguments;
use glave_ports::{MediaAsk, Purpose, Quality, ToolSpec};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) const IMAGE_GENERATE: &str = "image_generate";
pub(crate) const IMAGE_EDIT: &str = "image_edit";

/// The shapes an image may be asked for in, width to height: the ones every default model
/// makes.
const ASPECT_RATIOS: [&str; 8] = ["1:1", "2:3", "3:2", "3:4", "4:3", "9:16", "16:9", "21:9"];

/// A new image's shape when the call asks none.
const SQUARE: &str = "1:1";

/// The most images a call draws on beside the one it edits.
const MOST_REFERENCES: usize = 8;

/// The longest prompt, in characters.
const LONGEST_PROMPT: usize = 8_000;

/// The longest a call's prompt shows in its line, in characters.
const SHOWN_CHARS: usize = 80;

/// How much of a saved file's hash its line shows.
const BLOB_SHOWN: usize = 8;

/// The quality tiers, by name, as every media tool takes them.
const QUALITIES: [(&str, Quality); 3] = [
    ("low", Quality::Low),
    ("medium", Quality::Medium),
    ("high", Quality::High),
];

/// How the model is told of the tool for `purpose`.
pub(crate) fn spec(purpose: Purpose) -> ToolSpec {
    let file = "a saved file (blob:<sha256>) or a path on this computer";
    let mut properties = json!({
        "prompt": {"type": "string", "maxLength": LONGEST_PROMPT, "description": "The whole \
            description: subject, style, composition, colours and any words it shows. The image \
            model sees nothing else of this conversation; name each reference by its order \
            (\"the first reference\")."},
        "aspect_ratio": {"type": "string", "enum": ASPECT_RATIOS, "description": "Width to \
            height. Every default model makes these; another model makes the nearest."},
        "quality": {"type": "string", "enum": ["low", "medium", "high"], "description": "low: \
            seconds and cents, for trying ideas; medium (the default); high: slowest and \
            dearest, for finals."},
        "references": {"type": "array", "maxItems": MOST_REFERENCES, "items": {"type": "string"},
            "description": format!("Images to draw on, such as a character, a style or a \
            product, in order: each {file}.")},
        "transparent": {"type": "boolean", "description": "A transparent background, as a PNG. \
            Default false."},
    });
    if purpose == Purpose::ImageEdit {
        if let Some(fields) = properties.as_object_mut() {
            fields.insert(
                "image".to_owned(),
                json!({"type": "string", "description": format!("The image to change: {file}.")}),
            );
        }
        if let Some(ratio) = properties.pointer_mut("/aspect_ratio/description") {
            *ratio = Value::from("Width to height. Default the image's own.");
        }
        return ToolSpec {
            name: IMAGE_EDIT.to_owned(),
            description: "Change an image as described, keeping the rest; the original stays \
                 as it was. One image a call. Returns the saved file, blob:<sha256>, and shows \
                 you the image."
                .to_owned(),
            parameters: json!({"type": "object", "properties": properties,
                "required": ["image", "prompt"]}),
        };
    }
    if let Some(ratio) = properties.pointer_mut("/aspect_ratio/description") {
        *ratio = Value::from(
            "Width to height, default 1:1. Every default model makes these; another model makes \
             the nearest.",
        );
    }
    ToolSpec {
        name: IMAGE_GENERATE.to_owned(),
        description: "Make an image from a description. One image a call: for several, call \
             it several times. Returns the saved file, blob:<sha256>, and shows you the image."
            .to_owned(),
        parameters: json!({"type": "object", "properties": properties, "required": ["prompt"]}),
    }
}

/// A call of a media tool in a line, naming each file it sends, as `web_fetch`'s names its
/// address: `image_edit: make it night (art/hero.png, blob:9f86d081…)`.
pub(crate) fn describe(name: &str, arguments: &Arguments) -> String {
    let Arguments::Json(map) = arguments else {
        return glave_ports::describe_call(name, arguments);
    };
    let Some(prompt) = map.get("prompt").and_then(Value::as_str) else {
        return glave_ports::describe_call(name, arguments);
    };
    let line: String = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut shown: String = line.chars().take(SHOWN_CHARS).collect();
    if line.chars().count() > SHOWN_CHARS {
        shown.push('…');
    }
    let named = map.get("image").into_iter().chain(
        map.get("references")
            .and_then(Value::as_array)
            .into_iter()
            .flatten(),
    );
    let files: Vec<String> = named.filter_map(Value::as_str).map(file_shown).collect();
    let name = name.trim().to_ascii_lowercase();
    if files.is_empty() {
        format!("{name}: {shown}")
    } else {
        format!("{name}: {shown} ({})", files.join(", "))
    }
}

/// A file a call names, as its line shows it: a saved one by the start of its hash.
fn file_shown(named: &str) -> String {
    let named = named.trim();
    match named.strip_prefix("blob:") {
        Some(hex) if hex.len() > BLOB_SHOWN => {
            let start: String = hex.chars().take(BLOB_SHOWN).collect();
            format!("blob:{start}…")
        }
        _ => named.to_owned(),
    }
}

/// A media call's arguments, as the model gave them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Given {
    prompt: String,
    #[serde(default)]
    aspect_ratio: Option<String>,
    #[serde(default)]
    quality: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    references: Vec<String>,
    #[serde(default)]
    transparent: bool,
}

/// What a media call asks the provider: its prompt, the images it draws on as the model named
/// them, in order (the one it changes first), and what else it asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Asked {
    pub(super) prompt: String,
    pub(super) images: Vec<String>,
    pub(super) ask: MediaAsk,
}

/// The purpose of the media tool `name`, if it is one.
pub(super) fn purpose(name: &str) -> Option<Purpose> {
    match name.trim().to_ascii_lowercase().as_str() {
        IMAGE_GENERATE => Some(Purpose::ImageGenerate),
        IMAGE_EDIT => Some(Purpose::ImageEdit),
        _ => None,
    }
}

/// `arguments` for the tool for `purpose`, or what the model is told is wrong with them.
pub(super) fn asked(purpose: Purpose, arguments: &Arguments) -> Result<Asked, String> {
    let Arguments::Json(map) = arguments else {
        return Err("The arguments were not JSON.".to_owned());
    };
    let given: Given = serde_json::from_value(Value::Object(map.clone()))
        .map_err(|error| format!("The arguments don't fit the tool: {error}."))?;
    if given.prompt.trim().is_empty() {
        return Err("Say what to make in prompt.".to_owned());
    }
    if given.prompt.chars().count() > LONGEST_PROMPT {
        return Err(format!("prompt is at most {LONGEST_PROMPT} characters."));
    }
    if let Some(ratio) = &given.aspect_ratio
        && !ASPECT_RATIOS.contains(&ratio.as_str())
    {
        return Err(format!(
            "aspect_ratio {ratio:?} is not one of {}.",
            ASPECT_RATIOS.join(", ")
        ));
    }
    let quality = match given.quality.as_deref() {
        None => Quality::default(),
        Some(named) => QUALITIES
            .iter()
            .find(|(name, _)| *name == named)
            .map(|(_, quality)| *quality)
            .ok_or_else(|| format!("quality {named:?} is not low, medium or high."))?,
    };
    if given.references.len() > MOST_REFERENCES {
        return Err(format!("At most {MOST_REFERENCES} references a call."));
    }
    let aspect_ratio = match (purpose, &given.image) {
        (Purpose::ImageEdit, None) => return Err("Name the image to change in image.".to_owned()),
        (Purpose::ImageEdit, Some(_)) => given.aspect_ratio,
        (_, None) => given.aspect_ratio.or_else(|| Some(SQUARE.to_owned())),
        (_, Some(_)) => {
            return Err(
                "image_generate takes no image: use image_edit to change one, or name \
                 it in references to draw on it."
                    .to_owned(),
            );
        }
    };
    Ok(Asked {
        prompt: given.prompt,
        images: given.image.into_iter().chain(given.references).collect(),
        ask: MediaAsk {
            aspect_ratio,
            quality,
            transparent: given.transparent,
            ..MediaAsk::default()
        },
    })
}

#[cfg(test)]
mod tests;
