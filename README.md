A tool for validating planning models specified in the Hierarchical Domain Definition Language (HDDL)[^1] format based on the official HDDL syntax specification and concrete tests (including those pointing to *potential* issues) as proposed by Sleath and Bercher (2023)[^2]. HDDL Parser aims to help domain modelers catch bugs by providing strong type checking, error localization, and ensuring syntactic correctness. It can also convert models between HDDL and a machine-friendly JSON representation (in both directions), enabling programmatic inspection and modification of planning models.

# Detected Errors
In what follows, we provide the currently supported list of errors (for further details, see the paper by Sleath and Bercher (2023)[^2]). Once detected, actionable error messages in conjunction with the exact error location are returned to help modelers fix problems quickly.
* **Basic Syntax Issues**: Detects deviations from the HDDL grammar.
* **Inconsistent Parameter Usage**: Catches type mismatches and incorrect parameter counts in task/predicate/function usage.
* **Undefined Entities**: Identifies undefined predicates, functions, types, objects, and tasks.
* **Duplicate Definitions**: Finds duplicate tasks, methods, predicates, functions, and constants.
* **Cyclic Type Declarations**: Identifies cycles in the type hierarchy.
* **Undeclared Parameters**: Catches use of parameters not declared in tasks or methods.
* **Task Network Issues**: Detects cyclic ordering declaration.
* **Unrefinable Tasks**: Flags compound tasks that do not have a primitive refinement.

# Build Instruction
The following steps must be taken to compile the project. Wherever we mention "project_directory", we mean the root folder where the ```cargo.toml``` file is located.
1. This project was written in the Rust programming language and requires its compiler (and cargo package manager) to be built.
If you do not have it installed, follow the official installation guide ([link](https://www.rust-lang.org/tools/install)).
2. Open a terminal in the project_directory, and execute ```cargo build --release```.
3. If all steps are done successfully, the executable file can be located in ```/project_directory/target/release/hddl_analyzer.exe```. Notice that in this step and subsequent ones where we refer to the ```hddl_analyzer.exe``` file, the ".exe" part might be something else based on your operating system.

# Usage
The build process creates two binary files: The command line tool, ```hddl_analyzer.exe```, and the language server, ```language_server.exe```.
## Command Line
With ```hddl_analyzer.exe```, you can execute the following commands. Every command accepts its input either as HDDL files or as a whole-program JSON export; the format is inferred from the file extension (```.hddl``` or ```.json```, anything else is rejected). Since a JSON export contains the whole program, the ```-p``` option only applies to HDDL input.
* To verify a domain, use ```/path/to/hddl_analyzer.exe verify /path/to/domain.hddl```
* To verify a problem, use ```/path/to/hddl_analyzer.exe verify /path/to/domain.hddl -p /path/to/problem.hddl```
* To verify a JSON export, use ```/path/to/hddl_analyzer.exe verify /path/to/program.json```
* To get general information about the domain (e.g., hierarchy class), use ```/path/to/hddl_analyzer.exe metadata /path/to/domain.hddl```
* To convert between HDDL and JSON, use ```/path/to/hddl_analyzer.exe convert /path/to/input [-p /path/to/problem.hddl] [--to hddl|json] [-o output]```. The output format defaults to JSON. When emitting HDDL for a program that contains a problem, the domain is written to the ```-o``` path and the problem to a derived sibling file (e.g., ```-o out.hddl``` also produces ```out.problem.hddl```); without ```-o```, both are printed to stdout.
* For a complete list of commands and options, use ```/path/to/hddl_analyzer.exe --help```
## Language Server
Executing ```language_server.exe```, activates the language server on the current terminal. At the moment, we only support communication via the standard IO, specified by the Language Server Protocol (LSP).

## Sequential plan validation

`validate-plan` checks a source-level IPC 2020/2023 PANDA hierarchical certificate
against the original domain and problem. Convert grounded engine output with
`pandaPIparser -c` first:

```sh
pandaPIparser -c plan.raw plan.converted
target/release/hddl_analyzer validate-plan domain.hddl -p problem.hddl \
  --plan plan.converted --max-work 1000000
```

The command reconstructs the root and method parameter bindings omitted by that
format, checks the decomposition and inherited ordering, and replays the action
sequence. It supports typed objects and constants, equality, negative/disjunctive/
quantified conditions, propositional conditional/universal effects, method
preconditions, empty decompositions and goals. Method guards are effectless events
that may occur in any permitted state before their subtasks, following HDDL's
semantics. Conditional effects read the state before the action; adds prevail over
deletes. Input task networks may be partially ordered, but the submitted plan must
be a single sequence.

The validation command enforces the action-cost fragment in the
[18 November 2026 IPC 2027 language specification](https://ipc2027-htn.gitlab.io/files/HTN-IPC-2027-language-spec-1.0.pdf).
There may be at most one top-level cost increase per action; numeric effects inside
`when` or `forall` are rejected. Used static cost terms must be initialized.
Uninitialized terms for unused ground actions are allowed. Uncharged actions cost
zero with the total-cost metric, and actions cost one without it. Reported costs
use the parser's floating-point numeric representation; this is validation of
feasibility, not an optimality proof. Existing `verify`/`convert` commands continue
to accept their broader PDDL fragment.

The command prints a JSON result and exits with `0` only for a valid certificate,
`1` for invalid input, or `2` for unsupported input or a work limit. The work limit
covers parameter reconstruction and replay, including quantified instantiation;
it is not a wall-time or memory limit. Guard-only method parameters are searched
under the same budget. Source subtask lists in the PANDA certificate must retain
their declaration order. Decomposition depth above 512 is unsupported. An
unsolvability report is not a solution certificate and cannot produce a valid
result. The proposed HDDL 1.1 output syntax and partial-order plan validation are
not implemented by this command.

Library users can call `plan::validate_panda_plan` with a parsed `HDDLProgram`, or
`plan::validate_plan` with an explicit in-memory certificate (including parameter
bindings). Library inputs should use consistent lowercase names; the CLI
normalizes HDDL files and PANDA certificates. The certificate data structures do
not prescribe a new on-disk output format.

# Correctness
This tool is validated against all 33 hierarchical domains in IPC 2023 (<a href="https://github.com/ipc2023-htn/ipc2023-domains">link</a>), and the Sleath-Bercher benchmark (<a href="https://github.com/ProfDrChaos/flawedPlanningModels/tree/main/HDDL">link</a>).

# Feedback
We welcome contributions and feedback from the planning community. The tool is designed to be extensible for adding new error checks as domain modeling best practices evolve. If you have spotted any bugs, please report them using this repository's "Issues" tab. Alternatively, you can email
- Mohammad Yousefi (<a href="mailto:mohammad.yousefi@anu.edu.au">mohammad.yousefi@anu.edu.au</a>) for technical enquiries, or
- Pascal Bercher (<a href="mailto:pascal.bercher@anu.edu.au">pascal.bercher@anu.edu.au</a>) for other purposes.

# Citation

If you use HDDL Parser in your research, please cite:

```bibtex
@inproceedings{yousefi2025hddlparser,
  author    = {Yousefi, Mohammad and Bercher, Pascal},
  title     = {{HDDL Parser}: A Realtime Hierarchical Planning Language Validation Toolkit},
  booktitle = {ICAPS 2025 Demonstrations},
  address   = {Melbourne, Australia},
  year      = {2025},
  url       = {https://icaps25.icaps-conference.org/program/demos-pdfs/ICAPS25-Demo_paper_2.pdf}
}
```

[^1]: Höller, Daniel, Gregor Behnke, Pascal Bercher, Susanne Biundo, Humbert Fiorino, Damien Pellier, and Ron Alford. "HDDL: An Extension to PDDL for Expressing Hierarchical Planning Problems". In Proceedings of the 34th Association for the Advancement of Artificial Intelligence (AAAI) Conference, 9883–9891. AAAI Press, 2020.

[^2]: Sleath, Kayleigh, and Pascal Bercher. "Detecting AI Planning Modelling Mistakes -- Potential Errors and Benchmark Domains". In Proceedings of the 20th Pacific Rim International Conference on Artificial Intelligence (PRICAI), 448–454. Springer, 2023.
