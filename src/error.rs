use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("config error: {0}")]
    Config(String),

    #[error("interface '{interface}' not found")]
    InterfaceNotFound { interface: String },

    #[error("no {addr_type} address on interface '{interface}'")]
    NoAddress {
        interface: String,
        addr_type: &'static str,
    },

    #[error("web resolve failed: {0}")]
    WebResolve(#[from] reqwest::Error),

    #[error("invalid IP from web service: '{0}'")]
    InvalidWebIp(String),

    #[error("route53 error: {0}")]
    Route53(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config parse error: {0}")]
    Toml(#[from] toml::de::Error),
}

impl Error {
    /// Returns true for transient errors that are worth retrying.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Error::WebResolve(_) | Error::Route53(_))
    }
}
