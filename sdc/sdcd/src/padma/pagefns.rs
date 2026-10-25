//! The code steps of R-BUILD-PAGE: the blueprint becomes a plan of files, every file is written
//! from a fresh request, checked, and written only into the page folder (P3-4).

use super::codefns::{Data, Env};
use std::path::{Component, Path};

/// Where a built page goes. Nothing outside it is ever written.
pub const FOLDER: &str = "landing-page";
const MAX_FILES: usize = 8;
const FALLBACK: [(&str, &str); 2] = [("index.html", "the page"), ("styles.css", "the look of the page")];

fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 80
        && !path.contains('\\')
        && !path.contains(':')
        && Path::new(path).components().all(|part| matches!(part, Component::Normal(_)))
        && path.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/'))
        && Path::new(path).extension().is_some_and(|extension| matches!(extension.to_string_lossy().to_lowercase().as_str(), "html" | "css" | "js" | "svg" | "json" | "txt"))
}

/// `FILE path | purpose` lines of the blueprint, as (path, purpose).
fn blueprint_files(data: &Data) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = Vec::new();

    for line in data.get("blueprint").into_iter().flatten().flat_map(|text| text.lines()) {
        let line = line.trim().trim_start_matches(['-', '*', ' ']);
        let Some(rest) = line.strip_prefix("FILE").or_else(|| line.strip_prefix("File")) else {
            continue;
        };
        let (path, purpose) = rest.trim_start_matches([':', ' ']).split_once('|').map(|(path, purpose)| (path.trim(), purpose.trim())).unwrap_or((rest.trim(), ""));
        let path = path.trim_matches(['`', '"', '\'']).trim_start_matches("./");

        if safe_path(path) && !files.iter().any(|(known, _)| known == path) && files.len() < MAX_FILES {
            files.push((path.to_string(), purpose.to_string()));
        }
    }

    files
}

fn is_html(path: &str) -> bool {
    path.to_lowercase().ends_with(".html")
}

/// The files to write: the blueprint's, or a small safe default when the blueprint has no usable plan (a page needs at least one html file).
pub fn plan_files(_env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let mut files: Vec<String> = blueprint_files(data).into_iter().map(|(path, _)| path).collect();

    if !files.iter().any(|path| is_html(path)) {
        files = FALLBACK.iter().map(|(path, _)| path.to_string()).collect();
    }

    Ok(files)
}

/// What was chosen for the person: the blueprint's `DEFAULT:` lines, and a note when the plan fell back.
pub fn defaults_of(_env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let mut lines: Vec<String> = data
        .get("blueprint")
        .into_iter()
        .flatten()
        .flat_map(|text| text.lines())
        .map(str::trim)
        .filter_map(|line| line.trim_start_matches(['-', '*', ' ']).strip_prefix("DEFAULT:").map(|rest| rest.trim().to_string()))
        .filter(|line| !line.is_empty())
        .take(8)
        .collect();

    if !blueprint_files(data).iter().any(|(path, _)| is_html(path)) {
        lines.push("the model gave no usable file plan, so the page is index.html + styles.css".to_string());
    }

    Ok(lines)
}

/// The code of an answer: the inside of the first fenced block when there is one.
pub fn strip_fence(answer: &str) -> String {
    let text = answer.trim();

    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        let body = after.split_once('\n').map(|(_, body)| body).unwrap_or(after);

        return body.split("```").next().unwrap_or(body).trim_end().to_string();
    }

    text.to_string()
}

fn balanced(text: &str, open: char, close: char) -> bool {
    text.chars().filter(|c| *c == open).count() == text.chars().filter(|c| *c == close).count()
}

fn img_tags_have_alt(html: &str) -> Result<(), String> {
    let lower = html.to_lowercase();
    let mut from = 0;

    while let Some(at) = lower[from..].find("<img") {
        let start = from + at;
        let end = lower[start..].find('>').map(|end| start + end).unwrap_or(lower.len());

        if !lower[start..end].contains("alt=") {
            return Err("an <img> has no alt text".to_string());
        }

        from = end;
    }

    Ok(())
}

/// The check of one written file (the unit is its path; the answer is the `output` input).
pub fn valid_file(_env: &Env, unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let path = unit.ok_or("valid_file works on one file at a time")?;
    let answer = data.get("output").and_then(|lines| lines.first()).map(String::as_str).unwrap_or("");
    let code = strip_fence(answer);
    let lower = code.to_lowercase();

    if code.trim().is_empty() {
        return Err("the file is empty".to_string());
    }

    match Path::new(path).extension().map(|extension| extension.to_string_lossy().to_lowercase()).as_deref() {
        Some("html") => {
            for need in ["<html", "</html>", "<body", "</body>", "<title"] {
                if !lower.contains(need) {
                    return Err(format!("the html has no {need}"));
                }
            }

            img_tags_have_alt(&code)?;
        }
        Some("css") if !code.contains('{') || !balanced(&code, '{', '}') => return Err("the css has no rules or its braces do not match".to_string()),
        Some("js") if !balanced(&code, '{', '}') || !balanced(&code, '(', ')') => return Err("the script has unmatched braces or parentheses".to_string()),
        _ => {}
    }

    Ok(Vec::new())
}

/// Writes the answers into the page folder and returns the files written, relative to it.
pub fn write_files(env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let pages = data.get("pages").cloned().unwrap_or_default();
    let paths = data.get("_pages_units").cloned().unwrap_or_default();

    if pages.is_empty() || pages.len() != paths.len() {
        return Err("no page file was produced".to_string());
    }

    let folder = env.root.join(FOLDER);

    for path in &paths {
        if !safe_path(path) {
            return Err(format!("{path} is not a safe place to write"));
        }
    }

    /* P4-1: an existing page file is noted as an edit; the writes run under the hidden snapshot; what was made is recorded. */
    let artifacts: Vec<String> = paths.iter().map(|path| format!("{FOLDER}/{path}")).collect();

    super::engines::before_create(env, &artifacts);

    let mut write_all = || -> Result<Vec<String>, String> {
        let mut written = Vec::new();

        for (path, answer) in paths.iter().zip(&pages) {
            let target = folder.join(path);

            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| format!("could not create {}: {error}", parent.display()))?;
            }

            std::fs::write(&target, strip_fence(answer)).map_err(|error| format!("could not write {path}: {error}"))?;
            written.push(path.clone());
        }

        Ok(written)
    };
    let written = super::engines::shadowed(env, data, &mut write_all)?;

    super::engines::register(env, data, &written.iter().map(|path| format!("{FOLDER}/{path}")).collect::<Vec<_>>(), "page file written");

    Ok(written)
}

/// The written page, checked again from the disk: files exist, local css/js/image references exist, images have alt text.
pub fn verify_page(env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let written = data.get("written").cloned().unwrap_or_default();
    let folder = env.root.join(FOLDER);
    let mut problems = Vec::new();

    for path in &written {
        let Ok(text) = std::fs::read_to_string(folder.join(path)) else {
            problems.push(format!("{path} is missing"));

            continue;
        };

        if !is_html(path) {
            continue;
        }

        if let Err(reason) = img_tags_have_alt(&text) {
            problems.push(format!("{path}: {reason}"));
        }

        for reference in local_references(&text) {
            if !folder.join(&reference).exists() {
                problems.push(format!("{path} points to {reference}, which was not written"));
            }
        }
    }

    if !written.iter().any(|path| is_html(path)) {
        problems.push("there is no html file".to_string());
    }

    if problems.is_empty() { Ok(vec![format!("{} files checked", written.len())]) } else { Err(problems.join("; ")) }
}

fn local_references(html: &str) -> Vec<String> {
    let mut found = Vec::new();

    for attribute in ["href=\"", "src=\""] {
        let mut rest = html;

        while let Some(at) = rest.find(attribute) {
            rest = &rest[at + attribute.len()..];

            let value = rest.split('"').next().unwrap_or("");

            if !value.is_empty() && !value.starts_with(['#', '/']) && !value.contains(':') && !value.starts_with("mailto") {
                let value = value.split(['?', '#']).next().unwrap_or(value).trim_start_matches("./");

                if !found.contains(&value.to_string()) && Path::new(value).extension().is_some() {
                    found.push(value.to_string());
                }
            }
        }
    }

    found
}

/// The short report: where the page is, what each file is, which defaults were chosen, what was not done.
pub fn page_report(_env: &Env, _unit: Option<&str>, data: &Data) -> Result<Vec<String>, String> {
    let mut lines = vec![format!("# Page built in `{FOLDER}/`"), String::new()];

    for path in data.get("written").into_iter().flatten() {
        lines.push(format!("- `{FOLDER}/{path}`"));
    }

    let defaults: Vec<&String> = data.get("defaults").into_iter().flatten().collect();

    if !defaults.is_empty() {
        lines.push(String::new());
        lines.push("## Defaults I chose (change any of them)".to_string());
        lines.extend(defaults.iter().map(|line| format!("- {line}")));
    }

    let missing: Vec<&String> = data.get("_unreviewed").into_iter().flatten().collect();

    if !missing.is_empty() {
        lines.push(String::new());
        lines.push("## Not written".to_string());
        lines.extend(missing.iter().map(|line| format!("- {line}")));
    }

    lines.push(String::new());
    lines.push(format!("Open `{FOLDER}/index.html` in a browser to see it."));

    Ok(vec![lines.join("\n")])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(name: &str, lines: &[&str]) -> Data {
        let mut data = Data::new();

        data.insert(name.to_string(), lines.iter().map(|line| line.to_string()).collect());

        data
    }

    #[test]
    fn the_blueprint_becomes_safe_files_and_a_bad_plan_falls_back() {
        let env = Env::new(std::env::temp_dir());
        let plan = data("blueprint", &["FILE index.html | the page\n- FILE css/site.css | look\nFILE ../evil.html | no\nFILE C:/x.html | no\nDEFAULT: warm colours"]);

        assert_eq!(plan_files(&env, None, &plan).unwrap(), vec!["index.html", "css/site.css"]);
        assert_eq!(defaults_of(&env, None, &plan).unwrap(), vec!["warm colours"]);

        let bad = data("blueprint", &["a page about bread"]);

        assert_eq!(plan_files(&env, None, &bad).unwrap(), vec!["index.html", "styles.css"]);
        assert!(defaults_of(&env, None, &bad).unwrap().iter().any(|line| line.contains("no usable file plan")));
    }

    #[test]
    fn a_file_is_checked_by_its_kind() {
        let env = Env::new(std::env::temp_dir());
        let html = |body: &str| data("output", &[body]);

        assert!(valid_file(&env, Some("index.html"), &html("```html\n<html><head><title>x</title></head><body><img src=\"a.png\" alt=\"a\"></body></html>\n```")).is_ok());
        assert!(valid_file(&env, Some("index.html"), &html("<html><body><img src=\"a.png\"></body></html>")).is_err());
        assert!(valid_file(&env, Some("index.html"), &html("")).is_err());
        assert!(valid_file(&env, Some("styles.css"), &html("body { color: red; ")).is_err());
        assert!(valid_file(&env, Some("styles.css"), &html("body { color: red; }")).is_ok());
        assert!(valid_file(&env, Some("app.js"), &html("function a( { }")).is_err());
    }

    #[test]
    fn files_are_written_only_inside_the_page_folder() {
        let root = std::env::temp_dir().join(format!("sdc-page-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        std::fs::create_dir_all(&root).unwrap();

        let env = Env::new(root.clone());
        let mut input = data("pages", &["<html></html>", "```css\nbody { margin: 0; }\n```"]);

        input.insert("_pages_units".to_string(), vec!["index.html".to_string(), "styles.css".to_string()]);

        assert_eq!(write_files(&env, None, &input).unwrap(), vec!["index.html", "styles.css"]);
        assert_eq!(std::fs::read_to_string(root.join(FOLDER).join("styles.css")).unwrap(), "body { margin: 0; }");

        input.insert("_pages_units".to_string(), vec!["index.html".to_string(), "../out.css".to_string()]);

        assert!(write_files(&env, None, &input).is_err());
        assert!(!root.join("out.css").exists());

        let _ = std::fs::remove_dir_all(&root);
    }
}
