use std::{
    hash::{Hash, Hasher},
    path::Path,
    sync::Arc,
};

use futures::{StreamExt, future::BoxFuture, stream};
use rig::completion::{Chat, Prompt};
use rig::tool::Tool;
use serde::Serialize;
use tokio::sync::mpsc;
use twox_hash::XxHash3_64;

use crate::{
    agent::{Agent, AgentConfig, AgentError},
    conversation::{AgentShortMemory, Conversation, Role},
    persistence,
};

pub struct RigAgentBuilder<M>
where
    M: rig::completion::CompletionModel,
{
    rig_agent_builder: rig::agent::AgentBuilder<M>,
    config: AgentConfig,
    system_prompt: Option<String>,
    long_term_memory: Option<Arc<dyn rig::vector_store::VectorStoreIndexDyn>>,
}

impl<M> RigAgentBuilder<M>
where
    M: rig::completion::CompletionModel,
{
    pub fn new_with_model(model: M) -> Self {
        Self {
            rig_agent_builder: rig::agent::AgentBuilder::new(model),
            config: AgentConfig::default(),
            system_prompt: None,
            long_term_memory: None,
        }
    }

    pub fn config(mut self, config: AgentConfig) -> Self {
        self.config = config;
        self
    }

    pub fn system_prompt(mut self, system_prompt: impl Into<String>) -> Self {
        let system_prompt = system_prompt.into();
        self.config.system_prompt = system_prompt.clone();
        self.system_prompt = Some(system_prompt);
        self
    }

    pub fn long_term_memory(
        mut self,
        long_term_memory: impl Into<Option<Arc<dyn rig::vector_store::VectorStoreIndexDyn>>>,
    ) -> Self {
        self.long_term_memory = long_term_memory.into();
        self
    }

    pub fn add_tool(mut self, tool: impl Tool + 'static) -> Self {
        self.rig_agent_builder = self.rig_agent_builder.tool(tool);
        self
    }

    pub fn build(self) -> RigAgent<M> {
        let rig_agent = self
            .rig_agent_builder
            .preamble(
                &self
                    .system_prompt
                    .unwrap_or("You are a helpful assistant.".to_owned()),
            )
            .temperature(self.config.temperature)
            .max_tokens(self.config.max_tokens)
            .build();

        RigAgent {
            agent: Arc::new(rig_agent),
            config: self.config,
            short_memory: AgentShortMemory::new(),
            long_term_memory: self.long_term_memory,
        }
    }

    // Configuration methods

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

    pub fn max_tokens(mut self, max_tokens: u64) -> Self {
        self.config.max_tokens = max_tokens;
        self
    }

    pub fn max_loops(mut self, max_loops: u32) -> Self {
        self.config.max_loops = max_loops;
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

    pub fn save_state_dir(mut self, path: impl Into<String>) -> Self {
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
}

/// Wrapper for rig's Agent
#[derive(Clone, Serialize)]
pub struct RigAgent<M>
where
    M: rig::completion::CompletionModel,
{
    #[serde(skip)]
    agent: Arc<rig::agent::Agent<M>>,
    config: AgentConfig,
    short_memory: AgentShortMemory,
    #[serde(skip)]
    long_term_memory: Option<Arc<dyn rig::vector_store::VectorStoreIndexDyn>>,
}

impl<M> RigAgent<M>
where
    M: rig::completion::CompletionModel,
{
    /// Get the builder
    pub fn builder(model: M) -> RigAgentBuilder<M> {
        RigAgentBuilder::new_with_model(model)
    }

    /// Handle error in attempts
    async fn handle_error_in_attempts(&self, task: &str, error: AgentError, attempt: u32) {
        let err_msg = format!("Attempt {}, task: {}, failed: {}", attempt + 1, task, error);
        tracing::error!(err_msg);

        if self.config.autosave {
            let _ = self.save_task_state(task.to_owned()).await.map_err(|e| {
                tracing::error!(
                    "Failed to save agent<{}> task<{}>,  state: {}",
                    self.config.name,
                    task,
                    e
                )
            });
        }
    }

    fn plan(&self, task: String) -> BoxFuture<Result<(), AgentError>> {
        Box::pin(async move {
            if let Some(planning_prompt) = &self.config.planning_prompt {
                let planning_prompt = format!("{} {}", planning_prompt, task);
                let plan = self.agent.prompt(planning_prompt).await?;
                tracing::debug!("Plan: {}", plan);
                // Add plan to memory
                self.short_memory.add(
                    task,
                    self.config.name.clone(),
                    Role::Assistant(self.config.name.clone()),
                    plan,
                );
            };
            Ok(())
        })
    }

    fn query_long_term_memory(&self, task: String) -> BoxFuture<Result<(), AgentError>> {
        Box::pin(async move {
            if let Some(long_term_memory) = &self.long_term_memory {
                let (_score, _id, memory_retrieval) = &long_term_memory.top_n(&task, 1).await?[0];
                let memory_retrieval = format!("Documents Available: {memory_retrieval}");
                self.short_memory.add(
                    task,
                    &self.config.name,
                    Role::User("[RAG] Database".to_owned()),
                    memory_retrieval,
                );
            }

            Ok(())
        })
    }

    /// Save the agent state to a file
    fn save_task_state(&self, task: String) -> BoxFuture<Result<(), AgentError>> {
        let mut hasher = XxHash3_64::default();
        task.hash(&mut hasher);
        let task_hash = hasher.finish();
        let task_hash = format!("{:x}", task_hash & 0xFFFFFFFF); // lower 32 bits of the hash

        Box::pin(async move {
            let save_state_path = self.config.save_state_dir.clone();
            if let Some(save_state_path) = save_state_path {
                let save_state_path = Path::new(&save_state_path);
                if !save_state_path.exists() {
                    tokio::fs::create_dir_all(save_state_path).await?;
                }

                let path = save_state_path
                    .join(format!("{}_{}", self.name(), task_hash))
                    .with_extension("json");

                let json = serde_json::to_string_pretty(&self.short_memory.0.get(&task).unwrap())
                    .map_err(|e| AgentError::JsonError {
                    detail: "Failed to serialize short memory to JSON string when saving agent's task state".into(),
                    source: e,
                })?; // TODO: Safety?
                persistence::save_to_file(&json, path).await.map_err(|e| {
                    AgentError::PersistenceError {
                        detail: "Failed to save agent's task state to file".into(),
                        source: e,
                    }
                })?;
            }
            Ok(())
        })
    }

    fn is_response_complete(&self, response: String) -> bool {
        self.config
            .stop_words
            .iter()
            .any(|word| response.contains(word))
    }
}

impl<M> Agent for RigAgent<M>
where
    M: rig::completion::CompletionModel,
{
    fn run(&self, task: String) -> BoxFuture<Result<String, AgentError>> {
        Box::pin(async move {
            // Add task to short memory
            self.short_memory.add(
                &task,
                &self.config.name,
                Role::User(self.config.user_name.clone()),
                &task,
            );

            // Plan
            if self.config.plan_enabled {
                self.plan(task.clone()).await?;
            }

            // Query long term memory
            if self.long_term_memory.is_some() {
                self.query_long_term_memory(task.clone()).await?;
            }

            // Save state
            if self.config.autosave {
                self.save_task_state(task.clone()).await?;
            }

            // Run agent loop
            let mut last_response = String::new();
            let mut all_responses = vec![];
            for _loop_count in 0..self.config.max_loops {
                let mut success = false;
                let task_prompt = self.short_memory.0.get(&task).unwrap().to_string(); // Safety: task is in short_memory
                for attempt in 0..self.config.retry_attempts {
                    if success {
                        break;
                    }

                    if self.long_term_memory.is_some() && self.config.rag_every_loop {
                        // FIXME: if RAG success, but then LLM fails, then RAG is not removed and maybe causes issues
                        if let Err(e) = self.query_long_term_memory(task_prompt.clone()).await {
                            self.handle_error_in_attempts(&task, e, attempt).await;
                            continue;
                        };
                    }

                    // Generate response using LLM
                    let history = (&(*self.short_memory.0.get(&task).unwrap())).into(); // Safety: task is in short_memory
                    last_response = match self.agent.chat(task.clone(), history).await {
                        Ok(response) => response,
                        Err(e) => {
                            self.handle_error_in_attempts(&task, e.into(), attempt)
                                .await;
                            continue;
                        }
                    };

                    // Add response to memory
                    self.short_memory.add(
                        &task,
                        &self.config.name,
                        Role::Assistant(self.config.name.to_owned()),
                        last_response.clone(),
                    );

                    // Add response to all_responses
                    all_responses.push(last_response.clone());

                    // TODO: evaluate response
                    // TODO: Sentiment analysis

                    success = true;
                }

                if !success {
                    // Exit the loop if all retry failed
                    break;
                }

                if self.is_response_complete(last_response.clone()) {
                    break;
                }

                // TODO: Loop interval, maybe add a sleep here
            }

            // TODO: Apply the cleaning function to the responses
            // clean and add to short memory. role: Assistant(Output Cleaner)

            // Save state
            if self.config.autosave {
                self.save_task_state(task.clone()).await?;
            }

            // TODO: Handle artifacts

            // TODO: More flexible output types, e.g. JSON, CSV, etc.
            Ok(all_responses.concat())
        })
    }

    fn run_multiple_tasks(
        &mut self,
        tasks: Vec<String>,
    ) -> BoxFuture<Result<Vec<String>, AgentError>> {
        let agent_name = self.name();
        let mut results = Vec::with_capacity(tasks.len());

        Box::pin(async move {
            let agent_arc = Arc::new(self);
            let (tx, mut rx) = mpsc::channel(1);
            stream::iter(tasks)
                .for_each_concurrent(None, |task| {
                    let tx = tx.clone();
                    let agent = Arc::clone(&agent_arc);
                    async move {
                        let result = agent.run(task.clone()).await;
                        tx.send((task, result)).await.unwrap(); // Safety: we know rx is not dropped
                    }
                })
                .await;
            drop(tx);

            while let Some((task, result)) = rx.recv().await {
                match result {
                    Ok(result) => {
                        results.push(result);
                    }
                    Err(e) => {
                        tracing::error!("| Agent: {} | Task: {} | Error: {}", agent_name, task, e);
                    }
                }
            }

            Ok(results)
        })
    }

    fn id(&self) -> String {
        self.config.id.clone()
    }

    fn name(&self) -> String {
        self.config.name.clone()
    }

    fn description(&self) -> String {
        self.config.description.clone().unwrap_or_default()
    }
}

impl From<&Conversation> for Vec<rig::message::Message> {
    fn from(conv: &Conversation) -> Self {
        conv.history
            .iter()
            .map(|msg| match &msg.role {
                Role::User(name) => {
                    rig::message::Message::user(format!("{}: {}", name, msg.content))
                }
                Role::Assistant(name) => {
                    rig::message::Message::assistant(format!("{}: {}", name, msg.content))
                }
            })
            .collect()
    }
}
