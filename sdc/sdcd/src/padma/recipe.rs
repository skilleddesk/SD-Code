//! **The Recipe Engine (P3-1, switch `padma.recipes`, off until the bench says)**: a workflow is data, not an ad-hoc loop. A recipe is a TOML file - a DAG
//! of steps, each a **code** step (a Rust function by name), a **model** step (a prompt, the tools it may use, how it thinks) or a **gate** (pass or fail
//! with evidence). A step that names `for_each` runs once per line of an earlier step's output - fan-out is native, so map-reduce needs no special code.
//!
//! Every step and every unit is a row in the task ledger and goes through the Conductor (budget → model → gate → ledger), so a recipe is checkpointed,
//! shows `task.progress`, and picks up at the first unit that is not done when the person says "continue". What a step produced is read back from the
//! ledger's evidence, never kept only in memory - which is what makes resuming work. A recipe names no model: the model step is run by the caller's
//! closure through the Context Budget and the Model Adapter (thinking policy P2-2 and tool subset P2-3 are set around it here).

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::engines::{EngineEvent, EventSink};

use super::codefns::{CodeFn, Data, Env};
use super::conductor::{finish, run_units, Budget, Clock, Outcome, Phase, UnitRun};
use super::ledger::{Ledger, Status};
use super::think::StepKind;

/// The recipes that ship with SDC.
const BUILT_IN: &[&str] = &[include_str!("recipes/review-code.toml"), include_str!("recipes/build-page.toml"), include_str!("recipes/fix.toml"), include_str!("recipes/review-site.toml")];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Code,
    Model,
    Gate,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Step {
    pub id: String,
    pub kind: Kind,
    /// The function a code or gate step calls.
    #[serde(default)]
    pub run: String,
    /// The request a model step makes; `{unit}` and `{<input>}` are replaced.
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<String>,
    /// What the step produces (at most one name).
    #[serde(default)]
    pub outputs: Vec<String>,
    /// Run once per line of this earlier output.
    #[serde(default)]
    pub for_each: String,
    /// The 2-4 tools a model step is offered (P2-3).
    #[serde(default)]
    pub tools: Vec<String>,
    /// The step kind the thinking policy reads (P2-2): tool_call, map_review, boilerplate, blueprint, repair_after_fail, reduce_report.
    #[serde(default)]
    pub think: String,
    /// A lower tool-call budget than the Conductor's.
    #[serde(default)]
    pub calls: usize,
    /// The line of the contract this step proves when it is done.
    #[serde(default)]
    pub done_when: String,
    /// A gate a model step's answer must pass; the answer is its `output` input. A failed check asks again (up to `retries` times) with the reason.
    #[serde(default)]
    pub check: String,
    #[serde(default)]
    pub retries: usize,
    /// Which related files a model step is shown as skeletons when the engines are on (P4-1): `imports` (what the unit imports) or `suspects` (the files a failure points at).
    #[serde(default)]
    pub context: String,
    /// `test_first` (the step may not change a test) or `writes` - read by the engines (P4-1).
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Recipe {
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// The outputs sent to the person when the recipe finishes, in this order.
    #[serde(default)]
    pub deliver: Vec<String>,
    #[serde(default, rename = "step")]
    pub steps: Vec<Step>,
}

/// The functions a recipe may name.
pub struct Registry {
    code: BTreeMap<&'static str, CodeFn>,
    gates: BTreeMap<&'static str, CodeFn>,
}

impl Registry {
    pub fn standard() -> Registry {
        let mut registry = Registry { code: BTreeMap::new(), gates: BTreeMap::new() };

        registry.code.insert("list_files", super::codefns::list_files);
        registry.code.insert("count_lines", super::codefns::count_lines);
        registry.gates.insert("outputs_present", super::codefns::outputs_present);
        registry.code.insert("collect_errors", super::codefns::collect_errors);
        registry.code.insert("plan_units", super::codefns::plan_units);
        registry.code.insert("verify_findings", super::codefns::verify_findings);
        registry.code.insert("rank_findings", super::codefns::rank_findings);
        registry.code.insert("build_report", super::codefns::build_report);
        registry.code.insert("plan_files", super::pagefns::plan_files);
        registry.code.insert("defaults_of", super::pagefns::defaults_of);
        registry.code.insert("write_files", super::pagefns::write_files);
        registry.code.insert("page_report", super::pagefns::page_report);
        registry.code.insert("site_target", super::siteprobe::site_target);
        registry.code.insert("probe_site", super::siteprobe::probe_site);
        registry.code.insert("site_report", super::siteprobe::site_report);
        registry.code.insert("run_tests", super::fixfns::run_tests);
        registry.code.insert("localize", super::fixfns::localize);
        registry.code.insert("fix_report", super::fixfns::fix_report);
        registry.gates.insert("patch_fixes", super::fixfns::patch_fixes);
        registry.gates.insert("valid_file", super::pagefns::valid_file);
        registry.gates.insert("verify_page", super::pagefns::verify_page);

        registry
    }

    pub fn code(&mut self, name: &'static str, function: CodeFn) {
        self.code.insert(name, function);
    }

    pub fn gate(&mut self, name: &'static str, function: CodeFn) {
        self.gates.insert(name, function);
    }
}

pub fn parse(text: &str) -> Result<Recipe, String> {
    toml::from_str(text).map_err(|error| format!("recipe: {error}"))
}

/// The recipes that ship with SDC, and the project's own in `.sdc/recipes/*.toml` (a project recipe with the same id replaces the shipped one).
/// A file that does not parse or validate is skipped and named in the second list.
pub fn load_all(root: &std::path::Path, registry: &Registry) -> (Vec<Recipe>, Vec<String>) {
    let mut recipes: Vec<Recipe> = Vec::new();
    let mut problems = Vec::new();
    let mut add = |source: &str, text: &str| match parse(text).and_then(|recipe| validate(&recipe, registry).map(|_| recipe)) {
        Ok(recipe) => {
            recipes.retain(|known| known.id != recipe.id);
            recipes.push(recipe);
        }
        Err(problem) => problems.push(format!("{source}: {problem}")),
    };

    for (index, text) in BUILT_IN.iter().enumerate() {
        add(&format!("built-in recipe {index}"), text);
    }

    if let Ok(entries) = std::fs::read_dir(root.join(".sdc").join("recipes")) {
        let mut files: Vec<_> = entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|extension| extension == "toml")).collect();

        files.sort();

        for path in files {
            if let Ok(text) = std::fs::read_to_string(&path) {
                add(&path.display().to_string(), &text);
            }
        }
    }

    (recipes, problems)
}

pub fn find(root: &std::path::Path, id: &str, registry: &Registry) -> Option<Recipe> {
    load_all(root, registry).0.into_iter().find(|recipe| recipe.id == id)
}

fn ancestors(recipe: &Recipe, id: &str, into: &mut BTreeSet<String>) {
    if let Some(step) = recipe.steps.iter().find(|step| step.id == id) {
        for need in &step.needs {
            if into.insert(need.clone()) {
                ancestors(recipe, need, into);
            }
        }
    }
}

/// The steps in an order that respects `needs` (file order where it does not matter). A cycle or an unknown `needs` is an error.
pub fn order(recipe: &Recipe) -> Result<Vec<&Step>, String> {
    let mut placed: Vec<&Step> = Vec::new();
    let mut left: Vec<&Step> = recipe.steps.iter().collect();

    while !left.is_empty() {
        let ready = left.iter().position(|step| step.needs.iter().all(|need| placed.iter().any(|done| &done.id == need)));

        match ready {
            Some(at) => placed.push(left.remove(at)),
            None => return Err(format!("steps {} depend on each other or on a step that does not exist", left.iter().map(|step| step.id.as_str()).collect::<Vec<_>>().join(", "))),
        }
    }

    Ok(placed)
}

/// Checks a recipe before it runs: ids, kinds, named functions, `needs`, and that every `for_each` and input comes from a step this one depends on.
pub fn validate(recipe: &Recipe, registry: &Registry) -> Result<(), String> {
    if recipe.id.trim().is_empty() || recipe.steps.is_empty() {
        return Err("a recipe needs an id and at least one step".into());
    }

    let mut seen = BTreeSet::new();

    for step in &recipe.steps {
        if step.id.trim().is_empty() || step.id.contains(':') || !seen.insert(step.id.as_str()) {
            return Err(format!("step id '{}' is empty, has a ':' or is used twice", step.id));
        }

        if step.outputs.len() > 1 {
            return Err(format!("step {} has more than one output; a step produces one list", step.id));
        }

        match step.kind {
            Kind::Code if !registry.code.contains_key(step.run.as_str()) => return Err(format!("step {} runs '{}', which is not a code function", step.id, step.run)),
            Kind::Gate if !registry.gates.contains_key(step.run.as_str()) => return Err(format!("step {} runs '{}', which is not a gate", step.id, step.run)),
            Kind::Model if step.prompt.trim().is_empty() => return Err(format!("model step {} has no prompt", step.id)),
            _ => {}
        }

        if !step.check.is_empty() && (step.kind != Kind::Model || !registry.gates.contains_key(step.check.as_str())) {
            return Err(format!("step {} checks with '{}', which is not a gate (or the step is not a model step)", step.id, step.check));
        }

        if !matches!(step.context.as_str(), "" | "imports" | "suspects") {
            return Err(format!("step {} has the unknown context '{}' (imports or suspects)", step.id, step.context));
        }

        if let Some(tag) = step.tags.iter().find(|tag| !matches!(tag.as_str(), "test_first" | "writes")) {
            return Err(format!("step {} has the unknown tag '{tag}' (test_first or writes)", step.id));
        }

        if step.retries > 3 {
            return Err(format!("step {} may retry at most 3 times", step.id));
        }

        if !step.think.is_empty() && StepKind::from_word(&step.think).is_none() {
            return Err(format!("step {} has the unknown think kind '{}'", step.id, step.think));
        }
    }

    order(recipe)?;

    for name in &recipe.deliver {
        if !recipe.steps.iter().any(|step| step.outputs.first() == Some(name)) {
            return Err(format!("deliver '{name}', which no step produces"));
        }
    }

    for step in &recipe.steps {
        let mut before = BTreeSet::new();

        ancestors(recipe, &step.id, &mut before);

        for name in step.inputs.iter().chain(std::iter::once(&step.for_each)).filter(|name| !name.is_empty() && !name.starts_with('_')) {
            let produced = recipe.steps.iter().any(|other| before.contains(&other.id) && other.outputs.first() == Some(name));

            if !produced {
                return Err(format!("step {} reads '{name}', which no step it depends on produces", step.id));
            }
        }
    }

    Ok(())
}

/// One model request of a recipe, handed to the caller's closure.
#[derive(Debug, Clone)]
pub struct ModelCall {
    pub step: String,
    pub unit: Option<String>,
    pub prompt: String,
    pub tools: Vec<String>,
    pub kind: Option<StepKind>,
    /// The tool calls the unit may make.
    pub calls: usize,
    /// 0 for the first try; a retry after a failed check is 1, 2, ... (a retry may sample differently - Best-of-N).
    pub attempt: usize,
    /// The last chance (P4-1): the local model failed its check on every try, so the caller may hand this one request to the chain - only when it was told it may.
    pub relay: bool,
}

/// What a recipe run needs.
pub struct Job<'a> {
    pub ledger: &'a mut Ledger,
    pub task_id: &'a str,
    pub sink: &'a EventSink,
    pub registry: &'a Registry,
    pub env: &'a Env,
    pub bengali: bool,
    pub budget: Budget,
    /// Ends the run early (the person pressed Stop); the task is checkpointed.
    pub stop: Option<std::sync::Arc<dyn Fn() -> bool + Send + Sync>>,
    /// The caller can relay a unit the local model could not do (P4-1); a `ModelCall` with `relay` set is then sent to the chain.
    pub relay: bool,
}

/// What a recipe run came to: the conductor's outcome and everything the steps produced.
#[derive(Debug)]
pub struct Done {
    pub outcome: Outcome,
    pub data: Data,
}

const MAX_SUBSTITUTED: usize = 6_000;

fn expand(template: &str, unit: Option<&str>, data: &Data, env: &Env) -> String {
    let mut text = template.replace("{unit}", unit.unwrap_or(""));

    if text.contains("{file}") {
        text = text.replace("{file}", &unit.map(|unit| super::codefns::read_unit(env, unit).unwrap_or_else(|reason| format!("(could not be read: {reason})"))).unwrap_or_default());
    }

    if text.contains("{file_errors}") {
        let path = unit.map(|unit| super::codefns::split_unit(unit).0).unwrap_or("");
        let mine: Vec<&str> = data.get("errors").into_iter().flatten().filter(|line| line.starts_with(&format!("{path}:"))).map(String::as_str).take(20).collect();

        text = text.replace("{file_errors}", &if mine.is_empty() { "none".to_string() } else { mine.join("
") });
    }

    text = text.replace("{goal}", &env.goal);
    text = text.replace("{symbol_hint}", super::engines::symbol_hint());
    text = text.replace("{language}", if env.language_out == "bn" { "Bengali (বাংলা)" } else { "the same language as the person's request" });

    for (name, lines) in data {
        let marker = format!("{{{name}}}");

        if text.contains(&marker) {
            let joined: String = lines.join("
").chars().take(MAX_SUBSTITUTED).collect();

            text = text.replace(&marker, &joined);
        }
    }

    text
}

/// The ledger row of a unit, added if it is not there yet.
fn row_of(ledger: &mut Ledger, task_id: &str, unit: &str) -> Option<String> {
    if let Some(row) = ledger.get(task_id).and_then(|task| task.steps.into_iter().find(|row| row.unit == unit)) {
        return Some(row.step_id);
    }

    ledger.add_step(task_id, unit).ok()
}

fn unit_name(step: &Step, item: Option<&str>) -> String {
    match item {
        Some(item) => format!("{}:{item}", step.id),
        None => step.id.clone(),
    }
}

/// Reads what a step's units produced from the ledger, in unit order.
fn produced(ledger: &Ledger, task_id: &str, step: &Step, names: &[String]) -> Vec<String> {
    let rows = ledger.get(task_id).map(|task| task.steps).unwrap_or_default();
    let mut lines = Vec::new();

    for name in names {
        let Some(row) = rows.iter().find(|row| &row.unit == name && row.status == Status::Done) else {
            continue;
        };
        let Some(evidence) = row.evidence_refs.last() else {
            continue;
        };

        match step.kind {
            Kind::Code => lines.extend(serde_json::from_str::<Vec<String>>(evidence).unwrap_or_default()),
            Kind::Model => lines.push(evidence.clone()),
            Kind::Gate => {}
        }
    }

    lines
}

/// The units of the fan-out steps that are not done, each with the reason it stopped (what the Finish Gate lists as not reviewed).
fn unreviewed(ledger: &Ledger, task_id: &str, recipe: &Recipe) -> Vec<String> {
    let fans: Vec<String> = recipe.steps.iter().filter(|step| !step.for_each.is_empty()).map(|step| format!("{}:", step.id)).collect();
    let rows = ledger.get(task_id).map(|task| task.steps).unwrap_or_default();

    rows.iter()
        .filter(|row| row.status != Status::Done && fans.iter().any(|prefix| row.unit.starts_with(prefix.as_str())))
        .map(|row| {
            let unit = fans.iter().find_map(|prefix| row.unit.strip_prefix(prefix.as_str())).unwrap_or(&row.unit);

            if row.handover_note.is_empty() { format!("{unit} - not reached yet") } else { format!("{unit} - {}", row.handover_note) }
        })
        .collect()
}

/// Runs a recipe for the task `task_id` (its contract is already in the ledger). Steps that are done in the ledger are not run again; what they produced
/// is read back. `model` answers a model step - it is where the Context Budget and the Model Adapter come in.
pub fn execute(recipe: &Recipe, job: Job, model: &mut dyn FnMut(&ModelCall) -> UnitRun) -> Result<Done, String> {
    validate(recipe, job.registry)?;

    let Job { ledger, task_id, sink, registry, env, bengali, budget, stop, relay } = job;
    let steps = order(recipe)?;
    let clock = match stop {
        Some(stop) => Clock::start(budget).stoppable(stop),
        None => Clock::start(budget),
    };
    let mut trace = vec![Phase::Classify, Phase::Contract];
    let mut data = Data::new();
    let mut blocked_steps: BTreeSet<String> = BTreeSet::new();
    let mut paused = false;

    for step in &steps {
        if step.for_each.is_empty() {
            row_of(ledger, task_id, &step.id);
        }
    }

    for step in steps {
        if paused {
            break;
        }

        if let Some(need) = step.needs.iter().find(|need| blocked_steps.contains(*need)) {
            if let Some(id) = row_of(ledger, task_id, &step.id) {
                let _ = ledger.set_step(task_id, &id, Status::Blocked, None, Some(&format!("needs {need}, which is blocked")));
            }

            blocked_steps.insert(step.id.clone());

            continue;
        }

        let items: Vec<Option<String>> = if step.for_each.is_empty() { vec![None] } else { data.get(&step.for_each).cloned().unwrap_or_default().into_iter().map(Some).collect() };
        let names: Vec<String> = items.iter().map(|item| unit_name(step, item.as_deref())).collect();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut inputs: Data = step.inputs.iter().filter_map(|name| data.get(name).map(|lines| (name.clone(), lines.clone()))).collect();

        if step.inputs.iter().any(|name| name == "_unreviewed") {
            inputs.insert("_unreviewed".to_string(), unreviewed(ledger, task_id, recipe));
        }

        /* P4-1: what the engines need to know about the step they are working for (keys with `_` are never validated or substituted by a recipe). */
        inputs.insert("_task".to_string(), vec![task_id.to_string()]);
        inputs.insert("_step".to_string(), vec![step.id.clone()]);
        inputs.insert("_recipe".to_string(), vec![recipe.id.clone()]);
        inputs.insert("_tags".to_string(), step.tags.clone());

        let prefix = format!("{}:", step.id);
        let step_budget = if step.calls > 0 { step.calls.min(budget.unit_calls) } else { budget.unit_calls };
        let kind = StepKind::from_word(&step.think);

        let (ran, stopped) = run_units(ledger, task_id, &step.id, &name_refs, sink, &clock, |name, _limit| {
            let item = name.strip_prefix(&prefix);

            match step.kind {
                Kind::Code => match registry.code[step.run.as_str()](env, item, &inputs) {
                    Ok(lines) => UnitRun { calls: 0, result: Ok(serde_json::to_string(&lines).unwrap_or_else(|_| "[]".into())) },
                    Err(reason) => UnitRun { calls: 0, result: Err(reason) },
                },
                Kind::Gate => match registry.gates[step.run.as_str()](env, item, &inputs) {
                    Ok(lines) => UnitRun { calls: 0, result: Ok(if lines.is_empty() { "passed".to_string() } else { lines.join("; ") }) },
                    Err(reason) => UnitRun { calls: 0, result: Err(reason) },
                },
                Kind::Model => {
                    let base = expand(&step.prompt, item, &inputs, env);
                    /* P4-1: with the engines on, the step is a fresh short request: related files as skeletons, the goal first and again last. */
                    let base = super::engines::fresh(&env.goal, &step.done_when, super::engines::with_skeletons(env, &step.context, item, &inputs, base));
                    let mut prompt = base.clone();
                    let mut attempt = 0;
                    let mut spent = 0;
                    let mut ask = |call: &ModelCall| {
                        super::think::set(call.kind);

                        let run = model(call);

                        super::think::set(None);

                        run
                    };

                    loop {
                        let call = ModelCall { step: step.id.clone(), unit: item.map(str::to_string), prompt: prompt.clone(), tools: step.tools.clone(), kind, calls: step_budget, attempt, relay: false };
                        let run = if call.tools.is_empty() {
                            ask(&call)
                        } else {
                            let tools: Vec<&str> = call.tools.iter().map(String::as_str).collect();

                            super::tools::with_tools(&tools, || ask(&call))
                        };

                        spent += run.calls;

                        /* An empty answer is not a finished unit: asked again, up to twice, with or without a check (found by the P4-2 bench). */
                        if matches!(&run.result, Ok(answer) if answer.trim().is_empty()) && attempt < step.retries.max(2) && !clock.spent() {
                            attempt += 1;
                            prompt = format!("{base}

Your previous answer was empty. Answer now, in exactly the form asked.");

                            continue;
                        }

                        let answer = match &run.result {
                            Ok(answer) if !step.check.is_empty() => answer.clone(),
                            _ => return UnitRun { calls: spent, result: run.result },
                        };
                        let mut checked = inputs.clone();

                        checked.insert("output".to_string(), vec![answer.clone()]);

                        match registry.gates[step.check.as_str()](env, item, &checked) {
                            Ok(_) => return UnitRun { calls: spent, result: Ok(answer) },
                            Err(reason) if attempt < step.retries => {
                                attempt += 1;
                                prompt = format!("{base}

Your previous answer failed a check: {reason}
Answer again and fix exactly that.");
                            }
                            Err(reason) => {
                                /* P4-1: the local model had every try; the caller may hand this one request to the chain (it decides - consent, local-only, the allowance). */
                                if relay && super::engines::active() && !clock.spent() {
                                    let asked = format!("{base}

A smaller local model tried {} time(s) and its last answer failed a check: {reason}
Answer correctly, in exactly the form asked.", attempt + 1);
                                    let call = ModelCall { step: step.id.clone(), unit: item.map(str::to_string), prompt: asked, tools: Vec::new(), kind, calls: 1, attempt: attempt + 1, relay: true };
                                    let relayed = ask(&call);

                                    spent += relayed.calls;

                                    if let Ok(second) = relayed.result {
                                        checked.insert("output".to_string(), vec![second.clone()]);

                                        return match registry.gates[step.check.as_str()](env, item, &checked) {
                                            Ok(_) => UnitRun { calls: spent, result: Ok(second) },
                                            Err(again) => UnitRun { calls: spent, result: Err(format!("failed its check after {} tries and a relay to the chain: {again}", attempt + 1)) },
                                        };
                                    }
                                }

                                return UnitRun { calls: spent, result: Err(format!("failed its check after {} tries: {reason}", attempt + 1)) };
                            }
                        }
                    }
                }
            }
        });

        trace.extend(ran);
        paused = stopped;

        if let Some(name) = step.outputs.first() {
            data.insert(name.clone(), produced(ledger, task_id, step, &names));

            if step.kind == Kind::Model && !step.for_each.is_empty() {
                let rows = ledger.get(task_id).map(|task| task.steps).unwrap_or_default();
                let done: Vec<String> = items.iter().zip(&names).filter(|(_, name)| rows.iter().any(|row| &row.unit == *name && row.status == Status::Done)).filter_map(|(item, _)| item.clone()).collect();

                data.insert(format!("_{name}_units"), done);
            }
        }

        let rows = ledger.get(task_id).map(|task| task.steps).unwrap_or_default();
        let blocked = names.iter().filter(|name| rows.iter().any(|row| &row.unit == *name && row.status == Status::Blocked)).count();

        if !stopped && !names.is_empty() && blocked == names.len() {
            blocked_steps.insert(step.id.clone());
        }

        if !step.done_when.is_empty() {
            let proof = names.iter().find_map(|name| rows.iter().find(|row| &row.unit == name && row.status == Status::Done).and_then(|row| row.evidence_refs.last().cloned()));
            let index = ledger.get(task_id).and_then(|task| task.criteria.iter().position(|criterion| criterion.text == step.done_when));

            if let (Some(proof), Some(index)) = (proof, index) {
                let _ = ledger.prove(task_id, index, &proof);
            }
        }
    }

    if !paused {
        for name in &recipe.deliver {
            if let Some(lines) = data.get(name).filter(|lines| !lines.is_empty()) {
                sink.send(EngineEvent::Delta(format!("{}

", lines.join("
"))));
            }
        }
    }

    let outcome = finish(ledger, task_id, sink, bengali, &clock, paused, trace);

    Ok(Done { outcome, data })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::{EngineEvent, Recorder};
    use crate::padma::ledger::{contract_for, State};

    const TOY: &str = r#"
id = "R-TOY-COUNT"
title = "list the files, count their lines, report"

[[step]]
id = "list"
kind = "code"
run = "list_files"
outputs = ["files"]

[[step]]
id = "count"
kind = "code"
run = "count_lines"
needs = ["list"]
for_each = "files"
outputs = ["counts"]

[[step]]
id = "check"
kind = "gate"
run = "outputs_present"
needs = ["count"]
inputs = ["counts"]
done_when = "every file was counted"

[[step]]
id = "report"
kind = "model"
needs = ["check"]
inputs = ["counts"]
think = "reduce_report"
tools = ["read_file", "grep"]
prompt = "Write a two-line report of these line counts:\n{counts}"
"#;

    fn project(name: &str) -> (Env, Ledger) {
        let root = std::env::temp_dir().join(format!("sdc-recipe-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("a.txt"), "1\n2\n").unwrap();
        std::fs::write(root.join("b.txt"), "1\n").unwrap();
        std::fs::write(root.join("src/c.rs"), "1\n2\n3\n").unwrap();

        let mut ledger = Ledger::open(&root).unwrap();
        let mut contract = contract_for("t1", "s", "count the lines", "turn:t1");

        contract.done_when = vec!["every file was counted".to_string()];
        ledger.create(&contract, &[]).unwrap();

        (Env::new(root), ledger)
    }

    fn job<'a>(ledger: &'a mut Ledger, env: &'a Env, sink: &'a EventSink, registry: &'a Registry, budget: Budget) -> Job<'a> {
        Job { ledger, task_id: "t1", sink, registry, env, bengali: false, budget, stop: None, relay: false }
    }

    #[test]
    fn the_toy_recipe_runs_with_a_ledger_row_for_every_unit() {
        let (env, mut ledger) = project("toy");
        let registry = Registry::standard();
        let recipe = parse(TOY).unwrap();
        let recorder = Recorder::new();
        let mut asked = Vec::new();
        let done = execute(&recipe, job(&mut ledger, &env, &recorder.sink(), &registry, Budget::default()), &mut |call| {
            asked.push(call.clone());

            UnitRun { calls: 1, result: Ok("a.txt 2, b.txt 1, src/c.rs 3".into()) }
        })
        .unwrap();
        let task = ledger.get("t1").unwrap();
        let units: Vec<&str> = task.steps.iter().map(|row| row.unit.as_str()).collect();

        assert!(units.contains(&"list") && units.contains(&"count:a.txt") && units.contains(&"count:b.txt") && units.contains(&"count:src/c.rs") && units.contains(&"check") && units.contains(&"report"), "{units:?}");
        assert_eq!((done.outcome.done, done.outcome.blocked, done.outcome.remaining), (6, 0, 0), "{:?}", done.outcome);
        assert_eq!(done.data["counts"], vec!["a.txt: 2 lines", "b.txt: 1 lines", "src/c.rs: 3 lines"]);
        assert_eq!(task.state, State::Done);
        assert_eq!(task.criteria[0].evidence.as_deref(), Some("counts: 3 items"), "the gate proves the contract line");

        assert_eq!(asked.len(), 1);
        assert!(asked[0].prompt.contains("src/c.rs: 3 lines") && asked[0].tools == vec!["read_file", "grep"] && asked[0].kind == Some(StepKind::ReduceReport), "{:?}", asked[0]);

        let labels: Vec<String> = recorder.events().iter().filter_map(|event| if let EngineEvent::TaskProgress { label, .. } = event { Some(label.clone()) } else { None }).collect();

        assert!(labels.contains(&"count 3/3".to_string()), "{labels:?}");

        let _ = std::fs::remove_dir_all(&env.root);
    }

    #[test]
    fn the_model_step_is_offered_only_its_tools_and_thinks_as_its_kind_says() {
        let (env, mut ledger) = project("tools");
        let registry = Registry::standard();
        let recipe = parse(TOY).unwrap();
        let recorder = Recorder::new();
        let seen = std::cell::RefCell::new(None);
        let specs = [("read_file", "x"), ("edit_file", "x"), ("grep", "x")].map(|(name, description)| crate::agent::dialect::ToolSpec { name, description, schema: serde_json::json!({}) });

        execute(&recipe, job(&mut ledger, &env, &recorder.sink(), &registry, Budget::default()), &mut |_| {
            *seen.borrow_mut() = Some((super::super::tools::subset(&specs).map(|chosen| chosen.iter().map(|spec| spec.name).collect::<Vec<_>>()), super::super::think::current()));

            UnitRun { calls: 1, result: Ok("report".into()) }
        })
        .unwrap();

        assert_eq!(seen.into_inner(), Some((Some(vec!["read_file", "grep"]), Some(StepKind::ReduceReport))));
        assert!(super::super::tools::subset(&specs).is_none() && super::super::think::current().is_none(), "both are gone after the step");

        let _ = std::fs::remove_dir_all(&env.root);
    }

    #[test]
    fn a_checkpointed_recipe_goes_on_where_it_stopped_and_reads_back_what_it_made() {
        let (env, mut ledger) = project("resume");
        let registry = Registry::standard();
        let recipe = parse(TOY).unwrap();
        let recorder = Recorder::new();
        let first = execute(&recipe, job(&mut ledger, &env, &recorder.sink(), &registry, Budget { unit_calls: 8, turn: std::time::Duration::ZERO }), &mut |_| unreachable!("no time was left")).unwrap();

        assert!(first.outcome.paused && first.outcome.done == 0, "{:?}", first.outcome);
        assert_eq!(ledger.get("t1").unwrap().state, State::Paused);

        let mut asked = 0;
        let second = execute(&recipe, job(&mut ledger, &env, &recorder.sink(), &registry, Budget::default()), &mut |_| {
            asked += 1;

            UnitRun { calls: 1, result: Ok("report".into()) }
        })
        .unwrap();

        assert!(!second.outcome.paused && second.outcome.remaining == 0 && second.outcome.done == 6, "{:?}", second.outcome);
        assert_eq!(asked, 1);

        let mut again = 0;
        let third = execute(&recipe, job(&mut ledger, &env, &recorder.sink(), &registry, Budget::default()), &mut |_| {
            again += 1;

            UnitRun { calls: 1, result: Ok("report".into()) }
        })
        .unwrap();

        assert_eq!(again, 0, "a finished recipe is not run again");
        assert_eq!(third.data["counts"].len(), 3, "what the steps made is read back from the ledger");

        let _ = std::fs::remove_dir_all(&env.root);
    }

    #[test]
    fn a_blocked_step_blocks_what_needs_it_and_the_rest_is_reported_not_hidden() {
        let (env, mut ledger) = project("blocked");
        let registry = Registry::standard();
        let recipe = parse(TOY).unwrap();
        let recorder = Recorder::new();
        let empty = Env::new(env.root.join("nothing here"));
        let done = execute(&recipe, job(&mut ledger, &empty, &recorder.sink(), &registry, Budget::default()), &mut |_| unreachable!("the report needs the files")).unwrap();
        let task = ledger.get("t1").unwrap();

        assert!(done.outcome.blocked >= 2 && done.outcome.done == 0, "{:?}", done.outcome);
        assert!(task.steps.iter().any(|row| row.unit == "list" && row.status == Status::Blocked && row.handover_note.contains("no files")), "{:?}", task.steps);
        assert!(task.steps.iter().any(|row| row.unit == "report" && row.status == Status::Blocked && row.handover_note.contains("needs")), "{:?}", task.steps);
        assert_eq!(task.state, State::Blocked);

        let _ = std::fs::remove_dir_all(&env.root);
    }

    #[test]
    fn a_bad_recipe_is_refused_before_it_runs() {
        let registry = Registry::standard();
        let bad = |text: &str| validate(&parse(text).unwrap(), &registry).unwrap_err();

        assert!(bad("id = \"x\"\n[[step]]\nid = \"a\"\nkind = \"code\"\nrun = \"nope\"").contains("not a code function"));
        assert!(bad("id = \"x\"\n[[step]]\nid = \"a\"\nkind = \"model\"").contains("no prompt"));
        assert!(bad("id = \"x\"\n[[step]]\nid = \"a\"\nkind = \"code\"\nrun = \"list_files\"\nneeds = [\"b\"]\n[[step]]\nid = \"b\"\nkind = \"code\"\nrun = \"list_files\"\nneeds = [\"a\"]").contains("depend on each other"));
        assert!(bad("id = \"x\"\n[[step]]\nid = \"a\"\nkind = \"code\"\nrun = \"count_lines\"\nfor_each = \"files\"").contains("no step it depends on produces"));
        assert!(bad("id = \"x\"\n[[step]]\nid = \"a\"\nkind = \"model\"\nprompt = \"p\"\nthink = \"dreaming\"").contains("unknown think kind"));
        assert!(bad("id = \"x\"\n[[step]]\nid = \"a\"\nkind = \"code\"\nrun = \"list_files\"\n[[step]]\nid = \"a\"\nkind = \"code\"\nrun = \"list_files\"").contains("used twice"));
        assert!(validate(&parse(TOY).unwrap(), &registry).is_ok());
    }

    #[test]
    fn a_project_recipe_is_loaded_and_a_broken_one_is_named_not_fatal() {
        let (env, _ledger) = project("load");
        let registry = Registry::standard();

        std::fs::create_dir_all(env.root.join(".sdc/recipes")).unwrap();
        std::fs::write(env.root.join(".sdc/recipes/toy.toml"), TOY).unwrap();
        std::fs::write(env.root.join(".sdc/recipes/broken.toml"), "id = \"y\"\n[[step]]\nid = \"a\"\nkind = \"code\"\nrun = \"nope\"").unwrap();

        let (recipes, problems) = load_all(&env.root, &registry);

        assert_eq!(recipes.iter().map(|recipe| recipe.id.as_str()).collect::<Vec<_>>(), vec!["R-REVIEW-CODE", "R-BUILD-PAGE", "R-FIX", "R-REVIEW-SITE", "R-TOY-COUNT"]);
        assert!(problems.len() == 1 && problems[0].contains("broken.toml"), "{problems:?}");
        assert!(find(&env.root, "R-TOY-COUNT", &registry).is_some() && find(&env.root, "R-NOPE", &registry).is_none());

        let _ = std::fs::remove_dir_all(&env.root);
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::engines::Recorder;
    use crate::padma::ledger::{contract_for, State};

    const AUTH: &str = "def login(user, password):\n    if password == \"admin123\":\n        return True\n    return False\n";
    const SQL: &str = "def find(db, name):\n    query = \"SELECT * FROM users WHERE name = '\" + name + \"'\"\n    return db.execute(query)\n";
    const UTIL: &str = "def double(n):\n    return n * 2\n";

    fn fixture(name: &str) -> Env {
        let root = std::env::temp_dir().join(format!("sdc-review-{name}-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src dir")).unwrap();
        std::fs::write(root.join("src dir/auth.py"), AUTH).unwrap();
        std::fs::write(root.join("src dir/queries_sql.py"), SQL).unwrap();
        std::fs::write(root.join("src dir/util.py"), UTIL).unwrap();
        std::fs::write(root.join("README.md"), "# readme\n").unwrap();
        std::fs::write(root.join("logo.png"), [0u8, 159, 146, 150]).unwrap();
        std::fs::write(root.join("Cargo.lock"), "# lock\n").unwrap();

        Env::new(root)
    }

    fn started(env: &Env, request: &str) -> Ledger {
        let mut ledger = Ledger::open(&env.root).unwrap();

        ledger.create(&contract_for("t1", "s", request, "turn:t1"), &[]).unwrap();

        ledger
    }

    fn reviewer(call: &ModelCall) -> UnitRun {
        let answer = match call.unit.as_deref() {
            Some("src dir/auth.py") => "src dir/auth.py:2 · high · security · the password is compared to a hard-coded value · if password == \"admin123\":\nsrc dir/auth.py:3 · low · logic · returns before logging the attempt · log_attempt(user)".to_string(),
            Some("src dir/queries_sql.py") => "src dir/queries_sql.py:2 · high · security · SQL is built by joining the name into the text · query = \"SELECT * FROM users WHERE name = '\" + name + \"'\"".to_string(),
            Some(_) => "NO FINDINGS".to_string(),
            None if call.step == "summary" => "Two serious problems: a hard-coded password and an injectable query.".to_string(),
            None => "?".to_string(),
        };

        UnitRun { calls: 1, result: Ok(answer) }
    }

    fn said(recorder: &Recorder) -> String {
        recorder.events().iter().filter_map(|event| if let EngineEvent::Delta(text) = event { Some(text.as_str()) } else { None }).collect()
    }

    fn squashed(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn the_built_in_review_recipe_is_valid() {
        let registry = Registry::standard();
        let (recipes, problems) = load_all(std::path::Path::new("."), &registry);

        assert!(problems.is_empty(), "{problems:?}");
        assert!(recipes.iter().any(|recipe| recipe.id == "R-REVIEW-CODE"));
    }

    #[test]
    fn a_review_reports_only_what_is_proved_and_lists_what_it_dropped() {
        let env = fixture("full");
        let mut ledger = started(&env, "review the whole project and report errors");
        let registry = Registry::standard();
        let recipe = find(&env.root, "R-REVIEW-CODE", &registry).expect("the built-in recipe");
        let recorder = Recorder::new();
        let mut prompts = Vec::new();
        let done = execute(&recipe, Job { ledger: &mut ledger, task_id: "t1", sink: &recorder.sink(), registry: &registry, env: &env, bengali: false, budget: Budget::default(), stop: None, relay: false }, &mut |call| {
            prompts.push(call.prompt.clone());

            reviewer(call)
        })
        .unwrap();
        let text = said(&recorder);

        assert_eq!((done.outcome.blocked, done.outcome.remaining), (0, 0), "{:?}", done.outcome);
        assert_eq!(ledger.get("t1").unwrap().state, State::Done);

        let units = &done.data["units"];

        assert_eq!(units[0], "src dir/auth.py", "the file that guards something is read first: {units:?}");
        assert!(!units.iter().any(|unit| unit.contains("logo.png") || unit.contains("Cargo.lock")), "{units:?}");
        assert!(prompts[0].contains("1| def login(user, password):") && prompts[0].contains("File: src dir/auth.py (part 1 of 1"), "{}", prompts[0]);
        assert!(text.contains("Two serious problems") && text.find("Two serious").unwrap() < text.find("# Code review").unwrap(), "the summary comes first: {text}");
        assert!(text.contains("2 verified findings (2 high"), "{text}");
        assert!(text.contains("`src dir/auth.py:2` [security]") && text.contains("`src dir/queries_sql.py:2` [security]"), "{text}");
        assert!(text.contains("evidence: `if password == \"admin123\":`"), "{text}");
        assert!(!text.contains("log_attempt"), "the finding whose evidence is not in the file is not in the report: {text}");
        assert!(text.contains("Dropped: 1 whose evidence is not in the file"), "{text}");

        for line in done.data["ranked"].iter().filter(|line| !line.starts_with('#')) {
            let evidence = line.split(" · ").last().unwrap();
            let path = line.split(':').next().unwrap();

            assert!(squashed(&std::fs::read_to_string(env.root.join(path)).unwrap()).contains(evidence), "{line}");
        }

        let _ = std::fs::remove_dir_all(&env.root);
    }

    #[test]
    fn a_unit_that_cannot_be_reviewed_is_listed_not_hidden() {
        let env = fixture("blocked");
        let mut ledger = started(&env, "review the whole project");
        let registry = Registry::standard();
        let recipe = find(&env.root, "R-REVIEW-CODE", &registry).unwrap();
        let recorder = Recorder::new();
        let done = execute(&recipe, Job { ledger: &mut ledger, task_id: "t1", sink: &recorder.sink(), registry: &registry, env: &env, bengali: false, budget: Budget::default(), stop: None, relay: false }, &mut |call| {
            if call.unit.as_deref() == Some("src dir/util.py") {
                UnitRun { calls: 1, result: Err("ContextOverflow: this step is about 9000 tokens".into()) }
            } else {
                reviewer(call)
            }
        })
        .unwrap();
        let text = said(&recorder);

        assert_eq!((done.outcome.blocked, done.outcome.remaining), (1, 0), "{:?}", done.outcome);
        assert_eq!(ledger.get("t1").unwrap().state, State::Blocked);
        assert!(text.contains("## Not reviewed") && text.contains("- src dir/util.py - ContextOverflow"), "{text}");
        assert!(text.contains("1 blocked"), "{text}");

        let _ = std::fs::remove_dir_all(&env.root);
    }

    #[test]
    fn a_long_file_is_cut_into_parts_that_each_fit_and_none_is_lost() {
        let mut env = fixture("long");
        let long: String = (1..=300).map(|n| format!("let value_{n} = compute({n});\n")).collect();

        env.unit_chars = 2_000;
        std::fs::write(env.root.join("big.js"), &long).unwrap();

        let units = super::super::codefns::plan_units(&env, None, &Data::from([("files".to_string(), vec!["big.js".to_string()]), ("errors".to_string(), Vec::new())])).unwrap();

        assert!(units.len() > 3 && units[0] == format!("big.js#1/{}", units.len()), "{units:?}");

        let mut seen = 0;

        for unit in &units {
            let text = super::super::codefns::read_unit(&env, unit).unwrap();

            assert!(text.len() < 2_000 + 200, "{} chars", text.len());
            seen += text.lines().count() - 1;
        }

        assert_eq!(seen, 300, "every line is in exactly one part");

        let _ = std::fs::remove_dir_all(&env.root);
    }

    #[test]
    fn the_error_slicer_keeps_diagnostics_only_and_makes_paths_relative_to_the_project() {
        let root = std::path::Path::new("F:/SDC Code/proj");
        let dir = root.join("sdc/sdcd");
        let cargo = "   Compiling x v1\nsrc\\main.rs:12:5: error[E0308]: mismatched types\nwarning: `x` generated 1 warning\nsrc/lib.rs:3:1: warning: unused import: `a`\n";
        let sliced = super::super::codefns::slice_errors(cargo, root, &dir);

        assert_eq!(sliced, vec!["sdc/sdcd/src/main.rs:12: error[E0308]: mismatched types", "sdc/sdcd/src/lib.rs:3: warning: unused import: `a`"]);

        let tsc = "src/a.ts(7,3): error TS2322: Type 'string' is not assignable to type 'number'.\nFound 1 error.\n";

        assert_eq!(super::super::codefns::slice_errors(tsc, root, &root.join("ui")), vec!["ui/src/a.ts:7: error TS2322: Type 'string' is not assignable to type 'number'."]);
        assert_eq!(super::super::codefns::slice_errors("F:/SDC Code/proj/ui/x.ts:9:1: error nope", root, root), vec!["ui/x.ts:9: error nope"]);
    }
}
