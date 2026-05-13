//! HTTP implementation of the sync transport contract.

use std::io;

use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde_json::Value;
use url::Url;
use zk_domain::{
    RegisterDeviceRequest, RegisterDeviceResponse, ResolveConflictRequest, ResolveConflictResponse,
    SyncPullRequest, SyncPullResponse, SyncPushRequest, SyncPushResponse, id::DeviceId,
};

use crate::{SyncError, SyncResult, bootstrap::RemoteBootstrapProbe, transport::SyncTransport};

const REGISTER_DEVICE_PATH: &str = "/devices/register";
const PUSH_PATH: &str = "/sync/push";
const PULL_PATH: &str = "/sync/pull";
const RESOLVE_CONFLICT_PATH: &str = "/sync/resolve-conflict";

/// HTTP-backed sync transport for the V1 sync API.
#[derive(Debug, Clone)]
pub struct HttpTransport {
    client: Client,
    base_url: Url,
    bearer_token: Option<String>,
}

/// Raw text HTTP response for higher-level facades with their own error mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpTextResponse {
    /// Numeric HTTP response status.
    pub status: u16,
    /// Response body decoded as text.
    pub body: String,
}

/// Raw streaming HTTP response for higher-level facades with their own protocol parser.
pub struct HttpStreamResponse {
    /// Numeric HTTP response status.
    pub status: u16,
    /// Response body bytes.
    pub body: BoxStream<'static, SyncResult<Vec<u8>>>,
}

impl HttpTransport {
    /// Creates an HTTP transport rooted at the server base URL.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Transport`] if the underlying HTTP client cannot
    /// be constructed.
    pub fn new(base_url: Url, bearer_token: Option<String>) -> SyncResult<Self> {
        let client = Client::builder()
            .build()
            .map_err(|error| SyncError::Transport(Box::new(error)))?;

        Ok(Self {
            client,
            base_url,
            bearer_token,
        })
    }

    /// Posts a JSON value to an arbitrary server path and returns the JSON response value.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Transport`] when URL construction, the request,
    /// status handling, or JSON response decoding fails.
    pub async fn post_json(&self, path: &str, body: &Value) -> SyncResult<Value> {
        let response = self
            .post(path)?
            .json(body)
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;
        let response = response_for_status(response, "POST", path).await?;

        response
            .json::<Value>()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))
    }

    /// Sends a GET request to an arbitrary server path and returns the JSON response value.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Transport`] when URL construction, the request,
    /// status handling, or JSON response decoding fails.
    pub async fn get_json(&self, path: &str) -> SyncResult<Value> {
        let response = self
            .request(Method::GET, path)?
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;
        let response = response_for_status(response, "GET", path).await?;

        response
            .json::<Value>()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))
    }

    /// Posts JSON to an arbitrary path and returns status plus raw text body.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Transport`] when URL construction, request send,
    /// or response body decoding fails.
    pub async fn post_json_text(&self, path: &str, body: &Value) -> SyncResult<HttpTextResponse> {
        let response = self
            .post(path)?
            .json(body)
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;

        text_response(response).await
    }

    /// Sends a GET request and returns status plus raw text body.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Transport`] when URL construction, request send,
    /// or response body decoding fails.
    pub async fn get_text(&self, path: &str) -> SyncResult<HttpTextResponse> {
        let response = self
            .request(Method::GET, path)?
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;

        text_response(response).await
    }

    /// Sends a PATCH request with JSON and returns status plus raw text body.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Transport`] when URL construction, request send,
    /// or response body decoding fails.
    pub async fn patch_json_text(&self, path: &str, body: &Value) -> SyncResult<HttpTextResponse> {
        let response = self
            .request(Method::PATCH, path)?
            .json(body)
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;

        text_response(response).await
    }

    /// Sends a DELETE request and returns status plus raw text body.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Transport`] when URL construction, request send,
    /// or response body decoding fails.
    pub async fn delete_text(&self, path: &str) -> SyncResult<HttpTextResponse> {
        let response = self
            .request(Method::DELETE, path)?
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;

        text_response(response).await
    }

    /// Posts JSON to an arbitrary path and returns status plus streaming body bytes.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Transport`] when URL construction or request send
    /// fails.
    pub async fn post_json_stream(
        &self,
        path: &str,
        body: &Value,
    ) -> SyncResult<HttpStreamResponse> {
        let response = self
            .post(path)?
            .json(body)
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;
        let status = response.status().as_u16();
        let body = response
            .bytes_stream()
            .map(|result| {
                result
                    .map(|bytes| bytes.to_vec())
                    .map_err(|error| SyncError::Transport(Box::new(error)))
            })
            .boxed();

        Ok(HttpStreamResponse { status, body })
    }

    /// Sends a GET request to an arbitrary server path and returns the status code.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Transport`] when URL construction or the request
    /// fails.
    pub async fn get(&self, path: &str) -> SyncResult<StatusCode> {
        let response = self
            .request(Method::GET, path)?
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;

        Ok(response.status())
    }

    fn post(&self, path: &str) -> SyncResult<RequestBuilder> {
        self.request(Method::POST, path)
    }

    fn request(&self, method: Method, path: &str) -> SyncResult<RequestBuilder> {
        let url = self
            .base_url
            .join(path)
            .map_err(|error| SyncError::Transport(Box::new(error)))?;
        let builder = self.client.request(method, url);

        Ok(match &self.bearer_token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        })
    }
}

async fn text_response(response: Response) -> SyncResult<HttpTextResponse> {
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|error| SyncError::Transport(Box::new(error)))?;

    Ok(HttpTextResponse { status, body })
}

#[async_trait]
impl SyncTransport for HttpTransport {
    async fn register_device(
        &self,
        request: &RegisterDeviceRequest,
    ) -> SyncResult<RegisterDeviceResponse> {
        let response = self
            .post(REGISTER_DEVICE_PATH)?
            .json(request)
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;
        let response = response_for_status(response, "POST", REGISTER_DEVICE_PATH).await?;

        response
            .json::<RegisterDeviceResponse>()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))
    }

    async fn push(&self, request: &SyncPushRequest) -> SyncResult<SyncPushResponse> {
        let response = self
            .post(PUSH_PATH)?
            .json(request)
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;
        let response = response_for_status(response, "POST", PUSH_PATH).await?;

        response
            .json::<SyncPushResponse>()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))
    }

    async fn pull(&self, request: &SyncPullRequest) -> SyncResult<SyncPullResponse> {
        let response = self
            .post(PULL_PATH)?
            .json(request)
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;
        let response = response_for_status(response, "POST", PULL_PATH).await?;

        response
            .json::<SyncPullResponse>()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))
    }

    async fn resolve_conflict(
        &self,
        request: &ResolveConflictRequest,
    ) -> SyncResult<ResolveConflictResponse> {
        let response = self
            .post(RESOLVE_CONFLICT_PATH)?
            .json(request)
            .send()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))?;
        let response = response_for_status(response, "POST", RESOLVE_CONFLICT_PATH).await?;

        response
            .json::<ResolveConflictResponse>()
            .await
            .map_err(|error| SyncError::Transport(Box::new(error)))
    }

    async fn probe_remote_counts(&self, device_id: &DeviceId) -> SyncResult<RemoteBootstrapProbe> {
        let request = SyncPullRequest {
            device_id: device_id.to_string(),
            cursor: None,
            batch_size: Some(1),
        };
        let response = self.pull(&request).await?;

        // The V1 wire API only needs Mode A/B detection here. Bootstrap's
        // `detect_initial_mode` consumes "empty vs non-empty", so the
        // transport returns a sentinel instead of doing a real count.
        let note_count = u32::from(!response.changes.is_empty() || response.next_cursor.is_some());

        Ok(RemoteBootstrapProbe {
            note_count,
            asset_count: 0,
        })
    }
}

async fn response_for_status(response: Response, method: &str, path: &str) -> SyncResult<Response> {
    let status = response.status();

    if status < StatusCode::BAD_REQUEST {
        return Ok(response);
    }

    let body = response
        .text()
        .await
        .unwrap_or_else(|error| error.to_string());
    let error = io::Error::other(format!("{method} {path} -> {status}: {body}"));

    Err(SyncError::Transport(Box::new(error)))
}
