use std::net::IpAddr;

/// Check if a hostname/IP is a private or reserved address.
///
/// Covers: RFC 1918, CGNAT (100.64/10), loopback, link-local,
/// IPv6 ULA/link-local, IPv4-mapped IPv6.
pub fn is_private_ip(host: &str) -> bool {
    let addr: IpAddr = match host.parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    is_private_addr(addr)
}

fn is_private_addr(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            // 0.0.0.0/8
            if octets[0] == 0 {
                return true;
            }
            // 10.0.0.0/8
            if octets[0] == 10 {
                return true;
            }
            // 172.16.0.0/12
            if octets[0] == 172 && (16..=31).contains(&octets[1]) {
                return true;
            }
            // 192.168.0.0/16
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }
            // 127.0.0.0/8 (loopback)
            if octets[0] == 127 {
                return true;
            }
            // 169.254.0.0/16 (link-local)
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }
            // 100.64.0.0/10 (CGNAT, RFC 6598)
            if octets[0] == 100 && (64..=127).contains(&octets[1]) {
                return true;
            }
            false
        }
        IpAddr::V6(v6) => {
            // ::1 (loopback)
            if v6.is_loopback() {
                return true;
            }
            let segments = v6.segments();
            // fc00::/7 (ULA)
            if segments[0] & 0xfe00 == 0xfc00 {
                return true;
            }
            // fe80::/10 (link-local)
            if segments[0] & 0xffc0 == 0xfe80 {
                return true;
            }
            // IPv4-mapped IPv6 (::ffff:x.x.x.x)
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_addr(IpAddr::V4(v4));
            }
            false
        }
    }
}

/// Validate and normalize the ingestion endpoint URL.
///
/// Returns the validated endpoint string, or an error for:
///   - Non-HTTPS URLs (except localhost)
///   - Private/reserved IP addresses (SSRF protection)
///   - Embedded credentials in URL
///   - Malformed URLs
pub fn validate_endpoint(endpoint: &str) -> Result<String, String> {
    if endpoint.is_empty() {
        return Err("[apidash] 'endpoint' is required".to_string());
    }

    let url = url_parse(endpoint)?;

    let is_localhost = url.host == "localhost" || url.host == "127.0.0.1" || url.host == "::1";

    if url.scheme != "https" && !is_localhost {
        return Err(format!(
            "[apidash] Endpoint must use HTTPS. Plain HTTP is only allowed for localhost: {endpoint}"
        ));
    }

    if url.has_credentials {
        return Err("[apidash] Endpoint URL must not contain credentials".to_string());
    }

    if !is_localhost && is_private_ip(&url.host) {
        return Err(format!(
            "[apidash] Endpoint must not point to a private or internal IP address: {}",
            url.host
        ));
    }

    Ok(endpoint.to_string())
}

struct ParsedUrl {
    scheme: String,
    host: String,
    has_credentials: bool,
}

fn url_parse(endpoint: &str) -> Result<ParsedUrl, String> {
    // Minimal URL parsing — avoids pulling in the `url` crate
    let (scheme, rest) = endpoint
        .split_once("://")
        .ok_or_else(|| format!("[apidash] Invalid endpoint URL: {endpoint}"))?;

    let authority = rest.split('/').next().unwrap_or(rest);
    let has_credentials = authority.contains('@');

    // Strip credentials for host extraction
    let host_port = if has_credentials {
        authority.rsplit_once('@').map_or(authority, |(_, hp)| hp)
    } else {
        authority
    };

    // Strip port
    let host = if host_port.starts_with('[') {
        // IPv6: [::1]:8080
        host_port
            .split(']')
            .next()
            .unwrap_or(host_port)
            .trim_start_matches('[')
    } else {
        host_port.split(':').next().unwrap_or(host_port)
    };

    if host.is_empty() {
        return Err(format!("[apidash] Invalid endpoint URL: {endpoint}"));
    }

    Ok(ParsedUrl {
        scheme: scheme.to_lowercase(),
        host: host.to_lowercase(),
        has_credentials,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_ipv4_ranges() {
        assert!(is_private_ip("10.0.0.1"));
        assert!(is_private_ip("10.255.255.255"));
        assert!(is_private_ip("172.16.0.1"));
        assert!(is_private_ip("172.31.255.255"));
        assert!(is_private_ip("192.168.0.1"));
        assert!(is_private_ip("192.168.255.255"));
        assert!(is_private_ip("127.0.0.1"));
        assert!(is_private_ip("169.254.1.1"));
        assert!(is_private_ip("100.64.0.1"));
        assert!(is_private_ip("100.127.255.255"));
        assert!(is_private_ip("0.0.0.0"));
    }

    #[test]
    fn public_ipv4() {
        assert!(!is_private_ip("8.8.8.8"));
        assert!(!is_private_ip("1.1.1.1"));
        assert!(!is_private_ip("203.0.113.1"));
    }

    #[test]
    fn private_ipv6() {
        assert!(is_private_ip("::1"));
        assert!(is_private_ip("fc00::1"));
        assert!(is_private_ip("fd12:3456::1"));
        assert!(is_private_ip("fe80::1"));
    }

    #[test]
    fn non_ip_hostname_is_not_private() {
        assert!(!is_private_ip("example.com"));
        assert!(!is_private_ip("api.example.com"));
    }

    #[test]
    fn validate_rejects_http_non_localhost() {
        let result = validate_endpoint("http://example.com/ingest");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("HTTPS"));
    }

    #[test]
    fn validate_allows_http_localhost() {
        assert!(validate_endpoint("http://localhost:8080/ingest").is_ok());
        assert!(validate_endpoint("http://127.0.0.1:8080/ingest").is_ok());
    }

    #[test]
    fn validate_allows_https() {
        assert!(validate_endpoint("https://api.example.com/ingest").is_ok());
    }

    #[test]
    fn validate_rejects_private_ip() {
        assert!(validate_endpoint("https://10.0.0.1/ingest").is_err());
        assert!(validate_endpoint("https://192.168.1.1/ingest").is_err());
    }

    #[test]
    fn validate_rejects_credentials() {
        let result = validate_endpoint("https://user:pass@example.com/ingest");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("credentials"));
    }

    #[test]
    fn validate_rejects_empty() {
        assert!(validate_endpoint("").is_err());
    }

    #[test]
    fn validate_rejects_malformed() {
        assert!(validate_endpoint("not-a-url").is_err());
    }
}
