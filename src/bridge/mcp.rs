use crate::engine::resolver::Resolver;
use crate::engine::schema::Schema;
use async_graphql::Request;
use prism_mcp_rs::prelude::*;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct MCPServer {
    schema: Arc<RwLock<Arc<Schema>>>,
    #[allow(dead_code)]
    resolver: Box<dyn Resolver + Send + Sync>,
}

impl MCPServer {
    pub fn new(
        schema: Arc<RwLock<Arc<Schema>>>,
        resolver: Box<dyn Resolver + Send + Sync>,
    ) -> Self {
        Self { schema, resolver }
    }

    pub async fn run_stdio_server(self) -> anyhow::Result<()> {
        let mut server = McpServer::new("VardaDB".to_string(), "0.1.0".to_string());

        // 1. Query GraphQL Tool
        let query_handler = QueryGraphqlHandler {
            schema: self.schema.clone(),
        };
        server
            .add_tool(
                "query_graphql".to_string(),
                Some("Execute a GraphQL query against VardaDB".to_string()),
                json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "variables": { "type": "object" }
                    },
                    "required": ["query"]
                }),
                query_handler,
            )
            .await?;

        // 2. Search Vectors Tool
        // let search_handler = SearchVectorsHandler { ... };
        // server.add_tool(...)

        // 3. Get Schema Tool
        let schema_handler = GetSchemaHandler {
            schema: self.schema.clone(),
        };
        server
            .add_tool(
                "get_schema".to_string(),
                Some("Get the current GraphQL Schema Definition (SDL)".to_string()),
                json!({
                    "type": "object",
                    "properties": {},
                }),
                schema_handler,
            )
            .await?;

        println!("Starting VardaDB MCP Server on Stdio...");

        // Start with StdioTransport
        let transport = StdioServerTransport::new();
        server.start(transport).await?;

        Ok(())
    }
}

// Handlers

struct QueryGraphqlHandler {
    schema: Arc<RwLock<Arc<Schema>>>,
}

#[async_trait]
impl ToolHandler for QueryGraphqlHandler {
    async fn call(&self, arguments: HashMap<String, Value>) -> McpResult<CallToolResult> {
        let query = arguments
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| McpError::InvalidParams("Missing 'query' argument".to_string()))?;

        let vars_json = arguments.get("variables").cloned().unwrap_or(Value::Null);
        let vars: async_graphql::Variables = serde_json::from_value(vars_json).unwrap_or_default();

        let request = Request::new(query).variables(vars);

        let schema_guard = self.schema.read().await;
        let resp = schema_guard.execute(request).await;
        // resp -> Response. We need to serialize data/errors.
        let json_resp = serde_json::to_string(&resp).unwrap_or_default();
        let is_err = !resp.errors.is_empty();

        Ok(CallToolResult {
            content: vec![ContentBlock::text(json_resp)],
            is_error: Some(is_err),
            structured_content: None,
            meta: None,
        })
    }
}

struct GetSchemaHandler {
    schema: Arc<RwLock<Arc<Schema>>>,
}

#[async_trait]
impl ToolHandler for GetSchemaHandler {
    async fn call(&self, _arguments: HashMap<String, Value>) -> McpResult<CallToolResult> {
        let schema_guard = self.schema.read().await;
        let sdl = schema_guard.sdl();

        Ok(CallToolResult {
            content: vec![ContentBlock::text(sdl)],
            is_error: Some(false),
            structured_content: None,
            meta: None,
        })
    }
}

#[allow(dead_code)]
struct SearchVectorsHandler;
// Placeholder for future impl
