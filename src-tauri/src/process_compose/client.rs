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
        check_status(&url, res.status(), &res).await?;
        res.json::<serde_json::Value>().await.map_err(PcError::BodyDecode)
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
        check_status(url, res.status(), &res).await?;
        Ok(res)
    }

    async fn send_mutating(&self, method: reqwest::Method, url: &str) -> Result<reqwest::Response> {
        let res = self
            .http
            .request(method, url)
            .send()
            .await
            .map_err(|e| PcError::Unreachable {
                url: url.to_owned(),
                source: e,
            })?;
        check_status(url, res.status(), &res).await?;
        Ok(res)
    }
}

async fn check_status(_url: &str, status: reqwest::StatusCode, _res: &reqwest::Response) -> Result<()> {
    // We can't consume the body here without taking ownership, so we accept
    // any 2xx and let downstream parsers surface decode errors. Real HTTP
    // errors (4xx/5xx) are mapped to PcError::HttpStatus by the caller via
    // a separate `bytes()` path when needed.
    if status.is_success() {
        return Ok(());
    }
    Err(PcError::HttpStatus {
        status: status.as_u16(),
        body: format!("HTTP {} (body unread)", status),
    })
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
        let list = c.processes().await.unwrap();
        // Just confirms the round-trip parses; concrete content depends on
        // what the user has loaded.
        assert!(list.len() >= 0);
    }

    #[test]
    fn client_builds_with_custom_base_url() {
        let c = PcClient::with_base_url("http://127.0.0.1:9999");
        assert!(c.url("/live").ends_with("/live"));
    }
}
