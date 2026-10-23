use std::time::Duration;

use anyhow::{Context, Result};

use crate::error::LookupError;

/// Reusable HTTP connection and request timeout shared by providers.
pub(crate) struct HttpTransport {
    agent: ureq::Agent,
}

impl HttpTransport {
    pub fn new(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(concat!("geohueo/", env!("CARGO_PKG_VERSION")))
            .build();

        Self {
            agent: config.into(),
        }
    }

    pub fn get(&self, url: &str) -> Result<ureq::http::Response<ureq::Body>> {
        self.agent
            .get(url)
            .call()
            .with_context(|| format!("request to {url} failed"))
    }

    /// Authenticated requests must not follow redirects or expose credentials to another host.
    pub fn authorized(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(concat!("geohueo/", env!("CARGO_PKG_VERSION")))
            .max_redirects(0)
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }

    pub fn get_authorized(
        &self,
        url: &str,
        authorization: &str,
    ) -> Result<ureq::http::Response<ureq::Body>> {
        self.agent
            .get(url)
            .header("Authorization", authorization)
            .call()
            .map_err(|error| LookupError::from_http_client(&error, "MaxMind request failed").into())
    }

    /// No redirects, status errors or URLs in diagnostics: configured query parameters may
    /// contain credentials, and a third-party error can include the entire request URL.
    pub fn configured(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(concat!("geohueo/", env!("CARGO_PKG_VERSION")))
            .max_redirects(0)
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }

    pub fn get_configured(
        &self,
        url: &url::Url,
        header: Option<(&str, &str)>,
    ) -> Result<ureq::http::Response<ureq::Body>> {
        let request = self.agent.get(url.as_str());
        let request = if let Some((name, value)) = header {
            request.header(name, value)
        } else {
            request
        };
        request.call().map_err(|error| {
            LookupError::from_http_client(&error, "REST/JSON request failed").into()
        })
    }

    #[cfg(test)]
    pub(crate) fn configured_for_test(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }

    #[cfg(test)]
    pub(crate) fn authorized_for_test(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            // Loopback tests must not depend on proxy environment variables.
            .proxy(None)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::geoiplookup::DEFAULT_TIMEOUT;

    #[test]
    fn configures_requested_http_timeout() {
        let timeout = Duration::from_secs(5);
        let transport = HttpTransport::new(timeout);
        assert_eq!(transport.agent.config().timeouts().global, Some(timeout));
        assert_eq!(
            HttpTransport::new(DEFAULT_TIMEOUT)
                .agent
                .config()
                .timeouts()
                .global,
            Some(DEFAULT_TIMEOUT)
        );
    }
}
