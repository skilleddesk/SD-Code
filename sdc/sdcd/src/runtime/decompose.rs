//! **The task decomposer (P1-10, switch `runtime.decompose`)**: work is cut into steps the size a small model can hold - **one or two
//! functions a step** - and each step names the files it touches, so the step's prompt carries only those.
//!
//! Two shapes, and the small one is the default:
//!
//! * **As needed** ([`Breakdown::one_step_only`]): most requests are one step, and doing them in one step is the fastest and the cheapest
//!   thing SDC can do. A breakdown is made only when the request really names more work than a step: three or more files, or more than
//!   two symbols. [`breakdown`] says which, and why, in `reason`.
//! * **Planned**: the request's own structure is used, not invented - its numbered list, its semicolons, its "then"; and each piece is
//!   cut again by file when it names more than one file. A step that still names more than [`FILES_PER_STEP`] files is split by file, so
//!   no step is ever larger than the budget it is given.
//!
//! The next step's files are known as soon as the breakdown is ([`next_files`]), which is what lets the runtime **load the next step's
//! files while the current one runs** - the prefetch the plan asks for.

/// How many files one step may touch before it is split.
pub const FILES_PER_STEP: usize = 2;
/// How many symbols one step may name before the request counts as planned work.
pub const SYMBOLS_PER_STEP: usize = 2;

/// One step of the breakdown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Steplet {
    pub goal: String,
    pub files: Vec<String>,
    pub symbols: Vec<String>,
}

/// The breakdown of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breakdown {
    pub steps: Vec<Steplet>,
    /// `true` when the request is small enough to be one step - the fast path, which is the usual case.
    pub one_step_only: bool,
    /// Why it was cut (or not): one sentence, for the trace.
    pub reason: String,
}

/// Splits a request into pieces by its own structure: numbered items, semicolons, the Bengali `।`, and the words people join work with.
fn pieces(request: &str) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = request.chars().collect();
    let mut index = 0;

    while index < chars.len() {
        let c = chars[index];

        /* "1." "2)" at the start of a line: the person is numbering the work */
        let at_word_start = index == 0 || matches!(chars[index - 1], '\n' | ' ' | '\t');
        let one_or_two_digits = index == 0 || !chars.get(index - 1).is_some_and(|previous| previous.is_ascii_digit());
        let followed_by_space = chars.get(index + 2).is_some_and(|next| next.is_whitespace());
        let numbered = c.is_ascii_digit() && at_word_start && one_or_two_digits && followed_by_space && chars.get(index + 1).is_some_and(|next| matches!(next, '.' | ')'));

        if numbered && !current.trim().is_empty() {
            pieces.push(current.trim().to_string());
            current.clear();
            index += 2;
            continue;
        }

        if matches!(c, ';' | '।') {
            pieces.push(current.trim().to_string());
            current.clear();
            index += 1;
            continue;
        }

        current.push(c);
        index += 1;
    }

    if !current.trim().is_empty() {
        pieces.push(current.trim().to_string());
    }

    /* "do A and then do B": the join is a step boundary - but "charge and currency" is one step, so only the explicit joins count */
    let mut split: Vec<String> = Vec::new();

    for piece in pieces {
        split.extend(split_on(&piece, &[" and then ", " then ", " tarpor ", " তারপর ", " ar "]));
    }

    /* a piece naming several files is cut again by file, so no step is larger than the budget */
    let mut final_pieces: Vec<String> = Vec::new();

    for piece in split {
        let files = files_in(&piece);

        if files.len() <= FILES_PER_STEP {
            final_pieces.push(piece);
            continue;
        }

        let rest = files.iter().fold(piece.clone(), |text, file| text.replace(file.as_str(), "")).trim().trim_end_matches(['।', ',']).trim().to_string();

        for file in &files {
            final_pieces.push(format!("{rest} ({file})"));
        }
    }

    final_pieces
}

/// Splits a text at every marker, in order, keeping the text between them (and the markers before the first, which is nothing).
fn split_on(text: &str, markers: &[&str]) -> Vec<String> {
    let mut parts = vec![text.to_string()];

    for marker in markers {
        let mut next = Vec::new();

        for part in parts {
            let lower = part.to_lowercase();
            let mut from = 0;

            while let Some(at) = lower[from..].find(marker) {
                let start = from + at;
                let stop = start + marker.len();

                if part.is_char_boundary(start) && part.is_char_boundary(stop) {
                    next.push(part[from..start].to_string());
                    from = stop;
                } else {
                    break;
                }
            }

            next.push(part[from..].to_string());
        }

        parts = next;
    }

    parts.into_iter().map(|part| part.trim().trim_matches([',', ' ']).to_string()).filter(|part| !part.is_empty()).collect()
}

/// The paths a text names: a token with a slash, or a file with an extension.
pub fn files_in(text: &str) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();

    for word in text.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '"' | '\'' | '(' | ')' | '`' | '।' | '?' | '!'));

        let cleaned = cleaned.trim_end_matches('.');

        if cleaned.is_empty() || cleaned.len() > 160 {
            continue;
        }

        let last = cleaned.rsplit(['/', '\\']).next().unwrap_or(cleaned);
        let extension = last.rsplit_once('.').is_some_and(|(name, ext)| !name.is_empty() && (1..=6).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()));
        let deep_path = cleaned.matches('/').count() >= 2;
        let path = (cleaned.contains('/') || cleaned.contains('\\')) && extension;

        if (path || deep_path || (extension && last.contains('.'))) && !files.contains(&cleaned.to_string()) {
            files.push(cleaned.to_string());
        }
    }

    files
}

/// Breaks a request into steps. `known` is the project's symbols and their files (from the Code Graph); it is what turns "fix charge
/// and refund" into two steps that each name their own file.
pub fn breakdown(request: &str, known: &[(String, String)]) -> Breakdown {
    let symbols: Vec<String> = known.iter().map(|(name, _)| name.rsplit('.').next().unwrap_or(name).to_string()).filter(|name| !name.is_empty() && request.contains(name.as_str())).collect();
    let files = files_in(request);
    let one_step_only = files.len() <= FILES_PER_STEP && symbols.len() <= SYMBOLS_PER_STEP;

    if one_step_only {
        return Breakdown {
            steps: vec![Steplet { goal: request.trim().to_string(), files: files.clone(), symbols: symbols.clone() }],
            one_step_only: true,
            reason: format!("one step: {} file(s) and {} symbol(s) named, and a step holds {}", files.len(), symbols.len(), FILES_PER_STEP),
        };
    }

    let mut steps: Vec<Steplet> = Vec::new();

    for piece in pieces(request) {
        let piece_files = files_in(&piece);
        let piece_symbols: Vec<String> = known
            .iter()
            .filter(|(name, file)| {
                let short = name.rsplit('.').next().unwrap_or(name);

                piece.contains(short) || piece.contains(file.as_str())
            })
            .map(|(name, _)| name.clone())
            .collect();

        if steps.last().is_some_and(|last| last.goal == piece.trim()) {
            continue;
        }

        steps.push(Steplet { goal: piece.trim().to_string(), files: piece_files, symbols: piece_symbols });
    }

    if steps.is_empty() {
        steps.push(Steplet { goal: request.trim().to_string(), files: files.clone(), symbols: symbols.clone() });
    }

    for step in &mut steps {
        if step.files.is_empty() {
            step.files = files.iter().filter(|file| request.contains(file.as_str())).cloned().collect();
        }
    }

    let reason = format!("{} steps: the request names {} files and {} symbols, and a step holds {}", steps.len(), files.len(), symbols.len(), FILES_PER_STEP);

    Breakdown { steps, one_step_only: false, reason }
}

/// The note the model gets when a request is several jobs: the steps in order, each with its files, to be finished one at a time.
/// `None` for the usual case, one step.
pub fn note(breakdown: &Breakdown) -> Option<String> {
    if breakdown.one_step_only || breakdown.steps.len() < 2 {
        return None;
    }

    let mut lines = vec!["This request is several jobs. Do them one at a time, in this order, and finish and check each before the next:".to_string()];

    for (index, step) in breakdown.steps.iter().enumerate() {
        let files = if step.files.is_empty() { String::new() } else { format!(" (files: {})", step.files.join(", ")) };

        lines.push(format!("{}. {}{}", index + 1, step.goal, files));
    }

    Some(lines.join("\n"))
}

/// The files the next step needs - what the runtime reads while the current step still runs.
pub fn next_files(breakdown: &Breakdown, done: usize) -> Vec<String> {
    breakdown.steps.get(done + 1).map(|step| step.files.clone()).unwrap_or_default()
}

/// Puts a plan's steps through the same rule, and answers the steps that are too large - the sentence the model gets names them.
pub fn too_large(steps: &[String]) -> Vec<(usize, Vec<String>)> {
    steps.iter().enumerate().filter_map(|(index, step)| {
        let files = files_in(step);

        (files.len() > FILES_PER_STEP).then_some((index, files))
    }).collect()
}

#[cfg(test)]
mod tests;
