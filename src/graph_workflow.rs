use std::{
    collections::{HashMap, hash_map},
    fmt::Debug,
    sync::Arc,
    time::Duration,
};

use dashmap::DashMap;
use petgraph::{
    Direction,
    graph::{EdgeIndex, NodeIndex},
    prelude::StableGraph,
    visit::EdgeRef,
};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::agent::Agent;

/// The main orchestration structure
pub struct DAGWorkflow {
    pub name: String,
    pub description: String,
    /// Store all registered agents
    agents: DashMap<String, Arc<dyn Agent>>,
    /// The workflow graph
    workflow: StableGraph<AgentNode, Flow>,
    /// Map from agent name to node index for quick lookup
    name_to_node: HashMap<String, NodeIndex>,
}

impl DAGWorkflow {
    /// Create a new DAGWorkflow
    pub fn new<S: Into<String>>(name: S, description: S) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            agents: DashMap::new(),
            workflow: StableGraph::new(),
            name_to_node: HashMap::new(),
        }
    }

    /// Register an agent with the orchestrator
    pub fn register_agent(&mut self, agent: Arc<dyn Agent>) {
        let agent_name = agent.name();
        self.agents.insert(agent_name.clone(), agent);

        // If agent isn't already in the graph, add it
        if let hash_map::Entry::Vacant(e) = self.name_to_node.entry(agent_name.clone()) {
            let node_idx = self.workflow.add_node(AgentNode {
                name: agent_name.clone(),
                last_result: Mutex::new(None),
            });
            e.insert(node_idx);
        }
    }

    /// Add a flow connection between two agents
    pub fn connect_agents(
        &mut self,
        from: &str,
        to: &str,
        flow: Flow,
    ) -> Result<EdgeIndex, GraphWorkflowError> {
        // Ensure both agents exist
        if !self.agents.contains_key(from) {
            return Err(GraphWorkflowError::AgentNotFound(format!(
                "Source agent '{from}' not found",
            )));
        }
        if !self.agents.contains_key(to) {
            return Err(GraphWorkflowError::AgentNotFound(format!(
                "Target agent '{to}' not found",
            )));
        }

        // Get node indices, creating nodes if necessary
        let from_entry = self.name_to_node.entry(from.to_owned());
        let from_idx = *from_entry.or_insert_with(|| {
            self.workflow.add_node(AgentNode {
                name: from.to_owned(),
                last_result: Mutex::new(None),
            })
        });

        let to_entry = self.name_to_node.entry(to.to_owned());
        let to_idx = *to_entry.or_insert_with(|| {
            self.workflow.add_node(AgentNode {
                name: to.to_owned(),
                last_result: Mutex::new(None),
            })
        });

        // Add the edge
        let edge_idx = self.workflow.add_edge(from_idx, to_idx, flow);

        // Check for cycles
        if self.has_cycle() {
            // Remove the edge we just added
            self.workflow.remove_edge(edge_idx);
            return Err(GraphWorkflowError::CycleDetected);
        }

        Ok(edge_idx)
    }

    // Check if the workflow has a cycle
    fn has_cycle(&self) -> bool {
        // Implementation using DFS to detect cycles
        let mut visited = vec![false; self.workflow.node_count()];
        let mut rec_stack = vec![false; self.workflow.node_count()];

        for node in self.workflow.node_indices() {
            if !visited[node.index()] && self.is_cyclic_util(node, &mut visited, &mut rec_stack) {
                return true;
            }
        }
        false
    }

    fn is_cyclic_util(
        &self,
        node: NodeIndex,
        visited: &mut [bool],
        rec_stack: &mut [bool],
    ) -> bool {
        visited[node.index()] = true;
        rec_stack[node.index()] = true;

        for neighbor in self.workflow.neighbors_directed(node, Direction::Outgoing) {
            if !visited[neighbor.index()] {
                if self.is_cyclic_util(neighbor, visited, rec_stack) {
                    return true;
                }
            } else if rec_stack[neighbor.index()] {
                return true;
            }
        }

        rec_stack[node.index()] = false;
        false
    }

    /// Remove an agent connection
    pub fn disconnect_agents(&mut self, from: &str, to: &str) -> Result<(), GraphWorkflowError> {
        let from_idx = self.name_to_node.get(from).ok_or_else(|| {
            GraphWorkflowError::AgentNotFound(format!("Source agent '{from}' not found"))
        })?;
        let to_idx = self.name_to_node.get(to).ok_or_else(|| {
            GraphWorkflowError::AgentNotFound(format!("Target agent '{to}' not found"))
        })?;

        // Find and remove the edge
        if let Some(edge) = self.workflow.find_edge(*from_idx, *to_idx) {
            self.workflow.remove_edge(edge);
            Ok(())
        } else {
            Err(GraphWorkflowError::AgentNotFound(format!(
                "No connection from '{from}' to '{to}'"
            )))
        }
    }

    /// Remove an agent from the orchestrator
    pub fn remove_agent(&mut self, name: &str) -> Result<(), GraphWorkflowError> {
        if let Some(node_idx) = self.name_to_node.remove(name) {
            self.workflow.remove_node(node_idx);
            self.agents.remove(name);
            Ok(())
        } else {
            Err(GraphWorkflowError::AgentNotFound(format!(
                "Agent '{name}' not found"
            )))
        }
    }

    /// Execute a specific agent
    pub async fn execute_agent(
        &self,
        name: &str,
        input: String,
    ) -> Result<String, GraphWorkflowError> {
        if let Some(agent) = self.agents.get(name) {
            agent
                .run(input)
                .await
                .map_err(|e| GraphWorkflowError::AgentError(e.to_string()))
        } else {
            Err(GraphWorkflowError::AgentNotFound(format!(
                "Agent '{name}' not found"
            )))
        }
    }

    /// Execute the entire workflow starting from a specific agent
    ///
    /// # Arguments
    ///
    /// * `start_agent`: The name of the agent to start the workflow from
    /// * `input`: The input to the workflow
    ///
    /// # Returns
    ///
    /// * `Result<DashMap<String, Result<String, GraphWorkflowError>>, GraphWorkflowError>`: A map of agent names to their results
    ///
    pub async fn execute_workflow(
        &mut self,
        start_agents: &[&str],
        input: impl Into<String>,
    ) -> Result<DashMap<String, Result<String, GraphWorkflowError>>, GraphWorkflowError> {
        let input = input.into();

        let start_indices = start_agents
            .iter()
            .map(|agent| {
                self.name_to_node
                    .get(*agent)
                    .ok_or_else(|| {
                        GraphWorkflowError::AgentNotFound(format!(
                            "Start agent '{agent}' not found"
                        ))
                    })
                    .copied()
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Reset all results
        let node_idxs = self.workflow.node_indices().collect::<Vec<_>>();
        for idx in node_idxs {
            if let Some(node_weight) = self.workflow.node_weight_mut(idx) {
                let mut last_result = node_weight.last_result.lock().await;
                *last_result = None;
            }
        }

        // Create a shared results map for all agents to write to
        let results = Arc::new(DashMap::new());
        // Create a shared tracking state for the entire workflow
        let edge_tracker = Arc::new(DashMap::new());
        let processed_nodes = Arc::new(DashMap::new());
        // Execute the workflow
        let mut tasks = Vec::new();
        for &start_idx in &start_indices {
            let task = self.execute_node(
                start_idx,
                input.clone(),
                Arc::clone(&results),
                Arc::clone(&edge_tracker),
                Arc::clone(&processed_nodes),
            );
            tasks.push(task);
        }
        futures::future::join_all(tasks)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| GraphWorkflowError::ExecutionError(e.to_string()))?;
        Ok(Arc::into_inner(results).expect("Results should not be poisoned"))
    }

    async fn execute_node(
        &self,
        node_idx: NodeIndex,
        input: String,
        results: Arc<DashMap<String, Result<String, GraphWorkflowError>>>,
        edge_tracker: Arc<DashMap<(NodeIndex, NodeIndex), bool>>,
        processed_nodes: Arc<DashMap<NodeIndex, Vec<(NodeIndex, String)>>>,
    ) -> Result<String, GraphWorkflowError> {
        // Get the agent name from the node
        let agent_name = &self
            .workflow
            .node_weight(node_idx)
            .ok_or_else(|| GraphWorkflowError::AgentNotFound("Node not found in graph".to_owned()))?
            .name;

        // Check if we already have a result for this node (avoid duplicate work)
        if let Some(entry) = results.get(agent_name) {
            return entry.value().clone();
        }

        // Execute the agent with timeout protection
        let result = tokio::time::timeout(
            Duration::from_secs(3600), // 60-minute timeout
            self.execute_agent(agent_name, input),
        )
        .await
        .map_err(|_| GraphWorkflowError::Timeout(agent_name.clone()))?;

        // Store the result
        results.insert(agent_name.clone(), result.clone());

        // Update the node's last result
        if let Some(node_weight) = self.workflow.node_weight(node_idx) {
            let mut last_result = node_weight.last_result.lock().await;
            *last_result = Some(result.clone());
        }

        // If successful, propagate to connected agents
        match &result {
            Ok(output) => {
                // Find all outgoing edges that pass the condition (if any)
                let valid_edges = self
                    .workflow
                    .edges_directed(node_idx, Direction::Outgoing)
                    .filter(|edge| {
                        // Evaluate condition with the current output
                        let condition_result = edge
                            .weight()
                            .condition
                            .as_ref()
                            .map(|cond| {
                                // Apply condition to the current output
                                let result = cond(output);
                                tracing::debug!(
                                    "Condition for edge {:?} -> {:?}: {}",
                                    node_idx,
                                    edge.target(),
                                    result
                                );
                                result
                            })
                            .unwrap_or(true); // if no condition, always execute

                        condition_result
                    })
                    .collect::<Vec<_>>();

                let mut futures = Vec::new();

                for edge in valid_edges {
                    let source_node = node_idx;
                    let target_node = edge.target();
                    let flow = edge.weight().clone();
                    let results_clone = Arc::clone(&results);
                    let processed_nodes_clone = Arc::clone(&processed_nodes);
                    let edge_tracker_clone = Arc::clone(&edge_tracker);

                    let future = async move {
                        // Apply transformation if any
                        let next_input = flow
                            .transform
                            .as_ref()
                            .map_or_else(|| output.clone(), |transform| transform(output.clone()));

                        // mark this edge as processed
                        edge_tracker_clone.insert((source_node, target_node), true);

                        // record the input for this node with proper synchronization
                        // Use a scope to ensure the lock is released after the operation
                        {
                            processed_nodes_clone
                                .entry(target_node)
                                .and_modify(|v| v.push((source_node, next_input.clone())))
                                .or_insert_with(|| vec![(source_node, next_input.clone())]);
                        }

                        // Get all input edges (including those from different starting nodes)
                        let all_incoming_edges = self
                            .workflow
                            .edges_directed(target_node, Direction::Incoming)
                            .map(|e| (e.source(), target_node))
                            .collect::<Vec<_>>();

                        // Check that all input edges have completed processing (from different paths).
                        // For conditional flows, we need to check if the edge has a condition and if it evaluates to false
                        let all_processed = all_incoming_edges.iter().all(|edge| {
                            // Check if this edge is already processed
                            let processed = edge_tracker_clone.contains_key(edge);

                            // If not processed, check if it has a condition that evaluates to false
                            // In that case, we should consider it as "processed" (skipped)
                            let conditionally_skipped = if !processed {
                                if let Some(edge_idx) = self.workflow.find_edge(edge.0, edge.1) {
                                    let edge_weight = self.workflow.edge_weight(edge_idx).unwrap();
                                    if let Some(cond) = &edge_weight.condition {
                                        // If we can find the source node's result, check the condition
                                        if let Some(source_name) =
                                            self.workflow.node_weight(edge.0).map(|n| &n.name)
                                        {
                                            if let Some(source_result) =
                                                results_clone.get(source_name)
                                            {
                                                if let Ok(output) = source_result.as_ref() {
                                                    // If condition is false, this edge is conditionally skipped
                                                    let condition_result = !cond(output);
                                                    if condition_result {
                                                        // Mark this edge as processed (skipped due to condition)
                                                        edge_tracker_clone
                                                            .insert((edge.0, edge.1), true);
                                                    }
                                                    condition_result
                                                } else {
                                                    // Source node execution failed, consider edge as processed
                                                    edge_tracker_clone
                                                        .insert((edge.0, edge.1), true);
                                                    true
                                                }
                                            } else {
                                                false
                                            }
                                        } else {
                                            false
                                        }
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                }
                            } else {
                                false
                            };

                            tracing::debug!(
                                "Edge {:?} processed: {}, conditionally skipped: {}",
                                edge,
                                processed,
                                conditionally_skipped
                            );
                            processed || conditionally_skipped
                        });

                        // only execute if all incoming edges have been processed
                        if all_processed {
                            // Aggregate all inputs from different paths
                            let aggregated_input = processed_nodes_clone
                                .get(&target_node)
                                .map(|inputs| {
                                    // Sort inputs by source node to ensure consistent ordering
                                    let mut sorted_inputs = inputs.value().clone();
                                    sorted_inputs.sort_by_key(|(source_idx, _)| *source_idx);

                                    // Log the number of inputs for debugging
                                    tracing::debug!(
                                        "Node {:?} has {} inputs",
                                        target_node,
                                        sorted_inputs.len()
                                    );

                                    // Format each input with its source agent name
                                    let formatted_inputs = sorted_inputs
                                        .iter()
                                        .map(|(source_idx, input)| {
                                            let source_name = &self
                                                .workflow
                                                .node_weight(*source_idx)
                                                .unwrap()
                                                .name;
                                            format!("[From {source_name}] {input}")
                                        })
                                        .collect::<Vec<_>>();

                                    // Join all inputs with a clear separator
                                    let result = formatted_inputs.join("\n\n---\n\n");
                                    tracing::debug!(
                                        "Aggregated input for node {:?}: {}",
                                        target_node,
                                        result
                                    );
                                    result
                                })
                                .unwrap_or_default();

                            tracing::debug!(
                                "Executing node {:?} with aggregated input",
                                target_node
                            );

                            // execute the target node with the aggregated input
                            if let Err(e) = self
                                .execute_node(
                                    target_node,
                                    aggregated_input,
                                    results_clone,
                                    edge_tracker_clone,
                                    processed_nodes_clone,
                                )
                                .await
                            {
                                tracing::error!("Failed to execute node: {:?}", e);
                            }
                        }
                    };

                    futures.push(future);
                }

                // Execute connected agents concurrently
                futures::future::join_all(futures).await; // TODO: may use another way which can handle errors
            }
            Err(e) => {
                tracing::error!("Agent '{}' execution failed: {:?}", agent_name, e);
                // TODO: maybe we need to propagate the error to the caller?
            }
        }

        result
    }

    /// Get the current workflow as a visualization-friendly format
    pub fn get_workflow_structure(&self) -> HashMap<String, Vec<(String, Option<String>)>> {
        let mut structure = HashMap::new();

        for node_idx in self.workflow.node_indices() {
            if let Some(node) = self.workflow.node_weight(node_idx) {
                let mut connections = Vec::new();

                for edge in self.workflow.edges_directed(node_idx, Direction::Outgoing) {
                    if let Some(target) = self.workflow.node_weight(edge.target()) {
                        // TODO: can add more edge metadata here if needed
                        let edge_label = if edge.weight().transform.is_some() {
                            Some("transform".to_owned())
                        } else {
                            None
                        };

                        connections.push((target.name.clone(), edge_label));
                    }
                }

                structure.insert(node.name.clone(), connections);
            }
        }

        structure
    }

    /// Export the workflow to a format that can be visualized (e.g., DOT format for Graphviz)
    pub fn export_workflow_dot(&self) -> String {
        // TODO: can use petgraph's built-in dot
        // let dot = Dot::with_config(&self.workflow, &[dot::Config::EdgeNoLabel]);

        let mut dot = String::from("digraph {\n");

        // Add nodes
        for node_idx in self.workflow.node_indices() {
            if let Some(node) = self.workflow.node_weight(node_idx) {
                dot.push_str(&format!(
                    "    \"{}\" [label=\"{}\"];\n",
                    node.name, node.name
                ));
            }
        }

        // Add edges
        for edge in self.workflow.edge_indices() {
            if let Some((source, target)) = self.workflow.edge_endpoints(edge) {
                if let (Some(source_node), Some(target_node)) = (
                    self.workflow.node_weight(source),
                    self.workflow.node_weight(target),
                ) {
                    dot.push_str(&format!(
                        "    \"{}\" -> \"{}\";\n",
                        source_node.name, target_node.name
                    ));
                }
            }
        }

        dot.push_str("}\n");
        dot
    }

    /// Helper method to find all possible execution paths
    pub fn find_execution_paths(
        &self,
        start_agents: &[&str],
    ) -> Result<Vec<Vec<String>>, GraphWorkflowError> {
        let start_indices = start_agents
            .iter()
            .map(|agent| {
                self.name_to_node
                    .get(*agent)
                    .ok_or_else(|| {
                        GraphWorkflowError::AgentNotFound(format!(
                            "Start agent '{agent}' not found"
                        ))
                    })
                    .copied()
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut paths = Vec::new();
        let mut current_path = Vec::new();

        for start_idx in &start_indices {
            current_path.clear();
            self.dfs_paths(*start_idx, &mut current_path, &mut paths);
        }

        Ok(paths)
    }

    fn dfs_paths(
        &self,
        node_idx: NodeIndex,
        current_path: &mut Vec<String>,
        all_paths: &mut Vec<Vec<String>>,
    ) {
        if let Some(node) = self.workflow.node_weight(node_idx) {
            // Add current node to path
            current_path.push(node.name.clone());

            // Check if this is a leaf node (no outgoing edges)
            let has_outgoing = self
                .workflow
                .neighbors_directed(node_idx, Direction::Outgoing)
                .count()
                > 0;

            if !has_outgoing {
                // We've reached a leaf node, save this path
                all_paths.push(current_path.clone());
            } else {
                // Continue DFS for all neighbors
                for neighbor in self
                    .workflow
                    .neighbors_directed(node_idx, Direction::Outgoing)
                {
                    self.dfs_paths(neighbor, current_path, all_paths);
                }
            }

            // Backtrack
            current_path.pop();
        }
    }

    /// Detect potential deadlocks in the workflow. Whether there will actually be a deadlock depends on the flow at execution time.
    ///
    /// ## Info
    ///
    /// Maybe we need a monitor to detect deadlocks instead of this function.
    ///
    /// ## Returns
    ///
    /// Returns a vector of cycles (each cycle is a vector of agent names).
    ///
    /// Example: vec![vec!["A", "B", "C"], vec!["X", "Y"]]
    pub fn detect_potential_deadlocks(&self) -> Vec<Vec<String>> {
        // Build a dependency graph where an edge A→B means B depends on A
        let mut dependency_graph = petgraph::Graph::<String, ()>::new();
        let mut node_map = HashMap::new();

        // Create nodes
        for name in self.name_to_node.keys() {
            let idx = dependency_graph.add_node(name.clone());
            node_map.insert(name.clone(), idx);
        }

        // Add dependencies
        for node_idx in self.workflow.node_indices() {
            if let Some(node) = self.workflow.node_weight(node_idx) {
                let target_dep_idx = *node_map.get(&node.name).unwrap();

                // Add an edge for each incoming connection
                for source in self
                    .workflow
                    .neighbors_directed(node_idx, Direction::Incoming)
                {
                    if let Some(source_node) = self.workflow.node_weight(source) {
                        let source_dep_idx = *node_map.get(&source_node.name).unwrap();
                        dependency_graph.add_edge(source_dep_idx, target_dep_idx, ());
                    }
                }
            }
        }

        // Find strongly connected components (cycles in the dependency graph)
        let sccs = petgraph::algo::kosaraju_scc(&dependency_graph);

        // Return only the non-trivial SCCs (size > 1)
        sccs.into_iter()
            .filter(|scc| scc.len() > 1)
            .map(|scc| {
                scc.into_iter()
                    .map(|idx| dependency_graph[idx].clone())
                    .collect()
            })
            .collect()
    }
}

/// Edge weight to represent the flow of data between agents
#[allow(clippy::type_complexity)]
#[derive(Clone, Default)]
pub struct Flow {
    /// Optional transformation function to apply to the output before passing to the next agent
    pub transform: Option<Arc<dyn Fn(String) -> String + Send + Sync>>,
    /// Optional condition to determine if this flow should be taken
    pub condition: Option<Arc<dyn Fn(&str) -> bool + Send + Sync>>,
}

/// Node weight for the graph
#[derive(Debug)]
pub struct AgentNode {
    /// Name of the agent
    pub name: String,
    /// Cache for execution results
    pub last_result: Mutex<Option<Result<String, GraphWorkflowError>>>,
}

#[derive(Clone, Debug, Error)]
pub enum GraphWorkflowError {
    #[error("Agent Error: {0}")]
    AgentError(String),
    #[error("Agent not found: {0}")]
    AgentNotFound(String),
    #[error("Cycle detected in workflow")]
    CycleDetected,
    #[error("Execution error: {0}")]
    ExecutionError(String),
    #[error("Timeout executing agent: {0}")]
    Timeout(String),
    #[error("Deadlock detected in workflow execution")]
    Deadlock,
    #[error("Workflow execution canceled")]
    Canceled,
}

impl Debug for Flow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Flow")
            .field("transform", &self.transform.is_some())
            .field("condition", &self.condition.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use futures::future::{self, BoxFuture};
    use mockall::mock;

    use crate::agent::AgentError;

    mock! {
        #[derive(Debug)]
        pub Agent{}

        impl Agent for Agent {
            fn run(&self, task: String) -> BoxFuture<'static, Result<String, AgentError>> {
                Box::pin(future::ready(Ok(String::new())))
            }
            fn run_multiple_tasks(&mut self, tasks: Vec<String>) -> BoxFuture<'static, Result<Vec<String>, AgentError>> {
                Box::pin(future::ready(Ok(vec![])))
            }
            fn id(&self) -> String {
                String::new()
            }
            fn name(&self) -> String {
                String::new()
            }
            fn description(&self) -> String {
                String::new()
            }
        }
    }

    fn create_mock_agent(id: &str, name: &str, desc: &str, response: &str) -> Arc<MockAgent> {
        let mut agent = MockAgent::new();

        let id_str = id.to_owned();
        agent.expect_id().return_const(id_str);

        let name_str = name.to_owned();
        agent.expect_name().return_const(name_str);

        let desc_str = desc.to_owned();
        agent.expect_description().return_const(desc_str);

        let response_str = response.to_owned();
        let response_str_clone = response_str.clone();
        agent.expect_run().returning(move |_| {
            let res = response_str_clone.clone();
            Box::pin(future::ready(Ok(res)))
        });

        let response_str_clone = response_str.clone();
        agent.expect_run_multiple_tasks().returning(move |tasks| {
            let responses = tasks.iter().map(|_| response_str_clone.clone()).collect();
            Box::pin(future::ready(Ok(responses)))
        });

        Arc::new(agent)
    }

    fn create_failing_agent(id: &str, name: &str, error_msg: &str) -> Arc<MockAgent> {
        let mut agent = MockAgent::new();

        let id_str = id.to_owned();
        agent.expect_id().return_const(id_str);

        let name_str = name.to_owned();
        agent.expect_name().return_const(name_str);

        agent
            .expect_description()
            .return_const("Failing agent".to_owned());

        let error_str = error_msg.to_owned();
        let error_str_for_run = error_str.clone();
        agent.expect_run().returning(move |_| {
            let err = AgentError::TestError(error_str_for_run.clone());
            Box::pin(future::ready(Err(err)))
        });

        agent.expect_run_multiple_tasks().returning(move |_| {
            let err = AgentError::TestError(error_str.clone());
            Box::pin(future::ready(Err(err)))
        });

        Arc::new(agent)
    }

    #[test]
    fn test_dag_creation() {
        let workflow = DAGWorkflow::new("test", "Test workflow");
        assert_eq!(workflow.name, "test");
        assert_eq!(workflow.description, "Test workflow");
        assert_eq!(workflow.agents.len(), 0);
        assert_eq!(workflow.workflow.node_count(), 0);
        assert_eq!(workflow.workflow.edge_count(), 0);
    }

    #[test]
    fn test_agent_registration() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "Test agent", "response1"));

        assert_eq!(workflow.agents.len(), 1);
        assert_eq!(workflow.workflow.node_count(), 1);
        assert!(workflow.name_to_node.contains_key("agent1"));
    }

    #[test]
    fn test_agent_connection() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "First agent", "response1"));
        workflow.register_agent(create_mock_agent(
            "2",
            "agent2",
            "Second agent",
            "response2",
        ));

        let result = workflow.connect_agents("agent1", "agent2", Flow::default());
        assert!(result.is_ok());
        assert_eq!(workflow.workflow.edge_count(), 1);
    }

    #[test]
    fn test_agent_connection_failure_nonexistent_agent() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "Test agent", "response1"));

        let result = workflow.connect_agents("agent1", "nonexistent", Flow::default());
        assert!(matches!(result, Err(GraphWorkflowError::AgentNotFound(_))));

        let result = workflow.connect_agents("nonexistent", "agent1", Flow::default());
        assert!(matches!(result, Err(GraphWorkflowError::AgentNotFound(_))));
    }

    #[test]
    fn test_cycle_detection() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "First agent", "response1"));
        workflow.register_agent(create_mock_agent(
            "2",
            "agent2",
            "Second agent",
            "response2",
        ));
        workflow.register_agent(create_mock_agent("3", "agent3", "Third agent", "response3"));

        // agent1 -> agent2 -> agent3
        let result1 = workflow.connect_agents("agent1", "agent2", Flow::default());
        assert!(result1.is_ok());
        let result2 = workflow.connect_agents("agent2", "agent3", Flow::default());
        assert!(result2.is_ok());

        // cycle it: agent3 -> agent1
        let result3 = workflow.connect_agents("agent3", "agent1", Flow::default());
        assert!(matches!(result3, Err(GraphWorkflowError::CycleDetected)));

        // edge should not be added
        assert_eq!(workflow.workflow.edge_count(), 2);
    }

    #[test]
    fn test_agent_disconnection() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "First agent", "response1"));
        workflow.register_agent(create_mock_agent(
            "2",
            "agent2",
            "Second agent",
            "response2",
        ));

        workflow
            .connect_agents("agent1", "agent2", Flow::default())
            .unwrap();
        assert_eq!(workflow.workflow.edge_count(), 1);

        let result = workflow.disconnect_agents("agent1", "agent2");
        assert!(result.is_ok());
        assert_eq!(workflow.workflow.edge_count(), 0);
    }

    #[test]
    fn test_agent_disconnection_failure() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "First agent", "response1"));
        workflow.register_agent(create_mock_agent(
            "2",
            "agent2",
            "Second agent",
            "response2",
        ));

        // try to disconnect non-existent edge
        let result = workflow.disconnect_agents("agent1", "agent2");
        assert!(matches!(result, Err(GraphWorkflowError::AgentNotFound(_))));

        // try to disconnect non-existent agent
        let result = workflow.disconnect_agents("nonexistent", "agent2");
        assert!(matches!(result, Err(GraphWorkflowError::AgentNotFound(_))));
    }

    #[test]
    fn test_agent_removal() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "First agent", "response1"));
        workflow.register_agent(create_mock_agent(
            "2",
            "agent2",
            "Second agent",
            "response2",
        ));

        workflow
            .connect_agents("agent1", "agent2", Flow::default())
            .unwrap();
        assert_eq!(workflow.agents.len(), 2);
        assert_eq!(workflow.workflow.node_count(), 2);

        let result = workflow.remove_agent("agent1");
        assert!(result.is_ok());
        assert_eq!(workflow.agents.len(), 1);
        assert_eq!(workflow.workflow.node_count(), 1);
        assert!(!workflow.name_to_node.contains_key("agent1"));

        assert_eq!(workflow.workflow.edge_count(), 0);
    }

    #[test]
    fn test_agent_removal_nonexistent() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");

        let result = workflow.remove_agent("nonexistent");
        assert!(matches!(result, Err(GraphWorkflowError::AgentNotFound(_))));
    }

    #[tokio::test]
    async fn test_execute_single_agent() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "Test agent", "response1"));

        let result = workflow.execute_agent("agent1", "input".to_owned()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "response1");
    }

    #[tokio::test]
    async fn test_execute_single_agent_failure() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_failing_agent("1", "agent1", "test error"));

        let result = workflow.execute_agent("agent1", "input".to_owned()).await;
        assert!(matches!(result, Err(GraphWorkflowError::AgentError(_))));
    }

    #[tokio::test]
    async fn test_execute_single_agent_not_found() {
        let workflow = DAGWorkflow::new("test", "Test workflow");

        let result = workflow
            .execute_agent("nonexistent", "input".to_owned())
            .await;
        assert!(matches!(result, Err(GraphWorkflowError::AgentNotFound(_))));
    }

    #[tokio::test]
    async fn test_execute_workflow_linear() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "First agent", "response1"));
        workflow.register_agent(create_mock_agent(
            "2",
            "agent2",
            "Second agent",
            "response2",
        ));

        workflow
            .connect_agents("agent1", "agent2", Flow::default())
            .unwrap();

        let results = workflow
            .execute_workflow(&["agent1"], "input")
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            results.get("agent1").unwrap().as_ref().unwrap(),
            "response1"
        );
        assert_eq!(
            results.get("agent2").unwrap().as_ref().unwrap(),
            "response2"
        );
    }

    #[tokio::test]
    async fn test_execute_workflow_branching() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "Root agent", "response1"));
        workflow.register_agent(create_mock_agent("2", "agent2", "Branch 1", "response2"));
        workflow.register_agent(create_mock_agent("3", "agent3", "Branch 2", "response3"));

        workflow
            .connect_agents("agent1", "agent2", Flow::default())
            .unwrap();
        workflow
            .connect_agents("agent1", "agent3", Flow::default())
            .unwrap();

        let results = workflow
            .execute_workflow(&["agent1"], "input")
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(
            results.get("agent1").unwrap().as_ref().unwrap(),
            "response1"
        );
        assert_eq!(
            results.get("agent2").unwrap().as_ref().unwrap(),
            "response2"
        );
        assert_eq!(
            results.get("agent3").unwrap().as_ref().unwrap(),
            "response3"
        );
    }

    #[tokio::test]
    async fn test_execute_workflow_with_transformation() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "First agent", "response1"));
        workflow.register_agent(create_mock_agent(
            "2",
            "agent2",
            "Second agent",
            "response2",
        ));

        let transform_fn = Arc::new(|input: String| format!("transformed: {input}"));
        let flow = Flow {
            transform: Some(transform_fn),
            condition: None,
        };

        workflow.connect_agents("agent1", "agent2", flow).unwrap();

        let results = workflow
            .execute_workflow(&["agent1"], "input")
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        let structure = workflow.get_workflow_structure();
        let agent1_connections = &structure["agent1"];
        assert_eq!(agent1_connections.len(), 1);
        assert_eq!(agent1_connections[0].0, "agent2");
        assert_eq!(agent1_connections[0].1, Some("transform".to_owned()));
    }

    #[tokio::test]
    async fn test_execute_workflow_with_condition_true() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "First agent", "true"));
        workflow.register_agent(create_mock_agent("2", "agent2", "Second agent", "executed"));

        let true_condition = Arc::new(|output: &str| output.contains("true"));

        workflow
            .connect_agents(
                "agent1",
                "agent2",
                Flow {
                    transform: None,
                    condition: Some(true_condition),
                },
            )
            .unwrap();

        let results = workflow
            .execute_workflow(&["agent1"], "input")
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.contains_key("agent1"));
        assert!(results.contains_key("agent2"));
    }

    #[tokio::test]
    async fn test_execute_workflow_with_condition_false() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "First agent", "response1"));
        workflow.register_agent(create_mock_agent(
            "2",
            "agent2",
            "Second agent",
            "not executed",
        ));

        let false_condition = Arc::new(|output: &str| output.contains("nonexistent"));

        workflow
            .connect_agents(
                "agent1",
                "agent2",
                Flow {
                    transform: None,
                    condition: Some(false_condition),
                },
            )
            .unwrap();

        let results = workflow
            .execute_workflow(&["agent1"], "input")
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results.contains_key("agent1"));
        assert!(!results.contains_key("agent2"));
    }

    #[tokio::test]
    async fn test_workflow_execution_start_agent_not_found() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "First agent", "response1"));

        let result = workflow.execute_workflow(&["nonexistent"], "input").await;
        assert!(matches!(result, Err(GraphWorkflowError::AgentNotFound(_))));
    }

    #[tokio::test]
    async fn test_workflow_execution_with_failing_agent() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "agent1", "First agent", "response1"));
        workflow.register_agent(create_failing_agent("2", "agent2", "fail error"));
        workflow.register_agent(create_mock_agent("3", "agent3", "Third agent", "response3"));

        // agent1 -> agent2 -> agent3
        workflow
            .connect_agents("agent1", "agent2", Flow::default())
            .unwrap();
        workflow
            .connect_agents("agent2", "agent3", Flow::default())
            .unwrap();

        let results = workflow
            .execute_workflow(&["agent1"], "input")
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.contains_key("agent1"));
        assert!(results.contains_key("agent2"));
        assert!(!results.contains_key("agent3"));

        let agent2_result = results.get("agent2").unwrap();
        assert!(agent2_result.is_err());
    }

    #[tokio::test]
    async fn test_independent_multiple_starts() {
        let mut workflow = DAGWorkflow::new("test", "");

        let agent_a = create_mock_agent("1", "A", "A", "A_result");
        let agent_b = create_mock_agent("2", "B", "B", "B_result");
        let agent_c = create_mock_agent("3", "C", "C", "C_result");
        let agent_d = create_mock_agent("4", "D", "D", "D_result");

        workflow.register_agent(agent_a);
        workflow.register_agent(agent_b);
        workflow.register_agent(agent_c);
        workflow.register_agent(agent_d);

        workflow.connect_agents("A", "C", Flow::default()).unwrap();
        workflow.connect_agents("B", "D", Flow::default()).unwrap();

        let results = workflow
            .execute_workflow(&["A", "B"], "input")
            .await
            .unwrap();

        assert_eq!(results.get("A").unwrap().as_ref().unwrap(), "A_result");
        assert_eq!(results.get("B").unwrap().as_ref().unwrap(), "B_result");
        assert_eq!(results.get("C").unwrap().as_ref().unwrap(), "C_result");
        assert_eq!(results.get("D").unwrap().as_ref().unwrap(), "D_result");
    }

    /// FIXME: This test fails
    #[tokio::test]
    async fn test_converging_multiple_starts() {
        let mut workflow = DAGWorkflow::new("test", "");

        let agent_a = create_mock_agent("1", "A", "A", "A_result");
        let agent_b = create_mock_agent("2", "B", "B", "B_result");
        let agent_c = create_mock_agent("3", "C", "C", "C_result");

        workflow.register_agent(agent_a);
        workflow.register_agent(agent_b);
        workflow.register_agent(agent_c);

        workflow.connect_agents("A", "C", Flow::default()).unwrap();
        workflow.connect_agents("B", "C", Flow::default()).unwrap();

        let _results = workflow
            .execute_workflow(&["A", "B"], "input")
            .await
            .unwrap();

        let c_node = workflow.name_to_node.get("C").unwrap();
        let node_data = workflow.workflow.node_weight(*c_node).unwrap();
        let last_result = node_data.last_result.lock().await;
        assert!(last_result.is_some());
        assert!(
            last_result
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("A_result")
        );
        assert!(
            last_result
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .contains("B_result")
        );
    }

    /// FIXME: This test fails
    #[tokio::test]
    async fn test_conditional_branches() {
        let mut workflow = DAGWorkflow::new("test", "");

        let agent_a = create_mock_agent("1", "A", "A", "A_trigger");
        let agent_b = create_mock_agent("2", "B", "B", "B_result");
        let agent_c = create_mock_agent("3", "C", "C", "C_result");

        workflow.register_agent(agent_a);
        workflow.register_agent(agent_b);
        workflow.register_agent(agent_c);

        let conditional_flow = Flow {
            condition: Some(Arc::new(|output: &str| output.contains("trigger"))),
            transform: None,
        };

        workflow.connect_agents("A", "B", conditional_flow).unwrap();
        workflow.connect_agents("A", "C", Flow::default()).unwrap();

        let results = workflow.execute_workflow(&["A"], "input").await.unwrap();

        assert!(results.get("B").is_none());
        assert_eq!(results.get("C").unwrap().as_ref().unwrap(), "C_result");
    }

    #[test]
    fn test_find_execution_paths() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("0", "start", "Starting point", "start"));
        workflow.register_agent(create_mock_agent("1", "a", "Path A", "a"));
        workflow.register_agent(create_mock_agent("2", "b", "Path B", "b"));
        workflow.register_agent(create_mock_agent("3", "c", "End of A", "c"));
        workflow.register_agent(create_mock_agent("4", "d", "End of B", "d"));

        workflow
            .connect_agents("start", "a", Flow::default())
            .unwrap();
        workflow
            .connect_agents("start", "b", Flow::default())
            .unwrap();
        workflow.connect_agents("a", "c", Flow::default()).unwrap();
        workflow.connect_agents("b", "d", Flow::default()).unwrap();

        let paths = workflow.find_execution_paths(&["start"]).unwrap();
        assert_eq!(paths.len(), 2);

        // path should be [start, a, c] or [start, b, d]
        let has_path1 = paths
            .iter()
            .any(|p| p == &vec!["start".to_owned(), "a".to_owned(), "c".to_owned()]);
        let has_path2 = paths
            .iter()
            .any(|p| p == &vec!["start".to_owned(), "b".to_owned(), "d".to_owned()]);

        assert!(has_path1);
        assert!(has_path2);
    }

    #[test]
    fn test_find_execution_paths_start_agent_not_found() {
        let workflow = DAGWorkflow::new("test", "Test workflow");

        let result = workflow.find_execution_paths(&["nonexistent"]);
        assert!(matches!(result, Err(GraphWorkflowError::AgentNotFound(_))));
    }

    #[test]
    fn test_find_execution_paths_diamond_pattern() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("0", "start", "Start", "start"));
        workflow.register_agent(create_mock_agent("1", "a", "Middle A", "a"));
        workflow.register_agent(create_mock_agent("2", "b", "Middle B", "b"));
        workflow.register_agent(create_mock_agent("3", "end", "End", "end"));

        //            start -> a -> end
        //                 \-> b -/
        workflow
            .connect_agents("start", "a", Flow::default())
            .unwrap();
        workflow
            .connect_agents("start", "b", Flow::default())
            .unwrap();
        workflow
            .connect_agents("a", "end", Flow::default())
            .unwrap();
        workflow
            .connect_agents("b", "end", Flow::default())
            .unwrap();

        let paths = workflow.find_execution_paths(&["start"]).unwrap();
        assert_eq!(paths.len(), 2);

        // path should be [start, a, end] or [start, b, end]
        let has_path1 = paths
            .iter()
            .any(|p| p == &vec!["start".to_owned(), "a".to_owned(), "end".to_owned()]);
        let has_path2 = paths
            .iter()
            .any(|p| p == &vec!["start".to_owned(), "b".to_owned(), "end".to_owned()]);

        assert!(has_path1);
        assert!(has_path2);
    }

    #[test]
    fn test_detect_potential_deadlocks() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "a", "Agent A", "a"));
        workflow.register_agent(create_mock_agent("2", "b", "Agent B", "b"));
        workflow.register_agent(create_mock_agent("3", "c", "Agent C", "c"));

        // a -> b -> c
        workflow.connect_agents("a", "b", Flow::default()).unwrap();
        workflow.connect_agents("b", "c", Flow::default()).unwrap();

        // no cycles, should return empty vector
        let deadlocks = workflow.detect_potential_deadlocks();
        assert_eq!(deadlocks.len(), 0);

        // try to add c -> a, which should fail because has_cycle prevents it
        let result = workflow.connect_agents("c", "a", Flow::default());
        assert!(matches!(result, Err(GraphWorkflowError::CycleDetected)));
    }

    #[test]
    fn test_get_workflow_structure() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "a", "Agent A", "a"));
        workflow.register_agent(create_mock_agent("2", "b", "Agent B", "b"));
        workflow.register_agent(create_mock_agent("3", "c", "Agent C", "c"));

        workflow.connect_agents("a", "b", Flow::default()).unwrap();

        let transform_fn = Arc::new(|input: String| format!("transformed: {input}"));
        let flow = Flow {
            transform: Some(transform_fn),
            condition: None,
        };

        workflow.connect_agents("b", "c", flow).unwrap();

        let structure = workflow.get_workflow_structure();
        assert_eq!(structure.len(), 3);

        assert_eq!(structure["a"].len(), 1);
        assert_eq!(structure["a"][0].0, "b");
        assert_eq!(structure["a"][0].1, None);

        assert_eq!(structure["b"].len(), 1);
        assert_eq!(structure["b"][0].0, "c");
        assert_eq!(structure["b"][0].1, Some("transform".to_owned())); // has transform

        assert_eq!(structure["c"].len(), 0); // c is a leaf node
    }

    #[test]
    fn test_export_workflow_dot() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");
        workflow.register_agent(create_mock_agent("1", "a", "Agent A", "a"));
        workflow.register_agent(create_mock_agent("2", "b", "Agent B", "b"));

        workflow.connect_agents("a", "b", Flow::default()).unwrap();

        let dot = workflow.export_workflow_dot();

        assert!(dot.contains("digraph {"));
        assert!(dot.contains("\"a\" [label=\"a\"]"));
        assert!(dot.contains("\"b\" [label=\"b\"]"));
        assert!(dot.contains("\"a\" -> \"b\""));
        assert!(dot.contains("}"));
    }

    #[tokio::test]
    async fn test_caching_execution_results() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");

        // mock counter agent
        let mut agent = MockAgent::new();
        let agent_name = "counter".to_owned();
        agent.expect_name().return_const(agent_name.clone());
        agent.expect_id().return_const("1".to_owned());
        agent
            .expect_description()
            .return_const("Counter Agent".to_owned());

        let mut count = 0;
        agent.expect_run().returning(move |_| {
            count += 1;
            Box::pin(future::ready(Ok(format!("Called {count} times"))))
        });

        agent
            .expect_run_multiple_tasks()
            .returning(|_| Box::pin(future::ready(Ok(vec![]))));

        workflow.register_agent(Arc::new(agent));

        // first execution
        let results1 = workflow
            .execute_workflow(&["counter"], "input1")
            .await
            .unwrap();
        assert_eq!(
            results1.get("counter").unwrap().as_ref().unwrap(),
            "Called 1 times"
        );

        // second execution (should reset and call again)
        let results2 = workflow
            .execute_workflow(&["counter"], "input2")
            .await
            .unwrap();
        assert_eq!(
            results2.get("counter").unwrap().as_ref().unwrap(),
            "Called 2 times"
        );

        // call execute_agent directly (should not use cache)
        let result3 = workflow
            .execute_agent("counter", "input3".to_owned())
            .await
            .unwrap();
        assert_eq!(result3, "Called 3 times");
    }

    #[tokio::test]
    async fn test_execute_node_result_caching() {
        let mut workflow = DAGWorkflow::new("test", "Test workflow");

        // Create a mock agent that records the number of calls
        let mut agent1 = MockAgent::new();
        agent1.expect_name().return_const("agent1".to_owned());
        agent1.expect_id().return_const("1".to_owned());
        agent1
            .expect_description()
            .return_const("First agent".to_owned());

        // Set a counter to verify that the run method was called only once
        let mut run_count = 0;
        agent1.expect_run().returning(move |input| {
            run_count += 1;
            Box::pin(future::ready(Ok(format!(
                "response for '{input}' (call #{run_count})"
            ))))
        });

        agent1
            .expect_run_multiple_tasks()
            .returning(|_| Box::pin(future::ready(Ok(vec![]))));

        workflow.register_agent(Arc::new(agent1));

        // Create a normal second proxy
        workflow.register_agent(create_mock_agent(
            "2",
            "agent2",
            "Second agent",
            "response2",
        ));

        // connect the two agents
        workflow
            .connect_agents("agent1", "agent2", Flow::default())
            .unwrap();

        let agent1_idx = *workflow.name_to_node.get("agent1").unwrap();

        // create shared data structures
        let results = Arc::new(DashMap::new());
        let edge_tracker = Arc::new(DashMap::new());
        let processed_nodes = Arc::new(DashMap::new());

        // first execution of agent1
        let result1 = workflow
            .execute_node(
                agent1_idx,
                "input1".to_owned(),
                Arc::clone(&results),
                Arc::clone(&edge_tracker),
                Arc::clone(&processed_nodes),
            )
            .await
            .unwrap();

        assert_eq!(result1, "response for 'input1' (call #1)");
        assert!(results.contains_key("agent1"));
        assert!(results.contains_key("agent2")); // agent2 also executed

        // second execution of agent1 with a different input
        let result2 = workflow
            .execute_node(
                agent1_idx,
                "input2".to_owned(),
                Arc::clone(&results),
                Arc::clone(&edge_tracker),
                Arc::clone(&processed_nodes),
            )
            .await
            .unwrap();

        // the results should be the same, indicating that the agent was not executed again
        assert_eq!(result2, "response for 'input1' (call #1)"); // not "response for 'input2' (call #1)"

        // clear the results map
        results.clear();

        // third execution of agent1
        let result3 = workflow
            .execute_node(
                agent1_idx,
                "input3".to_owned(),
                Arc::clone(&results),
                Arc::clone(&edge_tracker),
                Arc::clone(&processed_nodes),
            )
            .await
            .unwrap();

        // the results should contain the new call count, indicating that the agent was re-executed
        assert_eq!(result3, "response for 'input3' (call #2)");
    }
}
