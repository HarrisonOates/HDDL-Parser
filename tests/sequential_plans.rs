use hddl_analyzer::HDDLProgram;
use hddl_analyzer::plan::{Validation, ValidationError, validate_panda_plan};

const DOMAIN: &str = "(define (domain travel)
 (:requirements :typing :hierarchy :method-preconditions :action-costs)
 (:types location vehicle - object truck - vehicle)
 (:predicates (at ?v - vehicle ?l - location) (finished))
 (:functions (total-cost) (distance ?a ?b - location))
 (:task go :parameters (?v - vehicle ?to - location))
 (:method via :parameters (?v - vehicle ?from ?mid ?to - location)
  :task (go ?v ?to) :precondition (at ?v ?from)
  :ordered-subtasks (and (drive ?v ?from ?mid) (drive ?v ?mid ?to) (finish)))
 (:action drive :parameters (?v - vehicle ?a ?b - location)
  :precondition (at ?v ?a)
  :effect (and (not (at ?v ?a)) (at ?v ?b) (increase (total-cost) (distance ?a ?b))))
 (:action finish :parameters () :effect (finished)))";
const PROBLEM: &str = "(define (problem trip) (:domain travel)
 (:objects v - truck a b c - location)
 (:htn :parameters (?v - vehicle) :ordered-subtasks (go ?v c))
 (:init (at v a) (= (total-cost) 0) (= (distance a b) 4) (= (distance b c) 5))
 (:goal (and (at v c) (finished))) (:metric minimize (total-cost)))";
const PLAN: &str = "==>\n1 drive v a b\n2 drive v b c\n3 finish\nroot 0\n0 go v c -> via 1 2 3\n";

fn check(domain: &str, problem: &str, plan: &str) -> Result<Validation, ValidationError> {
    let (domain, problem) = (domain.as_bytes().to_vec(), problem.as_bytes().to_vec());
    let program = HDDLProgram::from_hddl(&domain, Some(&problem)).unwrap();
    validate_panda_plan(&program, plan, 100_000)
}

#[test]
fn reconstructs_root_and_method_bindings_and_checks_source_costs() {
    assert!(matches!(
        check(DOMAIN, PROBLEM, PLAN),
        Ok(Validation::Valid {
            actions: 3,
            cost: 9.0,
            makespan: 3,
            ..
        })
    ));
    assert!(check(DOMAIN, PROBLEM, &format!("PANDA log\n{PLAN}<==\n")).is_ok());
}

#[test]
fn rejects_corrupted_certificates() {
    for (before, after) in [
        ("drive v a b", "drive a a b"),
        ("-> via", "-> unknown"),
        ("3 finish", "2 finish"),
        ("root 0", "root 99"),
        ("0 go v c", "0 go v b"),
        ("via 1 2 3", "via 1 2 99"),
        ("via 1 2 3", "via 1 2 2"),
        ("==>", ""),
    ] {
        assert!(
            check(DOMAIN, PROBLEM, &PLAN.replace(before, after)).is_err(),
            "{before} -> {after}"
        );
    }
    let reordered = PLAN.replace(
        "1 drive v a b\n2 drive v b c",
        "2 drive v b c\n1 drive v a b",
    );
    assert!(check(DOMAIN, PROBLEM, &reordered).is_err());
    assert!(check(DOMAIN, &PROBLEM.replace("(at v a)", "(at v b)"), PLAN).is_err());
    assert!(
        check(
            DOMAIN,
            &PROBLEM.replace("(:goal (and (at v c)", "(:goal (and (at v a)"),
            PLAN
        )
        .is_err()
    );
    assert!(check(DOMAIN, PROBLEM, &format!("{PLAN}{PLAN}")).is_err());
}

#[test]
fn undefined_cost_is_inapplicable_but_unused_undefined_terms_are_allowed() {
    assert!(check(DOMAIN, PROBLEM, PLAN).is_ok());
    let missing = PROBLEM.replace("(= (distance b c) 5)", "");
    assert!(
        check(DOMAIN, &missing, PLAN)
            .unwrap_err()
            .to_string()
            .contains("undefined cost")
    );
    assert!(check(DOMAIN, &PROBLEM.replace("(= (total-cost) 0)", ""), PLAN).is_err());
}

#[test]
fn rejects_multiple_conditional_and_nested_cost_increases() {
    for effect in [
        "(increase (total-cost) 1) (increase (total-cost) 2)",
        "(when (at ?v ?a) (increase (total-cost) 1))",
        "(forall (?x - location) (increase (total-cost) 1))",
        "(and (increase (total-cost) 1))",
    ] {
        assert!(
            check(
                &DOMAIN.replace("(increase (total-cost) (distance ?a ?b))", effect),
                PROBLEM,
                PLAN
            )
            .is_err(),
            "{effect}"
        );
    }
}

#[test]
fn limits_and_nonsequential_input_never_verify() {
    let domain = DOMAIN.as_bytes().to_vec();
    let problem = PROBLEM.as_bytes().to_vec();
    let program = HDDLProgram::from_hddl(&domain, Some(&problem)).unwrap();
    assert_eq!(
        validate_panda_plan(&program, PLAN, 0),
        Err(ValidationError::LimitExceeded)
    );
    assert!(validate_panda_plan(&program, "unsolvable", 100).is_err());
    assert!(validate_panda_plan(&program, "(define (plan p) (:unsolvable))", 100).is_err());
    assert!(validate_panda_plan(&program, &PLAN.replace("1 drive", "1 @0 drive"), 100).is_err());
}

fn small(
    domain_body: &str,
    tasks: &str,
    init_goal: &str,
    plan: &str,
) -> Result<Validation, ValidationError> {
    check(
        &format!(
            "(define (domain d) (:requirements :hierarchy :method-preconditions :adl) (:predicates (p) (q) (r)) {domain_body})"
        ),
        &format!("(define (problem p) (:domain d) (:htn {tasks}) {init_goal})"),
        plan,
    )
}

#[test]
fn conditional_effect_guards_read_the_pre_state_and_add_wins() {
    let domain = "(:action a :parameters () :effect (and (p) (when (p) (q)) (when (not (p)) (r)) (not (r))))";
    let result = small(
        domain,
        ":ordered-subtasks (a)",
        "(:init) (:goal (and (p) (not (q)) (r)))",
        "==>\n0 a\nroot 0\n",
    );
    assert!(matches!(result, Ok(Validation::Valid { cost: 1.0, .. })));
}

#[test]
fn guarded_empty_decompositions_and_orders_through_empty_tasks() {
    let domain =
        "(:task empty :parameters ()) (:method m :parameters () :task (empty) :ordered-subtasks ())
        (:action a :parameters () :effect (p)) (:action b :parameters () :effect (q))";
    let plan = "==>\n0 a\n2 b\nroot 0 1 2\n1 empty -> m\n";
    assert!(
        small(
            domain,
            ":ordered-subtasks (and (a) (empty) (b))",
            "(:init)",
            plan
        )
        .is_ok()
    );
    assert!(
        small(
            domain,
            ":ordered-subtasks (and (a) (empty) (b))",
            "(:init)",
            &plan.replace("0 a\n2 b", "2 b\n0 a")
        )
        .is_err()
    );
    let guarded = domain.replace(
        ":task (empty) :ordered-subtasks",
        ":task (empty) :precondition (p) :ordered-subtasks",
    );
    assert!(
        small(
            &guarded,
            ":ordered-subtasks (empty)",
            "(:init (p))",
            "==>\nroot 0\n0 empty -> m\n"
        )
        .is_ok()
    );
    assert!(
        small(
            &guarded,
            ":ordered-subtasks (empty)",
            "(:init)",
            "==>\nroot 0\n0 empty -> m\n"
        )
        .is_err()
    );
    assert!(small(domain, ":ordered-subtasks ()", "(:init)", "==>\nroot\n").is_ok());
}

#[test]
fn method_guards_may_hold_before_an_intervening_action_in_a_po_problem() {
    let domain = "(:task task :parameters ()) (:method m :parameters () :task (task) :precondition (p) :ordered-subtasks (a))
        (:action a :parameters () :effect (q)) (:action delete :parameters () :effect (not (p)))";
    let plan = "==>\n1 delete\n2 a\nroot 0 1\n0 task -> m 2\n";
    assert!(
        small(
            domain,
            ":subtasks (and (task) (delete))",
            "(:init (p)) (:goal (q))",
            plan
        )
        .is_ok()
    );
    assert!(
        small(
            domain,
            ":ordered-subtasks (and (delete) (task))",
            "(:init (p))",
            &plan.replace("root 0 1", "root 1 0")
        )
        .is_err()
    );
}

#[test]
fn quantified_conditions_and_effects_use_typed_object_domains() {
    let domain = "(define (domain d) (:requirements :hierarchy :adl)
      (:types item other - object) (:constants c - item) (:predicates (p ?x - item))
      (:action a :parameters () :precondition (forall (?x - item) (not (p ?x)))
       :effect (forall (?x - item) (when (not (p ?x)) (p ?x)))))";
    let problem = "(define (problem p) (:domain d) (:objects x - item y - other)
      (:htn :ordered-subtasks (a)) (:init)
      (:goal (and (forall (?x - item) (p ?x)) (exists (?x - item) (p ?x)))))";
    assert!(check(domain, problem, "==>\n0 a\nroot 0\n").is_ok());
}

#[test]
fn reconstructs_parameters_used_only_in_method_preconditions() {
    let domain = "(define (domain d) (:requirements :hierarchy :typing :method-preconditions)
      (:types item) (:predicates (p ?x - item)) (:task task :parameters ())
      (:method m :parameters (?x - item) :task (task) :precondition (p ?x) :ordered-subtasks (a))
      (:action a :parameters ()))";
    let problem = "(define (problem p) (:domain d) (:objects x y - item) (:htn :ordered-subtasks (task)) (:init (p y)))";
    let plan = "==>\n1 a\nroot 0\n0 task -> m 1\n";
    assert!(check(domain, problem, plan).is_ok());
    assert!(check(domain, &problem.replace("(p y)", ""), plan).is_err());
}

#[test]
fn rejects_ill_typed_quantified_terms_even_in_unused_branches() {
    let domain = "(define (domain d) (:requirements :hierarchy :adl)
      (:types item other - object) (:predicates (p ?x - item))
      (:action a :parameters () :precondition (forall (?x - other) (not (p ?x)))))";
    let problem = "(define (problem p) (:domain d) (:htn :ordered-subtasks (a)) (:init))";
    assert!(check(domain, problem, "==>\n0 a\nroot 0\n").is_err());
}

#[test]
fn method_binding_constraints_are_enforced() {
    let domain = "(define (domain d) (:requirements :hierarchy :typing :equality)
      (:types item) (:predicates (p)) (:task task :parameters (?x ?y - item))
      (:method m :parameters (?x ?y - item) :task (task ?x ?y)
       :ordered-subtasks (a) :constraints (not (= ?x ?y))) (:action a :parameters ()))";
    let problem = "(define (problem p) (:domain d) (:objects x y - item)
      (:htn :ordered-subtasks (task x y)) (:init))";
    let plan = "==>\n1 a\nroot 0\n0 task x y -> m 1\n";
    assert!(check(domain, problem, plan).is_ok());
    assert!(
        check(
            domain,
            &problem.replace("task x y", "task x x"),
            &plan.replace("task x y", "task x x")
        )
        .is_err()
    );
}

#[test]
fn cli_reports_valid_invalid_and_inconclusive_and_normalizes_case() {
    use std::{fs, process::Command};
    let path = std::env::temp_dir().join(format!("hddl-sequential-cli-{}", std::process::id()));
    fs::create_dir(&path).unwrap();
    let domain = path.join("domain.hddl");
    let problem = path.join("problem.hddl");
    let plan = path.join("plan.converted");
    fs::write(&domain, DOMAIN.to_uppercase()).unwrap();
    fs::write(&problem, PROBLEM.to_uppercase()).unwrap();
    fs::write(&plan, PLAN).unwrap();
    let run = |work: &str| {
        Command::new(env!("CARGO_BIN_EXE_hddl_analyzer"))
            .arg("validate-plan")
            .arg(&domain)
            .arg("-p")
            .arg(&problem)
            .arg("--plan")
            .arg(&plan)
            .arg("--max-work")
            .arg(work)
            .output()
            .unwrap()
    };
    let result = run("100000");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["status"], "valid");
    assert_eq!(json["cost"], 9.0);
    let result = run("0");
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()["status"],
        "inconclusive"
    );
    fs::write(
        &plan,
        PLAN.replace(
            "1 drive v a b\n2 drive v b c",
            "2 drive v b c\n1 drive v a b",
        ),
    )
    .unwrap();
    let result = run("100000");
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()["status"],
        "invalid"
    );
    fs::remove_dir_all(path).unwrap();
}
