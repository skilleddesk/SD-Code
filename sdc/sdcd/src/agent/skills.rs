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
}
