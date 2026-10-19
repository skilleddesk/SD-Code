use super::*;

const PAY: &str = "def charge(amount, currency):\n    total = amount * 2\n    return total\n\n\nclass Ledger:\n    def add(self, entry):\n        self.entries.append(entry)\n";

fn root(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("sdc-quote-{name}-{}", std::process::id()));

    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/pay.py"), PAY).unwrap();
    root
}

#[test]
fn an_exact_line_and_one_that_differs_only_in_whitespace_are_accepted() {
    assert_eq!(verify(&root("exact"), "src/pay.py", "def charge(amount, currency):"), Ok(()));
    assert_eq!(verify(&root("exact"), "src/pay.py", "    total = amount * 2"), Ok(()), "the indentation as written");
    assert_eq!(verify(&root("exact"), "src/pay.py", "total = amount   *  2"), Ok(()), "the same tokens, different spacing");
    assert_eq!(verify(&root("exact"), "src/pay.py", "def charge(amount, currency):\n        total = amount * 2"), Ok(()), "a quote over two lines, rewrapped");
    assert_eq!(verify(&root("exact"), "./src/pay.py", "class Ledger:"), Ok(()), "a leading ./ is the same file");
}

#[test]
fn a_fabricated_quote_is_refused_and_the_sentence_says_where_to_look() {
    let root = root("fabricated");
    let refused = verify(&root, "src/pay.py", "def charge(amount):     # one parameter fewer").unwrap_err();

    assert!(refused.contains("is not in src/pay.py") && refused.contains("was not kept"), "{refused}");
    assert!(refused.contains("The nearest line is"), "a refusal points at the line that is nearly it: {refused}");

    /* a word swapped is a different quote; a line from another file is not in this one */
    assert!(verify(&root, "src/pay.py", "total = amount * 3").is_err());
    assert!(verify(&root, "src/pay.py", "import os").is_err());
    assert!(verify(&root, "src/pay.py", "return total").is_ok());
    assert!(verify(&root, "src/pay.py", "totl = amount * 2").is_err(), "a typo is not a match");
}

#[test]
fn a_quote_needs_a_source_a_file_that_is_there_and_a_line_sized_length() {
    let root = root("needs");

    assert!(verify(&root, "", "class Ledger:").unwrap_err().contains("needs a `source`"));
    assert!(verify(&root, "src/pay.py", "   ").unwrap_err().contains("is empty"));
    assert!(verify(&root, "src/nope.py", "x").unwrap_err().contains("cannot be checked"));
    assert!(verify(&root, "src", "x").unwrap_err().contains("cannot be checked"), "a folder is not a file");
    assert!(verify(&root, "src/pay.py", &"x".repeat(401)).unwrap_err().contains("one line"));
}

#[test]
fn find_answers_the_first_line_of_the_whole_match() {
    assert_eq!(find("a\nbc\nd", "bc\nd"), Some(2));
    assert_eq!(find("one two  three", "one two three"), Some(0));
    assert_eq!(find("x", ""), None);
    assert_eq!(find("", "x"), None);
    assert_eq!(normalize("  a\n\tb  c "), "a b c");
}

/// P1-9's number: the fabricated-quote rate must be **zero**. A mutation that happens to still be a fragment of the file is a partial
/// quote, not a fabrication, so the corpus is built from mutations the file really does not contain - and every one of them is refused,
/// while every real line is accepted (no false alarms either way).
#[test]
fn fuzzed_mutations_of_real_lines_are_never_accepted_and_real_lines_are_never_refused() {
    let root = root("fuzz");
    let lines: Vec<&str> = PAY.lines().filter(|line| !line.trim().is_empty()).collect();
    let (mut accepted_real, mut accepted_fake, mut tried_fake) = (0, 0, 0);

    for line in &lines {
        if verify(&root, "src/pay.py", line).is_ok() {
            accepted_real += 1;
        }

        let trimmed = line.trim();
        let words: Vec<&str> = trimmed.split_whitespace().collect();
        let mut mutations: Vec<String> = vec![
            /* a token changed */
            trimmed.replace('2', "3").replace('1', "7"),
            /* a token swapped for one that is nowhere in this file */
            trimmed.replace("total", "aggregate").replace("amount", "quantity").replace("charge", "invoice"),
            /* a rewritten name (what a model does when it quotes from memory) */
            trimmed.replace('(', " (").replace("def ", "definitely_"),
            /* reordered words */
            words.iter().rev().cloned().collect::<Vec<_>>().join(" "),
        ];

        if words.len() > 3 {
            /* a word dropped from the middle */
            let mut fewer = words.clone();

            fewer.remove(words.len() / 2);
            mutations.push(fewer.join(" "));
        }

        for mutation in mutations {
            /* a mutation that is still a fragment of the file is a partial quote, not a fabrication */
            if mutation.trim().is_empty() || find(PAY, &mutation).is_some() {
                continue;
            }

            tried_fake += 1;

            if verify(&root, "src/pay.py", &mutation).is_ok() {
                accepted_fake += 1;
            }
        }
    }

    assert_eq!(accepted_real, lines.len(), "every real line is accepted");
    assert!(tried_fake >= 10, "the corpus must be big enough to mean something: {tried_fake}");
    assert_eq!(accepted_fake, 0, "the fabricated-quote rate is {accepted_fake}/{tried_fake}, and it must be 0");
}
