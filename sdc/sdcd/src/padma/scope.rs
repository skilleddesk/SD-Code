//! **Scope Classifier (P1-3, switch `padma.scope`)**: code decides what kind of job a request is and how big it is, before any model is asked.
//! Input: the request's words (English, Bengali, Banglish), what it points at (a repository, a folder, a file, a URL, a server path) and the
//! repository's size. Output: a recipe and `large`. A request whose words do not decide is *ambiguous*; only then is one tiny model call made,
//! with a fixed list of labels and no free text (`classify_with`).

use std::path::Path;

/// The recipe a request belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recipe {
    ReviewCode,
    ReviewSite,
    BuildPage,
    Fix,
    Answer,
}

impl Recipe {
    pub fn id(self) -> &'static str {
        match self {
            Recipe::ReviewCode => "R-REVIEW-CODE",
            Recipe::ReviewSite => "R-REVIEW-SITE",
            Recipe::BuildPage => "R-BUILD-PAGE",
            Recipe::Fix => "R-FIX",
            Recipe::Answer => "R-ANSWER",
        }
    }

    const ALL: [Recipe; 5] = [Recipe::ReviewCode, Recipe::ReviewSite, Recipe::BuildPage, Recipe::Fix, Recipe::Answer];
}

/// What the request points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// The whole project ("the project", "the codebase", "প্রজেক্ট").
    Repo,
    Folder,
    File,
    Url,
    /// A path on a server (`ssh`, `user@host`, an absolute Unix path).
    Remote,
    None,
}

/// What the repository looks like.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RepoStats {
    pub files: usize,
    pub lines: usize,
}

/// One Context Pack's worth of text, in tokens: a target set larger than this cannot be read in one prompt, so the job is large.
pub const PACK_TOKENS: usize = 6_000;
const TOKENS_PER_LINE: usize = 12;

impl RepoStats {
    /// Counted from the file system, without reading files: code and text files by size (about 38 bytes a line). Skips what is not the project's own.
    pub fn of(root: &Path) -> RepoStats {
        const SKIP: &[&str] = &[".git", "node_modules", "target", ".sdc", "dist", "build", "vendor", ".next", "__pycache__", "venv", ".venv", ".idea", ".vscode"];
        const EXT: &[&str] = &[
            "rs", "ts", "tsx", "js", "jsx", "mjs", "py", "go", "java", "kt", "c", "h", "cpp", "cs", "rb", "php", "swift", "html", "css", "scss", "vue", "svelte", "md", "toml", "json", "yaml", "yml", "sql", "sh",
        ];

        let mut stats = RepoStats::default();
        let mut bytes = 0u64;
        let mut stack = vec![root.to_path_buf()];
        let mut seen = 0usize;

        while let Some(folder) = stack.pop() {
            for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
                seen += 1;

                if seen > 60_000 {
                    stack.clear();

                    break;
                }

                let name = entry.file_name().to_string_lossy().to_string();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };

                if kind.is_dir() {
                    if !SKIP.contains(&name.as_str()) {
                        stack.push(entry.path());
                    }
                } else if kind.is_file() && entry.path().extension().and_then(|ext| ext.to_str()).is_some_and(|ext| EXT.contains(&ext.to_lowercase().as_str())) {
                    let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);

                    if size <= 2_000_000 {
                        stats.files += 1;
                        bytes += size;
                    }
                }
            }
        }

        stats.lines = (bytes / 38) as usize;

        stats
    }

    fn exceeds_one_pack(&self) -> bool {
        self.lines * TOKENS_PER_LINE > PACK_TOKENS
    }

    fn known(&self) -> bool {
        self.files > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    pub recipe: Recipe,
    pub large: bool,
    pub target: Target,
    /// The words decided it. `false`: two recipes fit, or a project is named with no verb - `classify_with` may ask the model.
    pub confident: bool,
}

const REVIEW: &[&str] = &[
    "review", "audit", "inspect", "check", "examine", "scan", "analyze", "analyse", "look at", "look through", "look over", "find bugs", "find errors", "find issues", "find problems", "report", "go through",
    "রিভিউ", "দেখো", "দেখে", "খুঁজ", "খুজ", "যাচাই", "পরীক্ষা", "রিপোর্ট",
    "review koro", "dekho", "dekhe", "khujo", "khuje", "jachai", "porikkha", "report koro", "check koro",
];
const BUILD: &[&str] = &["build", "make", "create", "design", "generate", "write", "বানাও", "তৈরি", "banao", "toiri", "likho", "বানিয়ে"];
const FIX: &[&str] = &[
    "fix", "repair", "solve", "debug", "resolve", "correct", "patch", "ঠিক করো", "ঠিক করে", "সমাধান", "মেরামত", "thik koro", "thik kore", "somadhan", "fix koro", "sarao",
];
const BREADTH: &[&str] = &[
    "whole", "entire", "all", "every", "everything", "full", "complete", "completely", "throughout", "end to end", "end-to-end", "across", "পুরো", "সম্পূর্ণ", "সব", "সকল", "সবকিছু", "প্রতিটি",
    "puro", "poora", "sompurno", "sompurnoru", "sob", "sokol", "shob", "shobkichu", "protiti",
];
const REPO_WORDS: &[&str] = &[
    "project", "repo", "repository", "codebase", "code base", "source", "app", "application", "প্রজেক্ট", "কোডবেস", "রেপো", "কোড", "projekt", "prozekt", "codebase", "repo",
];
const FOLDER_WORDS: &[&str] = &["folder", "directory", "dir", "src", "module", "package", "ফোল্ডার", "ডিরেক্টরি"];
const SITE_WORDS: &[&str] = &["website", "web site", "site", "web page", "webpage", "landing page", "landing", "ওয়েবসাইট", "সাইট", "ওয়েব পেজ", "লান্ডিং", "website er", "saite"];
const PAGE_WORDS: &[&str] = &["page", "website", "web site", "site", "landing", "homepage", "home page", "ui", "frontend", "dashboard", "পেজ", "ওয়েবসাইট", "সাইট", "ল্যান্ডিং", "ড্যাশবোর্ড", "peej", "page banao"];
const CODE_PROBLEM: &[&str] = &["error", "errors", "bug", "bugs", "issue", "issues", "problem", "problems", "vulnerab", "ত্রুটি", "বাগ", "সমস্যা", "এরর", "ভুল", "bug khujo", "error"];

fn has_word(lower: &str, needle: &str) -> bool {
    let mut from = 0;

    while let Some(at) = lower[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        let before = lower[..start].chars().next_back();
        let after = lower[end..].chars().next();
        let edge = |c: Option<char>| c.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));

        if edge(before) && (edge(after) || !needle.is_ascii()) {
            return true;
        }

        from = end;
    }

    false
}

fn any(lower: &str, words: &[&str]) -> bool {
    words.iter().any(|word| has_word(lower, word))
}

fn url_in(lower: &str) -> bool {
    lower.contains("http://") || lower.contains("https://") || lower.contains("www.") || lower.split_whitespace().any(|word| {
        let word = word.trim_matches(|c: char| matches!(c, '.' | ',' | ')' | '(' | '"' | '\''));

        word.contains('.') && !word.contains('/') && ["com", "org", "net", "io", "dev", "app", "co", "xyz", "bd"].iter().any(|tld| word.ends_with(&format!(".{tld}")))
    })
}

fn remote_in(lower: &str) -> bool {
    lower.contains("ssh ") || lower.contains("vps") || lower.split_whitespace().any(|word| (word.contains('@') && word.contains('.')) || (word.starts_with("/var/") || word.starts_with("/home/") || word.starts_with("/srv/") || word.starts_with("/etc/") || word.starts_with("/opt/")))
}

fn file_in(request: &str) -> bool {
    crate::rotation::preroute::mentions(request).0 > 0
}

/// What the request points at.
pub fn target_of(request: &str) -> Target {
    let lower = request.to_lowercase();

    if remote_in(&lower) {
        Target::Remote
    } else if url_in(&lower) {
        Target::Url
    } else if file_in(request) {
        Target::File
    } else if any(&lower, REPO_WORDS) {
        Target::Repo
    } else if any(&lower, FOLDER_WORDS) {
        Target::Folder
    } else {
        Target::None
    }
}

/// Classifies a request from its words and the repository's size. No model is asked.
pub fn classify(request: &str, repo: &RepoStats) -> Scope {
    let head: String = request.chars().take(600).collect();
    let lower = head.to_lowercase();
    let target = target_of(&head);
    let breadth = any(&lower, BREADTH);
    let asks_question = lower.trim_end().ends_with('?') || lower.trim_end().ends_with('？');

    let review = any(&lower, REVIEW);
    let build = any(&lower, BUILD);
    let fix = any(&lower, FIX);
    let site = any(&lower, SITE_WORDS) || matches!(target, Target::Url);
    let page = any(&lower, PAGE_WORDS);
    let problem = any(&lower, CODE_PROBLEM);
    let code_target = matches!(target, Target::Repo | Target::Folder | Target::File | Target::Remote) || (breadth && problem);

    /* The candidates the words allow, in the order they are decided. */
    let mut fits: Vec<Recipe> = Vec::new();

    if review && site && !asks_question {
        fits.push(Recipe::ReviewSite);
    }

    if build && page && !review && !fix {
        fits.push(Recipe::BuildPage);
    }

    if review && code_target && !site && !(fix && !problem) {
        fits.push(Recipe::ReviewCode);
    }

    if fix && (code_target || problem || target != Target::None || !asks_question) && !(review && breadth && !problem && fits.contains(&Recipe::ReviewCode)) && !fits.contains(&Recipe::BuildPage) {
        fits.push(Recipe::Fix);
    }

    if matches!(fits.as_slice(), [Recipe::ReviewCode, Recipe::Fix]) && breadth {
        /* "review the whole project and fix all errors": reading it all comes first, and the fixes are that recipe's findings. */
        fits.truncate(1);
    }

    let (recipe, confident) = match fits.as_slice() {
        [] => (Recipe::Answer, !(matches!(target, Target::Repo | Target::Url | Target::Remote) && head.chars().count() > 60 && !asks_question && (review || build || fix))),
        [only] => (*only, true),
        [first, ..] => (*first, false),
    };

    let large = match recipe {
        Recipe::Answer => false,
        Recipe::ReviewCode => {
            let whole = !matches!(target, Target::File) && (breadth || matches!(target, Target::Repo | Target::Remote));

            whole && (if repo.known() { repo.exceeds_one_pack() } else { breadth })
        }
        Recipe::Fix => breadth && !matches!(target, Target::File) && matches!(target, Target::Repo | Target::Folder | Target::Remote) && (!repo.known() || repo.exceeds_one_pack()),
        Recipe::ReviewSite => breadth,
        Recipe::BuildPage => breadth && any(&lower, &["site", "website", "app", "pages", "ওয়েবসাইট", "সাইট", "সব", "পেজগুলো"]),
    };

    Scope { recipe, large, target, confident }
}

/// The fixed question for the tiny model call: the labels, and nothing else to say.
pub fn label_prompt(request: &str) -> String {
    let labels = Recipe::ALL.iter().map(|recipe| recipe.id()).collect::<Vec<_>>().join(", ");
    let request: String = request.chars().take(600).collect();

    format!("Which kind of job is this request? Answer with exactly one of: {labels}.\n\nRequest: {request}\n\nLabel:")
}

/// The label a model's answer names - exactly one of the fixed ones, else nothing (no free text is ever used).
pub fn label_of(answer: &str) -> Option<Recipe> {
    let upper = answer.to_uppercase();
    let named: Vec<Recipe> = Recipe::ALL.iter().copied().filter(|recipe| upper.contains(recipe.id())).collect();

    match named.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// `classify`, and when the words do not decide, one tiny model call (`ask`) with the fixed label list. A reply that is not exactly one label
/// changes nothing.
pub fn classify_with(request: &str, repo: &RepoStats, ask: impl FnOnce(&str) -> Option<String>) -> Scope {
    let first = classify(request, repo);

    if first.confident {
        return first;
    }

    match ask(&label_prompt(request)).as_deref().and_then(label_of) {
        Some(recipe) if recipe != first.recipe => {
            let mut scope = Scope { recipe, ..first };

            /* the size follows the label: the same words, read as the labelled recipe */
            scope.large = match recipe {
                Recipe::Answer => false,
                _ => first.large || any(&request.to_lowercase(), BREADTH),
            };
            scope.confident = true;

            scope
        }
        _ => Scope { confident: true, ..first },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIG: RepoStats = RepoStats { files: 900, lines: 250_000 };
    const SMALL: RepoStats = RepoStats { files: 6, lines: 300 };

    /// The labelled set: (request, repo, recipe, large). Written with the card, in English, Bengali and Banglish; the labels are the author's own.
    fn cases() -> Vec<(&'static str, RepoStats, Recipe, bool)> {
        vec![
            ("review the whole project and report all errors", BIG, Recipe::ReviewCode, true),
            ("পুরো SDC প্রজেক্ট দেখো, error রিপোর্ট করো", BIG, Recipe::ReviewCode, true),
            ("puro project review koro, bug khujo", BIG, Recipe::ReviewCode, true),
            ("sompurno codebase audit koro", BIG, Recipe::ReviewCode, true),
            ("review src/auth.rs for bugs", BIG, Recipe::ReviewCode, false),
            ("check the whole project for errors", SMALL, Recipe::ReviewCode, false),
            ("fix the failing test in billing.py", BIG, Recipe::Fix, false),
            ("billing.py এর bug ঠিক করো", BIG, Recipe::Fix, false),
            ("login bug ta thik koro", SMALL, Recipe::Fix, false),
            ("fix all errors in the project", BIG, Recipe::Fix, true),
            ("সব error ঠিক করো এই প্রজেক্টে", BIG, Recipe::Fix, true),
            ("review https://example.com for problems", SMALL, Recipe::ReviewSite, false),
            ("audit the whole site at https://shop.example.com, every page", SMALL, Recipe::ReviewSite, true),
            ("ওয়েবসাইট https://example.com রিভিউ করো", SMALL, Recipe::ReviewSite, false),
            ("build a landing page for my bakery", SMALL, Recipe::BuildPage, false),
            ("একটা ল্যান্ডিং পেজ বানাও", SMALL, Recipe::BuildPage, false),
            ("build the complete website with all pages", SMALL, Recipe::BuildPage, true),
            ("what does this function do?", BIG, Recipe::Answer, false),
            ("hello", BIG, Recipe::Answer, false),
            ("why is the build slow?", BIG, Recipe::Answer, false),
            ("explain the router in simple words", BIG, Recipe::Answer, false),
            ("check the build", BIG, Recipe::Answer, false),
            ("ধন্যবাদ", SMALL, Recipe::Answer, false),
            ("rust er ownership ki?", SMALL, Recipe::Answer, false),
        ]
    }

    #[test]
    fn the_labelled_set_reaches_ninety_percent() {
        let set = cases();
        let mut wrong: Vec<String> = Vec::new();

        for (request, repo, recipe, large) in &set {
            let got = classify(request, repo);

            if got.recipe != *recipe || got.large != *large {
                wrong.push(format!("{request:?}: wanted {} large={large}, got {} large={}", recipe.id(), got.recipe.id(), got.large));
            }
        }

        let right = set.len() - wrong.len();

        assert!(set.len() >= 20, "the card asks for 20 cases");
        assert!(right * 100 >= set.len() * 90, "{right}/{} right; wrong: {wrong:#?}", set.len());
    }

    #[test]
    fn a_short_prompt_for_a_whole_project_is_a_large_review() {
        let scope = classify("পুরো SDC প্রজেক্ট দেখো, error রিপোর্ট করো", &BIG);

        assert_eq!((scope.recipe, scope.large, scope.target), (Recipe::ReviewCode, true, Target::Repo));
    }

    #[test]
    fn size_follows_the_repository_not_only_the_words() {
        assert!(classify("review the whole project", &BIG).large);
        assert!(!classify("review the whole project", &SMALL).large, "six files fit in one Context Pack");
    }

    #[test]
    fn what_the_request_points_at_is_told_apart() {
        assert_eq!(target_of("see https://example.com"), Target::Url);
        assert_eq!(target_of("look at /var/www/app on the vps"), Target::Remote);
        assert_eq!(target_of("open src/main.rs"), Target::File);
        assert_eq!(target_of("the whole project"), Target::Repo);
        assert_eq!(target_of("the docs folder"), Target::Folder);
        assert_eq!(target_of("hello"), Target::None);
    }

    #[test]
    fn only_an_unclear_request_asks_the_model_and_only_a_label_counts() {
        let mut asked = 0;
        let clear = classify_with("fix the typo in a.py", &SMALL, |_| {
            asked += 1;

            None
        });

        assert_eq!((clear.recipe, asked), (Recipe::Fix, 0), "a clear request never reaches the model");

        let vague = "make this project better, I want it ready for my customers soon";
        let unsure = classify(vague, &BIG);

        assert!(!unsure.confident, "no recipe fits the words: {unsure:?}");

        let labelled = classify_with(vague, &BIG, |prompt| {
            assert!(prompt.contains("R-REVIEW-CODE") && prompt.contains("R-ANSWER") && prompt.ends_with("Label:"), "{prompt}");

            Some("R-FIX".to_string())
        });

        assert_eq!(labelled.recipe, Recipe::Fix);

        let rambling = classify_with(vague, &BIG, |_| Some("I think it is probably a review, or maybe a fix".to_string()));

        assert_eq!(rambling.recipe, unsure.recipe, "free text changes nothing");
        assert_eq!(label_of("R-REVIEW-CODE and R-FIX"), None, "two labels are no label");
    }

    #[test]
    fn repo_stats_count_the_projects_own_files() {
        let root = std::env::temp_dir().join(format!("sdc-scope-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("src/a.rs"), "x".repeat(380)).unwrap();
        std::fs::write(root.join("src/b.py"), "x".repeat(380)).unwrap();
        std::fs::write(root.join("node_modules/x/c.js"), "x".repeat(4000)).unwrap();
        std::fs::write(root.join("logo.png"), "x".repeat(4000)).unwrap();

        let stats = RepoStats::of(&root);

        assert_eq!((stats.files, stats.lines), (2, 20), "node_modules and images are not the project");

        let _ = std::fs::remove_dir_all(&root);
    }
}
