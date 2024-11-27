use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

pub async fn create_websocket_client(
    proxy_addr: &str,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, Box<dyn std::error::Error>> {
    let url = format!("ws://{}", proxy_addr);
    let (ws_stream, _) = connect_async(url).await?;
    Ok(ws_stream)
}

pub async fn assert_proxy_response(
    client: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    message: &str,
    expected_response: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    client.send(Message::Text(message.to_string())).await?;

    if let Some(Ok(Message::Text(response))) = client.next().await {
        assert_eq!(response, expected_response);
        Ok(())
    } else {
        Err("Expected text message".into())
    }
}
