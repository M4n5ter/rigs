use rigs::llm_provider::LLMProvider;
use rigs::rig_agent::RigAgent;
use rigs::team_workflow::{ModelDescription, TeamWorkflow};
use std::error::Error;
use std::sync::Arc;

// This example demonstrates how to use the TeamWorkflow system
// to orchestrate a team of agents led by a leader agent
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv::dotenv().ok();
    // Initialize tracing for logging
    tracing_subscriber::fmt::init();

    // Create a new TeamWorkflow
    let mut team = TeamWorkflow::new(
        "Research Team",
        "A team of agents that collaborate on research tasks",
    );

    // Register models with the model registry

    // deepseek-reasoner doesn't support tools yet, and also doesn't support continuous output of the same role.
    let reasoning_model = LLMProvider::deepseek("deepseek-reasoner");
    let chat_model = LLMProvider::deepseek("deepseek-chat");
    team.register_model(
        "reasoning",
        reasoning_model,
        ModelDescription {
            name: "reasoning".to_owned(),
            description: "A model optimized for reasoning and planning".to_owned(),
            capabilities: vec!["reasoning".to_owned(), "planning".to_owned()],
            context_window: 16000,
            max_tokens: 4000,
        },
    );
    team.register_model(
        "chat",
        chat_model,
        ModelDescription {
            name: "chat".to_owned(),
            description: "A model optimized for chat interactions".to_owned(),
            capabilities: vec!["chat".to_owned(), "conversational".to_owned()],
            context_window: 16000,
            max_tokens: 4000,
        },
    );

    // Get the model from the registry
    let (leader_model, _) = team.get_model("chat")?;

    // Build the leader agent
    // IMPORTANT: default_system_prompt and default_tool must be set. If not set, team workflow won't work correctly.
    let (default_system_prompt, default_tool) = team.default_leader_system_prompt_and_tool();
    let leader = RigAgent::deepseek_builder()
        .provider(leader_model)?
        .agent_name("Leader")
        .description("A leader agent that orchestrates the team")
        .system_prompt(default_system_prompt)
        .tool(default_tool)?
        .save_state_dir("/temp/leader")
        .enable_autosave()
        .temperature(0.5)
        .build()?;

    // Set the leader agent
    team.set_leader(Arc::new(leader));

    // Execute the workflow with a task
    let result = team
        .execute(
            r#"
            Problem: "Quantum-Bio-Political" Interstellar Colony Crisis

            Background:
            Humanity's first interstellar colony (Alpha Centauri) faces a tripartite emergency:

            1. QUANTUM COMMUNICATIONS COLLAPSE:
            - Colony relies on quantum entanglement comms (4.3-year light delay from Earth)
            - Quantum predictive model shows:
                * 89% probability of quantum decoherence storm within 72 hours
                * Would disrupt all communications for 6+ months if occurs

            2. XENOBIOLOGICAL THREAT:
            - Alien microbe sample breach alert (93% confidence)
            - Potential outcomes:
                * 50%: Harmless symbiosis
                * 30%: Destroys quantum computer cryogenic systems
                * 20%: Infects human neural tissue

            3. POLITICAL DEADLOCK:
            - Earth Command orders immediate sample destruction
            - Colony scientists demand preservation (unique research opportunity)
            - Colonial council vote split: 45% compliance vs 55% resistance

            Decision Deadline: Must resolve within 36 Earth-hours

            Options:
            A) Activate quantum comms backup (consumes 80% energy reserves)
            - Risk: If no storm occurs → colony-wide energy famine
            B) Quarantine lab and continue research (requires quantum monitoring)
            - Risk: Microbe disrupts quantum systems → lose both comms and research
            C) Enforce sample destruction protocol
            - Risk: Triggers civil unrest (possible infrastructure damage)

            Critical Constraints:
            1. No real-time Earth guidance (4.3-year communication latency)
            2. All decisions irrevocably alter colony development paths
            3. Must produce:
            - Technical implementation plan
            - Public announcement script
            - Earth compliance report
            "#,
        )
        .await?;

    println!("Result: {result:#?}");

    println!("Dot graph: {}", team.get_workflow_dot());

    Ok(())
}
