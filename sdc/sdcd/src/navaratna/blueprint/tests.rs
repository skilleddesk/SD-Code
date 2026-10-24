use super::*;

fn prompt() -> String {
    let mut text = String::from("# Shop\n\nBuild a shop API. It must list items. It must compute a total with 3.5 percent tax.\n\n");

    for n in 0..30 {
        text.push_str(&format!("Also add the report number {n}. It must be exported as CSV.\n\n"));
    }

    text
}

fn allowed() -> Context {
    Context { relay_consent: true, local_only: false, manual_pin: false, architect_row: true }
}

fn good_answer(units: &[spec::Unit]) -> String {
    let first = &units[1..];
    let ids: Vec<String> = first.iter().map(|u| format!("\"{}\"", u.id)).collect();

    format!(
        "Here is the blueprint:\n{{\"folders\": [\"src/\", \"tests/\"], \"models\": [{{\"name\": \"Item\", \"fields\": [\"price: f64\"]}}], \"contracts\": [{{\"file\": \"src/shop.rs\", \"signature\": \"pub fn total(items: &[Item]) -> f64\", \"requirements\": [\"R-001\"]}}, {{\"file\": \"src/report.py\", \"signature\": \"def export(rows):\", \"requirements\": [\"R-001\"]}}], \"requirements\": [{{\"id\": \"R-001\", \"units\": [{}]}}], \"background\": [\"{}\"]}}",
        ids.join(", "),
        units[0].id
    )
}

fn local() -> (String, Map) {
    let prompt = prompt();
    let map = spec::analyse(&prompt);

    (prompt, map)
}

#[test]
fn with_consent_off_or_local_only_or_a_manual_pin_there_are_zero_cloud_calls() {
    let (prompt, map) = local();

    crate::v3::forcing(&["nr.blueprint"], || {
        for context in [Context { relay_consent: false, ..allowed() }, Context { local_only: true, ..allowed() }, Context { manual_pin: true, ..allowed() }, Context { architect_row: false, ..allowed() }] {
            let mut calls = 0;
            let outcome = run(&context, &prompt, &map, &|t| t.to_string(), &mut |_| {
                calls += 1;
                Ok(String::new())
            });

            assert_eq!(calls, 0, "{context:?}");
            assert!(matches!(outcome, Outcome::Local(_)));
        }
    });

    let mut calls = 0;

    crate::v3::ablating(&["nr.blueprint"], || {
        run(&allowed(), &prompt, &map, &|t| t.to_string(), &mut |_| {
            calls += 1;
            Ok(String::new())
        });
    });

    assert_eq!(calls, 0, "the switch is off by default");
}

#[test]
fn a_small_prompt_never_asks() {
    let map = spec::analyse("Add a button. It must be blue.");

    crate::v3::forcing(&["nr.blueprint"], || assert_eq!(may_ask(&allowed(), "Add a button. It must be blue.", &map), Err(Skip::TooSmall)));
}

#[test]
fn one_call_returns_a_validated_blueprint() {
    let (prompt, map) = local();
    let answer = good_answer(&map.units);
    let mut asked = Vec::new();

    crate::v3::forcing(&["nr.blueprint"], || {
        let outcome = run(&allowed(), &prompt, &map, &|t| t.replace("3.5", "X.X"), &mut |request| {
            asked.push(request.to_string());
            Ok(answer.clone())
        });

        let Outcome::Blueprint(blueprint, new_map) = outcome else { panic!("{outcome:?}") };

        assert_eq!(blueprint.contracts.len(), 2);
        assert!(new_map.unmapped().is_empty());
        assert_eq!(new_map.requirements.len(), 1);
    });

    assert_eq!(asked.len(), 1, "exactly one cloud call");
    assert!(asked[0].contains("S-001") && asked[0].contains("EMPTY signatures"));
    assert!(!asked[0].contains("3.5"), "the redactor ran on every unit before it left");
}

#[test]
fn a_blueprint_that_leaves_a_unit_out_fails_the_coverage_gate_and_falls_back() {
    let (prompt, map) = local();
    let answer = good_answer(&map.units).replace(&format!(", \"{}\"", map.units[5].id), "");

    crate::v3::forcing(&["nr.blueprint"], || {
        let Outcome::Local(reason) = run(&allowed(), &prompt, &map, &|t| t.to_string(), &mut |_| Ok(answer.clone())) else { panic!("should fall back") };

        assert!(reason.contains("not mapped") && reason.contains(&map.units[5].id), "{reason}");
    });
}

#[test]
fn a_contract_with_an_implementation_body_or_a_syntax_error_is_refused() {
    let (_, map) = local();
    let ok = parse_blueprint(&good_answer(&map.units)).unwrap();

    assert!(validate(&map.units, &ok).is_ok());

    for bad in ["pub fn total(items: &[Item]) -> f64 { items.iter().map(|i| i.price).sum() }", "def export(rows):\n    return rows", "pub fn total(items: &[Item -> f64"] {
        let mut blueprint = ok.clone();

        blueprint.contracts[0] = Contract { file: if bad.starts_with("def") { "src/report.py".into() } else { "src/shop.rs".into() }, signature: bad.into(), requirements: vec!["R-001".into()] };

        assert!(validate(&map.units, &blueprint).is_err(), "{bad}");
    }

    assert!(is_empty_signature("pub fn total() -> f64") && is_empty_signature("def f(x):\n    ...") && is_empty_signature("function f(a: number): number"));
}

#[test]
fn unsafe_paths_duplicate_ids_and_made_up_units_are_refused() {
    let (_, map) = local();
    let mut blueprint = parse_blueprint(&good_answer(&map.units)).unwrap();

    blueprint.folders.push("../../etc".into());
    blueprint.requirements.push(Requirement { id: "R-001".into(), units: vec!["S-999".into()] });

    let problems = validate(&map.units, &blueprint).unwrap_err().join(" | ");

    assert!(problems.contains("unsafe folder"), "{problems}");
    assert!(problems.contains("used twice"), "{problems}");
    assert!(problems.contains("S-999"), "{problems}");
}

#[test]
fn an_architect_error_or_garbage_falls_back_to_the_local_path() {
    let (prompt, map) = local();

    crate::v3::forcing(&["nr.blueprint"], || {
        assert!(matches!(run(&allowed(), &prompt, &map, &|t| t.to_string(), &mut |_| Err("HTTP 429".into())), Outcome::Local(reason) if reason.contains("429")));
        assert!(matches!(run(&allowed(), &prompt, &map, &|t| t.to_string(), &mut |_| Ok("I cannot do that".into())), Outcome::Local(_)));
    });
}
