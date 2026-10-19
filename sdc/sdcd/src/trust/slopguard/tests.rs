use super::*;

fn names(line: &str) -> Vec<String> {
    packages(line).into_iter().map(|package| format!("{:?}:{}", package.ecosystem, package.name)).collect()
}

#[test]
fn the_install_commands_of_the_three_ecosystems_are_read() {
    assert_eq!(names("npm install left-pad express@4.18.0 --save-dev"), ["Npm:left-pad", "Npm:express"]);
    assert_eq!(names("pnpm add @scope/pkg@1.2.3"), ["Npm:@scope/pkg"]);
    assert_eq!(names("pip install requests==2.31 'flask[async]>=2' -U"), ["PyPi:requests", "PyPi:flask"]);
    assert_eq!(names("python3 -m pip install numpy"), ["PyPi:numpy"]);
    assert_eq!(names("cargo add serde --features derive"), ["Crates:serde"]);
    assert_eq!(names("cd app && npm i react && npm run build"), ["Npm:react"]);
}

#[test]
fn commands_that_install_nothing_by_name_name_nothing() {
    for line in ["npm install", "npm run build", "pip install -r requirements.txt", "pip install ./local", "pip install git+https://github.com/a/b", "pip install pkg.whl", "cargo build", "ls -la", "pip install -e .", "npm install ../sibling"] {
        assert!(names(line).is_empty(), "{line}: {:?}", names(line));
    }
}

#[test]
fn a_package_the_registry_does_not_know_refuses_the_command_and_a_silent_registry_does_not() {
    let verdict = check_with("npm install express totally-made-up-pkg", |package| if package.name == "express" { Exists::Yes } else { Exists::No });

    assert_eq!(verdict.missing.len(), 1);
    assert!(verdict.refusal().unwrap().contains("`totally-made-up-pkg` (npm)"));
    assert!(verdict.note().is_none());

    let offline = check_with("pip install requests", |_| Exists::Unknown);

    assert!(offline.refusal().is_none(), "no answer is not evidence the package is fake");
    assert!(offline.note().unwrap().contains("`requests`"));

    assert_eq!(check_with("cargo add serde", |_| Exists::Yes), Verdict::default());
}
