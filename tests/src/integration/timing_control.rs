use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use tokio::time::{sleep, timeout};

/// Controls timing and synchronization for integration tests
/// Allows tests to control when events happen instead of waiting for real-time delays
#[derive(Clone)]
pub struct TimingController {
    /// Manual timing mode - events are triggered explicitly
    manual_mode: Arc<Mutex<bool>>,
    /// Notification for manual step progression
    step_notify: Arc<Notify>,
    /// Current logical time in the simulation
    logical_time: Arc<Mutex<u64>>,
    /// Time multiplier for speeding up/slowing down operations
    time_multiplier: Arc<Mutex<f64>>,
}

impl TimingController {
    pub fn new() -> Self {
        Self {
            manual_mode: Arc::new(Mutex::new(false)),
            step_notify: Arc::new(Notify::new()),
            logical_time: Arc::new(Mutex::new(0)),
            time_multiplier: Arc::new(Mutex::new(1.0)),
        }
    }

    /// Enable manual timing control
    /// In manual mode, tests control when time advances
    pub async fn enable_manual_mode(&self) {
        *self.manual_mode.lock().await = true;
    }

    /// Disable manual timing control (return to real-time)
    pub async fn disable_manual_mode(&self) {
        *self.manual_mode.lock().await = false;
        self.step_notify.notify_waiters();
    }

    /// Check if manual mode is enabled
    pub async fn is_manual_mode(&self) -> bool {
        *self.manual_mode.lock().await
    }

    /// Advance time by one step in manual mode
    pub async fn step(&self) {
        if *self.manual_mode.lock().await {
            self.step_notify.notify_waiters();
        }
    }

    /// Advance logical time by specified amount
    pub async fn advance_time(&self, duration_ms: u64) {
        let mut logical_time = self.logical_time.lock().await;
        *logical_time += duration_ms;
        self.step_notify.notify_waiters();
    }

    /// Get current logical time
    pub async fn get_logical_time(&self) -> u64 {
        *self.logical_time.lock().await
    }

    /// Set time multiplier (1.0 = real time, 2.0 = 2x speed, 0.5 = half speed)
    pub async fn set_time_multiplier(&self, multiplier: f64) {
        *self.time_multiplier.lock().await = multiplier.max(0.001); // Prevent division by zero
    }

    /// Sleep with timing control
    /// In manual mode, waits for explicit step() calls
    /// In automatic mode, applies time multiplier
    pub async fn sleep(&self, duration: Duration) {
        if *self.manual_mode.lock().await {
            // In manual mode, wait for explicit step
            self.step_notify.notified().await;
        } else {
            // Apply time multiplier
            let multiplier = *self.time_multiplier.lock().await;
            let adjusted_duration = Duration::from_nanos((duration.as_nanos() as f64 / multiplier) as u64);
            sleep(adjusted_duration).await;
        }
    }

    /// Wait for a condition to become true with timeout
    pub async fn wait_for_condition<F, Fut>(&self, condition: F, timeout_seconds: u64) -> Result<(), Box<dyn std::error::Error>>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = bool>,
    {
        let timeout_duration = Duration::from_secs(timeout_seconds);
        
        let result = timeout(timeout_duration, async {
            loop {
                if condition().await {
                    return;
                }
                
                // Check every 100ms or on manual step
                self.sleep(Duration::from_millis(100)).await;
            }
        }).await;

        match result {
            Ok(_) => Ok(()),
            Err(_) => Err("Timeout waiting for condition".into()),
        }
    }

    /// Create a barrier that waits for N parties
    pub fn barrier(&self, parties: usize) -> TimingBarrier {
        TimingBarrier::new(parties, self.step_notify.clone())
    }

    /// Reset logical time to zero
    pub async fn reset_time(&self) {
        *self.logical_time.lock().await = 0;
    }
}

/// A barrier for synchronizing multiple tasks in tests
#[derive(Clone)]
pub struct TimingBarrier {
    parties: usize,
    waiting: Arc<Mutex<usize>>,
    notify: Arc<Notify>,
    step_notify: Arc<Notify>,
}

impl TimingBarrier {
    fn new(parties: usize, step_notify: Arc<Notify>) -> Self {
        Self {
            parties,
            waiting: Arc::new(Mutex::new(0)),
            notify: Arc::new(Notify::new()),
            step_notify,
        }
    }

    /// Wait for all parties to reach the barrier
    pub async fn wait(&self) {
        let mut waiting = self.waiting.lock().await;
        *waiting += 1;
        
        if *waiting >= self.parties {
            // All parties have arrived, release everyone
            *waiting = 0;
            self.notify.notify_waiters();
        } else {
            // Wait for others
            drop(waiting);
            self.notify.notified().await;
        }
    }
}

/// Utilities for common timing patterns in tests
impl TimingController {
    /// Wait for stabilization period (useful for DHT operations)
    pub async fn wait_for_stabilization(&self, duration: Duration) {
        self.sleep(duration).await;
    }

    /// Execute multiple operations concurrently and wait for all to complete
    pub async fn run_concurrent<F, Fut>(&self, operations: Vec<F>) -> Vec<Fut::Output>
    where
        F: FnOnce() -> Fut,
        Fut: Future,
    {
        let futures: Vec<_> = operations.into_iter().map(|op| op()).collect();
        futures::future::join_all(futures).await
    }

    /// Synchronize multiple nodes at a specific point
    pub async fn synchronize_nodes(&self, node_count: usize) {
        let barrier = self.barrier(node_count);
        barrier.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_manual_timing_control() {
        let controller = TimingController::new();
        controller.enable_manual_mode().await;

        let start = std::time::Instant::now();
        
        // Start a task that waits for step
        let controller_clone = controller.clone();
        let task = tokio::spawn(async move {
            controller_clone.sleep(Duration::from_secs(1)).await;
        });

        // Task should not complete immediately
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!task.is_finished());

        // Step should allow task to complete
        controller.step().await;
        task.await.unwrap();

        // Should have taken much less than 1 second
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    #[tokio::test]
    async fn test_time_multiplier() {
        let controller = TimingController::new();
        controller.set_time_multiplier(10.0).await; // 10x speed

        let start = std::time::Instant::now();
        controller.sleep(Duration::from_millis(100)).await;
        let elapsed = start.elapsed();

        // Should take approximately 10ms (100ms / 10x)
        assert!(elapsed < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn test_barrier() {
        let controller = TimingController::new();
        let barrier = controller.barrier(3);

        let mut tasks = Vec::new();
        
        for i in 0..3 {
            let barrier_clone = barrier.clone();
            let task = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(i * 10)).await;
                barrier_clone.wait().await;
                i
            });
            tasks.push(task);
        }

        let results = futures::future::join_all(tasks).await;
        assert_eq!(results.len(), 3);
    }
}