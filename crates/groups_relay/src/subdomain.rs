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
    }

    #[test]
    fn test_extract_subdomain_ip_address() {
        assert_eq!(extract_subdomain("127.0.0.1", 2), None);
        assert_eq!(extract_subdomain("192.168.1.100", 2), None);
    }

    #[test]
    fn test_extract_subdomain_ipv6_address() {
        assert_eq!(extract_subdomain("::1", 2), None);
        assert_eq!(
            extract_subdomain("2001:0db8:85a3:0000:0000:8a2e:0370:7334", 2),
            None
        );
    }

    #[test]
    fn test_extract_subdomain_with_port() {
        assert_eq!(extract_subdomain("oslo.hol.is:8080", 2), Some("oslo".to_string())); // With port
        assert_eq!(extract_subdomain("sub.example.com:3000", 2), Some("sub".to_string())); // With port
        assert_eq!(extract_subdomain("localhost:3000", 2), None); // localhost with port
        assert_eq!(extract_subdomain("127.0.0.1:8000", 2), None); // IP with port
    }

    #[test]
    fn test_extract_subdomain_empty_host() {
        assert_eq!(extract_subdomain("", 2), None);
    }

    #[test]
    fn test_extract_subdomain_just_tld() {
        assert_eq!(extract_subdomain("com", 1), None);
        assert_eq!(extract_subdomain("is", 1), None);
    }

    #[test]
    fn test_extract_subdomain_base_domain_parts_zero() {
        // This configuration might not be typical but testing behavior
        assert_eq!(
            extract_subdomain("oslo.hol.is", 0),
            Some("oslo.hol.is".to_string())
        );
        assert_eq!(
            extract_subdomain("example.com", 0),
            Some("example.com".to_string())
        );
    }

    #[test]
    fn test_extract_subdomain_base_domain_parts_greater_than_parts() {
        assert_eq!(extract_subdomain("example.com", 3), None);
        assert_eq!(extract_subdomain("sub.example.com", 4), None);
    }

    #[test]
    fn test_extract_subdomain_complex_subdomains() {
        assert_eq!(
            extract_subdomain("a.b.c.example.com", 2),
            Some("a.b.c".to_string())
        );
        assert_eq!(
            extract_subdomain("test.verylongsubdomain.example.co.uk", 3),
            Some("test.verylongsubdomain".to_string())
        );
    }
}
