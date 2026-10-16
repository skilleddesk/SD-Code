//! **`sdcd plan`** (P0-0): the Plan Tracker, so SDC can follow `sdc/docs/PROMPT-BOOK-SDC.md` by itself.
//!
//! ```text
//! sdcd plan next      [--docs DIR]   # the first task whose dependencies and phase gate allow it
//! sdcd plan status    [--docs DIR]   # counts per phase and per status
//! sdcd plan validate  [--docs DIR]   # missing IDs, unknown dependencies, cycles, a stale PLAN-STATUS.md
//! sdcd plan sync      [--docs DIR]   # (re)writes PLAN-STATUS.md from the Prompt Book, keeping each row's status
//! sdcd plan set ID STATUS [--note TEXT] [--report PATH] [--docs DIR]
//! ```
//!
//! The Prompt Book is the source of truth for IDs, order, dependencies and switch names; `PLAN-STATUS.md`
//! only carries what changes (status, report link, note). The plan itself is never written here.
//! Exit codes: `0` ok, `1` the plan is invalid or nothing is eligible, `3` the command line was wrong.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub const STATUSES: [&str; 4] = ["todo", "in-progress", "blocked", "done"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub phase: u8,
    /// Task IDs this one waits for (`all P1` is expanded; a `Gate N` goes to `gate`).
    pub depends_on: Vec<String>,
    /// The gate that must read PASS before this task starts (phase N needs Gate N-1).
    pub gate: Option<u8>,
    pub switch: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    pub status: String,
    pub report: String,
    pub note: String,
}

/// Parses the Prompt Book: `## Phase N` sets the phase, `### P0-0 Title` opens a task, and the task's
/// `TASK … Depends: … Switch: ….` line gives the dependencies and the switch.
pub fn parse_prompt_book(text: &str) -> Vec<Task> {
    let mut tasks: Vec<Task> = Vec::new();
    let mut phase_ids: BTreeMap<u8, Vec<String>> = BTreeMap::new();
    let mut raw_depends: Vec<String> = Vec::new();
    let mut in_code = false;

    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }

        if in_code {
            continue;
        }

        if let Some(rest) = line.strip_prefix("### ") {
            let mut parts = rest.splitn(2, ' ');
            let id = parts.next().unwrap_or("");

            if let Some(phase) = phase_of(id) {
                phase_ids.entry(phase).or_default().push(id.to_string());
                tasks.push(Task { id: id.to_string(), title: parts.next().unwrap_or("").trim().to_string(), phase, depends_on: Vec::new(), gate: None, switch: String::new() });
                raw_depends.push(String::new());
            }

            continue;
        }

        if let Some(rest) = line.strip_prefix("TASK ") {
            if let (Some(task), Some(at)) = (tasks.last_mut(), rest.find("Depends:")) {
                if rest.starts_with(&task.id) {
                    let after = &rest[at + "Depends:".len()..];
                    let (depends, switch) = after.split_once("Switch:").unwrap_or((after, ""));

                    *raw_depends.last_mut().expect("one per task") = depends.trim().trim_end_matches('.').to_string();
                    task.switch = switch.trim().trim_end_matches('.').to_string();
                }
            }
        }
    }

    for (index, task) in tasks.iter_mut().enumerate() {
        let mut ids: Vec<String> = Vec::new();

        for part in raw_depends[index].split(',').map(str::trim).filter(|part| !part.is_empty() && !part.eq_ignore_ascii_case("none")) {
            if let Some(number) = part.strip_prefix("Gate ").and_then(|n| n.trim().parse::<u8>().ok()) {
                task.gate = Some(number);
            } else if let Some(phase) = part.strip_prefix("all P").or_else(|| part.strip_prefix("all ")).and_then(|p| p.trim().trim_start_matches('P').parse::<u8>().ok()) {
                ids.extend(phase_ids.get(&phase).into_iter().flatten().filter(|id| **id != task.id).cloned());
            } else {
                ids.push(part.to_string());
            }
        }

        if task.gate.is_none() && task.phase > 0 {
            task.gate = Some(task.phase - 1);
        }

        ids.dedup();
        task.depends_on = ids;
    }

    tasks
}

fn phase_of(id: &str) -> Option<u8> {
    let rest = id.strip_prefix('P')?;
    let (phase, number) = rest.split_once('-')?;

    number.parse::<u8>().ok()?;
    phase.parse().ok()
}

/// The IDs in the Master Plan's roadmap bullets (`- P0-0 Plan Tracker …`).
pub fn roadmap_ids(master: &str) -> Vec<String> {
    master
        .lines()
        .filter_map(|line| line.strip_prefix("- "))
        .filter_map(|rest| rest.split_whitespace().next())
        .filter(|id| phase_of(id).is_some())
        .map(str::to_string)
        .collect()
}

/// `PLAN-STATUS.md` rows by ID (columns: ID, title, phase, depends_on, status, switch, report, note).
pub fn parse_status(text: &str) -> HashMap<String, Row> {
    let mut rows = HashMap::new();

    for line in text.lines().filter(|line| line.starts_with('|')) {
        let cells: Vec<&str> = line.trim().trim_matches('|').split('|').map(str::trim).collect();

        if cells.len() >= 8 && phase_of(cells[0]).is_some() {
            rows.insert(cells[0].to_string(), Row { status: cells[4].to_string(), report: cells[6].to_string(), note: cells[7].to_string() });
        }
    }

    rows
}

pub fn render_status(tasks: &[Task], rows: &HashMap<String, Row>) -> String {
    let mut out = String::from(
        "# Plan Status\n\nGenerated from `PROMPT-BOOK-SDC.md` by `sdcd plan sync`; only the status, report and note columns are edited (by `sdcd plan set`).\nStatus: `todo` · `in-progress` · `blocked` · `done`.\n\n| ID | title | phase | depends_on | status | switch | report | note |\n|---|---|---|---|---|---|---|---|\n",
    );

    for task in tasks {
        let row = rows.get(&task.id).cloned().unwrap_or(Row { status: "todo".into(), ..Row::default() });
        let mut depends = task.depends_on.join(", ");

        if let Some(gate) = task.gate {
            if !depends.is_empty() {
                depends.push_str(", ");
            }

            let _ = write!(depends, "Gate {gate}");
        }

        let _ = writeln!(out, "| {} | {} | {} | {} | {} | {} | {} | {} |", task.id, task.title.replace('|', "/"), task.phase, depends, row.status, task.switch.replace('|', "/"), row.report, row.note.replace('|', "/"));
    }

    out
}

/// `GATES.md`: a gate passes only when its section says `Result: PASS`.
pub fn gate_passed(gates: &str, gate: u8) -> bool {
    let mut inside = false;

    for line in gates.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            inside = rest.trim().to_ascii_lowercase().starts_with(&format!("gate {gate}"));
        } else if inside && line.trim().eq_ignore_ascii_case("Result: PASS") {
            return true;
        }
    }

    false
}

/// The task `NEXT` should run: an interrupted one first, else the first `todo` whose dependencies are
/// `done` and whose phase gate passed. `Err` says why nothing is eligible.
pub fn next<'a>(tasks: &'a [Task], rows: &HashMap<String, Row>, gates: &str) -> Result<&'a Task, String> {
    let status = |id: &str| rows.get(id).map_or("todo", |row| row.status.as_str());

    if let Some(task) = tasks.iter().find(|task| status(&task.id) == "in-progress") {
        return Ok(task);
    }

    let mut waiting: Vec<String> = Vec::new();

    for task in tasks.iter().filter(|task| status(&task.id) == "todo") {
        if let Some(gate) = task.gate {
            if !gate_passed(gates, gate) {
                waiting.push(format!("{} waits for Gate {gate}", task.id));
                continue;
            }
        }

        if let Some(open) = task.depends_on.iter().find(|id| status(id) != "done") {
            waiting.push(format!("{} waits for {open}", task.id));
            continue;
        }

        return Ok(task);
    }

    Err(match tasks.iter().filter(|task| status(&task.id) == "blocked").map(|task| task.id.as_str()).collect::<Vec<_>>() {
        blocked if !blocked.is_empty() => format!("blocked: {} - {}", blocked.join(", "), waiting.first().cloned().unwrap_or_default()),
        _ if waiting.is_empty() => "every task is done".to_string(),
        _ => waiting.first().cloned().unwrap_or_default(),
    })
}

/// Problems with the plan or the status file; empty means valid.
pub fn validate(tasks: &[Task], master: &str, status_text: Option<&str>) -> Vec<String> {
    let mut problems = Vec::new();
    let ids: HashSet<&str> = tasks.iter().map(|task| task.id.as_str()).collect();

    let mut seen = HashSet::new();

    for task in tasks {
        if !seen.insert(task.id.as_str()) {
            problems.push(format!("duplicate ID {} in the Prompt Book", task.id));
        }

        for dep in &task.depends_on {
            if !ids.contains(dep.as_str()) {
                problems.push(format!("{} depends on unknown task {dep}", task.id));
            }
        }

        if task.switch.is_empty() {
            problems.push(format!("{} has no `Depends:`/`Switch:` line", task.id));
        }
    }

    let roadmap = roadmap_ids(master);

    for id in roadmap.iter().filter(|id| !ids.contains(id.as_str())) {
        problems.push(format!("{id} is in the Master Plan roadmap but has no prompt"));
    }

    let on_roadmap: HashSet<&str> = roadmap.iter().map(String::as_str).collect();

    for task in tasks.iter().filter(|task| !on_roadmap.contains(task.id.as_str())) {
        problems.push(format!("{} has a prompt but is missing from the Master Plan roadmap", task.id));
    }

    // Cycles: a depth-first walk with a visiting set.
    let by_id: HashMap<&str, &Task> = tasks.iter().map(|task| (task.id.as_str(), task)).collect();
    let mut done: HashSet<&str> = HashSet::new();

    for task in tasks {
        let mut path: Vec<&str> = Vec::new();

        if let Some(cycle) = find_cycle(task.id.as_str(), &by_id, &mut path, &mut done) {
            problems.push(format!("dependency cycle: {}", cycle.join(" -> ")));
            break;
        }
    }

    if let Some(text) = status_text {
        let rows = parse_status(text);

        for task in tasks.iter().filter(|task| !rows.contains_key(&task.id)) {
            problems.push(format!("PLAN-STATUS.md has no row for {}", task.id));
        }

        for (id, row) in &rows {
            if !ids.contains(id.as_str()) {
                problems.push(format!("PLAN-STATUS.md has a row for unknown task {id}"));
            }

            if !STATUSES.contains(&row.status.as_str()) {
                problems.push(format!("{id} has status `{}`; allowed: {}", row.status, STATUSES.join(" | ")));
            }
        }
    }

    problems
}

fn find_cycle<'a>(id: &'a str, by_id: &HashMap<&'a str, &'a Task>, path: &mut Vec<&'a str>, done: &mut HashSet<&'a str>) -> Option<Vec<String>> {
    if done.contains(id) {
        return None;
    }

    if let Some(at) = path.iter().position(|seen| *seen == id) {
        let mut cycle: Vec<String> = path[at..].iter().map(|id| id.to_string()).collect();

        cycle.push(id.to_string());

        return Some(cycle);
    }

    path.push(id);

    for dep in by_id.get(id).map(|task| task.depends_on.as_slice()).unwrap_or(&[]) {
        if by_id.contains_key(dep.as_str()) {
            if let Some(cycle) = find_cycle(dep.as_str(), by_id, path, done) {
                return Some(cycle);
            }
        }
    }

    path.pop();
    done.insert(id);

    None
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|arg| arg == name).and_then(|index| args.get(index + 1)).cloned()
}

fn docs_dir(args: &[String]) -> PathBuf {
    flag(args, "--docs").map(PathBuf::from).unwrap_or_else(|| {
        let mut here = std::env::current_dir().unwrap_or_default();

        loop {
            for candidate in [here.join("sdc/docs"), here.join("docs")] {
                if candidate.join("PROMPT-BOOK-SDC.md").exists() {
                    return candidate;
                }
            }

            if !here.pop() {
                return PathBuf::from("sdc/docs");
            }
        }
    })
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

struct Loaded {
    docs: PathBuf,
    tasks: Vec<Task>,
    master: String,
    status: Option<String>,
    gates: String,
}

fn load(args: &[String]) -> Result<Loaded, String> {
    let docs = docs_dir(args);
    let tasks = parse_prompt_book(&read(&docs.join("PROMPT-BOOK-SDC.md"))?);
    let master = read(&docs.join("MASTER-PLAN-SDC.md"))?;
    let status = std::fs::read_to_string(docs.join("PLAN-STATUS.md")).ok();
    let gates = std::fs::read_to_string(docs.join("GATES.md")).unwrap_or_default();

    Ok(Loaded { docs, tasks, master, status, gates })
}

pub fn main(args: &[String]) -> i32 {
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("usage: sdcd plan next|status|validate|sync|set [--docs DIR]");
        return 3;
    };

    let loaded = match load(args) {
        Ok(loaded) => loaded,
        Err(message) => {
            eprintln!("{message}");
            return 1;
        }
    };

    let rows = loaded.status.as_deref().map(parse_status).unwrap_or_default();

    match command {
        "next" => {
            // Without PLAN-STATUS.md the next task is P0-0, as the NEXT prompt says.
            let chosen = if loaded.status.is_none() { loaded.tasks.first().ok_or("the Prompt Book has no tasks".to_string()) } else { next(&loaded.tasks, &rows, &loaded.gates) };

            match chosen {
                Ok(task) => {
                    let resumed = rows.get(&task.id).is_some_and(|row| row.status == "in-progress");
                    let depends = if task.depends_on.is_empty() { "none".to_string() } else { task.depends_on.join(", ") };

                    println!("{}{}", task.id, if resumed { " (resume)" } else { "" });
                    println!("title: {}", task.title);
                    println!("depends: {depends}");
                    println!("gate: {}", task.gate.map_or("none".to_string(), |gate| format!("Gate {gate}")));
                    println!("switch: {}", task.switch);
                    0
                }
                Err(why) => {
                    println!("no eligible task: {why}");
                    1
                }
            }
        }
        "status" => {
            let mut counts: BTreeMap<u8, BTreeMap<String, usize>> = BTreeMap::new();

            for task in &loaded.tasks {
                let status = rows.get(&task.id).map_or("todo", |row| row.status.as_str());

                *counts.entry(task.phase).or_default().entry(status.to_string()).or_default() += 1;
            }

            for (phase, by_status) in counts {
                let line: Vec<String> = by_status.iter().map(|(status, count)| format!("{status} {count}")).collect();

                println!("Phase {phase}: {}  (Gate {phase}: {})", line.join(" · "), if gate_passed(&loaded.gates, phase) { "PASS" } else { "not passed" });
            }

            println!("tasks: {}", loaded.tasks.len());
            0
        }
        "validate" => {
            let problems = validate(&loaded.tasks, &loaded.master, loaded.status.as_deref());

            if problems.is_empty() {
                println!("plan valid: {} tasks", loaded.tasks.len());
                0
            } else {
                problems.iter().for_each(|problem| println!("problem: {problem}"));
                1
            }
        }
        "sync" => {
            let text = render_status(&loaded.tasks, &rows);

            for (name, body) in [("PLAN-STATUS.md", text.as_str()), ("GATES.md", GATES_TEMPLATE)] {
                let path = loaded.docs.join(name);

                if name == "GATES.md" && path.exists() {
                    continue;
                }

                if let Err(error) = std::fs::write(&path, body) {
                    eprintln!("cannot write {}: {error}", path.display());
                    return 1;
                }
            }

            let _ = std::fs::create_dir_all(loaded.docs.join("reports"));
            println!("wrote {} rows to {}", loaded.tasks.len(), loaded.docs.join("PLAN-STATUS.md").display());
            0
        }
        "set" => {
            let (Some(id), Some(status)) = (args.get(1), args.get(2)) else {
                eprintln!("usage: sdcd plan set ID STATUS [--note TEXT] [--report PATH]");
                return 3;
            };

            if !STATUSES.contains(&status.as_str()) || !loaded.tasks.iter().any(|task| task.id == *id) {
                eprintln!("unknown task or status (allowed: {})", STATUSES.join(" | "));
                return 3;
            }

            let mut rows = rows;
            let row = rows.entry(id.clone()).or_default();

            row.status = status.clone();

            if let Some(report) = flag(args, "--report") {
                row.report = report;
            }

            if let Some(note) = flag(args, "--note") {
                row.note = note;
            }

            match std::fs::write(loaded.docs.join("PLAN-STATUS.md"), render_status(&loaded.tasks, &rows)) {
                Ok(()) => {
                    println!("{id} -> {status}");
                    0
                }
                Err(error) => {
                    eprintln!("cannot write PLAN-STATUS.md: {error}");
                    1
                }
            }
        }
        other => {
            eprintln!("unknown plan command `{other}`");
            3
        }
    }
}

const GATES_TEMPLATE: &str = "# Gates\n\nA gate passes only when its section contains the line `Result: PASS`, written with the measured numbers by the last task of the phase.\n\n## Gate 0\n\n## Gate 1\n\n## Gate 2\n\n## Gate 3\n\n## Gate 4\n";

#[cfg(test)]
mod tests {
    use super::*;

    const BOOK: &str = "## Phase 0: x\n\n### P0-0 First\n\nTASK P0-0 — a. Depends: none. Switch: n/a.\n\n### P0-1 Second\n\nTASK P0-1 — b. Depends: P0-0. Switch: s.one.\n\n### P0-2 Closing\n\nTASK P0-2 — c. Depends: all P0. Switch: n/a.\n\n## Phase 1\n\n### P1-1 Graph\n\nTASK P1-1 — d. Depends: Gate 0. Switch: graph.v1.\n\n### P1-2 Next\n\nTASK P1-2 — e. Depends: P1-1, P0-1. Switch: x.\n";
    const MASTER: &str = "- P0-0 a\n- P0-1 b\n- P0-2 c\n- P1-1 d\n- P1-2 e\n";

    fn rows(pairs: &[(&str, &str)]) -> HashMap<String, Row> {
        pairs.iter().map(|(id, status)| (id.to_string(), Row { status: status.to_string(), ..Row::default() })).collect()
    }

    #[test]
    fn parses_ids_dependencies_and_expands_all() {
        let tasks = parse_prompt_book(BOOK);

        assert_eq!(tasks.iter().map(|task| task.id.as_str()).collect::<Vec<_>>(), ["P0-0", "P0-1", "P0-2", "P1-1", "P1-2"]);
        assert_eq!(tasks[1].depends_on, ["P0-0"]);
        assert_eq!(tasks[2].depends_on, ["P0-0", "P0-1"]);
        assert_eq!(tasks[3].gate, Some(0));
        assert_eq!(tasks[1].switch, "s.one");
        assert_eq!(tasks[0].gate, None);
    }

    #[test]
    fn next_follows_order_and_dependencies() {
        let tasks = parse_prompt_book(BOOK);

        assert_eq!(next(&tasks, &rows(&[]), "").unwrap().id, "P0-0");
        assert_eq!(next(&tasks, &rows(&[("P0-0", "done")]), "").unwrap().id, "P0-1");
        assert_eq!(next(&tasks, &rows(&[("P0-0", "done"), ("P0-1", "done")]), "").unwrap().id, "P0-2");
    }

    #[test]
    fn a_failed_gate_blocks_the_next_phase() {
        let tasks = parse_prompt_book(BOOK);
        let all = rows(&[("P0-0", "done"), ("P0-1", "done"), ("P0-2", "done")]);

        assert!(next(&tasks, &all, "## Gate 0\n\nResult: FAIL\n").is_err());
        assert_eq!(next(&tasks, &all, "## Gate 0\n\nResult: PASS\n").unwrap().id, "P1-1");
    }

    #[test]
    fn an_in_progress_task_is_resumed_first() {
        let tasks = parse_prompt_book(BOOK);

        assert_eq!(next(&tasks, &rows(&[("P0-0", "done"), ("P0-1", "in-progress")]), "").unwrap().id, "P0-1");
    }

    #[test]
    fn a_blocked_dependency_is_reported_not_skipped() {
        let tasks = parse_prompt_book(BOOK);
        let error = next(&tasks, &rows(&[("P0-0", "blocked")]), "").unwrap_err();

        assert!(error.contains("blocked: P0-0"), "{error}");
    }

    #[test]
    fn validate_catches_a_missing_id_unknown_dependency_and_cycle() {
        let tasks = parse_prompt_book(BOOK);

        assert!(validate(&tasks, MASTER, None).is_empty());
        assert!(validate(&tasks, &format!("{MASTER}- P9-9 gone\n"), None).iter().any(|p| p.contains("P9-9")));

        let broken = BOOK.replace("Depends: P0-0. Switch: s.one", "Depends: P7-7. Switch: s.one");

        assert!(validate(&parse_prompt_book(&broken), MASTER, None).iter().any(|p| p.contains("unknown task P7-7")));

        let cyclic = BOOK.replace("Depends: none. Switch: n/a.\n\n### P0-1", "Depends: P0-1. Switch: n/a.\n\n### P0-1");

        assert!(validate(&parse_prompt_book(&cyclic), MASTER, None).iter().any(|p| p.contains("cycle")));
    }

    #[test]
    fn status_round_trips_and_keeps_notes() {
        let tasks = parse_prompt_book(BOOK);
        let mut kept = rows(&[("P0-0", "done")]);

        kept.get_mut("P0-0").unwrap().note = "ok".into();
        kept.get_mut("P0-0").unwrap().report = "docs/reports/P0-0.md".into();

        let text = render_status(&tasks, &kept);
        let back = parse_status(&text);

        assert_eq!(back["P0-0"], kept["P0-0"]);
        assert_eq!(back["P0-1"].status, "todo");
        assert!(validate(&tasks, MASTER, Some(&text)).is_empty());
        assert!(validate(&tasks, MASTER, Some(&text.replace("| done |", "| finished |"))).iter().any(|p| p.contains("finished")));
        assert!(validate(&tasks, MASTER, Some(&text.replace("| P1-2 |", "| X |"))).iter().any(|p| p.contains("no row for P1-2")));
    }

    /// The real plan in this repository: 66 tasks, valid, and `next` after P0-0 is P0-1.
    #[test]
    fn the_real_plan_is_valid() {
        let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs");
        let (Ok(book), Ok(master)) = (std::fs::read_to_string(docs.join("PROMPT-BOOK-SDC.md")), std::fs::read_to_string(docs.join("MASTER-PLAN-SDC.md"))) else {
            return;
        };

        let tasks = parse_prompt_book(&book);

        assert_eq!(tasks.len(), 66);
        assert_eq!(validate(&tasks, &master, None), Vec::<String>::new());
        assert_eq!(next(&tasks, &rows(&[("P0-0", "done")]), "").unwrap().id, "P0-1");
    }
}
