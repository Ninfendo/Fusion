//! REST requests: book snapshots and missed trades.

use std::future::Future;
use std::time::Duration;

/// How the live connection reads a REST address. The real one is
/// [`HttpFetch`]; tests give their own.
pub trait Fetch: Send + Sync + 'static {
    /// The body of a GET request, or why it failed.
    fn get(&self, url: String) -> impl Future<Output = Result<String, String>> + Send;
}

/// HTTPS GET requests.
#[derive(Debug, Clone)]
pub struct HttpFetch {
    client: reqwest::Client,
}

/// How long a REST request may take.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

impl HttpFetch {
    /// A client with [`REQUEST_TIMEOUT`].
    pub fn new() -> Result<Self, String> {
        crate::live::install_tls();
        reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map(|client| Self { client })
            .map_err(|err| err.to_string())
    }
}

impl Fetch for HttpFetch {
    fn get(&self, url: String) -> impl Future<Output = Result<String, String>> + Send {
        let client = self.client.clone();
        async move {
            let response = client.get(&url).send().await.map_err(|e| e.to_string())?;
            let status = response.status();
            let body = response.text().await.map_err(|e| e.to_string())?;
            if status.is_success() {
                Ok(body)
            } else {
                Err(format!("{status}: {body}"))
            }
        }
    }
}
