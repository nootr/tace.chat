use async_trait::async_trait;
use tace_lib::dht_messages::DhtMessage;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait NetworkClient: Send + Sync + 'static {
    async fn call_node(
        &self,
        address: &str,
        message: DhtMessage,
    ) -> Result<DhtMessage, Box<dyn std::error::Error>>;
}

pub struct RealNetworkClient;

#[async_trait]
impl NetworkClient for RealNetworkClient {
    async fn call_node(
        &self,
        address: &str,
        message: DhtMessage,
    ) -> Result<DhtMessage, Box<dyn std::error::Error>> {
        let mut stream = TcpStream::connect(address).await?;
        let encoded = bincode::serialize(&message)?;
        // In a real scenario, you might want to log this with the node's own address
        // log_info!(address, "Sending message to {}: {:?}", address, message);

        stream.write_all(&encoded).await?;

        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).await?;
        let response = bincode::deserialize(&buffer)?;
        // log_info!(address, "Received response from {}: {:?}", address, response);

        Ok(response)
    }
}
