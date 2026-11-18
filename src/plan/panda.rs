use super::*;
use std::collections::BTreeMap;

fn invalid(message: &str) -> ValidationError {
    ValidationError::Invalid(message.into())
}

fn id(value: &str) -> Result<String, ValidationError> {
    let id: u64 = value
        .parse()
        .map_err(|_| invalid("PANDA task identifiers must be nonnegative integers"))?;
    Ok(format!("n{id}"))
}

fn task(words: &[&str]) -> Result<GroundTask, ValidationError> {
    if words.is_empty()
        || words.iter().any(|s| {
            s.is_empty()
                || !s
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        })
    {
        return Err(invalid(
            "expected source task names; run pandaPIparser -c before validation",
        ));
    }
    Ok(GroundTask {
        name: words[0].into(),
        arguments: words[1..].iter().map(|s| (*s).into()).collect(),
    })
}

pub(super) fn parse(text: &str, domain: &str, problem: &str) -> Result<PlanFile, ValidationError> {
    let mut started = false;
    let mut ended = false;
    let mut roots = None;
    let mut tasks = BTreeMap::new();
    let mut actions = vec![];
    let mut methods = vec![];
    for line in text.lines() {
        let line = line.split(';').next().unwrap().trim().to_ascii_lowercase();
        let words: Vec<_> = line.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        if words == ["==>"] {
            if started {
                return Err(invalid("multiple PANDA plan blocks"));
            }
            started = true;
            continue;
        }
        if !started {
            continue;
        }
        if words == ["<=="] {
            if ended || roots.is_none() {
                return Err(invalid("unexpected PANDA end marker"));
            }
            ended = true;
            continue;
        }
        if ended {
            return Err(invalid("unexpected text after PANDA plan"));
        }
        if words[0] == "root" {
            if roots.is_some() {
                return Err(invalid("duplicate PANDA root row"));
            }
            roots = Some(
                words[1..]
                    .iter()
                    .map(|s| id(s))
                    .collect::<Result<Vec<_>, _>>()?,
            );
            continue;
        }
        let key = id(words[0])?;
        let occurrence = if roots.is_none() {
            let occurrence = TaskOccurrence {
                id: key.clone(),
                task: task(&words[1..])?,
            };
            actions.push(occurrence.clone());
            occurrence
        } else {
            let arrow = words
                .iter()
                .position(|w| *w == "->")
                .ok_or_else(|| invalid("missing method arrow"))?;
            if arrow + 1 >= words.len() {
                return Err(invalid("missing method name"));
            }
            let occurrence = TaskOccurrence {
                id: key.clone(),
                task: task(&words[1..arrow])?,
            };
            let method = task(&words[arrow + 1..arrow + 2])?;
            let children = words[arrow + 2..]
                .iter()
                .map(|s| id(s))
                .collect::<Result<Vec<_>, _>>()?;
            methods.push((occurrence.clone(), method, children));
            occurrence
        };
        if tasks.insert(key, occurrence).is_some() {
            return Err(invalid("duplicate PANDA task id"));
        }
    }
    let roots = roots.ok_or_else(|| invalid("missing PANDA plan or root row"))?;
    let lookup = |key: &String| {
        tasks
            .get(key)
            .cloned()
            .ok_or_else(|| invalid("unknown PANDA subtask id"))
    };
    let root_tasks = roots.iter().map(lookup).collect::<Result<_, _>>()?;
    let decompositions = methods
        .into_iter()
        .map(|(task, method, children)| {
            Ok(Decomposition {
                task,
                method,
                subtasks: children.iter().map(lookup).collect::<Result<_, _>>()?,
            })
        })
        .collect::<Result<_, ValidationError>>()?;
    Ok(PlanFile {
        name: "panda-certificate".into(),
        domain: domain.into(),
        problem: problem.into(),
        solution: Some(Solution {
            root_parameters: vec![],
            root_tasks,
            decompositions,
            actions,
            ordered: true,
            ordering: vec![],
        }),
    })
}
