use rig::{
    agent::AgentBuilder,
    providers::{
        anthropic,
        deepseek::{self, DeepSeekCompletionModel},
        gemini, openai, openrouter,
    },
};
use thiserror::Error;

#[derive(Clone)]
pub enum LLMProvider {
    Anthropic(ModelConfig),
    DeepSeek(ModelConfig),
    Gemini(ModelConfig),
    OpenAI(ModelConfig),
    OpenRouter(ModelConfig),
}

macro_rules! impl_agent_builder {
    ($method:ident, $variant:ident, $client:ty, $model:ty) => {
        pub fn $method(&self) -> Result<AgentBuilder<$model>, LLMProviderError> {
            let LLMProvider::$variant(config) = self else {
                return Err(LLMProviderError::LLMProviderNotMatch);
            };
            let client = <$client>::from_env();
            Ok(client.agent(&config.model))
        }
    };
}

macro_rules! impl_agent_builder_auto {
    ($variant:ident, $client:ty, $model:ty) => {
        paste::paste! {
            impl_agent_builder!(
                [<get_ $variant:snake _agent_builder>],
                $variant,
                $client,
                $model
            );
        }
    };
}

impl LLMProvider {
    pub fn anthropic(model: impl Into<String>) -> Self {
        Self::Anthropic(ModelConfig {
            model: model.into(),
            stream: false,
        })
    }

    pub fn deepseek(model: impl Into<String>) -> Self {
        Self::DeepSeek(ModelConfig {
            model: model.into(),
            stream: false,
        })
    }

    pub fn gemini(model: impl Into<String>) -> Self {
        Self::Gemini(ModelConfig {
            model: model.into(),
            stream: false,
        })
    }

    pub fn openai(model: impl Into<String>) -> Self {
        Self::OpenAI(ModelConfig {
            model: model.into(),
            stream: false,
        })
    }

    pub fn openrouter(model: impl Into<String>) -> Self {
        Self::OpenRouter(ModelConfig {
            model: model.into(),
            stream: false,
        })
    }

    pub fn get_config(&self) -> &ModelConfig {
        match self {
            LLMProvider::Anthropic(config)
            | LLMProvider::DeepSeek(config)
            | LLMProvider::Gemini(config)
            | LLMProvider::OpenAI(config)
            | LLMProvider::OpenRouter(config) => config,
        }
    }

    impl_agent_builder_auto!(
        Anthropic,
        anthropic::Client,
        anthropic::completion::CompletionModel
    );

    impl_agent_builder_auto!(DeepSeek, deepseek::Client, DeepSeekCompletionModel);

    impl_agent_builder_auto!(Gemini, gemini::Client, gemini::completion::CompletionModel);

    impl_agent_builder_auto!(OpenAI, openai::Client, openai::CompletionModel);

    impl_agent_builder_auto!(OpenRouter, openrouter::Client, openrouter::CompletionModel);
}

#[derive(Clone)]
pub struct ModelConfig {
    pub model: String,
    pub stream: bool,
}

#[derive(Debug, Error)]
pub enum LLMProviderError {
    #[error("LLM provider not match")]
    LLMProviderNotMatch,
}
