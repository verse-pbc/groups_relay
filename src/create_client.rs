use anyhow::Result;
use nostr_sdk::prelude::*;
use tokio::time::Duration;

pub async fn create_client(relay_url: &str, relay_keys: Keys) -> Result<Client> {
    let opts = Options::default()
        .autoconnect(true)
        .timeout(Duration::from_secs(5));

    let client = ClientBuilder::default()
        .opts(opts)
        .signer(relay_keys)
        .build();

    client.add_relay(relay_url).await?;
    Ok(client)
}
