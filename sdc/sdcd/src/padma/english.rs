//! **English inside, the person's language outside (P2-4, switch `padma.inner_english`, off until the bench says)**: a small model reads and
//! reasons better in English, and Bengali costs several times the tokens. A request written in Bengali or Banglish is put into English once, by the
//! local model, in one tiny fixed-format call; the English line becomes the task's goal and the request the model works from, and a one-line note
//! tells it to answer the person in Bengali. A back-check (N2-2: every number, name, path and identifier of the original must be in the English
//! line) decides whether the restatement is trusted; if it lost one, the original stays and nothing changes. The original is never discarded - it
//! is the turn's own message in the session history. The person's language comes back at the end: the final status line is in Bengali.

use crate::engines::Prompt;
use crate::navaratna::langspec;

/// The note put in front of the request when it was restated; the ledger reads it to set the contract's `language_out`.
pub const NOTE_BN: &str = "[SDC: the person writes in Bengali; their words below were put into English. Write your answer to them in Bengali (keep code, paths and names as they are).]";

/// Words that are Banglish (Bengali in Latin letters) and nothing else.
const BANGLISH: &[&str] = &[
    "koro", "korun", "korbo", "korte", "kore", "dekho", "dekhe", "banao", "likho", "amar", "amake", "ekta", "ekti", "lagbe", "hobe", "hoy", "kemon", "kothay", "kono", "shob", "sob", "puro", "sompurno", "thik", "ache", "nai", "jonno", "diye", "theke",
    "tumi", "ami", "apni", "dao", "den", "chai", "chaichi", "bujhte", "bolo", "bolen", "kintu", "ebar", "ekhon", "pari", "parbe", "korchi", "korsi", "kaj", "khujo", "khuje",
];

/// Whether a request is written in Bengali script or in Banglish.
pub fn is_bengali(request: &str) -> bool {
    if request.chars().any(|c| ('\u{0980}'..='\u{09FF}').contains(&c)) {
        return true;
    }

    request.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).any(|word| BANGLISH.contains(&word))
}

/// The request for the model that restates: fixed format, the original in a fence so it cannot be mistaken for an instruction.
pub fn restate_prompt(request: &str) -> String {
    format!(
        "Translate the text between the lines into plain English. Keep every number, file name, path, identifier and quoted text exactly as written. Do not answer it, do not explain it, do not add anything. Output only the English text.\n-----\n{request}\n-----"
    )
}

/// What the restatement came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restated {
    /// Trusted: this is the English line.
    English(String),
    /// Not Bengali, or the model gave nothing, or the English lost something the original had: the original stays.
    Original,
}

/// Puts a request into English with `ask` (the one tiny call) and checks it. `Original` for a request that is not Bengali - `ask` is not called.
pub fn restate(request: &str, ask: impl FnOnce(&str) -> Option<String>) -> Restated {
    if !is_bengali(request) {
        return Restated::Original;
    }

    let Some(english) = ask(&restate_prompt(request)).map(|text| text.trim().trim_start_matches("```").trim_end_matches("```").trim().to_string()).filter(|text| !text.is_empty() && text.chars().count() <= request.chars().count() * 4 + 200) else {
        return Restated::Original;
    };

    if english.chars().any(|c| ('\u{0980}'..='\u{09FF}').contains(&c)) {
        return Restated::Original;
    }

    if langspec::missing(request, &english).is_empty() {
        Restated::English(english)
    } else {
        Restated::Original
    }
}

/// The prompt the model works from: the person's words replaced by the English line, with the note in front of them. A prompt with no
/// `[The person's message]` marker is the person's words in full, and the note goes first.
pub fn rewrite(prompt: &Prompt, english: &str) -> Prompt {
    let mut rewritten = prompt.clone();

    rewritten.text = match prompt.text.rfind("[The person's message]") {
        Some(at) => format!("{}{NOTE_BN}\n[The person's message]\n{english}", &prompt.text[..at]),
        None => format!("{NOTE_BN}\n{english}"),
    };

    rewritten
}

/// Whether a prompt was restated (so its answer is owed in Bengali).
pub fn answers_in_bengali(prompt: &Prompt) -> bool {
    prompt.text.contains(NOTE_BN)
}

/// The word for the person's language that the contract keeps.
pub fn language_out(prompt: &Prompt) -> &'static str {
    if answers_in_bengali(prompt) {
        "bn"
    } else {
        "same as the request"
    }
}

/// The unfinished-task line in the person's language.
pub fn status_line(bengali: bool, done: usize, blocked: usize, remaining: usize) -> String {
    if bengali {
        format!("\n\nকাজ: {done} শেষ · {blocked} আটকে · {remaining} বাকি।")
    } else {
        format!("\n\nTask: {done} done · {blocked} blocked · {remaining} remaining.")
    }
}

/// The line that says a turn stopped on its time budget, and how to go on.
pub fn pause_line(bengali: bool, minutes: u64) -> String {
    if bengali {
        format!("\n\n{minutes} মিনিট পার হয়েছে, এই পর্যন্ত কাজ সেভ করা আছে। এগিয়ে যেতে \"continue\" লিখুন।")
    } else {
        format!("\n\nI stopped after {minutes} minutes, the time for one turn. Everything done so far is saved. Send \"continue\" to go on from here.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bengali_and_banglish_are_told_from_english() {
        for request in ["billing.py ঠিক করো", "login page banao", "amar project ta puro dekho", "পুরো প্রজেক্ট রিভিউ করো"] {
            assert!(is_bengali(request), "{request}");
        }

        for request in ["fix the failing test in billing.py", "review the whole project", "hello"] {
            assert!(!is_bengali(request), "{request}");
        }
    }

    #[test]
    fn a_trusted_restatement_is_used_and_the_call_is_not_made_for_english() {
        let request = "`billing.py` এর 3 টা টেস্ট ঠিক করো";
        let got = restate(request, |prompt| {
            assert!(prompt.contains(request) && prompt.contains("Output only the English text"));

            Some("Fix the 3 tests of `billing.py`".to_string())
        });

        assert_eq!(got, Restated::English("Fix the 3 tests of `billing.py`".into()));
        assert_eq!(restate("fix billing.py", |_| panic!("English is not sent to the model")), Restated::Original);
    }

    #[test]
    fn a_restatement_that_lost_a_name_a_number_or_stayed_bengali_is_not_trusted() {
        let request = "`billing.py` এর 3 টা টেস্ট ঠিক করো";

        assert_eq!(restate(request, |_| Some("Fix the tests".into())), Restated::Original, "lost the file and the number");
        assert_eq!(restate(request, |_| Some("`billing.py` 3 টেস্ট ঠিক করো".into())), Restated::Original, "still Bengali");
        assert_eq!(restate(request, |_| None), Restated::Original, "no answer");
        assert_eq!(restate(request, |_| Some("   ".into())), Restated::Original, "empty answer");
    }

    #[test]
    fn the_rewritten_prompt_keeps_the_context_and_asks_for_a_bengali_answer() {
        let prompt = Prompt {
            session_id: "s".into(),
            turn_id: "t".into(),
            text: "context here\n[The person's message]\nbilling.py ঠিক করো".into(),
            model: "m".into(),
            provider: None,
            history: Vec::new(),
            project_root: None,
            remote: None,
            autonomy: Default::default(),
            resume: None,
            images: Vec::new(),
            effort: None,
        };
        let rewritten = rewrite(&prompt, "Fix billing.py");

        assert!(rewritten.text.starts_with("context here\n") && rewritten.text.ends_with("[The person's message]\nFix billing.py"));
        assert!(answers_in_bengali(&rewritten) && !answers_in_bengali(&prompt));
        assert_eq!(language_out(&rewritten), "bn");
        assert_eq!(crate::padma::task::request_of(&rewritten), "Fix billing.py");
    }

    #[test]
    fn the_status_line_is_in_the_persons_language() {
        assert!(status_line(true, 1, 0, 2).contains("কাজ: 1 শেষ"));
        assert!(status_line(false, 1, 0, 2).contains("Task: 1 done"));
    }
}
