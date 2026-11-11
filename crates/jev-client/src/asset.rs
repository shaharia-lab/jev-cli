//! Images attached to a request: the entries of `assets`.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The image formats the API accepts, as the `format` field spells them.
pub const IMAGE_FORMATS: [&str; 3] = ["png", "jpg", "jpeg"];

/// One image a request carries, keyed in [`Request::assets`](crate::Request::assets) by a
/// reference string. The model sees the image wherever that reference appears in the `state`, in
/// a question's `instructions` or in its `criteria`.
///
/// The API is told that a request carries images with the header
/// `TypeSafe-Danger-Zone-Images: true`, which [`HttpTransport`](crate::HttpTransport) adds on its
/// own. Images are experimental on the API side, so the shape here can change between releases.
///
/// ```
/// use jev_client::Asset;
///
/// let image = Asset::image_base64("iVBORw0KGgoAAAANSUhEUgAAAZAAAAEsCAIAAABi1XKV", "png");
/// assert_eq!(
///     serde_json::to_string(&image).unwrap(),
///     r#"{"type":"image","data_base64":"iVBORw0KGgoAAAANSUhEUgAAAZAAAAEsCAIAAABi1XKV","format":"png"}"#
/// );
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(
    description = "One image, keyed by a reference string. Write that reference in the state, in a question's instructions or in its criteria, and the model sees the image there. `data_base64` is the file, base64 encoded, with `format` saying which kind."
)]
#[non_exhaustive]
pub struct Asset {
    /// The kind of asset. Only `image` exists; the API treats an absent `type` as `image`.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,

    /// The file, standard base64 with `=` padding, no line breaks and no `data:` prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,

    /// `png`, `jpg` or `jpeg`. Required with `data_base64`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,

    /// The name of a multipart file part holding the image. This crate sends JSON bodies only,
    /// so validation refuses it; it is kept so that a request is parsed as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,

    /// Fields this version of the crate does not know, sent to the API unchanged.
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: IndexMap<String, Value>,
}

impl Asset {
    /// An image given as base64, in `format` (`png`, `jpg` or `jpeg`).
    #[must_use]
    pub fn image_base64(data_base64: impl Into<String>, format: impl Into<String>) -> Self {
        Self {
            kind: Some("image".to_owned()),
            data_base64: Some(data_base64.into()),
            format: Some(format.into()),
            part: None,
            extra: IndexMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::Asset;

    #[test]
    fn unknown_fields_and_the_part_form_round_trip_as_written() {
        let written = json!({ "part": "photo", "format": "jpg", "width": 640 });
        let asset: Asset = serde_json::from_value(written.clone()).unwrap();
        assert_eq!(asset.part.as_deref(), Some("photo"));
        assert_eq!(asset.kind, None);
        assert_eq!(asset.extra.get("width"), Some(&json!(640)));
        assert_eq!(serde_json::to_value(&asset).unwrap(), written);
    }
}
