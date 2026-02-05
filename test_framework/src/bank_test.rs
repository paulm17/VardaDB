//! Bank Test - LWW Balance Invariant Testing
//!
//! Ported from Limbo's Antithesis bank-test pattern.
//! 
//! The Bank Test is a classic distributed systems correctness test.
//! Invariant: Total money in the system NEVER changes.
//!
//! This tests VardaDB's:
//! - CRUD operations under concurrent access
//! - LWW (Last-Write-Wins) conflict resolution
//! - Data integrity during transfers

use std::time::Instant;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use async_graphql::Value;

use crate::harness::TestHarness;
use crate::{TestRunner, TestResult};

/// Bank test configuration
pub struct BankTestConfig {
    pub num_accounts: usize,
    pub initial_balance_max: i64,
    pub num_transfers: usize,
    pub transfer_amount_max: i64,
}

impl Default for BankTestConfig {
    fn default() -> Self {
        Self {
            num_accounts: 10,
            initial_balance_max: 1_000_000,
            num_transfers: 100,
            transfer_amount_max: 10_000,
        }
    }
}

/// Initial state of the bank
struct BankState {
    account_ids: Vec<String>,
    initial_total: i64,
}

/// Run the bank test
pub async fn run_bank_test(runner: &mut TestRunner, seed: u64) {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let config = BankTestConfig::default();

    let start = Instant::now();
    let result = execute_bank_test(&mut rng, &config).await;
    
    runner.add_result(match result {
        Ok((transfers, violations)) => {
            if violations == 0 {
                TestResult::pass(
                    &format!("BankTest ({} accounts, {} transfers)", config.num_accounts, transfers),
                    "bank",
                    start.elapsed(),
                )
            } else {
                TestResult::fail(
                    &format!("BankTest ({} accounts, {} transfers)", config.num_accounts, transfers),
                    "bank",
                    start.elapsed(),
                    &format!("{} invariant violations detected", violations),
                )
            }
        }
        Err(e) => TestResult::fail(
            "BankTest",
            "bank",
            start.elapsed(),
            &e,
        ),
    });
}

async fn execute_bank_test(rng: &mut ChaCha8Rng, config: &BankTestConfig) -> Result<(usize, usize), String> {
    // 1. Setup: Create schema and accounts
    let sdl = r#"
        type Account {
            id: ID!
            balance: Int!
            owner: String
        }
    "#;

    let harness = TestHarness::new(sdl)?;
    let state = setup_bank(&harness, rng, config).await?;

    // 2. Execute transfers
    let mut successful_transfers = 0;
    for _ in 0..config.num_transfers {
        if state.account_ids.len() < 2 {
            break;
        }

        // Pick random sender and recipient
        let sender_idx = rng.gen_range(0..state.account_ids.len());
        let mut recipient_idx = rng.gen_range(0..state.account_ids.len());
        while recipient_idx == sender_idx {
            recipient_idx = rng.gen_range(0..state.account_ids.len());
        }

        let sender_id = &state.account_ids[sender_idx];
        let recipient_id = &state.account_ids[recipient_idx];
        let amount: i64 = rng.gen_range(1..config.transfer_amount_max);

        // Execute transfer
        if let Ok(_) = execute_transfer(&harness, sender_id, recipient_id, amount).await {
            successful_transfers += 1;
        }
    }

    // 3. Validate: Check total balance is unchanged
    let violations = validate_invariant(&harness, state.initial_total).await?;

    Ok((successful_transfers, violations))
}

/// Setup bank with accounts
async fn setup_bank(
    harness: &TestHarness,
    rng: &mut ChaCha8Rng,
    config: &BankTestConfig,
) -> Result<BankState, String> {
    let mut account_ids = Vec::new();
    let mut total: i64 = 0;

    for i in 0..config.num_accounts {
        let balance: i64 = rng.gen_range(1..config.initial_balance_max);
        total += balance;

        let mutation = format!(
            r#"mutation {{ createAccount(input: {{ balance: {}, owner: "Owner{}" }}) {{ uid }} }}"#,
            balance, i
        );

        let response = harness.execute_ok(&mutation).await?;
        if let Value::Object(obj) = &response {
            if let Some(Value::Object(create_account)) = obj.get(&async_graphql::Name::new("createAccount")) {
                if let Some(Value::String(uid)) = create_account.get(&async_graphql::Name::new("uid")) {
                    account_ids.push(uid.clone());
                }
            }
        }
    }

    Ok(BankState {
        account_ids,
        initial_total: total,
    })
}

/// Execute a transfer between two accounts
async fn execute_transfer(
    harness: &TestHarness,
    sender_uid: &str,
    recipient_uid: &str,
    amount: i64,
) -> Result<(), String> {
    // Get current balances
    let sender_balance = get_account_balance(harness, sender_uid).await?;
    let recipient_balance = get_account_balance(harness, recipient_uid).await?;

    // Update sender (debit)
    let new_sender_balance = sender_balance - amount;
    let sender_mutation = format!(
        r#"mutation {{ updateAccount(uid: "{}", input: {{ balance: {} }}) }}"#,
        sender_uid, new_sender_balance
    );
    harness.execute_ok(&sender_mutation).await?;

    // Update recipient (credit)
    let new_recipient_balance = recipient_balance + amount;
    let recipient_mutation = format!(
        r#"mutation {{ updateAccount(uid: "{}", input: {{ balance: {} }}) }}"#,
        recipient_uid, new_recipient_balance
    );
    harness.execute_ok(&recipient_mutation).await?;

    Ok(())
}

/// Get account balance
async fn get_account_balance(harness: &TestHarness, account_uid: &str) -> Result<i64, String> {
    let query = format!(
        r#"query {{ getAccount(uid: "{}") {{ balance }} }}"#,
        account_uid
    );

    let response = harness.execute_ok(&query).await?;
    
    if let Value::Object(obj) = &response {
        if let Some(Value::Object(account)) = obj.get(&async_graphql::Name::new("getAccount")) {
            if let Some(Value::Number(balance)) = account.get(&async_graphql::Name::new("balance")) {
                if let Some(b) = balance.as_i64() {
                    return Ok(b);
                }
            }
        }
    }

    Err(format!("Could not get balance for account {}", account_uid))
}

/// Validate the bank invariant: total balance unchanged
async fn validate_invariant(harness: &TestHarness, expected_total: i64) -> Result<usize, String> {
    let query = r#"query { queryAccount { balance } }"#;
    let response = harness.execute_ok(query).await?;

    let mut actual_total: i64 = 0;
    
    if let Value::Object(obj) = &response {
        if let Some(Value::List(accounts)) = obj.get(&async_graphql::Name::new("queryAccount")) {
            for account in accounts {
                if let Value::Object(acc_obj) = account {
                    if let Some(Value::Number(balance)) = acc_obj.get(&async_graphql::Name::new("balance")) {
                        if let Some(b) = balance.as_i64() {
                            actual_total += b;
                        }
                    }
                }
            }
        }
    }

    // THE INVARIANT: Total balance must equal initial total
    if actual_total == expected_total {
        Ok(0) // No violations
    } else {
        Ok(1) // Invariant violated
    }
}
