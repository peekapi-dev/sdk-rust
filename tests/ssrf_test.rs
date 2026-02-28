use peekapi::{is_private_ip, validate_endpoint};

#[test]
fn private_ipv4_10_range() {
    assert!(is_private_ip("10.0.0.1"));
    assert!(is_private_ip("10.255.255.255"));
}

#[test]
fn private_ipv4_172_range() {
    assert!(is_private_ip("172.16.0.1"));
    assert!(is_private_ip("172.31.255.255"));
    // Outside the range
    assert!(!is_private_ip("172.15.0.1"));
    assert!(!is_private_ip("172.32.0.1"));
}

#[test]
fn private_ipv4_192_168_range() {
    assert!(is_private_ip("192.168.0.1"));
    assert!(is_private_ip("192.168.255.255"));
}

#[test]
fn loopback() {
    assert!(is_private_ip("127.0.0.1"));
    assert!(is_private_ip("127.255.255.255"));
}

#[test]
fn link_local() {
    assert!(is_private_ip("169.254.0.1"));
    assert!(is_private_ip("169.254.255.255"));
}

#[test]
fn cgnat() {
    assert!(is_private_ip("100.64.0.1"));
    assert!(is_private_ip("100.127.255.255"));
    // Outside CGNAT
    assert!(!is_private_ip("100.63.255.255"));
    assert!(!is_private_ip("100.128.0.1"));
}

#[test]
fn zero_network() {
    assert!(is_private_ip("0.0.0.0"));
    assert!(is_private_ip("0.1.2.3"));
}

#[test]
fn public_ipv4() {
    assert!(!is_private_ip("8.8.8.8"));
    assert!(!is_private_ip("1.1.1.1"));
    assert!(!is_private_ip("203.0.113.1"));
    assert!(!is_private_ip("151.101.1.140"));
}

#[test]
fn ipv6_loopback() {
    assert!(is_private_ip("::1"));
}

#[test]
fn ipv6_ula() {
    assert!(is_private_ip("fc00::1"));
    assert!(is_private_ip("fd12:3456:789a::1"));
}

#[test]
fn ipv6_link_local() {
    assert!(is_private_ip("fe80::1"));
}

#[test]
fn hostname_not_private() {
    assert!(!is_private_ip("example.com"));
    assert!(!is_private_ip("api.example.com"));
    assert!(!is_private_ip("localhost")); // hostname, not IP
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
fn validate_rejects_private_ip_endpoint() {
    assert!(validate_endpoint("https://10.0.0.1/ingest").is_err());
    assert!(validate_endpoint("https://192.168.1.1/ingest").is_err());
    assert!(validate_endpoint("https://172.16.0.1/ingest").is_err());
    assert!(validate_endpoint("https://100.64.0.1/ingest").is_err());
}

#[test]
fn validate_rejects_embedded_credentials() {
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
    assert!(validate_endpoint("://missing-scheme").is_err());
}
