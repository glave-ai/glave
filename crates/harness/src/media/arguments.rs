//! What the media tools are called and take (`specs/gateway.md`, "Media tools"): how the
//! model is told of each, a call in a line, and a call's arguments read into what the harness
//! asks the provider. The one place their shapes live.

use glave_event::Arguments;
use glave_ports::{MediaAsk, Purpose, ToolSpec};
use serde::Deserialize;
use serde_json::json;

pub(crate) const IMAGE_GENERATE: &str = "image_generate";
pub(crate) const IMAGE_EDIT: &str = "image_edit";

/// The shapes an image may be asked for in, width to height: the ones every image model glave
/// serves makes.
const ASPECT_RATIOS: [&str; 8] = ["1:1", "2:3", "3:2", "3:4", "4:3", "9:16", "16:9", "21:9"];

/// The most images a call draws on beside the one it edits.
const MOST_REFERENCES: usize = 8;

/// The longest a call's prompt shows in its line, in characters.
const SHOWN_CHARS: usize = 80;

/// How the model is told of the tool for `purpose`.
pub(crate) fn spec(purpose: Purpose) -> ToolSpec {
    let prompt = json!({"type": "string", "description": "The whole description: subject, \
        style, composition, colours and any words it shows. The image model sees nothing else \
        of this conversation."});
    let aspect_ratio = json!({"type": "string", "enum": ASPECT_RATIOS,
        "description": "Width to height. Unset: square for a new image, the image's own shape \
        for an edit."});
    let references = json!({"type": "array", "maxItems": MOST_REFERENCES, "items": {"type": "string"},
        "description": "Images to draw on, such as a character, a style or a product: each a \
        saved file (blob:<sha256>) or a path in the workspace."});
    if purpose == Purpose::ImageEdit {
        return ToolSpec {
            name: IMAGE_EDIT.to_owned(),
            description: "Change an image as described, keeping the rest; the original stays \
                 as it was. The image comes first and any references after it, in order, so \
                 the prompt can name them (\"the second image\"). One image a call. Returns the \
                 saved file, blob:<sha256>, and shows you the image."
                .to_owned(),
            parameters: json!({"type": "object", "properties": {
                "image": {"type": "string", "description": "The image to change: a saved file \
                    (blob:<sha256>) or a path in the workspace."},
                "prompt": prompt,
                "aspect_ratio": aspect_ratio,
                "references": references,
            }, "required": ["image", "prompt"]}),
        };
    }
    ToolSpec {
        name: IMAGE_GENERATE.to_owned(),
        description: "Make an image from a description. One image a call: for several, call \
             it several times. Returns the saved file, blob:<sha256>, and shows you the image."
            .to_owned(),
        parameters: json!({"type": "object", "properties": {
            "prompt": prompt,
            "aspect_ratio": aspect_ratio,
            "references": references,
        }, "required": ["prompt"]}),
    }
}

/// A call of a media tool in a line: `image_generate: a red fox at dawn`.
pub(crate) fn describe(name: &str, arguments: &Arguments) -> String {
    let prompt = match arguments {
        Arguments::Json(map) => map.get("prompt").and_then(|prompt| prompt.as_str()),
        Arguments::Malformed(_) => None,
    };
    let Some(prompt) = prompt else {
        return glave_ports::describe_call(name, arguments);
    };
    let line: String = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut shown: String = line.chars().take(SHOWN_CHARS).collect();
    if line.chars().count() > SHOWN_CHARS {
        shown.push('…');
    }
    format!("{}: {shown}", name.trim().to_ascii_lowercase())
}

/// A media call's arguments, as the model gave them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Given {
    prompt: String,
    #[serde(default)]
    aspect_ratio: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    references: Vec<String>,
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
    let given: Given = serde_json::from_value(serde_json::Value::Object(map.clone()))
        .map_err(|error| format!("The arguments don't fit the tool: {error}."))?;
    if given.prompt.trim().is_empty() {
        return Err("Say what to make in prompt.".to_owned());
    }
    if let Some(ratio) = &given.aspect_ratio
        && !ASPECT_RATIOS.contains(&ratio.as_str())
    {
        return Err(format!(
            "aspect_ratio {ratio:?} is not one of {}.",
            ASPECT_RATIOS.join(", ")
        ));
    }
    if given.references.len() > MOST_REFERENCES {
        return Err(format!("At most {MOST_REFERENCES} references a call."));
    }
    match (purpose, &given.image) {
        (Purpose::ImageEdit, None) => return Err("Name the image to change in image.".to_owned()),
        (Purpose::ImageEdit, Some(_)) | (_, None) => {}
        (_, Some(_)) => {
            return Err(
                "image_generate takes no image: use image_edit to change one, or name \
                 it in references to draw on it."
                    .to_owned(),
            );
        }
    }
    Ok(Asked {
        prompt: given.prompt,
        images: given.image.into_iter().chain(given.references).collect(),
        ask: MediaAsk {
            aspect_ratio: given.aspect_ratio,
        },
    })
}

#[cfg(test)]
mod tests;
