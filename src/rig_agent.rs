use std::{
    hash::{Hash, Hasher},
    path::Path,
    sync::Arc,
    vec,
};

use futures::{StreamExt, future::BoxFuture, stream};
use rig::{
    agent::AgentBuilder,
    providers::{anthropic, deepseek, gemini, openrouter},
    tool::Tool,
};
use rig::{
    completion::{Chat, Prompt},
    providers::openai,
};
use serde::Serialize;
use tokio::sync::mpsc;
use twox_hash::XxHash3_64;

use crate::{
    agent::{Agent, AgentConfig, AgentError},
    conversation::{AgentShortMemory, Conversation, Role},
    llm_provider::LLMProvider,
    persistence,
};

pub struct RigAgentBuilder<M: rig::completion::CompletionModel> {
    agent_builder: Option<AgentBuilder<M>>,
    config: AgentConfig,
    system_prompt: Option<String>,
    long_term_memory: Option<Arc<dyn rig::vector_store::VectorStoreIndexDyn>>,
}

impl<M: rig::completion::CompletionModel> RigAgentBuilder<M> {
    pub fn new() -> Self {
        Self {
            agent_builder: None,
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

    pub fn tool(mut self, tool: impl Tool + 'static) -> Result<Self, AgentError> {
        let Some(agent_builder) = self.agent_builder else {
            return Err(AgentError::AgentBuilderNotInitialized);
        };
        self.agent_builder = Some(agent_builder.tool(tool));
        Ok(self)
    }

    pub fn build(self) -> Result<RigAgent<impl rig::completion::CompletionModel>, AgentError> {
        let Some(agent_builder) = self.agent_builder else {
            return Err(AgentError::AgentBuilderNotInitialized);
        };

        let config = self.config.clone();
        let short_memory = AgentShortMemory::new();
        let long_term_memory = self.long_term_memory.clone();
        let system_prompt = self.system_prompt.clone();

        let rig_agent = agent_builder
            .preamble(&system_prompt.unwrap_or("You are a helpful assistant.".to_owned()))
            .temperature(self.config.temperature)
            .max_tokens(self.config.max_tokens)
            .build();

        Ok(RigAgent {
            agent: Arc::new(rig_agent),
            config,
            short_memory,
            long_term_memory,
        })
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

impl<M: rig::completion::CompletionModel> Default for RigAgentBuilder<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl RigAgentBuilder<anthropic::completion::CompletionModel> {
    pub fn provider(mut self, provider: LLMProvider) -> Result<Self, AgentError> {
        let model_config = provider.get_config();
        self.config.model_name = model_config.model.clone();
        self.agent_builder = Some(provider.get_anthropic_agent_builder()?);
        Ok(self)
    }
}

impl RigAgentBuilder<deepseek::DeepSeekCompletionModel> {
    pub fn provider(mut self, provider: LLMProvider) -> Result<Self, AgentError> {
        let model_config = provider.get_config();
        self.config.model_name = model_config.model.clone();
        self.agent_builder = Some(provider.get_deep_seek_agent_builder()?);
        Ok(self)
    }
}

impl RigAgentBuilder<gemini::completion::CompletionModel> {
    pub fn provider(mut self, provider: LLMProvider) -> Result<Self, AgentError> {
        let model_config = provider.get_config();
        self.config.model_name = model_config.model.clone();
        self.agent_builder = Some(provider.get_gemini_agent_builder()?);
        Ok(self)
    }
}

impl RigAgentBuilder<openai::CompletionModel> {
    pub fn provider(mut self, provider: LLMProvider) -> Result<Self, AgentError> {
        let model_config = provider.get_config();
        self.config.model_name = model_config.model.clone();
        self.agent_builder = Some(provider.get_open_a_i_agent_builder()?);
        Ok(self)
    }
}

impl RigAgentBuilder<openrouter::CompletionModel> {
    pub fn provider(mut self, provider: LLMProvider) -> Result<Self, AgentError> {
        let model_config = provider.get_config();
        self.config.model_name = model_config.model.clone();
        self.agent_builder = Some(provider.get_open_router_agent_builder()?);
        Ok(self)
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

impl RigAgent<anthropic::completion::CompletionModel> {
    pub fn anthropic_builder() -> RigAgentBuilder<anthropic::completion::CompletionModel> {
        RigAgentBuilder::new()
    }
}

impl RigAgent<deepseek::DeepSeekCompletionModel> {
    pub fn deepseek_builder() -> RigAgentBuilder<deepseek::DeepSeekCompletionModel> {
        RigAgentBuilder::new()
    }
}

impl RigAgent<gemini::completion::CompletionModel> {
    pub fn gemini_builder() -> RigAgentBuilder<gemini::completion::CompletionModel> {
        RigAgentBuilder::new()
    }
}

impl RigAgent<openai::CompletionModel> {
    pub fn openai_builder() -> RigAgentBuilder<openai::CompletionModel> {
        RigAgentBuilder::new()
    }
}

impl RigAgent<openrouter::CompletionModel> {
    pub fn openrouter_builder() -> RigAgentBuilder<openrouter::CompletionModel> {
        RigAgentBuilder::new()
    }
}

impl<M> RigAgent<M>
where
    M: rig::completion::CompletionModel,
{
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

    async fn plan(&self, task: String) -> Result<(), AgentError> {
        if let Some(planning_prompt) = &self.config.planning_prompt {
            let planning_prompt = format!("{planning_prompt} {task}");
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
    }

    async fn query_long_term_memory(&self, task: String) -> Result<(), AgentError> {
        if let Some(long_term_memory) = &self.long_term_memory {
            let (_score, _id, memory_retrieval) = &long_term_memory.top_n(&task, 1).await?[0];
            let memory_retrieval = format!("Documents Available: {memory_retrieval}");
            self.short_memory.add(
                task,
                &self.config.name,
                Role::Assistant("[RAG] Database".to_owned()),
                memory_retrieval,
            );
        }

        Ok(())
    }

    /// Save the agent state to a file
    async fn save_task_state(&self, task: String) -> Result<(), AgentError> {
        let mut hasher = XxHash3_64::default();
        task.hash(&mut hasher);
        let task_hash = hasher.finish();
        let task_hash = format!("{:x}", task_hash & 0xFFFFFFFF); // lower 32 bits of the hash

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
    fn run(&self, task: String) -> BoxFuture<'_, Result<String, AgentError>> {
        Box::pin(async move {
            // Add task to memory
            self.short_memory.add(
                &task,
                &self.config.name,
                Role::User(self.config.user_name.clone()),
                task.clone(),
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
            if self.config.autosave && !self.short_memory.0.is_empty() {
                self.save_task_state(task.clone()).await?;
            }

            // Run agent loop
            let mut last_response = String::new();
            let mut all_responses = vec![];
            for loop_count in 0..self.config.max_loops {
                let mut success = false;
                for attempt in 0..self.config.retry_attempts {
                    if success {
                        break;
                    }

                    if self.long_term_memory.is_some() && self.config.rag_every_loop {
                        // FIXME: if RAG success, but then LLM fails, then RAG is not removed and maybe causes issues
                        if let Err(e) = self.query_long_term_memory(task.clone()).await {
                            self.handle_error_in_attempts(&task, e, attempt).await;
                            continue;
                        };
                    }

                    // Generate response using LLM
                    let mut history = (&(*self
                        .short_memory
                        .0
                        .entry(task.clone())
                        .or_insert(Conversation::new(self.name()))))
                        .into();

                    // Since rig's agent requires concatenating prompt and chat_history,
                    // this would cause the initial prompt to be duplicated.
                    // Here we check if it's the first loop by verifying loop_count == 0
                    // If it's the first loop, use empty chat_history
                    if loop_count == 0 {
                        history = vec![];
                    }

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
    ) -> BoxFuture<'_, Result<Vec<String>, AgentError>> {
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
