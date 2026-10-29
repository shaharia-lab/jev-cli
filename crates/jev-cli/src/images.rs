//! `--image NAME=PATH`: a PNG or JPEG file attached to the request as an asset, and named in the
//! state so that a question can refer to it.
//!
//! The API reads an image wherever its reference string appears. `jev` writes the reference as
//! the value of `NAME` in an object state, so the question says `` `NAME` `` the way it names any
//! other part of the state, and the file is sent once however many questions use it.

use std::fs;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use jev_client::ImageInfo;
use jev_client::validate::Document;
use serde_json::{Map, Value, json};

use crate::error::CliError;

/// Attaches every `--image` to the request: the file goes under `assets`, and its reference under
/// `NAME` in the state, which must be an object (the one given outside the file, else the file's,
/// else a new one holding only the images).
///
/// # Errors
///
/// A usage error when a flag is malformed, a file cannot be read, is not a PNG or JPEG or is
/// outside the API's size limits, a name is already used in the state, or the state is text and
/// so has no place for a name.
pub(crate) fn attach(
    flags: &[String],
    document: &mut Document,
    state: &mut Option<Value>,
) -> Result<(), CliError> {
    if flags.is_empty() {
        return Ok(());
    }
    let images = flags
        .iter()
        .map(|flag| Image::read(flag))
        .collect::<Result<Vec<_>, _>>()?;

    let Value::Object(root) = document.value_mut() else {
        // Not a request at all; validation reports it in better terms than this could.
        return Ok(());
    };
    // Taken out while the state is borrowed, and put back below.
    let Value::Object(mut assets) = root
        .remove("assets")
        .unwrap_or_else(|| Value::Object(Map::new()))
    else {
        return Err(CliError::usage(
            "the request file's `assets` is not an object, so --image has nowhere to put the file",
        )
        .hint("make `assets` an object keyed by reference, or remove it"));
    };

    let fields = match (state.as_mut(), root.get_mut("state")) {
        (Some(Value::Object(fields)), _) | (None, Some(Value::Object(fields))) => fields,
        (Some(_), _) => {
            return Err(CliError::usage(
                "--image needs an object state to name the image in, but the state is text",
            )
            .hint("give the state as JSON (--state-format json), or leave it out so that the state holds only the images"));
        }
        (None, Some(_)) => {
            return Err(CliError::usage(
                "--image needs an object state to name the image in, but the request file's `state` is not an object",
            )
            .hint("make the file's `state` an object, or give an object state with --state-file"));
        }
        (None, None) => {
            *state = Some(Value::Object(Map::new()));
            match state.as_mut() {
                Some(Value::Object(fields)) => fields,
                _ => unreachable!("the state was just made an object"),
            }
        }
    };

    for image in images {
        if fields.contains_key(&image.name) {
            return Err(CliError::usage(format!(
                "--image {}: the state already has a `{}`",
                image.name, image.name
            ))
            .hint("pick a name that is not a key of the state"));
        }
        if assets.contains_key(&image.reference) {
            return Err(CliError::usage(format!(
                "--image {}: an asset `{}` is already in the request",
                image.name, image.reference
            ))
            .hint("every --image needs its own name"));
        }
        fields.insert(image.name.clone(), Value::String(image.reference.clone()));
        assets.insert(
            image.reference,
            json!({ "type": "image", "data_base64": image.data_base64, "format": image.format }),
        );
    }
    root.insert("assets".to_owned(), Value::Object(assets));
    Ok(())
}

/// One `--image`, read and encoded.
struct Image {
    name: String,
    /// The string the API replaces with the image. Fixed and readable, so a dry run shows
    /// where the image goes; unlikely to occur in real text.
    reference: String,
    data_base64: String,
    format: &'static str,
}

impl Image {
    fn read(flag: &str) -> Result<Self, CliError> {
        let invalid = |problem: &str| {
            CliError::usage(format!("--image `{flag}` is not NAME=PATH: {problem}"))
                .hint("write --image photo=door.jpg, then ask about `photo`")
        };
        let (name, path) = flag
            .split_once('=')
            .ok_or_else(|| invalid("there is no `=`"))?;
        if name.trim().is_empty() {
            return Err(invalid("the name is empty"));
        }
        if name
            .chars()
            .any(|character| character == '`' || character.is_whitespace())
        {
            return Err(invalid("the name may not contain spaces or backticks"));
        }
        if path.is_empty() {
            return Err(invalid("the path is empty"));
        }
        let bytes = fs::read(path).map_err(|error| {
            CliError::usage(format!("cannot read the image {path}: {error}"))
                .hint("check the path passed to --image")
        })?;
        let info = ImageInfo::read(&bytes).ok_or_else(|| {
            CliError::usage(format!("{path} is not a PNG or JPEG file")).hint(
                "the API accepts still PNG and JPEG images only; convert GIF, WebP or HEIC first",
            )
        })?;
        if let Some(problem) = info.size_problem() {
            return Err(CliError::usage(format!("{path} is {problem}")).hint(
                "resize the image before sending it; the API rejects it rather than resizing",
            ));
        }
        // The API spells JPEG both ways; keep the file's own spelling.
        let format = match (info.format, Path::new(path).extension()) {
            ("jpg", Some(extension)) if extension.eq_ignore_ascii_case("jpeg") => "jpeg",
            (format, _) => format,
        };
        Ok(Self {
            name: name.to_owned(),
            reference: format!("jev-image:{name}"),
            data_base64: STANDARD.encode(&bytes),
            format,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use jev_client::validate::Document;
    use serde_json::{Value, json};

    use super::attach;

    /// A PNG header saying 400x300, which is all that is read of it.
    const PNG_HEADER: [u8; 24] = [
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0,
        1, 144, 0, 0, 1, 44,
    ];

    /// The same header, base64 encoded, as it is sent.
    const PNG_HEADER_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAZAAAAEs";

    fn scratch(name: &str, content: &[u8]) -> String {
        let path: PathBuf =
            std::env::temp_dir().join(format!("jev-images-{}-{name}", std::process::id()));
        fs::write(&path, content).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn png(name: &str) -> String {
        scratch(&format!("{name}.png"), &PNG_HEADER)
    }

    fn document() -> Document {
        Document::from_value(
            json!({ "questions": { "q": { "type": "box", "instructions": "the mug in `photo`" } } }),
        )
    }

    #[test]
    fn an_image_goes_under_assets_and_its_reference_under_the_name_in_the_state() {
        let flag = format!("photo={}", png("plain"));
        let mut document = document();
        let mut state = None;

        attach(&[flag], &mut document, &mut state).unwrap();

        assert_eq!(state, Some(json!({ "photo": "jev-image:photo" })));
        assert_eq!(
            document.value()["assets"],
            json!({ "jev-image:photo": { "type": "image", "data_base64": PNG_HEADER_BASE64, "format": "png" } })
        );
    }

    #[test]
    fn the_name_joins_an_object_state_given_outside_or_inside_the_file() {
        let flag = format!("photo={}", png("join"));
        let mut document = document();
        let mut state = Some(json!({ "caption": "a desk" }));
        attach(std::slice::from_ref(&flag), &mut document, &mut state).unwrap();
        assert_eq!(
            state,
            Some(json!({ "caption": "a desk", "photo": "jev-image:photo" }))
        );

        let mut document =
            Document::from_value(json!({ "state": { "caption": "a desk" }, "questions": {} }));
        let mut state = None;
        attach(&[flag], &mut document, &mut state).unwrap();
        assert_eq!(state, None);
        assert_eq!(
            document.value()["state"],
            json!({ "caption": "a desk", "photo": "jev-image:photo" })
        );
    }

    #[test]
    fn a_text_state_a_taken_name_a_bad_file_and_a_small_image_are_usage_errors() {
        let flag = format!("photo={}", png("errors"));
        let mut document = document();

        let mut text = Some(Value::String("a desk".into()));
        let error = attach(std::slice::from_ref(&flag), &mut document, &mut text).unwrap_err();
        assert!(
            error.message.contains("the state is text"),
            "{}",
            error.message
        );

        let mut taken = Some(json!({ "photo": "already here" }));
        let error = attach(std::slice::from_ref(&flag), &mut document, &mut taken).unwrap_err();
        assert!(
            error.message.contains("already has a `photo`"),
            "{}",
            error.message
        );

        let flag = format!("photo={}", scratch("text.png", b"hello"));
        let error = attach(&[flag], &mut document, &mut None).unwrap_err();
        assert!(
            error.message.contains("not a PNG or JPEG"),
            "{}",
            error.message
        );

        let error = attach(&["photo".to_owned()], &mut document, &mut None).unwrap_err();
        assert!(error.message.contains("not NAME=PATH"), "{}", error.message);

        let mut small = PNG_HEADER;
        small[18..20].copy_from_slice(&[0, 100]);
        small[22..24].copy_from_slice(&[0, 100]);
        let flag = format!("photo={}", scratch("small.png", &small));
        let error = attach(&[flag], &mut document, &mut None).unwrap_err();
        assert!(
            error.message.contains("each side must be"),
            "{}",
            error.message
        );
    }
}
