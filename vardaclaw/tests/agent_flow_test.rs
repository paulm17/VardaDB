
use vardadb::storage::backend::Storage;
use vardadb::engine::resolver::Resolver;
use vardadb::bridge::fjall_resolver::FjallResolver;
use vardaclaw::llm::{LLMProvider, LLMMessage, LLMResponse, ValidUsage, LLMGateway, ToolDefinition};
use vardaclaw::sandbox::SkillExecutor;
use vardadb::worker::Worker;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use async_graphql::Value;

// Mock LLM Provider
#[derive(Clone)]
struct MockLLMProvider {
    calls: Arc<Mutex<Vec<String>>>, // Stores prompts received
}

impl MockLLMProvider {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl LLMProvider for MockLLMProvider {
    async fn chat_complete(&self, model: &str, messages: &[LLMMessage], tools: &[ToolDefinition]) -> anyhow::Result<LLMResponse> {
        let mut calls = self.calls.lock().unwrap();
        // Store call info: "Model: [model], LastMsg: [content]"
        let last_msg = messages.last().and_then(|m| m.content.clone()).unwrap_or_default();
        calls.push(format!("Model: {}, LastMsg: {}, Tools: {}", model, last_msg, tools.len()));
        
        Ok(LLMResponse {
            content: Some("Yes, I am active and ready.".to_string()),
            tool_calls: None,
            usage: ValidUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Logic moved to VardaClawRunner, not yet implemented"]
async fn test_agent_heartbeat_flow() -> anyhow::Result<()> {
    // 1. Setup Storage
    let path = tempfile::tempdir()?;
    let storage = Arc::new(Storage::new(path.path(), Some(1))?);
    
    // 2. Setup Mock LLM
    let mock_llm = MockLLMProvider::new();
    let _llm_gateway = Arc::new(LLMGateway::new(Arc::new(mock_llm.clone())));
    
    // 3. Setup Skill Executor
    let _skill_executor = Arc::new(SkillExecutor::new());

    // 4. Setup Worker
    let worker = Worker::new(storage.clone(), 0);
    
    // Spawn Worker
    let worker_handle = tokio::spawn(async move {
        worker.run().await;
    });

    // 5. Create Active Agent
    let resolver = FjallResolver::new(storage.clone(), "default");
    
    let mut agent_fields = HashMap::new();
    agent_fields.insert("name".to_string(), Value::String("TestAgent".to_string()));
    agent_fields.insert("model".to_string(), Value::String("gpt-4-turbo".to_string()));
    agent_fields.insert("status".to_string(), Value::String("ACTIVE".to_string())); // Or implicit if active field exists as bool?
    // In Defaults, active is Boolean!
    agent_fields.insert("active".to_string(), Value::Boolean(true));
    agent_fields.insert("systemPrompt".to_string(), Value::String("You are a test bot.".to_string()));
    
    // Create Agent Node
    let agent_uid = resolver.create_node(
        "Agent", 
        agent_fields, 
        &[], 
        &[], 
        &HashMap::new(), 
        None
    ).map_err(|e| anyhow::anyhow!(e))?;
    
    println!("Created Agent with UID: {}", agent_uid);

    // 6. Trigger Heartbeat Manually (Push Job)
    let heartbeat_payload = b"HEARTBEAT".to_vec();
    let job = vardadb::jobs::types::Job::new(
        9999,
        "system_queue".to_string(),
        heartbeat_payload
    );
    storage.system_queue.push(job).map_err(|e| anyhow::anyhow!(e))?;

    println!("Pushed HEARTBEAT job...");

    // 7. Wait for Worker to Process
    // Worker picks up HEARTBEAT -> Spawns AGENT_TASK -> Calls LLM
    // We poll check mock_llm calls
    
    let start = std::time::Instant::now();
    let mut success = false;
    
    while start.elapsed().as_secs() < 10 { // Wait up to 10s
        let calls = mock_llm.calls.lock().unwrap();
        if !calls.is_empty() {
            println!("Received LLM Call: {:?}", calls[0]);
            assert!(calls[0].contains("Model: gpt-4-turbo"));
            assert!(calls[0].contains("LastMsg: Hello!")); // Worker sends "Hello! ..."
            assert!(calls[0].contains("Tools: 1")); // We added "echo" tool
            success = true;
            break;
        }
        drop(calls);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    if !success {
        panic!("Timed out waiting for LLM call from Agent Worker");
    }

    // Cleanup worker (abort handle or let satisfy test scope drop?)
    worker_handle.abort();
    
    Ok(())
}
