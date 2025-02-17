use tokio::time::sleep;

#[async_trait]
impl MessageConverter<String, String> for ErrorConverter {
    fn outbound_to_string(&self, message: String) -> Result<String, anyhow::Error> {
        Ok(message)
    }

    async fn inbound_from_string(&self, message: String) -> Result<Option<String>, anyhow::Error> {
        if message.contains("slow") {
            sleep(Duration::from_millis(100)).await;
        }
        if message.contains("error") {
            Err(anyhow!("Error processing message"))
        } else {
            Ok(Some(message))
        }
    }
}