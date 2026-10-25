//! `sdcd navaratna ...` - the parts of NAVARATNA that work on a file, usable by hand and by scripts.
//!
//! * `spec FILE` - cut a prompt into units, label them (deterministic labeller), run the coverage gate, print the map
//! * `skeleton FILE` - imports and signatures with line numbers
//! * `slice LOG [--root DIR]` - the first error of a build/test log, with the code around it
//! * `status` - the `nr.*` switches and whether each is on

use std::path::Path;

pub fn main(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("spec") => with_file(args, |_, text| {
            let map = super::spec::analyse(text);

            for unit in &map.units {
                let label = match map.labels.get(&unit.id) {
                    Some(super::spec::Label::Requirement) => format!("requirement {}", map.requirement_of(&unit.id).unwrap_or("?")),
                    Some(super::spec::Label::PartOf(owner)) => format!("part of {}", map.requirement_of(owner).unwrap_or("?")),
                    Some(super::spec::Label::Background) => "background".to_string(),
                    None => "UNMAPPED".to_string(),
                };

                println!("{} [{:?}] lines {}-{}: {label}", unit.id, unit.kind, unit.line_from, unit.line_to);
            }

            match super::spec::coverage_gate(&map) {
                Ok(()) => {
                    println!("coverage: {} units, {} requirements, all mapped", map.units.len(), map.requirements.len());
                    0
                }
                Err(blocked) => {
                    println!("{blocked}");
                    1
                }
            }
        }),
        Some("skeleton") => with_file(args, |path, text| match super::symbols::skeleton(path, text) {
            Some(skeleton) => {
                print!("{skeleton}");
                0
            }
            None => {
                eprintln!("no grammar for {path}: use search/replace");
                1
            }
        }),
        Some("slice") => {
            let Some(log) = args.get(1).and_then(|path| std::fs::read_to_string(path).ok()) else {
                eprintln!("usage: sdcd navaratna slice LOGFILE [--root DIR]");
                return 3;
            };
            let root = args.iter().position(|arg| arg == "--root").and_then(|at| args.get(at + 1)).map_or(std::path::PathBuf::from("."), std::path::PathBuf::from);

            match super::errslice::slice(Path::new(&root), &log, None, 800) {
                Some(slice) => {
                    println!("{}", slice.text);
                    0
                }
                None => {
                    eprintln!("no error this reader knows (rustc, tsc, eslint, pytest, jest, go) in that log");
                    1
                }
            }
        }
        Some("status") => {
            for feature in crate::v3::FEATURES.iter().filter(|feature| feature.name.starts_with("nr.")) {
                println!("{:<24} {}", feature.name, if crate::v3::on(feature.name) { "on" } else { "off" });
            }

            0
        }
        _ => {
            eprintln!("usage: sdcd navaratna spec FILE | skeleton FILE | slice LOG [--root DIR] | status   (and: sdcd bench navaratna, sdcd probe window --model M)");
            3
        }
    }
}

fn with_file(args: &[String], body: impl FnOnce(&str, &str) -> i32) -> i32 {
    let Some(path) = args.get(1) else {
        eprintln!("a file is needed");
        return 3;
    };

    match std::fs::read_to_string(path) {
        Ok(text) => body(path, &text),
        Err(error) => {
            eprintln!("{path}: {error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subcommands_run_on_files_and_a_missing_file_or_command_is_an_error_code() {
        let dir = tempfile::tempdir().unwrap();
        let prompt = dir.path().join("p.md");
        let source = dir.path().join("a.rs");

        std::fs::write(&prompt, "# Shop\n\nAdd an endpoint /items. It must list items.\n").unwrap();
        std::fs::write(&source, "pub fn a() {}\n").unwrap();

        assert_eq!(main(&["spec".into(), prompt.to_string_lossy().into()]), 0);
        assert_eq!(main(&["skeleton".into(), source.to_string_lossy().into()]), 0);
        assert_eq!(main(&["skeleton".into(), prompt.to_string_lossy().into()]), 1);
        assert_eq!(main(&["spec".into(), "/no/such/file".into()]), 1);
        assert_eq!(main(&["status".into()]), 0);
        assert_eq!(main(&[]), 3);
    }
}
