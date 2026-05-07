//! Blocking-task adapter for synchronous HTTP storage work.

use std::io;

use crate::{ServerError, ServerResult, http::ApiError};

pub(crate) async fn run_blocking<R>(
    operation: &'static str,
    task: impl FnOnce() -> ServerResult<R> + Send + 'static,
) -> Result<R, ApiError>
where
    R: Send + 'static,
{
    let span = tracing::Span::current();
    let result = tokio::task::spawn_blocking(move || {
        let _entered = span.enter();
        task()
    })
    .await;

    match result {
        Ok(result) => result.map_err(ApiError::from),
        Err(error) => Err(ApiError::from(join_error(operation, &error))),
    }
}

fn join_error(operation: &'static str, error: &tokio::task::JoinError) -> ServerError {
    let reason = if error.is_panic() {
        "panicked"
    } else if error.is_cancelled() {
        "was cancelled"
    } else {
        "failed"
    };

    ServerError::Internal(Box::new(io::Error::other(format!(
        "{operation} blocking task {reason}"
    ))))
}
