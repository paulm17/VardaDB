//! Fault Injection
//!
//! Simulates various failure modes for testing VardaDB's resilience:
//! - Storage failures (IO errors)
//! - Network partitions
//! - Message delays
//! - Clock skew

use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

/// Fault injection configuration
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FaultConfig {
    /// Probability of storage failure (0.0 - 1.0)
    pub storage_fail_rate: f32,
    
    /// Probability of network partition (0.0 - 1.0)
    pub network_partition_rate: f32,
    
    /// Range of message delay in milliseconds
    pub message_delay_range: (u64, u64),
    
    /// Probability of message drop (0.0 - 1.0)
    pub message_drop_rate: f32,
    
    /// Probability of clock skew injection (0.0 - 1.0)
    pub clock_skew_rate: f32,
}

impl Default for FaultConfig {
    fn default() -> Self {
        Self {
            storage_fail_rate: 0.01,      // 1% chance
            network_partition_rate: 0.005, // 0.5% chance
            message_delay_range: (0, 100), // 0-100ms delay
            message_drop_rate: 0.02,       // 2% chance
            clock_skew_rate: 0.01,         // 1% chance
        }
    }
}

#[allow(dead_code)]
impl FaultConfig {
    /// No faults (for clean testing)
    pub fn none() -> Self {
        Self {
            storage_fail_rate: 0.0,
            network_partition_rate: 0.0,
            message_delay_range: (0, 0),
            message_drop_rate: 0.0,
            clock_skew_rate: 0.0,
        }
    }

    /// High fault rate (stress testing)
    pub fn stress() -> Self {
        Self {
            storage_fail_rate: 0.05,       // 5% chance
            network_partition_rate: 0.02,   // 2% chance
            message_delay_range: (0, 500),  // 0-500ms delay
            message_drop_rate: 0.1,         // 10% chance
            clock_skew_rate: 0.05,          // 5% chance
        }
    }
}

/// Fault injector
#[allow(dead_code)]
pub struct FaultInjector {
    config: FaultConfig,
    rng: ChaCha8Rng,
    
    /// Current fault state
    in_partition: bool,
    partition_end_tick: u64,
}

#[allow(dead_code)]
impl FaultInjector {
    pub fn new(seed: u64) -> Self {
        Self {
            config: FaultConfig::default(),
            rng: ChaCha8Rng::seed_from_u64(seed),
            in_partition: false,
            partition_end_tick: 0,
        }
    }

    pub fn with_config(seed: u64, config: FaultConfig) -> Self {
        Self {
            config,
            rng: ChaCha8Rng::seed_from_u64(seed),
            in_partition: false,
            partition_end_tick: 0,
        }
    }

    /// Check if the current operation should fail
    pub fn should_fail(&mut self) -> bool {
        self.rng.gen::<f32>() < self.config.storage_fail_rate
    }

    /// Check if a network partition should occur
    pub fn should_partition(&mut self) -> bool {
        self.rng.gen::<f32>() < self.config.network_partition_rate
    }

    /// Get a random delay for a message
    pub fn get_delay(&mut self) -> u64 {
        let (min, max) = self.config.message_delay_range;
        if max > min {
            self.rng.gen_range(min..max)
        } else {
            min
        }
    }

    /// Check if a message should be dropped
    pub fn should_drop(&mut self) -> bool {
        self.rng.gen::<f32>() < self.config.message_drop_rate
    }

    /// Check if clock skew should be injected
    pub fn should_skew_clock(&mut self) -> bool {
        self.rng.gen::<f32>() < self.config.clock_skew_rate
    }

    /// Get a random clock skew value (in microseconds)
    pub fn get_clock_skew(&mut self) -> i64 {
        // Skew between -1 second and +1 second
        self.rng.gen_range(-1_000_000..1_000_000)
    }

    /// Update partition state at a given tick
    pub fn update(&mut self, current_tick: u64) {
        // End partition if time is up
        if self.in_partition && current_tick >= self.partition_end_tick {
            self.in_partition = false;
        }

        // Start new partition with probability
        if !self.in_partition && self.should_partition() {
            self.in_partition = true;
            // Partition lasts 10-100 ticks
            self.partition_end_tick = current_tick + self.rng.gen_range(10..100);
        }
    }

    /// Check if currently in a network partition
    pub fn is_partitioned(&self) -> bool {
        self.in_partition
    }
}

/// Types of faults that can occur
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum FaultType {
    StorageFailure,
    NetworkPartition,
    MessageDelay(u64),
    MessageDrop,
    ClockSkew(i64),
}

/// Record of injected faults for debugging
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FaultRecord {
    pub tick: u64,
    pub fault_type: FaultType,
    pub description: String,
}

/// Fault log for tracking injected faults
#[allow(dead_code)]
pub struct FaultLog {
    records: Vec<FaultRecord>,
}

#[allow(dead_code)]
impl FaultLog {
    pub fn new() -> Self {
        Self { records: Vec::new() }
    }

    pub fn record(&mut self, tick: u64, fault_type: FaultType, description: &str) {
        self.records.push(FaultRecord {
            tick,
            fault_type,
            description: description.to_string(),
        });
    }

    pub fn get_records(&self) -> &[FaultRecord] {
        &self.records
    }

    pub fn count(&self) -> usize {
        self.records.len()
    }

    pub fn count_by_type(&self, fault_type: &FaultType) -> usize {
        self.records.iter()
            .filter(|r| std::mem::discriminant(&r.fault_type) == std::mem::discriminant(fault_type))
            .count()
    }
}

impl Default for FaultLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fault_injector_deterministic() {
        // Same seed should produce same results
        let mut injector1 = FaultInjector::new(12345);
        let mut injector2 = FaultInjector::new(12345);

        for _ in 0..100 {
            assert_eq!(injector1.should_fail(), injector2.should_fail());
        }
    }

    #[test]
    fn test_no_faults_config() {
        let mut injector = FaultInjector::with_config(12345, FaultConfig::none());

        for _ in 0..1000 {
            assert!(!injector.should_fail());
            assert!(!injector.should_partition());
            assert!(!injector.should_drop());
        }
    }

    #[test]
    fn test_fault_log() {
        let mut log = FaultLog::new();
        
        log.record(1, FaultType::StorageFailure, "Test failure");
        log.record(2, FaultType::MessageDrop, "Dropped message");
        log.record(3, FaultType::StorageFailure, "Another failure");

        assert_eq!(log.count(), 3);
        assert_eq!(log.count_by_type(&FaultType::StorageFailure), 2);
        assert_eq!(log.count_by_type(&FaultType::MessageDrop), 1);
    }
}
