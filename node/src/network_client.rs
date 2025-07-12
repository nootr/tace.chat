use async_trait::async_trait;
use log::{debug, error};
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
        debug!("Attempting to connect to {}", address);
        let mut stream = match TcpStream::connect(address).await {
            Ok(stream) => {
                debug!("Successfully connected to {}", address);
                stream
            }
            Err(e) => {
                error!("Failed to connect to {}: {}", address, e);
                return Err(Box::new(e));
            }
        };

        let encoded = bincode::serialize(&message)?;
        debug!("Sending message to {}: {:?}", address, message);

        stream.write_all(&encoded).await?;
        stream.shutdown().await?; // Close the write side to signal we're done sending

        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).await?;
        let response = bincode::deserialize(&buffer)?;
        debug!("Received response from {}: {:?}", address, response);

        Ok(response)
    }
}
