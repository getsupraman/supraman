//! The one HTTP agent every WorkState call uses: blocking HTTP/1.1, TLS through the OS (schannel,
//! the machine's own root store) for an `https://` server, no proxy, no redirects, statuses as values.

use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .proxy(None)
        .max_redirects(0)
        .timeout_global(Some(timeout))
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into()
}
