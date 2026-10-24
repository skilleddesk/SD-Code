//! Golden samples: logs in the exact shape each tool prints.

use super::*;

const RUSTC: &str = "   Compiling demo v0.1.0 (/work/demo)\nwarning: unused variable: `y`\n --> src/lib.rs:3:9\n  |\n3 |     let y = 2;\n  |         ^ help: if this is intentional, prefix it with an underscore: `_y`\n\nerror[E0308]: mismatched types\n --> src/lib.rs:7:18\n  |\n7 |     let n: i32 = \"a\";\n  |            ---   ^^^ expected `i32`, found `&str`\n  |            |\n  |            expected due to this\n\nerror[E0425]: cannot find value `z` in this scope\n --> src/lib.rs:9:5\n  |\n9 |     z\n  |     ^ not found in this scope\n\nerror: could not compile `demo` (lib) due to 2 previous errors\n";
const TSC: &str = "src/app.ts(12,7): error TS2322: Type 'string' is not assignable to type 'number'.\nsrc/app.ts(20,3): error TS2304: Cannot find name 'foo'.\n\nFound 2 errors in the same file, starting at: src/app.ts:12\n";
const ESLINT: &str = "/work/web/src/util.js\n   4:7   error    'unused' is assigned a value but never used  no-unused-vars\n  10:1   warning  Unexpected console statement                 no-console\n  15:3   error    Missing semicolon                            semi\n\n✖ 3 problems (2 errors, 1 warning)\n";
const PYTEST: &str = "============================= test session starts ==============================\ncollected 2 items\n\ntests/test_calc.py F.                                                    [100%]\n\n=================================== FAILURES ===================================\n_________________________________ test_add _____________________________________\n\n    def test_add():\n>       assert add(1, 2) == 4\nE       assert 3 == 4\nE        +  where 3 = add(1, 2)\n\ntests/test_calc.py:5: AssertionError\n=========================== short test summary info ============================\nFAILED tests/test_calc.py::test_add - assert 3 == 4\n========================= 1 failed, 1 passed in 0.03s ==========================\n";
const JEST: &str = " FAIL  src/sum.test.js\n  ● sum › adds two numbers\n\n    expect(received).toBe(expected) // Object.is equality\n\n    Expected: 5\n    Received: 4\n\n      3 | test('adds two numbers', () => {\n    > 4 |   expect(sum(2, 2)).toBe(5);\n        |                     ^\n\n      at Object.<anonymous> (src/sum.test.js:4:21)\n\nTest Suites: 1 failed, 1 total\n";
const GO: &str = "# demo/pkg\n./main.go:10:5: undefined: widget\n./main.go:14:2: declared and not used: n\n";
const GOTEST: &str = "--- FAIL: TestAdd (0.00s)\n    calc_test.go:12: got 3, want 4\nFAIL\nFAIL\tdemo/calc\t0.002s\n";

#[test]
fn every_tool_is_recognised_and_gives_the_first_error_with_its_file_and_line() {
    let cases = [
        (RUSTC, Tool::Rustc, "src/lib.rs", 7usize, "mismatched types", 2usize),
        (TSC, Tool::Tsc, "src/app.ts", 12, "Type 'string' is not assignable", 2),
        (ESLINT, Tool::Eslint, "/work/web/src/util.js", 4, "'unused' is assigned", 2),
        (PYTEST, Tool::Pytest, "tests/test_calc.py", 5, "assert 3 == 4", 1),
        (JEST, Tool::Jest, "src/sum.test.js", 4, "adds two numbers", 1),
        (GO, Tool::Go, "main.go", 10, "undefined: widget", 2),
        (GOTEST, Tool::Go, "calc_test.go", 12, "got 3", 1),
    ];

    for (log, tool, file, line, message, total) in cases {
        assert_eq!(detect(log), Some(tool), "{log}");

        let found = find(tool, log).unwrap_or_else(|| panic!("{tool:?} found nothing in {log}"));

        assert_eq!((found.file.as_deref(), found.line), (Some(file), Some(line)), "{tool:?}");
        assert!(found.message.contains(message), "{tool:?}: {found:?}");
        assert_eq!(found.total, total, "{tool:?}");
    }
}

#[test]
fn test_failures_carry_expected_against_actual() {
    let pytest = find(Tool::Pytest, PYTEST).unwrap();
    let jest = find(Tool::Jest, JEST).unwrap();
    let go = find(Tool::Go, GOTEST).unwrap();

    assert_eq!((pytest.expected.as_deref(), pytest.actual.as_deref()), (Some("4"), Some("3")));
    assert_eq!((jest.expected.as_deref(), jest.actual.as_deref()), (Some("5"), Some("4")));
    assert_eq!((go.expected.as_deref(), go.actual.as_deref()), (Some("4"), Some("3")));
}

#[test]
fn a_slice_has_the_code_around_the_line_the_count_of_the_rest_and_the_vault_id() {
    let dir = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), (1..=30).map(|n| format!("// line {n}\n")).collect::<String>()).unwrap();

    let slice = slice(dir.path(), RUSTC, None, 400).unwrap();

    assert!(slice.text.contains("src/lib.rs:7"), "{}", slice.text);
    assert!(slice.text.contains(">   7 | // line 7"), "{}", slice.text);
    assert!(slice.text.contains("(+1 more error;"), "{}", slice.text);

    let id = slice.log_id.clone().unwrap();

    assert!(slice.text.contains(&id));
    assert_eq!(super::super::vault::get(dir.path(), "log", &id).unwrap(), RUSTC, "the whole log is kept");
}

#[test]
fn every_sample_stays_under_a_fixed_token_cap_and_keeps_the_location() {
    let dir = tempfile::tempdir().unwrap();

    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "x\n".repeat(200)).unwrap();

    for (log, location) in [(RUSTC, "src/lib.rs:7"), (TSC, "src/app.ts:12"), (ESLINT, "util.js:4"), (PYTEST, "tests/test_calc.py:5"), (JEST, "src/sum.test.js:4"), (GO, "main.go:10")] {
        let slice = slice(dir.path(), log, None, 120).unwrap();

        assert!(crate::tokens::estimate(&slice.text) <= 120 || slice.text.lines().count() < 12, "{}", slice.text);
        assert!(slice.text.contains(location), "{location} missing from {}", slice.text);
    }
}

#[test]
fn a_log_without_a_known_failure_gives_no_slice() {
    let dir = tempfile::tempdir().unwrap();

    assert!(slice(dir.path(), "all good\n", None, 200).is_none());
}
