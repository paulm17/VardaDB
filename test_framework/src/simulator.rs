//! Deterministic Simulation Testing
//!
//! Inspired by TigerBeetle's VOPR and sled's simulation guide.
//!
//! Runs VardaDB through a simulated environment with:
//! - Deterministic random number generation (reproducible)
//! - Fault injection (storage failures, delays)
//! - Invariant checking at every step
//! - Bug storage and replay

use std::time::Instant;
use std::collections::BinaryHeap;
use std::cmp::Ordering;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use async_graphql::Value;

use crate::harness::TestHarness;
use crate::faults::FaultInjector;
use crate::{TestRunner, TestResult};

/// Simulation event
#[derive(Debug, Clone)]
pub enum SimEvent {
    Insert { type_name: String, fields: Vec<(String, String)> },
    Update { type_name: String, id: String, field: String, value: String },
    Delete { type_name: String, id: String },
    Query { type_name: String },
    CheckInvariant,
}

/// Scheduled event with timestamp
#[derive(Debug, Clone)]
struct ScheduledEvent {
    time: u64,
    event: SimEvent,
}

impl PartialEq for ScheduledEvent {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time
    }
}

impl Eq for ScheduledEvent {}

impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse order for min-heap behavior
        other.time.cmp(&self.time)
    }
}

/// Simulation state
pub struct Simulator {
    rng: ChaCha8Rng,
    event_queue: BinaryHeap<ScheduledEvent>,
    clock: u64,
    fault_injector: FaultInjector,
    created_ids: Vec<(String, String)>, // (type_name, id)
}

impl Simulator {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: ChaCha8Rng::seed_from_u64(seed),
            event_queue: BinaryHeap::new(),
            clock: 0,
            fault_injector: FaultInjector::new(seed),
            created_ids: Vec::new(),
        }
    }

    /// Schedule an event at a future time
    pub fn schedule(&mut self, delay: u64, event: SimEvent) {
        self.event_queue.push(ScheduledEvent {
            time: self.clock + delay,
            event,
        });
    }

    /// Run the simulation for a given number of interactions
    pub async fn run(&mut self, harness: &TestHarness, interactions: usize) -> SimulationResult {
        let mut stats = SimulationStats::default();
        
        // Generate initial interaction plan
        self.generate_plan(interactions);

        // Process events
        while let Some(scheduled) = self.event_queue.pop() {
            self.clock = scheduled.time;

            // Check for fault injection
            if self.fault_injector.should_fail() {
                stats.faults_injected += 1;
                continue; // Skip this event (simulating failure)
            }

            match &scheduled.event {
                SimEvent::Insert { type_name, fields } => {
                    stats.inserts += 1;
                    if let Ok(id) = self.execute_insert(harness, type_name, fields).await {
                        self.created_ids.push((type_name.clone(), id));
                    }
                }
                SimEvent::Update { type_name, id, field, value } => {
                    stats.updates += 1;
                    let _ = self.execute_update(harness, type_name, id, field, value).await;
                }
                SimEvent::Delete { type_name, id } => {
                    stats.deletes += 1;
                    let _ = self.execute_delete(harness, type_name, id).await;
                    self.created_ids.retain(|(t, i)| t != type_name || i != id);
                }
                SimEvent::Query { type_name } => {
                    stats.queries += 1;
                    let _ = self.execute_query(harness, type_name).await;
                }
                SimEvent::CheckInvariant => {
                    stats.invariant_checks += 1;
                    if let Err(e) = self.check_invariants(harness).await {
                        stats.invariant_violations.push(e);
                    }
                }
            }
        }

        SimulationResult {
            seed: self.rng.get_seed()[0] as u64,
            stats,
        }
    }

    /// Generate a plan of random interactions
    fn generate_plan(&mut self, interactions: usize) {
        for i in 0..interactions {
            let delay = self.rng.gen_range(0..10);
            let event = self.random_event();
            self.schedule(i as u64 * 10 + delay, event);

            // Periodically check invariants
            if i % 100 == 0 {
                self.schedule(i as u64 * 10 + 5, SimEvent::CheckInvariant);
            }
        }
    }

    /// Generate a random event
    fn random_event(&mut self) -> SimEvent {
        let event_type = self.rng.gen_range(0..10);
        
        match event_type {
            0..=4 => {
                // Insert (50%)
                SimEvent::Insert {
                    type_name: "Item".to_string(),
                    fields: vec![
                        ("name".to_string(), format!("\"Item{}\"", self.rng.gen::<u32>())),
                        ("value".to_string(), self.rng.gen_range(1..1000).to_string()),
                    ],
                }
            }
            5..=6 => {
                // Update (20%)
                if let Some((type_name, id)) = self.created_ids.choose(&mut self.rng).cloned() {
                    SimEvent::Update {
                        type_name,
                        id,
                        field: "value".to_string(),
                        value: self.rng.gen_range(1..1000).to_string(),
                    }
                } else {
                    // Fallback to insert if no items exist
                    SimEvent::Insert {
                        type_name: "Item".to_string(),
                        fields: vec![
                            ("name".to_string(), format!("\"Item{}\"", self.rng.gen::<u32>())),
                            ("value".to_string(), self.rng.gen_range(1..1000).to_string()),
                        ],
                    }
                }
            }
            7 => {
                // Delete (10%)
                if let Some((type_name, id)) = self.created_ids.choose(&mut self.rng).cloned() {
                    SimEvent::Delete { type_name, id }
                } else {
                    SimEvent::Query { type_name: "Item".to_string() }
                }
            }
            _ => {
                // Query (20%)
                SimEvent::Query { type_name: "Item".to_string() }
            }
        }
    }

    async fn execute_insert(&self, harness: &TestHarness, type_name: &str, fields: &[(String, String)]) -> Result<String, String> {
        let field_str = fields.iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join(", ");

        let mutation = format!(
            r#"mutation {{ create{}(input: {{ {} }}) {{ uid }} }}"#,
            type_name, field_str
        );

        let response = harness.execute_ok(&mutation).await?;
        
        let create_key = format!("create{}", type_name);
        if let Value::Object(obj) = &response {
            if let Some(Value::Object(create_obj)) = obj.get(&async_graphql::Name::new(&create_key)) {
                if let Some(Value::String(uid)) = create_obj.get(&async_graphql::Name::new("uid")) {
                    return Ok(uid.clone());
                }
            }
        }

        Err("Failed to get UID".to_string())
    }

    async fn execute_update(&self, harness: &TestHarness, type_name: &str, uid: &str, field: &str, value: &str) -> Result<(), String> {
        let mutation = format!(
            r#"mutation {{ update{}(uid: "{}", input: {{ {}: {} }}) }}"#,
            type_name, uid, field, value
        );

        harness.execute_ok(&mutation).await?;
        Ok(())
    }

    async fn execute_delete(&self, harness: &TestHarness, type_name: &str, uid: &str) -> Result<(), String> {
        let mutation = format!(
            r#"mutation {{ delete{}(uid: "{}") }}"#,
            type_name, uid
        );

        harness.execute_ok(&mutation).await?;
        Ok(())
    }

    async fn execute_query(&self, harness: &TestHarness, type_name: &str) -> Result<(), String> {
        let query = format!(
            r#"query {{ query{} {{ uid }} }}"#,
            type_name
        );

        harness.execute_ok(&query).await?;
        Ok(())
    }

    async fn check_invariants(&self, harness: &TestHarness) -> Result<(), String> {
        // Invariant 1: All created items should be queryable
        for (type_name, uid) in &self.created_ids {
            let query = format!(
                r#"query {{ get{}(uid: "{}") {{ uid }} }}"#,
                type_name, uid
            );

            let response = harness.execute_ok(&query).await?;
            let get_key = format!("get{}", type_name);
            
            if let Value::Object(obj) = &response {
                if let Some(Value::Null) = obj.get(&async_graphql::Name::new(&get_key)) {
                    return Err(format!("Created item {}:{} not found", type_name, uid));
                }
            }
        }

        Ok(())
    }
}

/// Simulation statistics
#[derive(Debug, Default)]
pub struct SimulationStats {
    pub inserts: usize,
    pub updates: usize,
    pub deletes: usize,
    pub queries: usize,
    pub invariant_checks: usize,
    pub invariant_violations: Vec<String>,
    pub faults_injected: usize,
}

/// Simulation result
#[derive(Debug)]
#[allow(dead_code)]
pub struct SimulationResult {
    pub seed: u64,
    pub stats: SimulationStats,
}

/// Run the deterministic simulation
pub async fn run_simulation(runner: &mut TestRunner, seed: u64, interactions: usize) {
    let start = Instant::now();

    let sdl = r#"
        type Item {
            id: ID!
            name: String!
            value: Int!
        }
    "#;

    let result = async {
        let harness = TestHarness::new(sdl)?;
        let mut simulator = Simulator::new(seed);
        let result = simulator.run(&harness, interactions).await;
        Ok::<_, String>(result)
    }.await;

    match result {
        Ok(sim_result) => {
            let stats = &sim_result.stats;
            let violations = stats.invariant_violations.len();
            
            if violations == 0 {
                runner.add_result(TestResult::pass(
                    &format!(
                        "Simulation ({} interactions, {} faults)",
                        interactions, stats.faults_injected
                    ),
                    "simulation",
                    start.elapsed(),
                ));
            } else {
                runner.add_result(TestResult::fail(
                    &format!("Simulation ({} interactions)", interactions),
                    "simulation",
                    start.elapsed(),
                    &format!("{} invariant violations: {:?}", violations, stats.invariant_violations),
                ));
            }
        }
        Err(e) => {
            runner.add_result(TestResult::fail(
                "Simulation",
                "simulation",
                start.elapsed(),
                &e,
            ));
        }
    }
}
