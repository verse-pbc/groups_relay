//! Subdomain extraction utilities

/// Extract subdomain from a host string
///
/// # Arguments
/// * `host` - The host string, potentially with port (e.g., "sub.example.com:8080")
/// * `base_domain_parts` - Number of parts that constitute the base domain
///
/// # Examples
/// * `extract_subdomain("sub.example.com", 2)` returns `Some("sub")`
/// * `extract_subdomain("example.com", 2)` returns `None`
/// * `extract_subdomain("a.b.example.co.uk", 3)` returns `Some("a.b")`
pub fn extract_subdomain(host: &str, base_domain_parts: usize) -> Option<String> {
    // Remove port if present (host:port format)
    let host_without_port = host.split(':').next().unwrap_or(host);

    // Skip subdomain extraction for localhost and IP addresses
    if host_without_port == "localhost" || host_without_port.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }

    let parts: Vec<&str> = host_without_port.split('.').collect();
    if parts.len() <= base_domain_parts {
        return None; // No subdomain
    }

    // Extract subdomain (everything except the base domain parts)
    Some(parts[0..parts.len() - base_domain_parts].join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_subdomain_simple() {
        assert_eq!(
            extract_subdomain("oslo.hol.is", 2),
            Some("oslo".to_string())
        );
        assert_eq!(
            extract_subdomain("bergen.hol.is", 2),
            Some("bergen".to_string())
        );
    }

    #[test]
    fn test_extract_subdomain_no_subdomain() {
        assert_eq!(extract_subdomain("hol.is", 2), None);
    }

    #[test]
    fn test_extract_subdomain_multi_level_base() {
        assert_eq!(
            extract_subdomain("sub.example.co.uk", 3),
            Some("sub".to_string())
        );
        assert_eq!(
            extract_subdomain("another.sub.example.co.uk", 3),
            Some("another.sub".to_string())
        );
    }

    #[test]
    fn test_extract_subdomain_localhost() {
        assert_eq!(extract_subdomain("localhost", 2), None);
        assert_eq!(extract_subdomain("localhost:8080", 2), None);
    }

    #[test]
    fn test_extract_subdomain_ip_address() {
        assert_eq!(extract_subdomain("192.168.1.1", 2), None);
        assert_eq!(extract_subdomain("192.168.1.1:8080", 2), None);
    }

    #[test]
    fn test_extract_subdomain_with_port() {
        assert_eq!(
            extract_subdomain("sub.example.com:8080", 2),
            Some("sub".to_string())
        );
        assert_eq!(extract_subdomain("example.com:8080", 2), None);
    }
}
