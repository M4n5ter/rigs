//! A framework for building reliable multi-agent applications.
//!
//! Rigs is an agent orchestration framework. At a high level, it
//! provides a few major components:
//!
//! * Tools for [working with agents][agents], including
//!   [configuration and lifecycle management][agent_config] and [pre-built agent implementations][rig_agent].
//! * APIs for [handling conversations][conversation], including message passing between agents
//!   and conversation history management.
//! * A [graph-based workflow engine][graph_workflow] for orchestrating complex agent interactions
//!   and building sophisticated agent pipelines.
//! * Tools for [data persistence][persistence], including saving and loading agent states,
//!   conversation histories, and workflow configurations.
//!
//! [agents]: #working-with-agents
//! [agent_config]: crate::agent::AgentConfig
//! [rig_agent]: crate::rig_agent
//! [conversation]: crate::conversation
//! [graph_workflow]: crate::graph_workflow
//! [persistence]: crate::persistence
//!
//! # A Tour of Rigs
//!
//! Rigs consists of a number of modules that provide a range of functionality
//! essential for implementing agent-based applications in Rust. In this
//! section, we will take a brief tour of Rigs, summarizing the major APIs and
//! their uses.
//!
//! ## Working With Agents
//!
//! At the core of Rigs is the concept of an agent. The [`agent`] module provides
//! important tools for working with agents:
//!
//! * The [`Agent`] trait, which defines the core functionality that all agents must implement.
//! * The [`AgentConfig`] struct and [`AgentConfigBuilder`], for configuring agent behavior.
//! * Error handling with [`AgentError`] for managing agent-related failures.
//!
//! [`agent`]: crate::agent
//! [`Agent`]: crate::agent::Agent
//! [`AgentConfig`]: crate::agent::AgentConfig
//! [`AgentConfigBuilder`]: crate::agent::AgentConfigBuilder
//! [`AgentError`]: crate::agent::AgentError
//!
//! ### Example: Creating a Basic Agent
//!
//! ```rust
//! use rigs::agent::{Agent, AgentConfig};
//! use rigs::rig_agent::RigAgentBuilder;
//! use rigs::llm_provider::LLMProvider;
//!
//! // Create a configuration for our agent
//! let config = AgentConfig::builder()
//!     .agent_name("MyAssistant")
//!     .user_name("User")
//!     .description("A helpful assistant")
//!     .temperature(0.7)
//!     .max_tokens(2048)
//!     .build();
//!
//! // Create a provider for the model
//! let provider = LLMProvider::deepseek("deepseek-chat");
//!
//! // Build the agent with the configuration
//! let agent = RigAgent::deepseek_builder()
//!     .provider(provider)?
//!     .agent_name("MyAssistant")
//!     .user_name("User")
//!     .system_prompt("You are a helpful assistant.")
//!     .build()?;
//! ```
//!
//! ## Handling Conversations
//!
//! The [`conversation`] module provides tools for managing conversations between users and agents:
//!
//! * [`Conversation`] for tracking message history between a user and an agent.
//! * [`AgentShortMemory`] for storing multiple conversations across different tasks.
//! * Message handling with [`Role`] and [`Content`] types.
//!
//! [`conversation`]: crate::conversation
//! [`Conversation`]: crate::conversation::Conversation
//! [`AgentShortMemory`]: crate::conversation::AgentShortMemory
//! [`Role`]: crate::conversation::Role
//! [`Content`]: crate::conversation::Content
//!
//! ### Example: Managing a Conversation
//!
//! ```rust
//! use rigs::conversation::{Conversation, Role, Content};
//!
//! // Create a new conversation with an agent
//! let mut conversation = Conversation::new("MyAssistant".to_string());
//!
//! // Add messages to the conversation
//! conversation.add(Role::User("User".to_string()), "Hello, how are you?".to_string());
//! conversation.add(Role::Assistant("MyAssistant".to_string()), "I'm doing well, thank you for asking!".to_string());
//!
//! // Search for messages containing a keyword
//! let results = conversation.search("well");
//! ```
//!
//! ## Orchestrating Workflows
//!
//! The [`graph_workflow`] module provides a powerful system for creating complex agent workflows:
//!
//! * [`DAGWorkflow`] for defining directed acyclic graphs of agent interactions.
//! * Tools for connecting agents and defining the flow of information between them.
//! * Execution engines for running workflows with multiple starting agents.
//!
//! [`graph_workflow`]: crate::graph_workflow
//! [`DAGWorkflow`]: crate::graph_workflow::DAGWorkflow
//!
//! ### Example: Creating a Simple Workflow
//!
//! ```rust
//! use std::sync::Arc;
//! use rigs::graph_workflow::{DAGWorkflow, Flow};
//! use rigs::agent::Agent;
//!
//! // Create a new workflow
//! let mut workflow = DAGWorkflow::new("MyWorkflow", "A simple workflow example");
//!
//! // Register agents with the workflow
//! workflow.register_agent(Arc::new(agent1));
//! workflow.register_agent(Arc::new(agent2));
//! workflow.register_agent(Arc::new(agent3));
//!
//! // Connect agents in the workflow
//! workflow.connect_agents("agent1", "agent2", Flow::default())
//!     .expect("Failed to connect agents");
//! workflow.connect_agents("agent1", "agent3", Flow::default())
//!     .expect("Failed to connect agents");
//!
//! // Execute the workflow with multiple starting agents
//! let results = workflow.execute_workflow(&["agent1"], "Initial input")
//!     .await
//!     .expect("Failed to execute workflow");
//! ```
//!
//! ## Data Persistence
//!
//! The [`persistence`] module provides utilities for saving and loading data:
//!
//! * Functions for saving data to files and loading from files.
//! * Compression and decompression utilities.
//! * Error handling with [`PersistenceError`].
//!
//! [`persistence`]: crate::persistence
//! [`PersistenceError`]: crate::persistence::PersistenceError
//!
//! ### Example: Saving and Loading Data
//!
//! ```rust
//! use rigs::persistence;
//! use std::path::Path;
//!
//! async fn example() -> Result<(), persistence::PersistenceError> {
//!     // Save data to a file
//!     let data = "Hello, world!";
//!     persistence::save_to_file(data.as_bytes(), Path::new("hello.txt")).await?;
//!
//!     // Load data from a file
//!     let loaded_data = persistence::load_from_file(Path::new("hello.txt")).await?;
//!     
//!     // Compress data
//!     let compressed = persistence::compress(data.as_bytes())?;
//!     
//!     // Decompress data
//!     let decompressed = persistence::decompress(&compressed)?;
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Pre-built Agent Implementations
//!
//! The [`rig_agent`] module provides ready-to-use agent implementations:
//!
//! * [`RigAgent`] for creating agents based on the Rig framework.
//! * [`RigAgentBuilder`] for configuring and building Rig agents.
//!
//! [`rig_agent`]: crate::rig_agent
//! [`RigAgent`]: crate::rig_agent::RigAgent
//! [`RigAgentBuilder`]: crate::rig_agent::RigAgentBuilder
//!
//! ## Team Workflows
//!
//! The [`team_workflow`] module provides a higher-level abstraction for creating team-based workflows:
//!
//! * [`TeamWorkflow`] for defining team-based workflows with a leader agent.
//! * Model registry for managing different LLM models.
//! * Orchestration tools for dynamically creating and connecting agents.
//!
//! [`team_workflow`]: crate::team_workflow
//! [`TeamWorkflow`]: crate::team_workflow::TeamWorkflow
//!
//! For more examples, see the examples/ directory in the repository.
//!

pub mod agent;
pub mod conversation;
pub mod graph_workflow;
pub mod llm_provider;
pub mod persistence;
pub mod rig_agent;
pub mod team_workflow;

pub use rig;

use rig::providers::{anthropic, deepseek};
pub use rigs_macro::tool;

pub trait ProviderClient {
    fn completion_model(&self, model: impl Into<String>) -> impl rig::completion::CompletionModel;
}

impl ProviderClient for deepseek::Client {
    fn completion_model(&self, model: impl Into<String>) -> impl rig::completion::CompletionModel {
        self.completion_model(model.into().as_str())
    }
}
impl ProviderClient for anthropic::Client {
    fn completion_model(&self, model: impl Into<String>) -> impl rig::completion::CompletionModel {
        self.completion_model(model.into().as_str())
    }
}
