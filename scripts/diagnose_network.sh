#!/bin/bash

echo "Network Diagnostics for communities2.nos.social"
echo "=============================================="

# Test basic latency
echo -e "\n1. Basic ping latency:"
ping -c 5 communities2.nos.social | tail -n 1

# Traceroute to see network path
echo -e "\n2. Network path (first 10 hops):"
traceroute -m 10 communities2.nos.social 2>/dev/null | head -n 12 || echo "Traceroute not available"

# Check SSL certificate details
echo -e "\n3. SSL Certificate Chain:"
echo | openssl s_client -connect communities2.nos.social:443 -servername communities2.nos.social 2>/dev/null | grep -E "subject=|issuer=" | head -n 6

# Check SSL handshake time with openssl
echo -e "\n4. OpenSSL handshake timing:"
(time echo | openssl s_client -connect communities2.nos.social:443 -servername communities2.nos.social 2>/dev/null | grep "SSL-Session:") 2>&1 | grep real

# Check HTTP/2 support
echo -e "\n5. HTTP/2 and ALPN support:"
echo | openssl s_client -connect communities2.nos.social:443 -alpn h2,http/1.1 -servername communities2.nos.social 2>/dev/null | grep -E "ALPN|Protocol"

# Test with curl timing
echo -e "\n6. Detailed curl timing breakdown:"
curl -w "@-" -o /dev/null -s https://communities2.nos.social <<'EOF'
    time_namelookup:  %{time_namelookup}s
       time_connect:  %{time_connect}s
    time_appconnect:  %{time_appconnect}s
   time_pretransfer:  %{time_pretransfer}s
      time_redirect:  %{time_redirect}s
 time_starttransfer:  %{time_starttransfer}s
                    ----------
         time_total:  %{time_total}s
EOF

echo -e "\n\nPotential issues to investigate:"
echo "- SSL handshake taking ~266ms (should be <100ms)"
echo "- TLS session resumption not working"
echo "- Check if DO LoadBalancer is terminating SSL"
echo "- Check if Cloudflare proxy is enabled"