use std::{
    collections::HashMap,
    fmt::Display,
    path::{Path, PathBuf},
};

use chrono::Local;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::persistence::{self, PersistenceError};

/// A [AgentShortMemory] is a struct that stores multiple conversations.
/// It is a map from `Task` to [Conversation]. `Task` is a string, usually the first message from the user.
#[derive(Clone, Serialize)]
pub struct AgentShortMemory(pub DashMap<String, Conversation>);

impl AgentShortMemory {
    pub fn new() -> Self {
        Self(DashMap::new())
    }

    /// Add a [Conversation] to the agent short memory.
    ///
    /// # Arguments
    ///
    /// * `task` - The task that the conversation is for.
    /// * `conversation` - The conversation to add.
    /// * `conversation_owner` - The owner of the conversation.
    /// * `role` - The role of the message, which will be added to the conversation.
    /// * `message` - The message to add.
    pub fn add(
        &self,
        task: impl Into<String>,
        conversation_owner: impl Into<String>,
        role: Role,
        message: impl Into<String>,
    ) {
        let mut conversation = self
            .0
            .entry(task.into())
            .or_insert(Conversation::new(conversation_owner.into()));
        conversation.add(role, message.into())
    }
}

impl Default for AgentShortMemory {
    fn default() -> Self {
        Self::new()
    }
}

/// A [Conversation] is a struct that stores a list of messages.
/// This is an Agent's memory during a task. If other agents participate in the task,
/// the conversation can also contain the messages from other agents.
/// Because [Role] is not a string, it can be used to identify the sender of a message.
#[derive(Clone, Serialize)]
pub struct Conversation {
    agent_name: String,
    save_filepath: Option<PathBuf>,
    pub history: Vec<Message>,
}

impl Conversation {
    pub fn new(agent_name: String) -> Self {
        Self {
            agent_name,
            save_filepath: None,
            history: Vec::new(),
        }
    }

    /// Add a message to the conversation history.
    pub fn add(&mut self, role: Role, message: String) {
        let timestamp = Local::now().timestamp();
        let message = Message {
            role,
            content: Content::Text(format!("Time: {timestamp} \n{message}")),
        };
        self.history.push(message);

        if let Some(filepath) = &self.save_filepath {
            let filepath = filepath.clone();
            let history = self.history.clone();
            tokio::spawn(async move {
                let history = history;
                let _ = Self::save_as_json(&filepath, &history).await;
            });
        }
    }

    /// Delete a message from the conversation history.
    pub fn delete(&mut self, index: usize) {
        self.history.remove(index);
    }

    /// Update a message in the conversation history.
    pub fn update(&mut self, index: usize, role: Role, content: Content) {
        self.history[index] = Message { role, content };
    }

    /// Query a message in the conversation history.
    pub fn query(&self, index: usize) -> &Message {
        &self.history[index]
    }

    /// Search for a message in the conversation history.
    pub fn search(&self, keyword: &str) -> Vec<&Message> {
        self.history
            .iter()
            .filter(|message| message.content.to_string().contains(keyword))
            .collect()
    }

    // Clear the conversation history.
    pub fn clear(&mut self) {
        self.history.clear();
    }

    /// Convert the conversation history to a JSON string.
    pub fn to_json(&self) -> Result<String, ConversationError> {
        Ok(serde_json::to_string(&self.history)?)
    }

    /// Save the conversation history to a JSON file.
    async fn save_as_json(filepath: &Path, data: &[Message]) -> Result<(), ConversationError> {
        let json_data = serde_json::to_string_pretty(data)?;
        persistence::save_to_file(json_data.as_bytes(), filepath).await?;
        Ok(())
    }

    /// Export the conversation history to a file, the content of the file can be imported by `import_from_file`
    pub async fn export_to_file(&self, filepath: &Path) -> Result<(), ConversationError> {
        let data = self.to_string();
        persistence::save_to_file(data.as_bytes(), filepath).await?;
        Ok(())
    }

    /// Import the conversation history from a file, the content of the file should be exported by `export_to_file`
    pub async fn import_from_file(&mut self, filepath: &Path) -> Result<(), ConversationError> {
        let data = persistence::load_from_file(filepath).await?;
        let history = data
            .split(|s| *s == b'\n')
            .map(|line| {
                let line = String::from_utf8_lossy(line);
                // M4n5ter(User): hello
                let (role, content) = line.split_once(": ").unwrap();
                if role.contains("(User)") {
                    let role = Role::User(role.replace("(User)", "").to_string());
                    let content = Content::Text(content.to_owned());
                    Message { role, content }
                } else {
                    let role = Role::Assistant(role.replace("(Assistant)", "").to_string());
                    let content = Content::Text(content.to_owned());
                    Message { role, content }
                }
            })
            .collect();
        self.history = history;
        Ok(())
    }

    /// Count the number of messages by role
    pub fn count_messages_by_role(&self) -> HashMap<String, usize> {
        let mut count = HashMap::new();
        for message in &self.history {
            *count.entry(message.role.to_string()).or_insert(0) += 1;
        }
        count
    }
}

#[derive(Debug, Error)]
pub enum ConversationError {
    #[error("Json error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("FilePersistence error: {0}")]
    FilePersistenceError(#[from] PersistenceError),
}

/// A [Message] consists of a [Role] and a [Content].
#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Content,
}

/// A [Role] is a string that identifies the sender of a message.
#[derive(Clone, Serialize, Deserialize)]
pub enum Role {
    User(String),
    Assistant(String),
}

#[derive(Clone, Serialize, Deserialize)]
pub enum Content {
    Text(String),
}

impl Display for Conversation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for message in &self.history {
            writeln!(f, "{}: {}", message.role, message.content)?;
        }
        Ok(())
    }
}

impl Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::User(name) => write!(f, "{name}(User)"),
            Role::Assistant(name) => write!(f, "{name}(Assistant)"),
        }
    }
}

impl Display for Content {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Content::Text(text) => f.pad(text),
        }
    }
}
