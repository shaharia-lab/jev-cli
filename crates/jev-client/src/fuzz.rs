//! Entry points for the fuzz targets in `fuzz/`, which call them with arbitrary bytes. Every input
//! must be handled without a panic (PRD REL-7), and whatever is produced is thrown away.
//!
//! Compiled only for `cargo fuzz` (which sets `--cfg fuzzing`) and for this crate's tests, which
//! run each entry point on a few inputs so that the targets keep compiling. Not an API.

use serde_json::Value;

use crate::error::Error;
use crate::validate::{self, Document, Options};
use crate::{ModelList, Request, Response};

/// A request file as JSON: parsed as a [`Document`], validated under every combination of the
/// options that change which rules apply, and, when it has the shape of a request, converted,
/// validated again as a typed request and written back out.
pub fn request(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(document) = Document::from_json_str(text) else {
        return;
    };
    for bits in 0..16_u8 {
        let options = Options::default()
            .strict(bits & 1 != 0)
            .skip_size_check(bits & 2 != 0)
            .model_optional(bits & 4 != 0)
            .state_optional(bits & 8 != 0);
        let _ = serde_json::to_string(&validate::check_document(&document, &options));
    }
    if let Ok(request) = document.into_request() {
        let _ = validate::check(&request, &Options::default());
        if let Ok(text) = serde_json::to_string(&request) {
            let _ = serde_json::from_str::<Request>(&text);
        }
    }
}

/// A response body. The first byte picks a status: below 128 the rest is read as an evaluation
/// and a model listing, as a success is; otherwise it is an error body for a status from 400 to
/// 599, as a failure is.
pub fn response(data: &[u8]) {
    let Some((&first, body)) = data.split_first() else {
        return;
    };
    if first < 128 {
        if let Ok(response) = serde_json::from_slice::<Response>(body) {
            let _ = serde_json::to_string(&response);
        }
        if let Ok(models) = serde_json::from_slice::<ModelList>(body) {
            let _ = serde_json::to_string(&models);
        }
        let _ = serde_json::from_slice::<Value>(body);
    } else {
        let status = 400 + u16::from(first - 128) * 199 / 127;
        let error = Error::from_response(status, body, str::to_owned);
        let _ = error.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::{request, response};

    #[test]
    fn the_request_entry_point_takes_anything() {
        let valid = br#"{"state": "s", "model": "jev-latest", "questions": {"q": {"type": "noul", "instructions": "Is it?"}}}"#;
        for input in [
            &valid[..],
            br#"{"questions": {"q": {"type": "score", "criteria": ["a"]}, "q": 1}}"#,
            b"[]",
            b"\xff",
            b"",
        ] {
            request(input);
        }
    }

    #[test]
    fn the_response_entry_point_takes_anything() {
        for input in [
            &br#"{"model": "jev-1.13.0", "answers": {"q": {"type": "noul", "noul": 0.5}}}"#[..],
            br#"{"detail": [{"loc": ["body", 1], "msg": "bad"}]}"#,
            br#"{"detail": "overloaded"}"#,
            b"",
        ] {
            response(input);
            let mut error = vec![200_u8];
            error.extend_from_slice(input);
            response(&error);
        }
    }
}
