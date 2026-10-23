//! Skills (0.13): a project's packaged instructions - "how we deploy", "how to write a migration here" -
//! the way Claude Code and Codex read them: one folder per skill with a `SKILL.md` whose frontmatter names
//! it and says when it applies.
//!
//! ```text
//! .sdc/skills/deploy/SKILL.md      .claude/skills/…      .codex/skills/…      .agents/skills/…
//! ---
//! name: deploy
//! description: Deploy this site to staging or production with the release script.
//! ---
//! 1. Run `npm run build` …
//! ```
//!
//! Only the name and the description go into the agent's system prompt; the agent reads the whole file
//! with `read_file` when a task matches - so twenty skills cost twenty lines, not twenty documents.

use super::workspace::Workspace;

/// Where a project keeps its skills, in the order they are read; a name found twice keeps the first.
pub const SKILL_DIRS: &[&str] = &[".sdc/skills", ".claude/skills", ".codex/skills", ".agents/skills"];

/// One skill: its name, when to use it, and the file to read.
#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: String,
}

/// The `name:` and `description:` of a SKILL.md's frontmatter; the folder's name when it has none.
pub fn parse(text: &str, folder: &str) -> (String, String) {
    let mut name = folder.to_string();
    let mut description = String::new();

    if let Some(rest) = text.trim_start().strip_prefix("---") {
        for line in rest.lines().take_while(|line| line.trim() != "---") {
            if let Some((key, value)) = line.split_once(':') {
                let value = value.trim().trim_matches(['"', '\'']).to_string();

                match key.trim() {
                    "name" if !value.is_empty() => name = value,
                    "description" => description = value,
                    _ => {}
                }
            }
        }
    }

    if description.is_empty() {
        /* No frontmatter: the first line of prose says what it is. */
        description = text
            .lines()
            .map(|line| line.trim().trim_start_matches('#').trim())
            .find(|line| !line.is_empty() && *line != "---")
            .unwrap_or_default()
            .to_string();
    }

    (name, description.chars().take(300).collect())
}

/// The project's skills, on whichever machine the folder is.
pub fn find(workspace: &Workspace) -> Vec<Skill> {
    let mut skills: Vec<Skill> = Vec::new();

    for dir in SKILL_DIRS {
        let Ok((entries, _)) = workspace.list(dir) else {
            continue;
        };

        for entry in entries.iter().filter(|entry| entry.ends_with('/')).take(40) {
            let folder = entry.trim_end_matches('/');
            let path = format!("{dir}/{folder}/SKILL.md");
            let Ok((text, _)) = workspace.read(&path) else {
                continue;
            };
            let (name, description) = parse(&text, folder);

            if !skills.iter().any(|skill| skill.name == name) {
                skills.push(Skill { name, description, path });
            }
        }
    }

    skills
}

/// The system prompt's lines for the skills, or nothing when there are none.
pub fn brief(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }

    let lines: Vec<String> = skills.iter().map(|skill| format!("- {}: {} (read {})", skill.name, skill.description, skill.path)).collect();

    format!(
        "\n\nSkills this project provides - when a task matches one, read its file with read_file first and follow it:\n{}",
        lines.join("\n")
    )
}


/// A skill the person named in capitals (`MEGHNA next`) and what became of it (M0-0, switch `v3.skills.keyword_trigger`).
#[derive(Debug, Clone, PartialEq)]
pub struct Triggered {
    pub name: String,
    pub path: String,
    /// The SKILL.md text was put in front of the model whole.
    pub included: bool,
    /// What it would cost, in tokens.
    pub tokens: u64,
}

/// Is `name` written in capitals as a word of `message`? `MEGHNA next` yes; `meghna river`, `Meghna` and `MEGHNAS` no. The match
/// is case-insensitive on the skill's name and case-sensitive on the message: a person who types the word in capitals is calling
/// the skill, and the same word in a sentence is only a word.
pub fn named_in_capitals(name: &str, message: &str) -> bool {
    let wanted = name.trim().to_uppercase();

    if wanted.is_empty() || !wanted.chars().any(char::is_alphabetic) || wanted.chars().any(|c| !(c.is_alphanumeric() || c == '_' || c == '-')) {
        return false;
    }

    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    let mut from = 0;

    while let Some(at) = message[from..].find(&wanted) {
        let start = from + at;
        let end = start + wanted.len();
        let before = message[..start].chars().next_back();
        let after = message[end..].chars().next();

        if !before.is_some_and(word) && !after.is_some_and(word) {
            return true;
        }

        from = start + wanted.chars().next().map_or(1, char::len_utf8);
    }

    false
}

/// The skills whose name is in the message in capitals.
pub fn named<'a>(skills: &'a [Skill], message: &str) -> Vec<&'a Skill> {
    skills.iter().filter(|skill| named_in_capitals(&skill.name, message)).collect()
}

/// What goes into the system prompt for the skills the person called, and what happened to each.
///
/// The whole SKILL.md goes in - never a cut one, because half a procedure is worse than none. A skill that does not fit the share of
/// the window it may take (`share_percent`) is **not** cut: the model is told how large it is and to read it in parts with
/// `read_file`, and the person is told too (`Triggered.included == false`).
pub fn keyword_context(workspace: &Workspace, skills: &[Skill], message: &str, window_tokens: u64, share_percent: u64) -> (String, Vec<Triggered>) {
    let mut text = String::new();
    let mut report = Vec::new();
    let allowed = window_tokens * share_percent / 100;
    let mut used = 0u64;

    for skill in named(skills, message) {
        let Ok((body, _)) = workspace.read(&skill.path) else {
            continue;
        };
        let tokens = crate::tokens::estimate(&body);
        let folder = skill.path.rsplit_once('/').map_or("", |(folder, _)| folder);
        let fits = used + tokens <= allowed;

        if fits {
            used += tokens;
            text.push_str(&format!(
                "\n\n[Skill `{}` - the person called it by name, so it is loaded in full]\n{}\n[End of skill `{}`. Its companion files are in the same folder ({folder}); read them with read_file when you need them.]",
                skill.name,
                body.trim_end(),
                skill.name
            ));
        } else {
            text.push_str(&format!(
                "\n\n[Skill `{}` was called by name, but it is about {tokens} tokens and does not fit this model's window beside the task. It is NOT loaded and nothing was cut: read {} with read_file, a part at a time, before you act. Its companion files are in {folder}.]",
                skill.name, skill.path
            ));
        }

        report.push(Triggered { name: skill.name.clone(), path: skill.path.clone(), included: fits, tokens });
    }

    (text, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_names_the_skill_and_says_when_it_applies() {
        let (name, description) = parse("---\nname: deploy\ndescription: \"Deploy to staging with the release script.\"\n---\n1. Build", "x");

        assert_eq!(name, "deploy");
        assert_eq!(description, "Deploy to staging with the release script.");

        let (name, description) = parse("# Write a migration\nUse knex.", "migrations");

        assert_eq!(name, "migrations");
        assert_eq!(description, "Write a migration");
    }

    #[test]
    fn skills_are_found_in_every_known_folder() {
        let root = std::env::temp_dir().join(format!("sdc-skills-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".claude/skills/deploy")).unwrap();
        std::fs::create_dir_all(root.join(".sdc/skills/seo")).unwrap();
        std::fs::write(root.join(".claude/skills/deploy/SKILL.md"), "---\nname: deploy\ndescription: Deploy the site\n---\n").unwrap();
        std::fs::write(root.join(".sdc/skills/seo/SKILL.md"), "---\nname: seo\ndescription: Check meta tags\n---\n").unwrap();

        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let skills = find(&workspace);

        assert_eq!(skills.iter().map(|skill| skill.name.as_str()).collect::<Vec<_>>(), ["seo", "deploy"]);
        assert!(brief(&skills).contains("- deploy: Deploy the site (read .claude/skills/deploy/SKILL.md)"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_skill_is_called_by_its_name_in_capitals_and_by_nothing_else() {
        assert!(named_in_capitals("meghna", "MEGHNA next"));
        assert!(named_in_capitals("meghna", "please run MEGHNA M1-4 now"));
        assert!(named_in_capitals("meghna", "(MEGHNA)"));
        assert!(!named_in_capitals("meghna", "meghna river"), "the word in a sentence is only a word");
        assert!(!named_in_capitals("meghna", "Meghna next"), "capitals mean the whole word");
        assert!(!named_in_capitals("meghna", "MEGHNAS and MEGHNA_X and XMEGHNA"), "part of a longer word is not the name");
        assert!(named_in_capitals("deploy-site", "DEPLOY-SITE to staging"));
        assert!(!named_in_capitals("", "ANYTHING"));
        assert!(!named_in_capitals("123", "123"), "a name needs a letter");
    }

    #[test]
    fn a_called_skill_is_loaded_whole_and_one_that_does_not_fit_is_said_so_not_cut() {
        let root = std::env::temp_dir().join(format!("sdc-skill-keyword-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".sdc/skills/meghna")).unwrap();
        std::fs::create_dir_all(root.join(".sdc/skills/big")).unwrap();
        std::fs::write(root.join(".sdc/skills/meghna/SKILL.md"), "---\nname: meghna\ndescription: The plan.\n---\nStep one.\nStep two.\n").unwrap();
        std::fs::write(root.join(".sdc/skills/big/SKILL.md"), format!("---\nname: big\ndescription: Huge.\n---\n{}", "word ".repeat(20_000))).unwrap();

        let workspace = Workspace::new(root.to_str().unwrap(), None);
        let skills = find(&workspace);

        let (text, report) = keyword_context(&workspace, &skills, "MEGHNA next", 8_192, 40);

        assert!(text.contains("Step one.\nStep two.") && text.contains("companion files are in the same folder"), "{text}");
        assert_eq!(report.len(), 1);
        assert!(report[0].included && report[0].name == "meghna");

        /* Nothing called: nothing added. */
        assert!(keyword_context(&workspace, &skills, "meghna next", 8_192, 40).0.is_empty());

        /* A skill larger than its share is reported, not cut. */
        let (text, report) = keyword_context(&workspace, &skills, "run BIG please", 8_192, 40);

        assert!(!report[0].included && report[0].tokens > 3_000);
        assert!(text.contains("is NOT loaded and nothing was cut") && !text.contains("word word word"), "{text}");

        /* No skills at all: nothing happens. */
        assert!(keyword_context(&workspace, &[], "MEGHNA next", 8_192, 40).1.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }
}
