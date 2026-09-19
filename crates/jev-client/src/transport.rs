//! The seam between callers and the network.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use crate::error::Error;
use crate::models::ModelList;
use crate::request::Request;
use crate::response::Response;

/// Something that can carry requests to the TypeSafe API.
///
/// Code that needs the API depends on this trait rather than on
/// [`HttpTransport`](crate::HttpTransport), so it can be tested with an in-memory fake that
/// returns canned [`Reply`] values and never touches a socket.
///
/// The methods return `Send` futures, so a transport can be shared across the tasks of a
/// multi-threaded runtime. An implementation may simply write `async fn`. `&T` and `Arc<T>` are
/// transports whenever `T` is, which is how concurrent work shares one connection pool.
pub trait Transport: Send + Sync {
    /// Evaluates a request: `POST /v1/systemone`.
    fn evaluate(
        &self,
        request: &Request,
    ) -> impl Future<Output = Result<Reply<Response>, Error>> + Send;

    /// Lists the model names the account can use: `GET /v1/models`.
    fn list_models(&self) -> impl Future<Output = Result<Reply<ModelList>, Error>> + Send;
}

impl<T: Transport + ?Sized> Transport for &T {
    fn evaluate(
        &self,
        request: &Request,
    ) -> impl Future<Output = Result<Reply<Response>, Error>> + Send {
        (**self).evaluate(request)
    }

    fn list_models(&self) -> impl Future<Output = Result<Reply<ModelList>, Error>> + Send {
        (**self).list_models()
    }
}

impl<T: Transport + ?Sized> Transport for Arc<T> {
    fn evaluate(
        &self,
        request: &Request,
    ) -> impl Future<Output = Result<Reply<Response>, Error>> + Send {
        (**self).evaluate(request)
    }

    fn list_models(&self) -> impl Future<Output = Result<Reply<ModelList>, Error>> + Send {
        (**self).list_models()
    }
}

/// A successful answer from the API: the parsed body, and facts about the exchange.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Reply<T> {
    /// The parsed response body.
    pub body: T,
    /// Facts about the exchange that are not part of the body.
    pub meta: ReplyMeta,
    /// The response body exactly as the server sent it, when the transport kept it.
    ///
    /// `body` is the typed view, which drops fields this crate does not know. A caller that must
    /// pass the API's answer on untouched reads it from here. [`HttpTransport`](crate::HttpTransport)
    /// always fills it in; a fake transport usually does not.
    pub raw_body: Option<String>,
}

impl<T> Reply<T> {
    /// A reply, as a fake transport or a test would build one. It has no raw body.
    #[must_use]
    pub const fn new(body: T, meta: ReplyMeta) -> Self {
        Self {
            body,
            meta,
            raw_body: None,
        }
    }

    /// Attaches the response body as it was received.
    #[must_use]
    pub fn with_raw_body(mut self, raw_body: impl Into<String>) -> Self {
        self.raw_body = Some(raw_body.into());
        self
    }
}

/// Facts about one exchange with the API.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReplyMeta {
    /// The `x-typesafe-request-id` response header: the id to quote when asking TypeSafe about a
    /// request. `None` when the server did not send one.
    pub request_id: Option<String>,
    /// Wall-clock time from the first attempt to the parsed response, including any retries and
    /// the waits between them.
    pub latency: Duration,
    /// How many attempts were made, including the first. `1` means no retries.
    pub attempts: u32,
}

impl ReplyMeta {
    /// Metadata with the given values.
    #[must_use]
    pub const fn new(request_id: Option<String>, latency: Duration, attempts: u32) -> Self {
        Self {
            request_id,
            latency,
            attempts,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::{Future, ready};
    use std::sync::Arc;
    use std::time::Duration;

    use indexmap::IndexMap;

    use super::{Reply, ReplyMeta, Transport};
    use crate::{Error, ModelCard, ModelList, Request, Response, Usage};

    /// The kind of in-memory fake the trait exists to allow.
    struct Canned;

    impl Transport for Canned {
        fn evaluate(
            &self,
            request: &Request,
        ) -> impl Future<Output = Result<Reply<Response>, Error>> + Send {
            let body = Response::new(request.model.clone(), IndexMap::new(), Usage::new(1, 0));
            let meta = ReplyMeta::new(Some("req_fake".into()), Duration::ZERO, 1);
            ready(Ok(Reply::new(body, meta)))
        }

        fn list_models(&self) -> impl Future<Output = Result<Reply<ModelList>, Error>> + Send {
            let models = ModelList::new(vec![ModelCard::new("jev-latest")]);
            ready(Ok(Reply::new(models, ReplyMeta::default())))
        }
    }

    async fn model_names(transport: impl Transport) -> Vec<String> {
        let reply = transport.list_models().await.unwrap();
        reply
            .body
            .models
            .into_iter()
            .map(|model| model.name)
            .collect()
    }

    /// Compiles only if the futures are `Send`, which a multi-threaded runtime requires.
    fn assert_send<T: Send>(value: T) -> T {
        value
    }

    #[tokio::test]
    async fn a_fake_a_reference_and_an_arc_are_all_transports() {
        let shared = Arc::new(Canned);

        assert_eq!(model_names(Canned).await, ["jev-latest"]);
        assert_eq!(model_names(&Canned).await, ["jev-latest"]);
        assert_eq!(model_names(Arc::clone(&shared)).await, ["jev-latest"]);

        let request = Request::new("s", "jev-1.13.0");
        let reply = assert_send(shared.evaluate(&request)).await.unwrap();
        assert_eq!(reply.body.model, "jev-1.13.0");
        assert_eq!(reply.meta.request_id.as_deref(), Some("req_fake"));
    }
}
