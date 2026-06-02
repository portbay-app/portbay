//! REST client for the Process Compose admin API.
//!
//! Wraps every endpoint we need into typed methods. The wire format is
//! documented at https://f1bonacc1.github.io/process-compose/ and was
//! exercised end-to-end in `claudedocs/spike-process-compose.md`.

use std::time::Duration;

use crate::process_compose::error::{PcError, Result};
use crate::process_compose::types::{LogsResponse, Process, ProcessesEnvelope};

/// Thin async HTTP client. Cheap to clone — `reqwest::Client` is internally
/// reference-counted.
#[derive(Clone, Debug)]
pub struct PcClient {
    base_url: String,
    http: reqwest::Client,
}

impl PcClient {
    /// Build a client pointing at the daemon at `localhost:<port>`.
    pub fn new(port: u16) -> Self {
        Self::with_base_url(format!("http://localhost:{port}"))
    }

    /// Build a client pointing at an arbitrary base URL — useful for tests
    /// that spawn PC on a randomised port.
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        // 3-second connect timeout, 10-second request timeout. PC is local
        // and answers in milliseconds; if it hangs longer than this, the
        // daemon is wedged and the UI should surface "unreachable" rather
        // than spin forever.
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest builder failed — unreachable");
        Self {
            base_url: base_url.into(),
            http,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// `GET /live` — daemon liveness probe.
    pub async fn live(&self) -> Result<bool> {
        let url = self.url("/live");
        let res = self.http.get(&url).send().await;
        match res {
            Ok(r) => Ok(r.status().is_success()),
            Err(e) if e.is_connect() || e.is_timeout() => Ok(false),
            Err(e) => Err(PcError::Unreachable { url, source: e }),
        }
    }

    /// `GET /processes` — the list view.
    pub async fn processes(&self) -> Result<Vec<Process>> {
        let url = self.url("/processes");
        let res = self.send_get(&url).await?;
        let env: ProcessesEnvelope = res.json().await.map_err(PcError::BodyDecode)?;
        Ok(env.data)
    }

    /// `GET /process/{name}` — single-process detail.
    pub async fn process(&self, name: &str) -> Result<Process> {
        let url = self.url(&format!("/process/{name}"));
        let res = self.send_get(&url).await?;
        let p: Process = res.json().await.map_err(PcError::BodyDecode)?;
        Ok(p)
    }

    /// `POST /process/start/{name}` — start a single process.
    pub async fn start(&self, name: &str) -> Result<()> {
        let url = self.url(&format!("/process/start/{name}"));
        self.send_mutating(reqwest::Method::POST, &url).await?;
        Ok(())
    }

    /// `PATCH /process/stop/{name}` — stop a single process.
    pub async fn stop(&self, name: &str) -> Result<()> {
        let url = self.url(&format!("/process/stop/{name}"));
        self.send_mutating(reqwest::Method::PATCH, &url).await?;
        Ok(())
    }

    /// `POST /process/restart/{name}` — restart a single process.
    pub async fn restart(&self, name: &str) -> Result<()> {
        let url = self.url(&format!("/process/restart/{name}"));
        self.send_mutating(reqwest::Method::POST, &url).await?;
        Ok(())
    }

    /// `PATCH /processes/stop` with a JSON array body — stop many at once.
    ///
    /// Returns a map of `name -> result string`. PC returns "ok" for each
    /// process it stopped, and a free-form message for those it couldn't
    /// (e.g. "process X is not running"). Caller decides whether to treat
    /// each as success or failure.
    pub async fn stop_many(&self, names: &[&str]) -> Result<serde_json::Value> {
        let url = self.url("/processes/stop");
        let res = self
            .http
            .patch(&url)
            .json(names)
            .send()
            .await
            .map_err(|e| PcError::Unreachable {
                url: url.clone(),
                source: e,
            })?;
        let res = check_status(res).await?;
        res.json::<serde_json::Value>()
            .await
            .map_err(PcError::BodyDecode)
    }

    /// `GET /process/logs/{name}/{offset}/{limit}` — static log tail.
    pub async fn logs(&self, name: &str, offset: u64, limit: u32) -> Result<Vec<String>> {
        let url = self.url(&format!("/process/logs/{name}/{offset}/{limit}"));
        let res = self.send_get(&url).await?;
        let body: LogsResponse = res.json().await.map_err(PcError::BodyDecode)?;
        Ok(body.logs)
    }

    /// `POST /project/stop` — shut the entire daemon down.
    pub async fn shutdown(&self) -> Result<()> {
        let url = self.url("/project/stop");
        self.send_mutating(reqwest::Method::POST, &url).await?;
        Ok(())
    }

    // -- internals -----------------------------------------------------------

    async fn send_get(&self, url: &str) -> Result<reqwest::Response> {
        let res = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| PcError::Unreachable {
                url: url.to_owned(),
                source: e,
            })?;
        check_status(res).await
    }

    async fn send_mutating(&self, method: reqwest::Method, url: &str) -> Result<reqwest::Response> {
        let res =
            self.http
                .request(method, url)
                .send()
                .await
                .map_err(|e| PcError::Unreachable {
                    url: url.to_owned(),
                    source: e,
                })?;
        check_status(res).await
    }
}

/// Pass a 2xx response through untouched; on a 4xx/5xx, consume the body and
/// surface Process Compose's own error message. PC replies to a failed
/// `/process/start/{name}` with `{"error":"no such process: X"}` or
/// `{"error":"process X is already running"}` — reading that turns the old
/// opaque "HTTP 400 (body unread)" into something the user can act on. Taking
/// ownership of the `Response` is what lets us read the body and still hand it
/// back to the caller on success.
async fn check_status(res: reqwest::Response) -> Result<reqwest::Response> {
    let status = res.status();
    if status.is_success() {
        return Ok(res);
    }
    let raw = res.text().await.unwrap_or_default();
    let body = pc_error_message(&raw).unwrap_or_else(|| {
        if raw.trim().is_empty() {
            format!("HTTP {status}")
        } else {
            raw.trim().to_string()
        }
    });
    Err(PcError::HttpStatus {
        status: status.as_u16(),
        body,
    })
}

/// Pull the human message out of Process Compose's `{"error":"…"}` envelope.
/// Returns `None` when the body isn't that shape so the caller can fall back to
/// the raw text.
fn pc_error_message(raw: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()?
        .get("error")?
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests are `#[ignore]` so `cargo test` stays fast in CI and on
    /// fresh checkouts. To run them locally, start a process-compose
    /// instance on :9999 (e.g. the spike's `process-compose up --port 9999
    /// --keep-project`) and run `cargo test -- --ignored`.
    #[ignore]
    #[tokio::test]
    async fn live_returns_true_against_running_daemon() {
        let c = PcClient::new(9999);
        assert!(c.live().await.unwrap());
    }

    #[ignore]
    #[tokio::test]
    async fn processes_returns_at_least_zero() {
        let c = PcClient::new(9999);
        // Just confirms the round-trip parses; concrete content depends on
        // what the user has loaded. The `.unwrap()` is the assertion.
        let _list = c.processes().await.unwrap();
    }

    #[test]
    fn client_builds_with_custom_base_url() {
        let c = PcClient::with_base_url("http://127.0.0.1:9999");
        assert!(c.url("/live").ends_with("/live"));
    }

    #[test]
    fn pc_error_message_extracts_the_error_field() {
        // The two real 400 bodies PC returns from /process/start/{name}.
        assert_eq!(
            pc_error_message(r#"{"error":"no such process: web"}"#).as_deref(),
            Some("no such process: web")
        );
        assert_eq!(
            pc_error_message(r#"{"error":"process web is already running"}"#).as_deref(),
            Some("process web is already running")
        );
    }

    #[test]
    fn pc_error_message_is_none_for_non_envelope_bodies() {
        // Plain text or unrelated JSON → caller falls back to the raw text.
        assert_eq!(pc_error_message("Bad Request"), None);
        assert_eq!(pc_error_message(r#"{"name":"web"}"#), None);
        assert_eq!(pc_error_message(""), None);
    }
}
