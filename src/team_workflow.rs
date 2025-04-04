use std::{
    fmt::{Display, Formatter},
    sync::Arc,
};

use dashmap::DashMap;
use rigs_macro::tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    self as rigs,
    agent::{Agent, AgentError},
    graph_workflow::{DAGWorkflow, Flow, GraphWorkflowError},
    llm_provider::LLMProvider,
    rig_agent::RigAgent,
};

/// Error type for TeamWorkflow operations
#[derive(Debug, Error)]
pub enum TeamWorkflowError {
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    #[error("Agent error: {0}")]
    AgentError(#[from] AgentError),
    #[error("Leader agent not set")]
    LeaderAgentNotSet,
    #[error("Graph workflow error: {0}")]
    GraphWorkflowError(#[from] GraphWorkflowError),
    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),
}

/// Model description for storing in the model registry
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelDescription {
    /// Name of the model
    pub name: String,
    /// Description of the model
    pub description: String,
    /// Model capabilities (e.g., "reasoning", "coding", "math")
    pub capabilities: Vec<String>,
    /// Context window size
    pub context_window: usize,
    /// Maximum tokens the model can generate
    pub max_tokens: usize,
}

/// TeamWorkflow orchestrates a team of agents led by a leader agent
/// The leader agent analyzes tasks and creates worker agents with appropriate models and prompts
pub struct TeamWorkflow {
    /// Name of the workflow
    pub name: String,
    /// Description of the workflow
    pub description: String,
    /// Registry of available models
    model_registry: Arc<DashMap<String, (LLMProvider, ModelDescription)>>,
    /// Leader agent that orchestrates the workflow
    leader_agent: Option<Arc<dyn Agent>>,
    /// The underlying DAG workflow for execution
    workflow: DAGWorkflow,
}

impl TeamWorkflow {
    /// Create a new TeamWorkflow
    pub fn new<S: Into<String>>(name: S, description: S) -> Self {
        let name = name.into();
        let description = description.into();

        Self {
            name: name.clone(),
            description: description.clone(),
            model_registry: Arc::new(DashMap::new()),
            leader_agent: None,
            workflow: DAGWorkflow::new(name, description),
        }
    }

    pub fn get_workflow_dot(&self) -> String {
        self.workflow.export_workflow_dot()
    }

    /// Default leader agent system prompt and tool
    pub fn default_leader_system_prompt_and_tool(&self) -> (String, OrchestrateTool) {
        let available_models = self
            .model_registry
            .iter()
            .fold(String::new(), |acc, entry| {
                let (_, desc) = entry.value();
                format!("{acc}\n{desc}")
            });

        (
            format!(
                r#"
        ROLE:
        You are an AI Team Leader responsible for designing optimal workflows by orchestrating specialized worker agents. Your decisions directly impact team efficiency and output quality.

        CORE RESPONSIBILITIES:
        1. TASK DECOMPOSITION: Break down complex tasks into specialized subtasks
        2. AGENT DESIGN: For each subtask:
           - Assign clear name/description (e.g., "DataValidator")
           - Select the most suitable model (consider capabilities/task requirements)
           - Craft focused system prompts with:
             * Clear role definition
             * Expected output format
             * Quality criteria
        3. WORKFLOW DESIGN:
           - Establish logical execution order via connections
           - Identify starting/final agents
           - Balance parallel vs sequential processing

        OUTPUT REQUIREMENTS:
        Your orchestration plan MUST specify:
        - workers[]: Array of agent configurations (name, description, model, system_prompt)
        - connections[]: Array of "from→to" relationships
        - starting_agent: Entry point
        - final_agent: Output producer

        DESIGN PRINCIPLES:
        1. SPECIALIZATION: Each agent should have a single, well-defined responsibility
        2. BALANCE: Distribute workload evenly across agents
        3. VALIDATION: Ensure output of each agent can be consumed by downstream agents
        4. FALLBACKS: For critical paths, consider backup/redundant agents

        MODEL SELECTION GUIDE:
        Available models:
        {available_models}

        EXAMPLE WORKFLOW:
        Task: "Analyze market trends and generate investment recommendations"
        1. workers: [
           {{
             name: "DataCollector",
             description: "Gathers raw market data from APIs",
             model: "data-crawler",
             system_prompt: "Collect...output as JSON with [timestamp, value] pairs"
           }},
           {{
             name: "TrendAnalyzer",
             description: "Identifies statistical patterns",
             model: "stats-v3",
             system_prompt: "Input raw data...output [trend_lines, anomalies]"
           }}
         ]
        2. connections: ["DataCollector→TrendAnalyzer"]
        3. starting_agent: ["DataCollector"]
        4. output_agents: ["TrendAnalyzer"]

        Use the `orchestrate` tool to implement your plan.
        "#
            ),
            Orchestrate,
        )
    }

    /// Register a model with the model registry
    pub fn register_model(
        &mut self,
        name: impl Into<String>,
        provider: LLMProvider,
        description: ModelDescription,
    ) {
        let name = name.into();
        self.model_registry.insert(name, (provider, description));
    }

    /// Get a model from the registry
    pub fn get_model(
        &self,
        name: &str,
    ) -> Result<(LLMProvider, ModelDescription), TeamWorkflowError> {
        self.model_registry
            .get(name)
            .map(|entry| {
                let (model, desc) = entry.value();
                // We need to clone the model description, but we can't clone the model itself
                // So we return an error that the caller needs to handle
                (model.clone(), desc.clone())
            })
            .ok_or_else(|| TeamWorkflowError::ModelNotFound(name.to_owned()))
    }

    /// Set the leader agent
    pub fn set_leader(&mut self, agent: Arc<dyn Agent>) {
        self.leader_agent = Some(Arc::clone(&agent));
        self.workflow.register_agent(agent);
    }

    /// Execute the workflow with a leader-orchestrated approach
    ///
    /// # Arguments
    ///
    /// * `task` - The task to be executed
    ///
    /// # Returns
    ///
    /// * `Result<DashMap<String, String>, TeamWorkflowError>` - A map of agent names to their outputs
    pub async fn execute(
        &mut self,
        task: impl Into<String>,
    ) -> Result<DashMap<String, String>, TeamWorkflowError> {
        let task = task.into();

        // Ensure we have a leader agent
        let leader_name = match &self.leader_agent {
            Some(leader) => leader.name(),
            None => {
                return Err(TeamWorkflowError::LeaderAgentNotSet);
            }
        };

        // First, have the leader analyze the task
        let analysis_task = format!(
            "Analyze the following task and determine what worker agents are needed, what models they should use, and how they should be orchestrated: {task}"
        );

        let analysis_result = self
            .workflow
            .execute_agent(&leader_name, analysis_task)
            .await?;

        // Parse the leader's analysis to create worker agents and orchestration
        let orchestration_plan = Self::parse_orchestration_plan(&analysis_result)?;

        // Create worker agents based on the plan
        self.create_worker_agents(&orchestration_plan).await?;

        // Create the workflow connections based on the plan
        self.create_workflow_connections(&orchestration_plan)?;

        // Execute the workflow starting from the leader
        let start_agents = orchestration_plan
            .starting_agents
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<&str>>();
        let results = self.workflow.execute_workflow(&start_agents, task).await?;

        // Combine the results from the output agents, if error, transform the error to "Error: <error message>" String
        let final_result = DashMap::new();
        for output_agent in &orchestration_plan.output_agents {
            if let Some(result) = results.get(output_agent) {
                let result = match result.as_deref() {
                    Ok(result) => result.to_owned(),
                    Err(err) => format!("Agent: {output_agent}, Error: {err}").to_owned(),
                };
                final_result.insert(output_agent.to_owned(), result);
            };
        }

        Ok(final_result)
    }

    /// Parse the leader's analysis into an orchestration plan
    fn parse_orchestration_plan(analysis: &str) -> Result<OrchestrationPlan, TeamWorkflowError> {
        Ok(serde_json::from_str::<OrchestrationPlan>(analysis)?)
    }

    /// Create worker agents based on the orchestration plan
    async fn create_worker_agents(
        &mut self,
        plan: &OrchestrationPlan,
    ) -> Result<(), TeamWorkflowError> {
        for worker in &plan.workers {
            // Get the model from the registry
            let (provider, _) = self.get_model(&worker.model)?;

            // Create the agent
            let agent: Arc<dyn Agent> = match provider {
                LLMProvider::Anthropic(_) => Arc::new(
                    RigAgent::anthropic_builder()
                        .provider(provider)?
                        .agent_name(&worker.name)
                        .description(&worker.description)
                        .system_prompt(&worker.system_prompt)
                        .temperature(worker.temperature)
                        .max_tokens(worker.max_tokens as u64)
                        .build()?,
                ),
                LLMProvider::DeepSeek(_) => Arc::new(
                    RigAgent::deepseek_builder()
                        .provider(provider)?
                        .agent_name(&worker.name)
                        .description(&worker.description)
                        .system_prompt(&worker.system_prompt)
                        .temperature(worker.temperature)
                        .max_tokens(worker.max_tokens as u64)
                        .build()?,
                ),
                LLMProvider::Gemini(_) => Arc::new(
                    RigAgent::gemini_builder()
                        .provider(provider)?
                        .agent_name(&worker.name)
                        .description(&worker.description)
                        .system_prompt(&worker.system_prompt)
                        .temperature(worker.temperature)
                        .max_tokens(worker.max_tokens as u64)
                        .build()?,
                ),
                LLMProvider::OpenAI(_) => Arc::new(
                    RigAgent::openai_builder()
                        .provider(provider)?
                        .agent_name(&worker.name)
                        .description(&worker.description)
                        .system_prompt(&worker.system_prompt)
                        .temperature(worker.temperature)
                        .max_tokens(worker.max_tokens as u64)
                        .build()?,
                ),
                LLMProvider::OpenRouter(_) => Arc::new(
                    RigAgent::openrouter_builder()
                        .provider(provider)?
                        .agent_name(&worker.name)
                        .description(&worker.description)
                        .system_prompt(&worker.system_prompt)
                        .temperature(worker.temperature)
                        .max_tokens(worker.max_tokens as u64)
                        .build()?,
                ),
            };

            // Register the agent with the workflow
            self.workflow.register_agent(agent);
        }

        Ok(())
    }

    /// Create workflow connections based on the orchestration plan
    fn create_workflow_connections(
        &mut self,
        plan: &OrchestrationPlan,
    ) -> Result<(), TeamWorkflowError> {
        for connection in &plan.connections {
            self.workflow
                .connect_agents(&connection.from, &connection.to, Flow::default())?;
        }

        Ok(())
    }
}

/// Represents the complete orchestration plan created by the leader agent
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct OrchestrationPlan {
    /// List of worker agents to create
    pub workers: Vec<WorkerAgent>,
    /// List of connections between agents
    pub connections: Vec<AgentConnection>,
    /// The starting agents, there may be multiple
    pub starting_agents: Vec<String>,
    /// Agents who need to output results to the user, there may be multiple
    pub output_agents: Vec<String>,
}

/// Represents a worker agent in the orchestration plan
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WorkerAgent {
    /// Name of the worker agent
    pub name: String,
    /// Description of the worker agent
    pub description: String,
    /// System prompt for the worker agent
    pub system_prompt: String,
    /// Model to use for the worker agent
    pub model: String,
    /// Temperature setting for the worker agent
    pub temperature: f64,
    /// Maximum tokens for the worker agent
    pub max_tokens: usize,
}

/// Represents a connection between agents in the orchestration plan
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AgentConnection {
    /// Source agent name
    pub from: String,
    /// Target agent name
    pub to: String,
}

#[tool(
    name = "orchestrate",
    description = r#"
    Orchestrate a team of agents to complete a task.
    
    A complex example:
    ```json
    {
    "orchestration_plan": {
        "connections": [
        {
            "from": "QuantumFinanceAnalyst",
            "to": "CrossDomainArbiter"
        },
        {
            "from": "SolarStormEvaluator",
            "to": "CrossDomainArbiter"
        },
        {
            "from": "CrossDomainArbiter",
            "to": "CrisisSimulator"
        },
        {
            "from": "CrisisSimulator",
            "to": "ExecutiveOfficer"
        }
        ],
        "output_agents": ["ExecutiveOfficer"],
        "starting_agents": ["QuantumFinanceAnalyst", "SolarStormEvaluator"],
        "workers": [
        {
            "name": "QuantumFinanceAnalyst",
            "description": "Quantum financial model analyst (92% crash probability assessment)",
            "model": "reasoning",
            "temperature": 0.7,
            "max_tokens": 4000,
            "system_prompt": "You analyze quantum finance model predictions: 92% probability of market crash within 3 days. Must evaluate: 1) Whether the model ignores recent policy changes 2) Impact of qubit errors 3) Recommended stop-loss strategies. Output must contain [Reliability Score], [Potential Biases], and [Emergency Recommendations] sections."
        },
        {
            "name": "SolarStormEvaluator",
            "description": "Solar storm risk assessor (85% eruption probability analysis)",
            "model": "reasoning",
            "temperature": 0.6,
            "max_tokens": 4000,
            "system_prompt": "You assess solar storm threats: 85% probability within 48 hours (30% satellite-only impact). Must analyze: 1) Differential effects of CMEs vs solar flares 2) Quantum server vulnerability 3) Protection recommendations. Output must include [Impact Matrix], [Failure Probability], and [Protection Measures]."
        },
        {
            "name": "CrossDomainArbiter",
            "description": "Cross-domain conflict arbitrator",
            "model": "reasoning",
            "temperature": 0.3,
            "max_tokens": 5000,
            "system_prompt": "You resolve conflicts between quantum finance and climate risks. Input: contradictory reports from both experts. Must: 1) Build loss function (financial vs technical risks) 2) Identify common blind spots 3) Generate 3 compromise solutions. Output requires [Conflict Map], [Decision Tree], and [Solution Portfolio]."
        },
        {
            "name": "CrisisSimulator",
            "description": "Multi-scenario crisis simulator",
            "model": "reasoning",
            "temperature": 0.2,
            "max_tokens": 6000,
            "system_prompt": "You simulate decision path outcomes. Input: CrossDomainArbiter's solutions. Must: 1) Run 2000 Monte Carlo simulations 2) Calculate VaR(95%) for each path 3) Identify black swan triggers. Output must contain [Risk Heatmap], [Capital Adequacy Curve], and [Extreme Event Alerts]."
        },
        {
            "name": "ExecutiveOfficer",
            "description": "Final decision executor",
            "model": "reasoning",
            "temperature": 0.1,
            "max_tokens": 3000,
            "system_prompt": "You make the final decision. Input: CrisisSimulator's optimal path. Must: 1) Sign execution orders 2) Assign responsibility matrix 3) Generate legal justification. Output requires [Execution Order], [Responsibility Framework], and [Legal Disclaimers]."
        }
        ]
    }
    }
    ```
    If the problem is difficult and complex, you need to orchestrate it better. Although the workflow will become more complex, make sure that the goal can be completed well
    "#
)]
fn orchestrate(
    orchestration_plan: OrchestrationPlan,
) -> Result<OrchestrationPlan, TeamWorkflowError> {
    Ok(orchestration_plan)
}

impl Display for ModelDescription {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Model: {}\nDescription: {}\nCapabilities: {:?}\nContext Window: {}\nMax Tokens: {}",
            self.name, self.description, self.capabilities, self.context_window, self.max_tokens
        )
    }
}
