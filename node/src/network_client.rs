use async_trait::async_trait;
use log::{debug, error};
use std::time::Duration;
use tace_lib::dht_messages::DhtMessage;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait NetworkClient: Send + Sync + 'static {
    async fn call_node(
        &self,
        address: &str,
        message: DhtMessage,
    ) -> Result<DhtMessage, Box<dyn std::error::Error + Send + Sync>>;
}

pub struct RealNetworkClient;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RETRIES: u32 = 3;
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(100);

impl RealNetworkClient {
    async fn call_node_with_retry(
        &self,
        address: &str,
        message: DhtMessage,
    ) -> Result<DhtMessage, Box<dyn std::error::Error + Send + Sync>> {
        let mut delay = INITIAL_RETRY_DELAY;
        let mut last_error = None;
        
        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                debug!("Retrying connection to {} (attempt {}/{})", address, attempt + 1, MAX_RETRIES);
                tokio::time::sleep(delay).await;
                delay *= 2; // Exponential backoff
            }
            
            match self.call_node_internal(address, message.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    last_error = Some(e);
                }
            }
        }
        
        Err(last_error.unwrap_or_else(|| "Unknown error".into()))
    }

    async fn call_node_internal(
        &self,
        address: &str,
        message: DhtMessage,
    ) -> Result<DhtMessage, Box<dyn std::error::Error + Send + Sync>> {
        debug!("Attempting to connect to {} with timeout {:?}", address, DEFAULT_TIMEOUT);
        
        // Apply timeout to connection attempt
        let mut stream = match timeout(DEFAULT_TIMEOUT, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => {
                debug!("Successfully connected to {}", address);
                stream
            }
            Ok(Err(e)) => {
                error!("Failed to connect to {}: {}", address, e);
                return Err(Box::new(e));
            }
            Err(_) => {
                error!("Connection to {} timed out after {:?}", address, DEFAULT_TIMEOUT);
                return Err("Connection timeout".into());
            }
        };

        let encoded = bincode::serialize(&message)?;
        debug!("Sending message to {}: {:?}", address, message);

        // Apply timeout to write operation
        match timeout(DEFAULT_TIMEOUT, stream.write_all(&encoded)).await {
            Ok(Ok(_)) => {},
            Ok(Err(e)) => {
                error!("Failed to write to {}: {}", address, e);
                return Err(Box::new(e));
            }
            Err(_) => {
                error!("Write to {} timed out", address);
                return Err("Write timeout".into());
            }
        }

        // Apply timeout to shutdown
        match timeout(Duration::from_secs(1), stream.shutdown()).await {
            Ok(Ok(_)) => {},
            Ok(Err(e)) => {
                debug!("Shutdown error (non-critical): {}", e);
            }
            Err(_) => {
                debug!("Shutdown timed out (non-critical)");
            }
        }

        let mut buffer = Vec::new();
        
        // Apply timeout to read operation
        match timeout(DEFAULT_TIMEOUT, stream.read_to_end(&mut buffer)).await {
            Ok(Ok(_)) => {},
            Ok(Err(e)) => {
                error!("Failed to read from {}: {}", address, e);
                return Err(Box::new(e));
            }
            Err(_) => {
                error!("Read from {} timed out", address);
                return Err("Read timeout".into());
            }
        }

        let response = bincode::deserialize(&buffer)?;
        debug!("Received response from {}: {:?}", address, response);

        Ok(response)
    }
}

#[async_trait]
impl NetworkClient for RealNetworkClient {
    async fn call_node(
        &self,
        address: &str,
        message: DhtMessage,
    ) -> Result<DhtMessage, Box<dyn std::error::Error + Send + Sync>> {
        self.call_node_with_retry(address, message).await
    }
}
