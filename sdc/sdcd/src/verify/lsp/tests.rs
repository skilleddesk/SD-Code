use super::*;

fn mock(crash_after: Option<u32>) -> (String, Vec<String>) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lsp/mock_server.py");

    /* the crash setting travels in the argument list: the child inherits this process's environment, which tests share */
    let mut args = vec!["-I".to_string(), script.to_string_lossy().to_string()];

    if let Some(n) = crash_after {
        args.push(format!("--crash-after={n}"));
    }

    ("python3".to_string(), args)
}

fn project(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("sdc-lsp-{name}-{}", std::process::id()));

    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root.canonicalize().unwrap()
}

#[test]
fn a_server_is_started_asked_and_its_diagnostics_are_read_with_lines() {
    let root = project("basic");
    let (program, args) = mock(None);
    let mut server = Server::start(&program, &args, &root).expect("the mock server starts");
    let found = server.diagnostics(Lang::Python, &root.join("a.py"), "x = 1\ny = BAD + 1\n# TODO later\n", Duration::from_secs(10)).unwrap();

    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!((found[0].line, found[0].column, found[0].severity), (2, 5, "error"));
    assert!(found[0].message.contains("BAD"));
    assert_eq!((found[1].line, found[1].severity), (3, "warning"));

    /* the same document again: a change, and the clean version reports nothing */
    assert!(server.diagnostics(Lang::Python, &root.join("a.py"), "x = 1\n", Duration::from_secs(10)).unwrap().is_empty());
    assert!(server.alive());

    server.shutdown();

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_server_that_dies_is_started_again_and_the_answer_still_comes() {
    let root = project("crash");
    let command = mock(Some(1));

    stop_all();

    /* the mock exits after its first document: the second call finds it dead and starts another */
    let first = check(&root, "a.py", "BAD\n", Some(command.clone())).unwrap().unwrap();
    let second = check(&root, "a.py", "BAD again\n", Some(command)).unwrap().unwrap();

    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1, "the restarted server answered");

    stop_all();

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn no_server_for_a_language_is_an_ordinary_answer_not_an_error() {
    let root = project("none");

    assert_eq!(check(&root, "notes.txt", "x", None), Ok(None), "not a language");
    assert!(check(&root, "a.py", "x", Some(("definitely-not-installed-lsp".into(), vec![]))).is_err(), "a command that cannot start is reported");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_repair_note_carries_the_declaration_of_the_names_the_error_mentions() {
    let root = project("repair");

    std::fs::write(root.join("pay.py"), "def charge(amount, currency):\n    return amount\n").unwrap();

    let diagnostics = vec![
        Diagnostic { line: 4, column: 5, severity: "error", message: "Argument missing for parameter `currency` in call to `charge`".into(), source: "pyright".into() },
        Diagnostic { line: 9, column: 1, severity: "warning", message: "unused".into(), source: "pyright".into() },
    ];
    let note = repair_note(&root, "app.py", &diagnostics);

    assert!(note.contains("1 error(s) in app.py") && note.contains("line 4:5"), "{note}");
    assert!(note.contains("`charge` is declared at pay.py:1: def charge(amount, currency):"), "{note}");
    assert!(note.contains("1 warning(s) not shown"));
    assert_eq!(repair_note(&root, "app.py", &diagnostics[1..]), "", "only warnings: nothing to say");
    assert_eq!(quoted_names("cannot find name `Foo.bar` and 'baz'"), vec!["bar".to_string(), "baz".to_string()]);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn diagnostics_json_is_read_with_its_severities() {
    let list = json!([{ "range": { "start": { "line": 0, "character": 2 } }, "severity": 2, "message": "first line\nsecond", "source": "x" }, { "range": { "start": { "line": 3, "character": 0 } }, "message": "no severity means error" }]);
    let read = parse_diagnostics(&list);

    assert_eq!((read[0].line, read[0].column, read[0].severity, read[0].message.as_str()), (1, 3, "warning", "first line"));
    assert_eq!(read[1].severity, "error");
}
