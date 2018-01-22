use std::net::{SocketAddr, ToSocketAddrs};
use std::io;
use std::str::FromStr;

use url::Url;

use smith_common::{Dsn, Scheme};

/// Indicates failures in the upstream error api.
#[derive(Fail, Debug)]
pub enum UpstreamError {
    /// Raised if the DNS lookup for an upstream host failed.
    #[fail(display="dns lookup failed")]
    LookupFailed(#[cause] io::Error),
    /// Raised if the DNS lookup succeeded but an empty result was
    /// returned.
    #[fail(display="dns lookup returned no results")]
    EmptyLookupResult,
}

/// Raised if a URL cannot be parsed into an upstream descriptor.
#[derive(Fail, Debug, PartialEq, Eq, Hash)]
pub enum UpstreamParseError {
    /// Raised if an upstream could not be parsed as URL.
    #[fail(display="invalid upstream URL: bad URL format")]
    BadUrl,
    /// Raised if a path was added to a URL.
    #[fail(display="invalid upstream URL: non root URL given")]
    NonOriginUrl,
    /// Raised if an unknown or unsupported scheme is encountered.
    #[fail(display="invalid upstream URL: unknown or unsupported URL scheme")]
    UnknownScheme,
    /// Raised if no host was provided.
    #[fail(display="invalid upstream URL: no host")]
    NoHost,
}

/// The upstream target is a type that holds all the information
/// to uniquely identify an upstream target.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UpstreamDescriptor {
    host: String,
    port: Option<u16>,
    scheme: Scheme,
}

impl UpstreamDescriptor {
    /// Manually constructs an upstream descriptor.
    pub fn new(host: &str, port: u16, scheme: Scheme) -> UpstreamDescriptor {
        UpstreamDescriptor {
            host: host.to_string(),
            port: Some(port),
            scheme: scheme,
        }
    }

    /// Given a DSN this returns an upstream descriptor that
    /// describes it.
    pub fn from_dsn(dsn: &Dsn) -> UpstreamDescriptor {
        UpstreamDescriptor {
            host: dsn.host().to_string(),
            port: dsn.port(),
            scheme: dsn.scheme(),
        }
    }

    /// Returns the host as a string.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns the upstream port
    pub fn port(&self) -> u16 {
        self.port.unwrap_or_else(|| self.scheme().default_port())
    }

    /// Returns the socket address of the upstream.
    ///
    /// This might perform a DSN lookup and could fail.  Callers are
    /// encouraged this call this regularly as DNS might be used for
    /// load balancing purposes and results might expire.
    pub fn socket_addr(self) -> Result<SocketAddr, UpstreamError> {
        (self.host(), self.port())
            .to_socket_addrs()
            .map_err(UpstreamError::LookupFailed)?
            .next().ok_or(UpstreamError::EmptyLookupResult)
    }

    /// Returns the upstream's connection scheme.
    pub fn scheme(&self) -> Scheme {
        self.scheme
    }
}

impl FromStr for UpstreamDescriptor {
    type Err = UpstreamParseError;

    fn from_str(s: &str) -> Result<UpstreamDescriptor, UpstreamParseError> {
        let url = Url::parse(s).map_err(|_| UpstreamParseError::BadUrl)?;
        if url.path() != "/" || !(url.query() == None || url.query() == Some("")) {
            return Err(UpstreamParseError::NonOriginUrl);
        }

        let scheme = match url.scheme() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            _ => return Err(UpstreamParseError::UnknownScheme)
        };

        Ok(UpstreamDescriptor {
            host: match url.host_str() {
                Some(host) => host.to_string(),
                None => return Err(UpstreamParseError::NoHost)
            },
            port: url.port(),
            scheme: scheme,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use smith_common::Dsn;

    #[test]
    fn test_basic_parsing() {
        let desc: UpstreamDescriptor = "https://ingest.sentry.io/".parse().unwrap();
        assert_eq!(desc.host(), "ingest.sentry.io");
        assert_eq!(desc.port(), 443);
        assert_eq!(desc.scheme(), Scheme::Https);
    }

    #[test]
    fn test_from_dsn() {
        let dsn: Dsn = "https://username:password@domain:8888/path".parse().unwrap();
        let desc = UpstreamDescriptor::from_dsn(&dsn);
        assert_eq!(desc.host(), "domain");
        assert_eq!(desc.port(), 8888);
        assert_eq!(desc.scheme(), Scheme::Https);
    }
}