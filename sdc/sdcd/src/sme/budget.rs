//! **Model classes and per-step token budgets (P1-3, switch `sme.budget`)**: a local model is sent what its window holds, and the room
//! is divided before the request is built instead of being discovered by an overflow.
//!
//! The class comes from the window the server reports (P0-9, `sme::probe`): **S** under 16K, **M** under 64K, **L** above it. For each
//! class there is one budget - what the step may spend on the system prompt, the task card, working memory, code, the tool schemas and
//! one tool result - and a reserve for the answer and for thinking, so a step can always finish what it started.
//!
//! Two rules hold everywhere:
//!
//! * **The reserve is never lent.** A prompt that would leave no room to answer is not trimmed into shape: [`Budget::fits`] says no,
//!   and the caller drops the lowest-purity section (a tool result before a source, a source before the goal - [`Section::priority`]).
//! * **Trimming is a sentence, not a surprise.** [`Budget::trim`] cuts at a line boundary and says `(trimmed to fit the model's
//!   window)`, so the model knows its evidence is partial; a silent cut is what makes a small model answer about code it cannot see.

/// A local model's class, from its context window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Under 16K: the 4K-8K windows Ollama's default `num_ctx` gives a small machine.
    Small,
    /// 16K to 64K: a 32K window, what SDC asks Ollama for by default.
    Medium,
    /// Above 64K.
    Large,
}

impl Class {
    pub fn of(window: u64) -> Self {
        match window {
            0..=16_383 => Class::Small,
            16_384..=65_535 => Class::Medium,
            _ => Class::Large,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Class::Small => "S",
            Class::Medium => "M",
            Class::Large => "L",
        }
    }
}

/// What one step spends its tokens on. The order is the priority order: when room runs out, the last one goes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// The rules - never trimmed below the class's floor (they are what keeps the model honest).
    System,
    /// The task card: the goal, the files, what is done when.
    Card,
    /// Working memory: the few facts this task established (P1-7).
    Memory,
    /// The code and the sources.
    Code,
    /// The tool schemas offered this step.
    Tools,
    /// One tool's result (a command's output, a fetched page).
    ToolOutput,
}

impl Section {
    pub fn name(self) -> &'static str {
        match self {
            Section::System => "system",
            Section::Card => "card",
            Section::Memory => "memory",
            Section::Code => "code",
            Section::Tools => "tools",
            Section::ToolOutput => "tool output",
        }
    }

    /// 0 keeps the longest: the goal and the rules. 5 goes first.
    pub fn priority(self) -> u8 {
        match self {
            Section::System => 0,
            Section::Card => 1,
            Section::Code => 2,
            Section::Memory => 3,
            Section::Tools => 4,
            Section::ToolOutput => 5,
        }
    }
}

/// One class's budget, in tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub class: Class,
    pub window: u64,
    /// The answer's room, held back from the first counted token.
    pub output: usize,
    /// A thinking model's room for its reasoning, held back beside the answer's.
    pub thinking: usize,
    caps: [(Section, usize); 6],
}

/// The share of the prompt each section gets, per class: a small window spends more of itself on the goal and less on code, because
/// a small model loses the goal first.
const SHARES_SMALL: [(Section, f64); 6] = [(Section::System, 0.17), (Section::Card, 0.10), (Section::Memory, 0.07), (Section::Code, 0.37), (Section::Tools, 0.14), (Section::ToolOutput, 0.15)];
const SHARES_MEDIUM: [(Section, f64); 6] = [(Section::System, 0.13), (Section::Card, 0.07), (Section::Memory, 0.08), (Section::Code, 0.42), (Section::Tools, 0.14), (Section::ToolOutput, 0.16)];
const SHARES_LARGE: [(Section, f64); 6] = [(Section::System, 0.08), (Section::Card, 0.04), (Section::Memory, 0.06), (Section::Code, 0.49), (Section::Tools, 0.15), (Section::ToolOutput, 0.18)];

impl Budget {
    /// One seventh of the window for the answer and a tenth for thinking: a small model still has to finish a sentence, and a
    /// thinking model still has to think. What is left is divided by the class's shares - so a 4K window is a smaller budget of the
    /// same shape, not an impossible one.
    pub fn for_window(window: u64) -> Self {
        let class = Class::of(window);
        let window = window.clamp(2_048, 1_048_576) as usize;
        let output = (window / 7).max(256);
        let thinking = (window / 10).max(192);
        let shares = match class {
            Class::Small => SHARES_SMALL,
            Class::Medium => SHARES_MEDIUM,
            Class::Large => SHARES_LARGE,
        };
        let available = window.saturating_sub(output + thinking);
        let caps = shares.map(|(section, share)| (section, (available as f64 * share) as usize));

        Self { class, window: window as u64, output, thinking, caps }
    }

    /// The room one section may take, in tokens.
    pub fn cap(&self, section: Section) -> usize {
        self.caps.iter().find(|(name, _)| *name == section).map(|(_, cap)| *cap).unwrap_or(0)
    }

    /// The same for text: a section's cap in characters, which is what a tool result is measured in.
    pub fn chars(&self, section: Section) -> usize {
        self.cap(section) * 4
    }

    /// The whole input the parts may take (the sum of the caps), and what is held back for the answer.
    pub fn input(&self) -> usize {
        self.caps.iter().map(|(_, cap)| cap).sum()
    }

    /// Does a step that spends `parts` still have room for its answer and its thinking?
    pub fn fits(&self, parts: &[(Section, usize)]) -> bool {
        let wanted: usize = parts.iter().map(|(_, tokens)| *tokens).sum();

        wanted + self.output + self.thinking <= self.window as usize
    }

    /// Which section to drop when the step does not fit: the lowest priority that still has something in it. `None` when the step
    /// fits, or when only the system prompt is left (then the answer is a smaller step, not a smaller goal).
    pub fn drop_first(&self, parts: &[(Section, usize)]) -> Option<Section> {
        if self.fits(parts) {
            return None;
        }

        parts
            .iter()
            .filter(|(section, tokens)| *tokens > 0 && *section != Section::System)
            .min_by_key(|(section, _)| std::cmp::Reverse(section.priority()))
            .map(|(section, _)| *section)
    }

    /// Trims a text to its section's cap, at a line boundary, and says so. The second value is whether anything was cut.
    pub fn trim(&self, text: &str, section: Section) -> (String, bool) {
        let cap = self.chars(section);

        if text.len() <= cap {
            return (text.to_string(), false);
        }

        /* the note itself must fit, so a little is taken off the cap */
        let note = "\n(trimmed to fit the model's window)";
        let room = cap.saturating_sub(note.len());
        let mut cut = room.min(text.len());

        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }

        if let Some(break_at) = text[..cut].rfind('\n') {
            cut = break_at;
        }

        (format!("{}{note}", text[..cut].trim_end()), true)
    }

    /// How many of the costs fit under a section's cap, in order. Used to keep the tool schemas the room can hold.
    pub fn take_within<'a>(&self, section: Section, costs: impl Iterator<Item = &'a usize>) -> usize {
        let cap = self.cap(section);
        let mut spent = 0;
        let mut kept = 0;

        for cost in costs {
            if spent + cost > cap && kept > 0 {
                break;
            }

            spent += cost;
            kept += 1;
        }

        kept
    }

    /// One line for the trace: the class, the window, and the room the step has.
    pub fn sentence(&self) -> String {
        format!("class {} of {} tokens: {} for the prompt, {} kept for the answer, {} for thinking", self.class.name(), self.window, self.input(), self.output, self.thinking)
    }
}

#[cfg(test)]
mod tests;
