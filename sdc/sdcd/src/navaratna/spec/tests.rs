use super::*;

const ENGLISH: &str = "# Shop API\n\nWe need a small shop. It must keep 3.5 percent tax, e.g. on food. Use the file src/shop.rs for it.\n\nThe model:\n\n```rust\nstruct Item {\n    price: f64,\n\n    name: String,\n}\n```\n\n| field | type |\n|-------|------|\n| price | f64  |\n\n{\n  \"currency\": \"USD\",\n  \"rate\": 0.035\n}\n\n> Note from the owner: do not touch billing.\n\n- Add an endpoint /items.\n- If the cart is empty, show a message. Otherwise, show the total.\n- Unless the user is an admin:\n  - hide the cost column\n  - if the session is old, ask to sign in again\n- else show everything\n";
const BANGLA: &str = "আমাদের একটা দোকানের অ্যাপ চাই। এতে পণ্যের তালিকা দেখাও। যদি কার্ট খালি থাকে তবে একটা বার্তা দেখাও। নাহলে মোট দাম দেখাও।\n\nshop.py ফাইলে total() ফাংশনটা ঠিক করো. jodi stock na thake tahole out of stock dekhao.\n\n```python\ndef total(cart):\n    return sum(i.price for i in cart)\n\n# not a sentence. It stays whole.\n```\n";

fn kinds(units: &[Unit]) -> Vec<Kind> {
    units.iter().map(|unit| unit.kind).collect()
}

#[test]
fn nothing_is_dropped_and_nothing_counted_twice() {
    for prompt in [ENGLISH, BANGLA, "", "one", "   \n\n", "A. B. C?"] {
        let units = parse(prompt);

        assert!(accounts_for_everything(prompt, &units), "{prompt:?} -> {units:#?}");
    }
}

#[test]
fn a_code_block_is_one_unit_even_with_blank_lines_sentences_and_hashes_inside() {
    for prompt in [ENGLISH, BANGLA] {
        let units = parse(prompt);
        let fences = prompt.matches("```").count() / 2;
        let code: Vec<&Unit> = units.iter().filter(|unit| unit.kind == Kind::Code).collect();

        assert_eq!(code.len(), fences);

        for unit in code {
            assert!(unit.text.starts_with("```") && unit.text.ends_with("```"), "{unit:?}");
            assert_eq!(unit.text.matches("```").count(), 2, "a code block is never split");
        }

        /* and no other unit starts inside a code block */
        for unit in units.iter().filter(|u| u.kind != Kind::Code) {
            assert!(!units.iter().any(|c| c.kind == Kind::Code && c.start < unit.start && unit.start < c.end), "{unit:?} starts inside a code block");
        }
    }
}

#[test]
fn tables_json_quotes_and_headings_are_whole_units() {
    let units = parse(ENGLISH);
    let kinds = kinds(&units);

    assert_eq!(kinds.iter().filter(|k| **k == Kind::Table).count(), 1);
    assert_eq!(kinds.iter().filter(|k| **k == Kind::Json).count(), 1);
    assert_eq!(kinds.iter().filter(|k| **k == Kind::Quote).count(), 1);
    assert_eq!(kinds.iter().filter(|k| **k == Kind::Heading).count(), 1);
    assert!(units.iter().any(|u| u.kind == Kind::Table && u.text.lines().count() == 3));
    assert!(units.iter().any(|u| u.kind == Kind::Json && u.text.contains("\"rate\": 0.035") && u.text.lines().count() == 4));
}

#[test]
fn decimals_abbreviations_and_file_names_do_not_end_a_sentence() {
    let units = parse("It must keep 3.5 percent tax, e.g. on food. Use src/shop.rs for it. Version 1.0 ships soon.");
    let texts: Vec<&str> = units.iter().map(|u| u.text.as_str()).collect();

    assert_eq!(texts, ["It must keep 3.5 percent tax, e.g. on food.", "Use src/shop.rs for it.", "Version 1.0 ships soon."]);
}

#[test]
fn a_condition_and_its_otherwise_stay_one_unit_in_english_bengali_and_banglish() {
    let english = parse("If the cart is empty, show a message. Otherwise, show the total. Then log it.");
    let bangla = parse("যদি কার্ট খালি থাকে তবে একটা বার্তা দেখাও। নাহলে মোট দাম দেখাও। আরেকটা কাজ করো।");
    let banglish = parse("jodi stock na thake tahole out of stock dekhao. nahole dam dekhao. Arekta kaj korbe.");

    assert_eq!(english.len(), 2, "{english:#?}");
    assert!(english[0].text.ends_with("show the total."));
    assert_eq!(bangla.len(), 2, "{bangla:#?}");
    assert!(bangla[0].text.contains("নাহলে"));
    assert_eq!(banglish.len(), 2, "{banglish:#?}");
    assert!(banglish[0].text.contains("nahole"));
}

#[test]
fn nested_conditions_in_a_list_stay_whole() {
    let units = parse(ENGLISH);
    let unless = units.iter().find(|u| u.text.starts_with("- Unless")).expect("the unless item");

    assert!(unless.text.contains("hide the cost column") && unless.text.contains("if the session is old"), "{unless:?}");
    assert!(unless.text.contains("- else show everything"), "the else item belongs to the condition: {unless:?}");

    let cart = units.iter().find(|u| u.text.starts_with("- If the cart")).unwrap();

    assert!(cart.text.contains("Otherwise, show the total."));
}

#[test]
fn the_units_are_numbered_in_order() {
    let units = parse(ENGLISH);

    for (n, unit) in units.iter().enumerate() {
        assert_eq!(unit.id, format!("S-{:03}", n + 1));
    }

    assert!(units.windows(2).all(|pair| pair[0].end <= pair[1].start));
}

#[test]
fn a_full_pass_covers_one_hundred_percent_and_gives_requirement_ids_with_spans() {
    for prompt in [ENGLISH, BANGLA] {
        let map = analyse(prompt);

        assert!(map.unmapped().is_empty(), "{:?}", map.unmapped());
        assert!(coverage_gate(&map).is_ok());
        assert!(!map.requirements.is_empty());

        for (r_id, units) in &map.requirements {
            assert!(r_id.starts_with("R-"));

            for id in units {
                let unit = map.units.iter().find(|u| &u.id == id).unwrap();

                assert_eq!(&prompt[unit.start..unit.end], unit.text, "the source span of {r_id}");
            }
        }
    }
}

#[test]
fn a_code_block_after_a_requirement_belongs_to_it() {
    let map = analyse("Add a function double(x) that returns x * 2.\n\n```python\ndef double(x):\n    return x * 2\n```\n");
    let (r_id, units) = map.requirements.iter().next().unwrap();

    assert_eq!(r_id, "R-001");
    assert_eq!(units, &vec!["S-001".to_string(), "S-002".to_string()]);
}

#[test]
fn one_deliberately_unmapped_unit_blocks_planning_and_names_it() {
    let units = parse(ENGLISH);
    let skip = units[3].id.clone();
    let map = label(units, &mut |unit, last| if unit.id == skip { None } else { heuristic(unit, last) });
    let blocked = coverage_gate(&map).unwrap_err();

    assert_eq!(blocked.unmapped, vec![skip.clone()]);
    assert!(blocked.to_string().contains(&skip), "{blocked}");
    assert!(blocked.to_string().starts_with("planning is blocked"));
    crate::v3::ablating(&["nr.sentence_cover"], || assert!(coverage_gate(&map).is_ok(), "with the switch off the gate is open"));
}

#[test]
fn a_part_of_nothing_is_not_covered() {
    let units = parse("Hello there. Add a thing.");
    let map = label(units, &mut |unit, _| Some(if unit.id == "S-001" { Label::PartOf("S-099".into()) } else { Label::Requirement }));

    assert_eq!(map.unmapped().len(), 1);
}

#[test]
fn the_prompt_and_its_map_are_kept_whole_in_the_vault() {
    let dir = tempfile::tempdir().unwrap();
    let map = analyse(ENGLISH);
    let (spec, map_id) = store(dir.path(), ENGLISH, &map).unwrap();

    assert_eq!(super::super::vault::get(dir.path(), "spec", &spec).unwrap(), ENGLISH);

    let back: Map = serde_json::from_str(&super::super::vault::get(dir.path(), "specmap", &map_id).unwrap()).unwrap();

    assert_eq!(back, map);
}

#[test]
fn only_a_big_prompt_is_checked_and_stored() {
    let dir = tempfile::tempdir().unwrap();

    assert!(check_big_prompt(dir.path(), "Add a button.").is_none());

    let big = ENGLISH.repeat(4);
    let (spec, _) = check_big_prompt(dir.path(), &big).unwrap().unwrap();

    assert_eq!(super::super::vault::get(dir.path(), "spec", &spec).unwrap(), big);
    crate::v3::ablating(&["nr.sentence_cover"], || assert!(check_big_prompt(dir.path(), &big).is_none()));
}
