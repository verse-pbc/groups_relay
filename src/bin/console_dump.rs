//! Tokio Console Dump - Programmatic access to tokio task diagnostics
//!
//! This binary connects to console-subscriber's gRPC API and dumps task
//! information. Based on tokio-rs/console dump.rs example.
//!
//! Usage:
//!   console_dump [address]
//!
//! Examples:
//!   console_dump                              # Uses default http://localhost:6669
//!   console_dump http://localhost:6669        # Specify address
//!

use console_api::instrument::{instrument_client::InstrumentClient, InstrumentRequest};
use futures::stream::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args();
    args.next(); // drop the first arg (the name of the binary)
    let target = args.next().unwrap_or_else(|| {
        eprintln!("using default address (http://localhost:6669)");
        String::from("http://localhost:6669")
    });

    eprintln!("CONNECTING: {}", target);
    let mut client = InstrumentClient::connect(target).await?;

    let request = tonic::Request::new(InstrumentRequest {});
    let mut stream = client.watch_updates(request).await?.into_inner();

    let mut i: usize = 0;
    while let Some(update) = stream.next().await {
        match update {
            Ok(update) => {
                println!("UPDATE {}: {:#?}\n", i, update);
                i += 1;
            }
            Err(e) => {
                eprintln!("update stream error: {}", e);
                return Err(e.into());
            }
        }
    }

    eprintln!("update stream terminated");
    Ok(())
}
