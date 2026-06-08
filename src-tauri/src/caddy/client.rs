//! REST client for Caddy's admin API.
//!
//! Wraps the operations the spike exercised in
//! `claudedocs/spike-caddy.md`. The `@id`-based methods are the killer
//! feature — single REST call to add or delete a route at runtime, no full
//! reload.

use std::time::Duration;
use tokio::time::sleep;

use serde::Serialize;

use crate::caddy::error::{CaddyError, Result};
use crate::caddy::types::{CaddyConfig, Route};

#[derive(Clone, Debug)]
pub struct CaddyClient {
    base_url: String,
    http: reqwest::Client,
}

impl CaddyClient {
    pub fn new(admin_port: u16) -> Self {
        Self::with_base_url(format!("http://localhost:{admin_port}"))
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Self {
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

    /// `POST /load` — replace the entire running config.
    pub async fn load(&self, config: &CaddyConfig) -> Result<()> {
        self.load_with_retries(config, 3).await
    }

    async fn load_with_retries(&self, config: &CaddyConfig, attempts: usize) -> Result<()> {
        let url = self.url("/load");
        let attempts = attempts.max(1);
        let mut last_err = None;
        for attempt in 0..attempts {
            let result = async {
                let res = self
                    .http
                    .post(&url)
                    .json(config)
                    .send()
                    .await
                    .map_err(|e| CaddyError::Unreachable {
                        url: url.clone(),
                        source: e,
                    })?;
                ensure_success(&url, res).await?;
                Ok(())
            }
            .await;
            match result {
                Ok(()) => return Ok(()),
                Err(err) if attempt + 1 < attempts => {
                    last_err = Some(err);
                    sleep(Duration::from_millis(150 * (attempt as u64 + 1))).await;
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_err.expect("at least one load attempt ran"))
    }

    /// `GET /config/<path>` — fetch any subtree of the running config.
    /// `path` should start with `/` (e.g. `/apps/http/servers/portbay/routes`).
    pub async fn get_config(&self, path: &str) -> Result<serde_json::Value> {
        let url = self.url(&format!("/config{path}"));
        let res = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| CaddyError::Unreachable {
                url: url.clone(),
                source: e,
            })?;
        let res = ensure_success(&url, res).await?;
        res.json::<serde_json::Value>()
            .await
            .map_err(CaddyError::BodyDecode)
    }

    /// `POST /config/apps/http/servers/<server>/routes/0` — prepend a route.
    ///
    /// Inserting at index 0 means newer routes win when host matchers
    /// overlap. PortBay's design treats each project's `route_<id>` as
    /// unique, so position is mostly for cosmetic ordering.
    pub async fn prepend_route(&self, server: &str, route: &Route) -> Result<()> {
        let url = self.url(&format!("/config/apps/http/servers/{server}/routes/0"));
        let res =
            self.http
                .post(&url)
                .json(route)
                .send()
                .await
                .map_err(|e| CaddyError::Unreachable {
                    url: url.clone(),
                    source: e,
                })?;
        ensure_success(&url, res).await?;
        Ok(())
    }

    /// `DELETE /id/<route_id>` — remove a route by its `@id`. One call,
    /// no full reload. The spike's killer feature.
    pub async fn delete_route(&self, id: &str) -> Result<()> {
        let url = self.url(&format!("/id/{id}"));
        let res = self
            .http
            .delete(&url)
            .send()
            .await
            .map_err(|e| CaddyError::Unreachable {
                url: url.clone(),
                source: e,
            })?;
        ensure_success(&url, res).await?;
        Ok(())
    }

    /// `PATCH /id/<route_id>` — replace a route's body, keeping the `@id`.
    pub async fn update_route<T: Serialize>(&self, id: &str, route: &T) -> Result<()> {
        let url = self.url(&format!("/id/{id}"));
        let res = self
            .http
            .patch(&url)
            .json(route)
            .send()
            .await
            .map_err(|e| CaddyError::Unreachable {
                url: url.clone(),
                source: e,
            })?;
        ensure_success(&url, res).await?;
        Ok(())
    }

    /// `POST /stop` — graceful shutdown of the daemon.
    pub async fn shutdown(&self) -> Result<()> {
        let url = self.url("/stop");
        // `/stop` closes the connection mid-request; ignore connection
        // errors here. Anything else propagates.
        match self.http.post(&url).send().await {
            Ok(_) => Ok(()),
            Err(e) if e.is_connect() || e.is_timeout() => Ok(()),
            Err(e) => Err(CaddyError::Unreachable { url, source: e }),
        }
    }

    /// Lightweight reachability check.
    pub async fn is_alive(&self) -> Result<bool> {
        let url = self.url("/config/");
        match self.http.get(&url).send().await {
            Ok(r) => Ok(r.status().is_success()),
            Err(e) if e.is_connect() || e.is_timeout() => Ok(false),
            Err(e) => Err(CaddyError::Unreachable { url, source: e }),
        }
    }
}

async fn ensure_success(url: &str, res: reqwest::Response) -> Result<reqwest::Response> {
    let status = res.status();
    if status.is_success() {
        return Ok(res);
    }
    let body = res.text().await.unwrap_or_default();
    Err(CaddyError::HttpStatus {
        status: status.as_u16(),
        body: if body.is_empty() {
            format!("(empty body, URL {url})")
        } else {
            body
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration tests — require a Caddy daemon on :2019. Run manually
    /// after starting one: `caddy run --config bootstrap.json --resume`.
    #[ignore]
    #[tokio::test]
    async fn is_alive_against_running_daemon() {
        let c = CaddyClient::new(2019);
        assert!(c.is_alive().await.unwrap());
    }

    #[test]
    fn client_builds_with_custom_base_url() {
        let c = CaddyClient::with_base_url("http://127.0.0.1:2020");
        assert!(c.url("/load").ends_with("/load"));
    }
}
