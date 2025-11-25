use anyhow::Result;
use nostr_sdk::prelude::*;

pub async fn create_client(relay_url: &str, relay_keys: Keys) -> Result<Client> {
    let relay_url = RelayUrl::parse(relay_url)?;

    let client = ClientBuilder::default().signer(relay_keys).build();

    client.add_relay(relay_url).await?;
    Ok(client)
}
