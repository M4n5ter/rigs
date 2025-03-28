use anyhow::Result;
use rig::providers::deepseek;
use rigs::{agent::Agent, rig_agent::RigAgent};
use rigs_macro::tool;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv().ok();

    let subscriber = tracing_subscriber::fmt::Subscriber::builder()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_line_number(true)
        .with_file(true)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let client = deepseek::Client::from_env();
    let model = client.completion_model("deepseek-chat");

    let agent = RigAgent::builder(model.clone())
        .system_prompt("You need to select the right tool to answer the question.")
        .agent_name("TestToolAgent")
        .user_name("M4n5ter")
        .enable_autosave()
        .max_loops(1)
        .save_state_dir("./temp")
        .add_tool(SubTool)
        .add_tool(Add) // or AddTool, Add is a pub static variable of AddTool
        .add_tool(MultiplyTool)
        .add_tool(Exec) // or ExecTool, Exec is a pub static variable of ExecTool
        .build();

    let mut result = agent.run("10 - 5".into()).await.unwrap();
    println!("{result}");
    // The output will be:
    // 5.0

    result = agent.run(format!("{} + 5", result)).await.unwrap();
    println!("{result}");
    // The output will be:
    // 10.0

    result = agent.run(format!("{} * 5", result)).await.unwrap();
    println!("{result}");
    // The output will be:
    // 50.0

    result = agent
        .run("
        Use docker to run a postgres database(newest version, alpine as base), set the network mode to host.
        Then get current system's release.
        Finally, curl something to get the IP address of current machine.
        ".to_string()
    )
        .await
        .unwrap();
    println!("{result}");
    // The output might be:
    // "{\"don_t_tell_you_what_it_means_1\":[\"docker run --network host -e POSTGRES_PASSWORD=mysecretpassword -d postgres:alpine\",\"cat /etc/os-release\",\"curl ifconfig.me\"],\"don_t_tell_you_what_it_means_2\":true,\"don_t_tell_you_what_it_means_3\":\"M4n5ter\"}"

    Ok(())
}

/// The return type of a tool must be [Result<T, E>], where `T` is the type of the return value and `E` is the type of the error.
///
/// T must implement [serde::Serialize] trait.
///
/// E must implement [core::error::Error] trait, maybe [thiserror::Error] is a good choice.
#[tool(
    description = "Subtract y from x (i.e.: x - y)",
    arg(x, description = "The number to subtract from"),
    arg(y, description = "The number to subtract")
)]
fn sub(x: f64, y: f64) -> Result<f64, CalcError> {
    tracing::info!("Sub tool is called");
    Ok(x - y)
}

#[tool]
fn add(x: f64, y: f64) -> Result<f64, CalcError> {
    tracing::info!("Add tool is called");
    Ok(x + y)
}

#[tool(name = "Multiply", description = "Multiply x and y (i.e.: x * y)")]
fn mul(x: f64, y: f64) -> Result<f64, CalcError> {
    tracing::info!("Mul tool is called");
    Ok(x * y)
}

/// This shows how to use a struct as parameter.
/// We can describe the field of the struct, just see the `ExecShell` struct.
#[tool(description = "
Execute the shell command, can execute multiple commands at once.
")]
fn exec(x: ExecShell) -> Result<String, CalcError> {
    tracing::info!("exec tool is called");
    let results = serde_json::to_string(&x).unwrap();

    Ok(results)
}

/// The request to execute the shell command.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ExecShell {
    #[doc = "The commands to execute, can execute multiple commands at once."]
    don_t_tell_you_what_it_means_1: Vec<String>,
    /// The flag to execute the command
    don_t_tell_you_what_it_means_2: bool,
    /// Who wants to execute the command
    don_t_tell_you_what_it_means_3: String,
}

/// ## IMPORTANT
///
/// You can use a struct as parameter too, but only one parameter is allowed.
///
/// The struct must implement [serde::Serialize] and [serde::Deserialize] traits.
///
/// The struct must also implement [schemars::JsonSchema] trait. [schemars] must newer than 1.0.0.
///
/// Both #\[doc = "..."\] and `///` comments are supported, the contents of both will be a description of the parameter.
/// If #\[doc\] or `///` is above the struct, it will be the description of the struct.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ExampleStructParameterToHaveADescription {
    #[doc = "The first field"]
    first_field: String,
    /// The second field
    second_field: String,
    /// The third field
    third_field: Vec<ThirdField>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct ThirdField {
    /// The first field
    first_field: String,
    /// The second field
    second_field: String,
}

#[derive(Debug, Error)]
pub enum CalcError {}
