use std::time::Duration;

use anyhow::{Context, Result};

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
