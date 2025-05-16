use axum::{
    extract::Host,
    routing::get,
    Router,
    response::Html,
};
use std::net::SocketAddr;

// Function to extract the subdomain from a hostname
// This is a basic example and might need adjustment based on your specific needs
// (e.g., handling domains like 'localhost' or multi-level TLDs like '.co.uk')
fn extract_subdomain(hostname: &str, base_domain_parts: usize) -> Option<String> {
    let parts: Vec<&str> = hostname.split('.').collect();
    if parts.len() > base_domain_parts {
        Some(parts[0..(parts.len() - base_domain_parts)].join("."))
    } else {
        None // No subdomain found
    }
}

async fn handler(Host(hostname): Host) -> Html<String> {
    println!("Received request with hostname: {}", hostname);

    // Assuming your main domain has 2 parts (e.g., "example.com")
    // For "sub.example.com", base_domain_parts would be 2.
    // For "my.custom.domain.co.uk", base_domain_parts might be 3.
    let base_domain_parts_count = 2; // Adjust this to your needs

    if hostname == "localhost" { // Handle localhost separately if needed
         return Html(format!(
            "<h1>Hello from Axum!</h1><p>You are accessing from localhost (hostname: {}). No specific subdomain.</p>",
            hostname
        ));
    }

    match extract_subdomain(&hostname, base_domain_parts_count) {
        Some(subdomain) => {
            Html(format!(
                "<h1>Hello from Axum!</h1><p>Hostname: {}</p><p>Subdomain: {}</p>",
                hostname, subdomain
            ))
        }
        None => {
            Html(format!(
                "<h1>Hello from Axum!</h1><p>Hostname: {}</p><p>No subdomain detected or accessing the base domain.</p>",
                hostname
            ))
        }
    }
}

#[tokio::main]
async fn main() {
    let app = Router::new().route("/", get(handler));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on {}", addr);

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
