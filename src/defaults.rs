pub const AGENT_SCHEMA: &str = r#"
# VardaDB Default Schema
# Incorporates LocalGPT patterns for Autonomous Agent capabilities

"""
The `Agent` type defines the configuration for the autonomous agent attached to this database.
Singleton: There should typically be only one active Agent profile per database, or one per 'persona'.
"""
type Agent @model {
  id: ID!
  name: String! @search(by: [term])
  model: String! # e.g. "claude-3-5-sonnet"
  contextWindow: Int!
  reserveTokens: Int!
  
  # Configuration
  active: Boolean! @search
  systemPrompt: String # Base system prompt override
  
  # Relations
  sessions: [Session!] @hasInverse(field: "agent")
  skills: [Skill!] @hasInverse(field: "agents")
}

"""
`MemoryBlock` represents the singleton memory files from LocalGPT (SOUL, USER, IDENTITY).
We use a 'key' to distinguish them (e.g., key: "SOUL", key: "USER").
"""
type MemoryBlock @model {
  id: ID!
  key: String! @unique @search(by: [term]) # "SOUL", "USER", "IDENTITY", "TOOLS"
  content: String! @search(by: [fulltext]) # The actual markdown content
  lastUpdated: DateTime!
}

"""
`MemoryChunk` represents the granular, vector-searchable long-term memory.
Corresponds to chunks derived from `MEMORY.md` and other documentation.
"""
type MemoryChunk @model {
  id: ID!
  content: String! @search(by: [fulltext])
  sourceFile: String! @search(by: [term]) # e.g. "MEMORY.md", "knowledge/deploy.md"
  
  # Vector embedding for semantic search
  # VardaDB will automatically manage the HNSW index for this field
  embedding: [Float!] @search(by: [hnsw]) 
  
  # Relevance/Quality score
  utilityScore: Float
  lastAccessed: DateTime
}

"""
`Skill` represents a capability available to the agent.
Corresponds to `SKILL.md` folders.
"""
type Skill @model {
  id: ID!
  name: String! @unique @search(by: [term])
  description: String
  commandName: String! @search(by: [term]) # e.g. "github-pr"
  
  # Capability Gating
  requiredBinaries: [String!]
  requiredEnvVars: [String!]
  
  # Implementation
  scriptContent: String # The actual script or instructions
  
  agents: [Agent!] @hasInverse(field: "skills")
}

"""
`Session` tracks a conversational thread.
Replaces `sessions.json`.
"""
type Session @model {
  id: ID!
  title: String @search(by: [fulltext])
  createdAt: DateTime!
  updatedAt: DateTime!
  
  agent: Agent! @hasInverse(field: "sessions")
  messages: [Message!] @hasInverse(field: "session")
  
  # Metrics
  tokenUsageInput: Int
  tokenUsageOutput: Int
  compactionCount: Int
}

"""
`Message` is an individual log entry in a session.
Replaces flat markdown logs.
"""
type Message @model {
  id: ID!
  role: MessageRole!
  content: String! @search(by: [fulltext])
  timestamp: DateTime!
  
  session: Session! @hasInverse(field: "messages")
  
  # For tool use
  toolCalls: [ToolCall!]
}

enum MessageRole {
  USER
  ASSISTANT
  SYSTEM
  TOOL
}

type ToolCall {
  name: String!
  arguments: String!
  output: String
}
"#;
