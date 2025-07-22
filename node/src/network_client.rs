use async_trait::async_trait;
use log::{debug, error, warn};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tace_lib::dht_messages::DhtMessage;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
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

struct ConnectionStats {
    active_count: usize,
    last_cleanup: Instant,
}

struct ConnectionLimiter {
    stats: HashMap<String, ConnectionStats>,
    max_connections_per_host: usize,
    cleanup_interval: Duration,
}

impl ConnectionLimiter {
    fn new(max_connections_per_host: usize) -> Self {
        Self {
            stats: HashMap::new(),
            max_connections_per_host,
            cleanup_interval: Duration::from_secs(30),
        }
    }

    fn can_connect(&mut self, address: &str) -> bool {
        let now = Instant::now();
        
        // Cleanup old entries periodically
        if self.stats.values().any(|s| now.duration_since(s.last_cleanup) > self.cleanup_interval) {
            self.cleanup_old_entries(now);
        }

        let stats = self.stats.entry(address.to_string()).or_insert(ConnectionStats {
            active_count: 0,
            last_cleanup: now,
        });

        stats.active_count < self.max_connections_per_host
    }

    fn increment_count(&mut self, address: &str) {
        if let Some(stats) = self.stats.get_mut(address) {
            stats.active_count += 1;
        }
    }

    fn decrement_count(&mut self, address: &str) {
        if let Some(stats) = self.stats.get_mut(address) {
            stats.active_count = stats.active_count.saturating_sub(1);
        }
    }

    fn cleanup_old_entries(&mut self, now: Instant) {
        self.stats.retain(|_, stats| {
            stats.active_count > 0 || now.duration_since(stats.last_cleanup) < self.cleanup_interval * 2
        });
    }
}

struct ConnectionGuard {
    address: String,
    limiter: Arc<Mutex<ConnectionLimiter>>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        let limiter = Arc::clone(&self.limiter);
        let address = self.address.clone();
        tokio::spawn(async move {
            if let Ok(mut limiter) = limiter.try_lock() {
                limiter.decrement_count(&address);
            }
        });
    }
}

pub struct RealNetworkClient {
    limiter: Arc<Mutex<ConnectionLimiter>>,
}

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RETRIES: u32 = 3;
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_CONNECTIONS_PER_HOST: usize = 5;

impl RealNetworkClient {
    pub fn new() -> Self {
        Self {
            limiter: Arc::new(Mutex::new(ConnectionLimiter::new(MAX_CONNECTIONS_PER_HOST))),
        }
    }
    async fn call_node_with_retry(
        &self,
        address: &str,
        message: DhtMessage,
    ) -> Result<DhtMessage, Box<dyn std::error::Error + Send + Sync>> {
        let mut delay = INITIAL_RETRY_DELAY;
        let mut last_error = None;

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                debug!(
                    "Retrying connection to {} (attempt {}/{})",
                    address,
                    attempt + 1,
                    MAX_RETRIES
                );
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
        debug!(
            "Attempting to connect to {} with timeout {:?}",
            address, DEFAULT_TIMEOUT
        );

        // Check if we can create a new connection
        {
            let mut limiter = self.limiter.lock().await;
            if !limiter.can_connect(address) {
                warn!("Connection limit reached for {}, rejecting request", address);
                return Err(format!("Connection limit exceeded for {}", address).into());
            }
            limiter.increment_count(address);
        }

        // Create connection with auto-cleanup on drop
        let connection_guard = ConnectionGuard {
            address: address.to_string(),
            limiter: Arc::clone(&self.limiter),
        };

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
                error!(
                    "Connection to {} timed out after {:?}",
                    address, DEFAULT_TIMEOUT
                );
                return Err("Connection timeout".into());
            }
        };

        let encoded = bincode::serialize(&message)?;
        debug!("Sending message to {}: {:?}", address, message);

        // Apply timeout to write operation
        match timeout(DEFAULT_TIMEOUT, stream.write_all(&encoded)).await {
            Ok(Ok(_)) => {}
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
            Ok(Ok(_)) => {}
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
            Ok(Ok(_)) => {}
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

        // Keep the guard alive until the end of the function
        drop(connection_guard);
        
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
