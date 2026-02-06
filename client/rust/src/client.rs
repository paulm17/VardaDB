use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use anyhow::anyhow;

pub struct VardaClient {
    client: Client,
    url: String,
}

#[derive(Serialize)]
struct GraphqlRequest {
    query: String,
    variables: Value,
}

#[derive(Deserialize)]
struct GraphqlResponse {
    data: Option<Value>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

impl VardaClient {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            url: url.into(),
        }
    }

    pub fn post_dynamic(
        &self,
        query: &str,
        variables: Value,
    ) -> anyhow::Result<Value> {
        let body = GraphqlRequest {
            query: query.to_string(),
            variables,
        };

        let res = self.client.post(&self.url).json(&body).send()?;
        let response_body: GraphqlResponse = res.json()?;
        
        if let Some(errors) = response_body.errors {
            if let Some(first_error) = errors.first() {
                return Err(anyhow!("GraphQL Error: {}", first_error.message));
            }
        }

        response_body.data.ok_or_else(|| anyhow!("No data returned"))
    }
}
