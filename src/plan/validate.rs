use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::{GroundTask, PlanFile, Solution, TaskOccurrence};
use crate::*;

type Bindings = BTreeMap<String, String>;
type Atom = Vec<String>;
type State = BTreeSet<Atom>;
type Result<T> = std::result::Result<T, ValidationError>;

#[derive(Clone, Debug, PartialEq)]
pub enum Validation {
    Valid {
        actions: usize,
        cost: f64,
        makespan: usize,
        explored: usize,
    },
    /// Parsing an unsolvability report does not establish unsolvability.
    UnsolvabilityReport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    Invalid(String),
    Unsupported(String),
    LimitExceeded,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "invalid plan or model: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported: {message}"),
            Self::LimitExceeded => write!(f, "validation inconclusive: work limit exceeded"),
        }
    }
}
impl std::error::Error for ValidationError {}

fn invalid(message: impl Into<String>) -> ValidationError {
    ValidationError::Invalid(message.into())
}

struct Budget {
    remaining: usize,
    used: usize,
}
impl Budget {
    fn tick(&mut self) -> Result<()> {
        if self.remaining == 0 {
            return Err(ValidationError::LimitExceeded);
        }
        self.remaining -= 1;
        self.used += 1;
        Ok(())
    }
}

struct World<'a> {
    program: &'a HDDLProgram<'a>,
    objects: BTreeMap<String, String>,
    parents: BTreeMap<String, String>,
    values: BTreeMap<Atom, f64>,
}

fn number(value: &NumberType) -> Result<f64> {
    let value = match value {
        NumberType::Integer(n) => *n as f64,
        NumberType::Real(n) => *n,
    };
    if !value.is_finite() || value < 0.0 {
        return Err(invalid("costs must be finite and nonnegative"));
    }
    Ok(value)
}

fn resolve(term: &str, bindings: &Bindings) -> Result<String> {
    if term.starts_with('?') {
        bindings
            .get(term)
            .cloned()
            .ok_or_else(|| invalid(format!("unbound variable {term}")))
    } else {
        Ok(term.into())
    }
}

fn atom(name: &str, arguments: &[Symbol<'_>], bindings: &Bindings) -> Result<Atom> {
    let mut result = vec![name.into()];
    for argument in arguments {
        result.push(resolve(argument.name, bindings)?);
    }
    Ok(result)
}

impl<'a> World<'a> {
    fn new(program: &'a HDDLProgram<'a>) -> Result<Self> {
        program.verify().map_err(|e| invalid(e.to_string()))?;
        let problem = program
            .problem
            .as_ref()
            .ok_or_else(|| invalid("a problem is required"))?;
        if program.domain.name != problem.domain_name {
            return Err(invalid("problem domain name mismatch"));
        }
        let mut world = Self {
            program,
            objects: BTreeMap::new(),
            parents: BTreeMap::new(),
            values: BTreeMap::new(),
        };
        for kind in program.domain.types.iter().flatten() {
            world.parents.insert(
                kind.name.into(),
                kind.symbol_type.unwrap_or("object").into(),
            );
        }
        for object in program
            .domain
            .constants
            .iter()
            .flatten()
            .chain(&problem.objects)
        {
            if world
                .objects
                .insert(
                    object.name.into(),
                    object.symbol_type.unwrap_or("object").into(),
                )
                .is_some()
            {
                return Err(invalid(format!(
                    "duplicate object or constant {}",
                    object.name
                )));
            }
        }
        for value in problem.init_values.iter().flatten() {
            let key = atom(value.fluent.name, &value.fluent.variables, &Bindings::new())?;
            if world.values.insert(key, number(&value.value)?).is_some() {
                return Err(invalid("duplicate numeric initialization"));
            }
        }
        world.check_fragment()?;
        Ok(world)
    }

    fn subtype(&self, actual: &str, expected: &str) -> bool {
        let mut current = actual;
        let mut seen = BTreeSet::new();
        loop {
            if current == expected {
                return true;
            }
            if current == "object" || !seen.insert(current) {
                return false;
            }
            current = self
                .parents
                .get(current)
                .map(String::as_str)
                .unwrap_or("object");
        }
    }

    fn bind(&self, parameters: &[Symbol<'_>], arguments: &[String]) -> Result<Bindings> {
        if parameters.len() != arguments.len() {
            return Err(invalid("parameter count mismatch"));
        }
        let mut bindings = Bindings::new();
        for (parameter, argument) in parameters.iter().zip(arguments) {
            let actual = self
                .objects
                .get(argument)
                .ok_or_else(|| invalid(format!("unknown object {argument}")))?;
            if !self.subtype(actual, parameter.symbol_type.unwrap_or("object")) {
                return Err(invalid(format!(
                    "ill-typed argument {argument} for {}",
                    parameter.name
                )));
            }
            if bindings
                .insert(parameter.name.into(), argument.clone())
                .is_some()
            {
                return Err(invalid("duplicate parameter"));
            }
        }
        Ok(bindings)
    }

    fn check_task(&self, task: &GroundTask) -> Result<()> {
        let domain = &self.program.domain;
        let parameters = if let Some(action) = domain.actions.iter().find(|a| a.name == task.name) {
            &action.parameters
        } else if let Some(compound) = domain.compound_tasks.iter().find(|t| t.name == task.name) {
            &compound.parameters
        } else {
            return Err(invalid(format!("unknown task {}", task.name)));
        };
        self.bind(parameters, &task.arguments)?;
        Ok(())
    }

    fn check_fragment(&self) -> Result<()> {
        let domain = &self.program.domain;
        let problem = self.program.problem.as_ref().unwrap();
        let requirements: Vec<_> = domain
            .requirements
            .iter()
            .chain(&problem.requirements)
            .collect();
        let costs = requirements.contains(&&RequirementType::ActionCosts);
        let makespan = requirements.contains(&&RequirementType::Makespan);
        if costs && makespan {
            return Err(invalid(
                ":action-costs and :makespan are mutually exclusive",
            ));
        }
        if requirements.iter().any(|r| {
            matches!(
                r,
                RequirementType::Fluents
                    | RequirementType::NumericFluents
                    | RequirementType::ProbabilisticEffects
                    | RequirementType::DomainAxioms
                    | RequirementType::DurativeActions
                    | RequirementType::DurationInequalities
                    | RequirementType::ContinuousEffects
            )
        }) {
            return Err(ValidationError::Unsupported(
                "requirements outside HDDL 1.1".into(),
            ));
        }
        let mut numeric_actions = 0;
        for action in &domain.actions {
            if let Some(effect) = &action.effects {
                numeric_actions += check_effect(effect, true)?;
            }
        }
        if domain.functions.iter().any(|f| f.name == "makespan")
            || self.values.keys().any(|k| k[0] == "makespan")
        {
            return Err(invalid("makespan is not a function"));
        }
        if !costs
            && (!domain.functions.is_empty() || !self.values.is_empty() || numeric_actions != 0)
        {
            return Err(invalid("numeric features require :action-costs"));
        }
        let cost_metric = matches!(&problem.metric, Some(Metric::Minimize(NumericExpr::Fluent(f))) if f.name == "total-cost" && f.variables.is_empty());
        match &problem.metric {
            None if costs || numeric_actions != 0 => {
                return Err(invalid("action costs require the total-cost metric"));
            }
            Some(Metric::Makespan) if !makespan || numeric_actions != 0 => {
                return Err(invalid(
                    "makespan requires :makespan and forbids cost effects",
                ));
            }
            Some(Metric::Makespan) | None => (),
            _ if !cost_metric || !costs || numeric_actions == 0 => {
                return Err(invalid(
                    "expected total-cost metric with at least one cost effect",
                ));
            }
            _ => (),
        }
        if costs
            && (!domain
                .functions
                .iter()
                .any(|f| f.name == "total-cost" && f.variables.is_empty())
                || self.values.get(&vec!["total-cost".into()]) != Some(&0.0))
        {
            return Err(invalid(
                "action costs require a zero-argument total-cost initialized to 0",
            ));
        }
        let constants: Bindings = domain
            .constants
            .iter()
            .flatten()
            .map(|s| (s.name.into(), s.symbol_type.unwrap_or("object").into()))
            .collect();
        for action in &domain.actions {
            let scope = self.scope(&constants, &action.parameters)?;
            if let Some(condition) = &action.preconditions {
                self.check_condition(condition, &scope)?;
            }
            if let Some(effect) = &action.effects {
                self.check_effect_types(effect, &scope)?;
            }
        }
        for method in &domain.methods {
            let scope = self.scope(&constants, &method.params)?;
            if let Some(condition) = &method.precondition {
                self.check_condition(condition, &scope)?;
            }
            self.check_constraints(&method.tn, &scope)?;
        }
        if let Some(root) = &problem.init_tn {
            let scope = self.scope(
                &self.objects,
                root.parameters.as_deref().unwrap_or_default(),
            )?;
            self.check_constraints(&root.tn, &scope)?;
        }
        if let Some(goal) = &problem.goal {
            self.check_condition(goal, &self.objects)?;
        }
        Ok(())
    }

    fn scope(&self, outer: &Bindings, variables: &[Symbol<'_>]) -> Result<Bindings> {
        let mut scope = outer.clone();
        let mut declared = BTreeSet::new();
        for variable in variables {
            let kind = variable.symbol_type.unwrap_or("object");
            if !variable.name.starts_with('?') || !declared.insert(variable.name) {
                return Err(invalid("invalid or duplicate quantified parameter"));
            }
            if kind != "object" && !self.parents.contains_key(kind) {
                return Err(invalid(format!("unknown type {kind}")));
            }
            scope.insert(variable.name.into(), kind.into());
        }
        Ok(scope)
    }

    fn check_term(&self, term: &str, scope: &Bindings) -> Result<String> {
        scope
            .get(term)
            .cloned()
            .ok_or_else(|| invalid(format!("undeclared term {term}")))
    }

    fn check_call(
        &self,
        terms: &[Symbol<'_>],
        parameters: &[Symbol<'_>],
        scope: &Bindings,
    ) -> Result<()> {
        if terms.len() != parameters.len() {
            return Err(invalid("predicate or function arity mismatch"));
        }
        for (term, parameter) in terms.iter().zip(parameters) {
            let actual = self.check_term(term.name, scope)?;
            if !self.subtype(&actual, parameter.symbol_type.unwrap_or("object")) {
                return Err(invalid(format!("ill-typed term {}", term.name)));
            }
        }
        Ok(())
    }

    fn check_predicate(&self, predicate: &Predicate<'_>, scope: &Bindings) -> Result<()> {
        let declaration = self
            .program
            .domain
            .predicates
            .iter()
            .find(|p| p.name == predicate.name)
            .ok_or_else(|| invalid("undefined predicate"))?;
        self.check_call(&predicate.variables, &declaration.variables, scope)
    }

    fn check_condition(&self, condition: &Precondition<'_>, scope: &Bindings) -> Result<()> {
        match condition {
            Precondition::Empty => (),
            Precondition::Atom(p) => self.check_predicate(p, scope)?,
            Precondition::Not(p) => self.check_condition(p, scope)?,
            Precondition::And(parts) | Precondition::Or(parts) => {
                for part in parts {
                    self.check_condition(part, scope)?;
                }
            }
            Precondition::Imply(a, b) => {
                self.check_condition(a, scope)?;
                self.check_condition(b, scope)?;
            }
            Precondition::Equals(a, b) => {
                self.check_term(a, scope)?;
                self.check_term(b, scope)?;
            }
            Precondition::ForAll(vars, body) | Precondition::Exists(vars, body) => {
                self.check_condition(body, &self.scope(scope, vars)?)?
            }
            _ => {
                return Err(ValidationError::Unsupported(
                    "condition outside HDDL 1.1".into(),
                ));
            }
        }
        Ok(())
    }

    fn check_effect_types(&self, effect: &Effect<'_>, scope: &Bindings) -> Result<()> {
        match effect {
            Effect::Empty => (),
            Effect::Atom(p) => self.check_predicate(p, scope)?,
            Effect::Not(p) => self.check_effect_types(p, scope)?,
            Effect::And(parts) => {
                for part in parts {
                    self.check_effect_types(part, scope)?;
                }
            }
            Effect::Imply(guard, body) => {
                self.check_condition(guard, scope)?;
                self.check_effect_types(body, scope)?;
            }
            Effect::ForAll(vars, body) => {
                self.check_effect_types(body, &self.scope(scope, vars)?)?
            }
            Effect::Numeric(_, _, NumericExpr::Fluent(f)) => {
                let declaration = self
                    .program
                    .domain
                    .functions
                    .iter()
                    .find(|d| d.name == f.name)
                    .ok_or_else(|| invalid("undefined function"))?;
                self.check_call(&f.variables, &declaration.variables, scope)?;
            }
            Effect::Numeric(_, _, NumericExpr::Number(_)) => (),
            _ => {
                return Err(ValidationError::Unsupported(
                    "effect outside HDDL 1.1".into(),
                ));
            }
        }
        Ok(())
    }

    fn check_constraints(&self, network: &HTN<'_>, scope: &Bindings) -> Result<()> {
        for constraint in network.constraints.iter().flatten() {
            let (a, b) = match constraint {
                Constraint::Equal(a, b) | Constraint::NotEqual(a, b) => (a, b),
            };
            self.check_term(a, scope)?;
            self.check_term(b, scope)?;
        }
        Ok(())
    }

    fn assignments(
        &self,
        variables: &[Symbol<'_>],
        bindings: &Bindings,
        budget: &mut Budget,
    ) -> Result<Vec<Bindings>> {
        let mut result = vec![bindings.clone()];
        for variable in variables {
            let mut next = vec![];
            for binding in &result {
                for (object, kind) in &self.objects {
                    budget.tick()?;
                    if self.subtype(kind, variable.symbol_type.unwrap_or("object")) {
                        let mut binding = binding.clone();
                        binding.insert(variable.name.into(), object.clone());
                        next.push(binding);
                    }
                }
            }
            result = next;
        }
        Ok(result)
    }

    fn condition(
        &self,
        formula: &Precondition<'_>,
        bindings: &Bindings,
        state: &State,
        budget: &mut Budget,
    ) -> Result<bool> {
        budget.tick()?;
        Ok(match formula {
            Precondition::Empty => true,
            Precondition::Atom(p) => state.contains(&atom(p.name, &p.variables, bindings)?),
            Precondition::Not(p) => !self.condition(p, bindings, state, budget)?,
            Precondition::Equals(a, b) => resolve(a, bindings)? == resolve(b, bindings)?,
            Precondition::And(parts) => {
                for part in parts {
                    if !self.condition(part, bindings, state, budget)? {
                        return Ok(false);
                    }
                }
                true
            }
            Precondition::Or(parts) => {
                for part in parts {
                    if self.condition(part, bindings, state, budget)? {
                        return Ok(true);
                    }
                }
                false
            }
            Precondition::Imply(a, b) => {
                !self.condition(a, bindings, state, budget)?
                    || self.condition(b, bindings, state, budget)?
            }
            Precondition::ForAll(vars, body) | Precondition::Exists(vars, body) => {
                let universal = matches!(formula, Precondition::ForAll(_, _));
                for binding in self.assignments(vars, bindings, budget)? {
                    if self.condition(body, &binding, state, budget)? != universal {
                        return Ok(!universal);
                    }
                }
                universal
            }
            Precondition::Xor(_) => {
                return Err(ValidationError::Unsupported("xor conditions".into()));
            }
        })
    }

    fn effects(
        &self,
        effect: &Effect<'_>,
        bindings: &Bindings,
        state: &State,
        add: &mut State,
        delete: &mut State,
        budget: &mut Budget,
    ) -> Result<()> {
        budget.tick()?;
        match effect {
            Effect::Empty | Effect::Numeric(..) => (),
            Effect::Atom(p) => {
                add.insert(atom(p.name, &p.variables, bindings)?);
            }
            Effect::Not(inner) => {
                let Effect::Atom(p) = inner.as_ref() else {
                    return Err(invalid("only atoms may be deleted"));
                };
                delete.insert(atom(p.name, &p.variables, bindings)?);
            }
            Effect::And(parts) => {
                for part in parts {
                    self.effects(part, bindings, state, add, delete, budget)?;
                }
            }
            Effect::Imply(guard, body) => {
                if self.condition(guard, bindings, state, budget)? {
                    self.effects(body, bindings, state, add, delete, budget)?;
                }
            }
            Effect::ForAll(vars, body) => {
                for binding in self.assignments(vars, bindings, budget)? {
                    self.effects(body, &binding, state, add, delete, budget)?;
                }
            }
            _ => {
                return Err(ValidationError::Unsupported(
                    "effect outside HDDL 1.1".into(),
                ));
            }
        }
        Ok(())
    }

    fn cost(&self, action: &Action<'_>, bindings: &Bindings) -> Result<f64> {
        fn numeric<'a>(effect: &'a Effect<'a>) -> Option<&'a NumericExpr<'a>> {
            match effect {
                Effect::Numeric(_, _, amount) => Some(amount),
                Effect::And(parts) => parts.iter().find_map(|e| numeric(e)),
                _ => None,
            }
        }
        let problem = self.program.problem.as_ref().unwrap();
        if !matches!(&problem.metric, Some(Metric::Minimize(_))) {
            return Ok(1.0);
        }
        match action.effects.as_ref().and_then(numeric) {
            None => Ok(0.0),
            Some(NumericExpr::Number(n)) => number(n),
            Some(NumericExpr::Fluent(f)) => {
                let key = atom(f.name, &f.variables, bindings)?;
                self.values.get(&key).copied().ok_or_else(|| {
                    invalid(format!(
                        "action {} reads undefined cost term {}",
                        action.name,
                        key.join(" ")
                    ))
                })
            }
            _ => Err(invalid("unsupported numeric cost expression")),
        }
    }
}

fn check_effect(effect: &Effect<'_>, top: bool) -> Result<usize> {
    let count = match effect {
        Effect::Empty | Effect::Atom(_) => 0,
        Effect::Not(inner) if matches!(inner.as_ref(), Effect::Atom(_)) => 0,
        Effect::Numeric(op, target, amount) => {
            if !top
                || *op != AssignmentOperator::Increase
                || target.name != "total-cost"
                || !target.variables.is_empty()
            {
                return Err(invalid(
                    "cost increase must target total-cost at the top level",
                ));
            }
            match amount {
                NumericExpr::Number(n) => {
                    number(n)?;
                }
                NumericExpr::Fluent(f) if f.name != "total-cost" => (),
                _ => {
                    return Err(invalid(
                        "cost amount must be a number or static function term",
                    ));
                }
            }
            1
        }
        Effect::And(parts) => {
            let mut count = 0;
            for part in parts {
                count += check_effect(part, top && matches!(part.as_ref(), Effect::Numeric(..)))?;
            }
            count
        }
        Effect::Imply(_, body) | Effect::ForAll(_, body) => check_effect(body, false)?,
        _ => {
            return Err(ValidationError::Unsupported(
                "effect outside HDDL 1.1".into(),
            ));
        }
    };
    if count > 1 {
        return Err(invalid("at most one cost increase is permitted per action"));
    }
    Ok(count)
}

struct Event<'a> {
    action: Option<&'a Action<'a>>,
    guard: Option<&'a Precondition<'a>>,
    bindings: Bindings,
    predecessors: BTreeSet<usize>,
}

struct Certificate<'a, 'b> {
    world: &'b World<'a>,
    solution: &'b Solution,
    events: Vec<Event<'a>>,
    actions: BTreeMap<String, usize>,
    introduced: BTreeSet<String>,
    expanded: BTreeSet<String>,
    budget: &'b mut Budget,
}

impl<'a> Certificate<'a, '_> {
    fn task(&mut self, occurrence: &TaskOccurrence, depth: usize) -> Result<Vec<usize>> {
        self.budget.tick()?;
        if depth > 512 {
            return Err(ValidationError::Unsupported(
                "decomposition depth above 512".into(),
            ));
        }
        if !self.introduced.insert(occurrence.id.clone()) {
            return Err(invalid(format!(
                "task id {} introduced more than once (or decomposition cycle)",
                occurrence.id
            )));
        }
        self.world.check_task(&occurrence.task)?;
        if let Some(action) = self
            .world
            .program
            .domain
            .actions
            .iter()
            .find(|a| a.name == occurrence.task.name)
        {
            let index = *self
                .actions
                .get(&occurrence.id)
                .ok_or_else(|| invalid("decomposition leaf missing from plan :htn"))?;
            if self.solution.actions[index] != *occurrence {
                return Err(invalid("action occurrence differs from decomposition leaf"));
            }
            self.events[index].bindings = self
                .world
                .bind(&action.parameters, &occurrence.task.arguments)?;
            return Ok(vec![index]);
        }
        let matches: Vec<_> = self
            .solution
            .decompositions
            .iter()
            .filter(|d| d.task.id == occurrence.id)
            .collect();
        let [entry] = matches.as_slice() else {
            return Err(invalid(
                "compound task must have exactly one decomposition entry",
            ));
        };
        if entry.task != *occurrence {
            return Err(invalid("decomposition task differs from its introduction"));
        }
        self.expanded.insert(occurrence.id.clone());
        let method = self
            .world
            .program
            .domain
            .methods
            .iter()
            .find(|m| m.name.name == entry.method.name)
            .ok_or_else(|| invalid("unknown method"))?;
        let bindings = self.world.bind(&method.params, &entry.method.arguments)?;
        if method.task.name != occurrence.task.name
            || method
                .task_terms
                .iter()
                .map(|s| resolve(s.name, &bindings))
                .collect::<Result<Vec<_>>>()?
                != occurrence.task.arguments
        {
            return Err(invalid("method does not decompose the stated task"));
        }
        let mut events = self.network(&method.tn, &entry.subtasks, &bindings, depth + 1)?;
        if let Some(guard) = &method.precondition {
            let index = self.events.len();
            for &child in &events {
                self.events[child].predecessors.insert(index);
            }
            self.events.push(Event {
                action: None,
                guard: Some(guard),
                bindings,
                predecessors: BTreeSet::new(),
            });
            events.push(index);
        }
        Ok(events)
    }

    fn network(
        &mut self,
        network: &HTN<'a>,
        tasks: &[TaskOccurrence],
        bindings: &Bindings,
        depth: usize,
    ) -> Result<Vec<usize>> {
        if network.subtasks.len() != tasks.len() {
            return Err(invalid("subtask count mismatch"));
        }
        if let Some(constraints) = &network.constraints {
            for constraint in constraints {
                let (a, b, equal) = match constraint {
                    Constraint::Equal(a, b) => (a, b, true),
                    Constraint::NotEqual(a, b) => (a, b, false),
                };
                if (resolve(a, bindings)? == resolve(b, bindings)?) != equal {
                    return Err(invalid("task-network binding constraint violated"));
                }
            }
        }
        let mut children = vec![];
        let mut ids = BTreeMap::new();
        for (index, (schema, occurrence)) in network.subtasks.iter().zip(tasks).enumerate() {
            if schema.task.name != occurrence.task.name
                || schema
                    .terms
                    .iter()
                    .map(|s| resolve(s.name, bindings))
                    .collect::<Result<Vec<_>>>()?
                    != occurrence.task.arguments
            {
                return Err(invalid(
                    "subtasks must match the instantiated network in declaration order",
                ));
            }
            if let Some(id) = &schema.id
                && ids.insert(id.name, index).is_some()
            {
                return Err(invalid("duplicate source subtask identifier"));
            }
            children.push(self.task(occurrence, depth)?);
        }
        let edges = match &network.orderings {
            TaskOrdering::Total => (1..children.len()).map(|i| (i - 1, i)).collect::<Vec<_>>(),
            TaskOrdering::Partial(edges) => edges
                .iter()
                .map(|(a, b)| {
                    Ok((
                        *ids.get(a)
                            .ok_or_else(|| invalid("unknown source ordering id"))?,
                        *ids.get(b)
                            .ok_or_else(|| invalid("unknown source ordering id"))?,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
        };
        let mut reachable = vec![BTreeSet::new(); children.len()];
        for (before, after) in edges {
            reachable[before].insert(after);
        }
        for via in 0..children.len() {
            for before in 0..children.len() {
                self.budget.tick()?;
                if reachable[before].contains(&via) {
                    let after = reachable[via].clone();
                    reachable[before].extend(after);
                }
            }
        }
        for (before, successors) in reachable.iter().enumerate() {
            if successors.contains(&before) {
                return Err(invalid("cyclic source task network"));
            }
            for &after in successors {
                for &after in &children[after] {
                    self.events[after].predecessors.extend(&children[before]);
                }
            }
        }
        Ok(children.into_iter().flatten().collect())
    }
}

/// Validate the source model without requiring a solution certificate.
pub fn validate_model(program: &HDDLProgram<'_>) -> Result<()> {
    World::new(program).map(|_| ())
}

/// Checks a sequential certificate against the original model.
/// Partial-order output is unsupported; partial-order input hierarchies are allowed.
pub fn validate_plan(
    program: &HDDLProgram<'_>,
    plan: &PlanFile,
    max_work: usize,
) -> Result<Validation> {
    let world = World::new(program)?;
    let mut budget = Budget {
        remaining: max_work,
        used: 0,
    };
    validate(&world, plan, &mut budget)
}

fn validate(world: &World<'_>, plan: &PlanFile, budget: &mut Budget) -> Result<Validation> {
    let program = world.program;
    let problem = program.problem.as_ref().unwrap();
    if plan.domain != program.domain.name || plan.problem != problem.name {
        return Err(invalid("plan domain or problem name mismatch"));
    }
    let Some(solution) = &plan.solution else {
        return Ok(Validation::UnsolvabilityReport);
    };
    if !solution.ordered || !solution.ordering.is_empty() {
        return Err(ValidationError::Unsupported(
            "partially ordered plan validation".into(),
        ));
    }
    let count = solution.actions.len();
    let mut certificate = Certificate {
        world,
        solution,
        events: vec![],
        actions: BTreeMap::new(),
        introduced: BTreeSet::new(),
        expanded: BTreeSet::new(),
        budget,
    };
    for (i, occurrence) in solution.actions.iter().enumerate() {
        if certificate
            .actions
            .insert(occurrence.id.clone(), i)
            .is_some()
        {
            return Err(invalid("duplicate action occurrence"));
        }
        let action = program
            .domain
            .actions
            .iter()
            .find(|a| a.name == occurrence.task.name)
            .ok_or_else(|| invalid("plan :htn contains a non-action"))?;
        certificate.events.push(Event {
            action: Some(action),
            guard: None,
            bindings: Bindings::new(),
            predecessors: BTreeSet::new(),
        });
    }
    if let Some(root) = &problem.init_tn {
        let bindings = world.bind(
            root.parameters.as_deref().unwrap_or_default(),
            &solution.root_parameters,
        )?;
        certificate.network(&root.tn, &solution.root_tasks, &bindings, 0)?;
    } else {
        return Err(ValidationError::Unsupported(
            "plans without a source initial task network".into(),
        ));
    }
    if certificate.expanded.len() != solution.decompositions.len()
        || certificate.introduced.len() != count + solution.decompositions.len()
    {
        return Err(invalid("orphaned action or decomposition entry"));
    }
    let events = certificate.events;
    critical_path(
        &events
            .iter()
            .map(|e| e.predecessors.clone())
            .collect::<Vec<_>>(),
    )?;
    let mut cost = 0.0;
    for event in &events[..count] {
        cost += world.cost(event.action.unwrap(), &event.bindings)?;
    }
    if !cost.is_finite() {
        return Err(invalid("plan cost overflow"));
    }
    let mut state = problem
        .init_state
        .iter()
        .map(|p| atom(p.name, &p.variables, &Bindings::new()))
        .collect::<Result<State>>()?;
    let mut done = vec![false; events.len()];
    for next in 0..=count {
        budget.tick()?;
        loop {
            let mut changed = false;
            for (i, event) in events.iter().enumerate().skip(count) {
                budget.tick()?;
                if !done[i]
                    && event.predecessors.iter().all(|&p| done[p])
                    && world.condition(event.guard.unwrap(), &event.bindings, &state, budget)?
                {
                    done[i] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        if next == count {
            break;
        }
        let event = &events[next];
        if !event.predecessors.iter().all(|&p| done[p]) {
            return Err(invalid(format!(
                "hierarchy ordering or method precondition fails before {}",
                solution.actions[next].id
            )));
        }
        let action = event.action.unwrap();
        if let Some(guard) = &action.preconditions
            && !world.condition(guard, &event.bindings, &state, budget)?
        {
            return Err(invalid(format!(
                "action precondition fails for {}",
                solution.actions[next].id
            )));
        }
        let (mut add, mut delete) = (State::new(), State::new());
        if let Some(effect) = &action.effects {
            world.effects(
                effect,
                &event.bindings,
                &state,
                &mut add,
                &mut delete,
                budget,
            )?;
        }
        state.retain(|a| !delete.contains(a));
        state.extend(add);
        done[next] = true;
    }
    if !done.iter().all(|d| *d) {
        return Err(invalid(
            "method preconditions cannot be satisfied in this sequence",
        ));
    }
    if let Some(goal) = &problem.goal
        && !world.condition(goal, &Bindings::new(), &state, budget)?
    {
        return Err(invalid("goal fails after the sequence"));
    }
    Ok(Validation::Valid {
        actions: count,
        cost,
        makespan: count,
        explored: budget.used,
    })
}

fn critical_path(predecessors: &[BTreeSet<usize>]) -> Result<usize> {
    let mut lengths = vec![0; predecessors.len()];
    let mut remaining = predecessors.len();
    while remaining > 0 {
        let previous = remaining;
        for (i, parents) in predecessors.iter().enumerate() {
            if lengths[i] == 0 && parents.iter().all(|&p| lengths[p] > 0) {
                lengths[i] = 1 + parents.iter().map(|&p| lengths[p]).max().unwrap_or(0);
                remaining -= 1;
            }
        }
        if remaining == previous {
            return Err(invalid("cyclic ordering"));
        }
    }
    Ok(lengths.into_iter().max().unwrap_or(0))
}

fn unify(terms: &[Symbol<'_>], arguments: &[String], bindings: &mut Bindings) -> Result<()> {
    if terms.len() != arguments.len() {
        return Err(invalid("source task arity mismatch"));
    }
    for (term, argument) in terms.iter().zip(arguments) {
        if term.name.starts_with('?') {
            if let Some(previous) = bindings.insert(term.name.into(), argument.clone())
                && previous != *argument
            {
                return Err(invalid("inconsistent source method binding"));
            }
        } else if term.name != argument {
            return Err(invalid("source task constant mismatch"));
        }
    }
    Ok(())
}

fn infer_bindings(
    world: &World<'_>,
    parameters: &[Symbol<'_>],
    network: &HTN<'_>,
    tasks: &[TaskOccurrence],
    mut bindings: Bindings,
    budget: &mut Budget,
) -> Result<Vec<Vec<String>>> {
    if network.subtasks.len() != tasks.len() {
        return Err(invalid("source subtask count mismatch"));
    }
    for (schema, task) in network.subtasks.iter().zip(tasks) {
        if schema.task.name != task.task.name {
            return Err(ValidationError::Unsupported(
                "PANDA subtasks must be listed in source declaration order".into(),
            ));
        }
        unify(&schema.terms, &task.task.arguments, &mut bindings)?;
    }
    let unbound: Vec<_> = parameters
        .iter()
        .filter(|p| !bindings.contains_key(p.name))
        .cloned()
        .collect();
    let mut candidates = vec![];
    for binding in world.assignments(&unbound, &bindings, budget)? {
        budget.tick()?;
        let arguments = parameters
            .iter()
            .map(|p| resolve(p.name, &binding))
            .collect::<Result<Vec<_>>>()?;
        world.bind(parameters, &arguments)?;
        let constraints_hold = network
            .constraints
            .iter()
            .flatten()
            .map(|c| {
                let (a, b, equal) = match c {
                    Constraint::Equal(a, b) => (a, b, true),
                    Constraint::NotEqual(a, b) => (a, b, false),
                };
                Ok((resolve(a, &binding)? == resolve(b, &binding)?) == equal)
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .all(|c| c);
        if constraints_hold {
            candidates.push(arguments);
        }
    }
    if candidates.is_empty() {
        return Err(invalid("no legal source parameter binding"));
    }
    Ok(candidates)
}

/// Validate a source-level IPC 2020/2023 PANDA certificate, after `pandaPIparser -c`.
/// Missing method/root parameters are reconstructed under a shared work budget.
pub fn validate_panda_plan(
    program: &HDDLProgram<'_>,
    text: &str,
    max_work: usize,
) -> Result<Validation> {
    let world = World::new(program)?;
    let problem = program.problem.as_ref().unwrap();
    let mut plan = super::panda::parse(text, &program.domain.name, problem.name)?;
    let solution = plan.solution.as_mut().unwrap();
    let mut budget = Budget {
        remaining: max_work,
        used: 0,
    };
    let root = problem.init_tn.as_ref().ok_or_else(|| {
        ValidationError::Unsupported("plans without a source initial task network".into())
    })?;
    let mut candidates = vec![infer_bindings(
        &world,
        root.parameters.as_deref().unwrap_or_default(),
        &root.tn,
        &solution.root_tasks,
        Bindings::new(),
        &mut budget,
    )?];
    for entry in &solution.decompositions {
        let method = program
            .domain
            .methods
            .iter()
            .find(|m| m.name.name == entry.method.name)
            .ok_or_else(|| invalid("unknown method"))?;
        if method.task.name != entry.task.task.name {
            return Err(invalid("method decomposes a different task"));
        }
        let mut bindings = Bindings::new();
        unify(
            &method.task_terms,
            &entry.task.task.arguments,
            &mut bindings,
        )?;
        candidates.push(infer_bindings(
            &world,
            &method.params,
            &method.tn,
            &entry.subtasks,
            bindings,
            &mut budget,
        )?);
    }
    let mut indices = vec![0; candidates.len()];
    loop {
        budget.tick()?;
        let solution = plan.solution.as_mut().unwrap();
        solution.root_parameters = candidates[0][indices[0]].clone();
        for (i, entry) in solution.decompositions.iter_mut().enumerate() {
            entry.method.arguments = candidates[i + 1][indices[i + 1]].clone();
        }
        let error = match validate(&world, &plan, &mut budget) {
            Ok(result) => return Ok(result),
            Err(error @ ValidationError::Invalid(_)) => error,
            Err(error) => return Err(error),
        };
        let mut column = 0;
        loop {
            if column == indices.len() {
                return Err(error);
            }
            indices[column] += 1;
            if indices[column] < candidates[column].len() {
                break;
            }
            indices[column] = 0;
            column += 1;
        }
    }
}
