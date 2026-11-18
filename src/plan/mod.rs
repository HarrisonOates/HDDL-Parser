//! HDDL 1.1 solution certificates (IPC 2027 language specification, 18 November 2026).
mod panda;
mod validate;

pub use validate::{
    Validation, ValidationError, validate_model, validate_panda_plan, validate_plan,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroundTask {
    pub name: String,
    pub arguments: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskOccurrence {
    pub id: String,
    pub task: GroundTask,
}

#[derive(Clone, Debug)]
pub struct Decomposition {
    pub task: TaskOccurrence,
    pub method: GroundTask,
    pub subtasks: Vec<TaskOccurrence>,
}

#[derive(Clone, Debug)]
pub struct Solution {
    pub root_parameters: Vec<String>,
    pub root_tasks: Vec<TaskOccurrence>,
    pub decompositions: Vec<Decomposition>,
    pub actions: Vec<TaskOccurrence>,
    pub ordered: bool,
    pub ordering: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct PlanFile {
    pub name: String,
    pub domain: String,
    pub problem: String,
    /// None is an unsolvability report, not a verified proof of unsolvability.
    pub solution: Option<Solution>,
}
