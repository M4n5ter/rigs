use futures::future::BoxFuture;
use rig::{completion::PromptError, vector_store::VectorStoreError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

use crate::persistence::PersistenceError;

pub trait Agent {
    /// Runs the autonomous agent loop to complete the given task.
    fn run(&self, task: String) -> BoxFuture<Result<String, AgentError>>;

    /// Run multiple tasks concurrently
    fn run_multiple_tasks(
        &mut self,
        tasks: Vec<String>,
    ) -> BoxFuture<Result<Vec<String>, AgentError>>;

    /// Get agent ID
    fn id(&self) -> String;

    /// Get agent name
    fn name(&self) -> String;

    /// Get agent description
    fn description(&self) -> String;
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Agent prompt error: {0}")]
    PromptError(#[from] PromptError),
    #[error("Vector store error: {0}")]
    VectorStoreError(#[from] VectorStoreError),
    #[error("JSON error, detail: {detail}, source: {source}")]
    JsonError {
        detail: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("Persistence error, detail: {detail}, source: {source}")]
    PersistenceError {
        detail: String,
        #[source]
        source: PersistenceError,
    },
    #[cfg(test)]
    #[error("Test error: {0}")]
    TestError(String),
}

#[derive(Clone)]
pub struct AgentConfigBuilder {
    config: AgentConfig,
}

impl AgentConfigBuilder {
    pub fn agent_name(mut self, name: impl Into<String>) -> Self {
        self.config.name = name.into();
        self
    }

    pub fn user_name(mut self, name: impl Into<String>) -> Self {
        self.config.user_name = name.into();
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.config.description = Some(description.into());
        self
    }

    pub fn temperature(mut self, temperature: f64) -> Self {
        self.config.temperature = temperature;
        self
    }

    pub fn max_loops(mut self, max_loops: u32) -> Self {
        self.config.max_loops = max_loops;
        self
    }

    pub fn max_tokens(mut self, max_tokens: u64) -> Self {
        self.config.max_tokens = max_tokens;
        self
    }

    pub fn enable_plan(mut self, planning_prompt: impl Into<Option<String>>) -> Self {
        self.config.plan_enabled = true;
        self.config.planning_prompt = planning_prompt.into();
        self
    }

    pub fn enable_autosave(mut self) -> Self {
        self.config.autosave = true;
        self
    }

    pub fn retry_attempts(mut self, retry_attempts: u32) -> Self {
        self.config.retry_attempts = retry_attempts;
        self
    }

    pub fn enable_rag_every_loop(mut self) -> Self {
        self.config.rag_every_loop = true;
        self
    }

    pub fn save_sate_path(mut self, path: impl Into<String>) -> Self {
        self.config.save_state_dir = Some(path.into());
        self
    }

    pub fn add_stop_word(mut self, stop_word: impl Into<String>) -> Self {
        self.config.stop_words.insert(stop_word.into());
        self
    }

    pub fn stop_words(self, stop_words: Vec<String>) -> Self {
        stop_words
            .into_iter()
            .fold(self, |builder, stop_word| builder.add_stop_word(stop_word))
    }

    pub fn build(self) -> AgentConfig {
        self.config
    }
}

/// Agent configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub user_name: String,
    pub model_name: String,
    pub system_prompt: String,
    pub description: Option<String>,
    pub temperature: f64,
    pub max_loops: u32,
    pub max_tokens: u64,
    pub plan_enabled: bool,
    pub planning_prompt: Option<String>,
    pub autosave: bool,
    pub retry_attempts: u32,
    pub rag_every_loop: bool,
    pub save_state_dir: Option<String>,
    pub stop_words: HashSet<String>,
}

impl AgentConfig {
    pub fn builder() -> AgentConfigBuilder {
        AgentConfigBuilder {
            config: AgentConfig::default(),
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Agent".to_owned(),
            user_name: "User".to_owned(),
            model_name: "gpt-3.5-turbo".to_owned(),
            system_prompt: "You are a helpful assistant.".to_owned(),
            description: None,
            temperature: 0.7,
            max_loops: 1,
            max_tokens: 8192,
            plan_enabled: false,
            planning_prompt: None,
            autosave: false,
            retry_attempts: 3,
            rag_every_loop: false,
            save_state_dir: None,
            stop_words: HashSet::new(),
        }
    }
}
