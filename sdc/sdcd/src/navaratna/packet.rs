//! **N8-2 - the Escalation Packet, merged into Capsule v2** (switch `nr.escalation_packet`, off).
//!
//! The cloud should receive exactly what it needs to fix one step. That is Capsule v2 (`rotation::capsule`) plus: the requirement ids and
//! what done means, the locked contracts that touch the step, the skeletons of the related files (not the files), the local model's last
//! diff, the **sliced** error instead of the raw log, and the new tests. One structure, not two: these are extra sections of the same
//! capsule, inside the same token budget, behind the same stable prefix, through the same redaction and the same audit line in
//! `.sdc/audit/sent.jsonl`. With the switch off the capsule is exactly what it was.

/// The extra material of an escalation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Packet {
    /// `(requirement id, done-when)`.
    pub requirements: Vec<(String, String)>,
    /// Signatures of locked contracts the step touches.
    pub contracts: Vec<String>,
    /// `(path, skeleton)` of related files.
    pub skeletons: Vec<(String, String)>,
    /// The local model's last attempt, as a diff.
    pub last_diff: String,
    /// The error as the Error Slicer cut it; it replaces the raw error when present.
    pub sliced_error: Option<String>,
    /// `(path, text)` of the tests written for the step.
    pub new_tests: Vec<(String, String)>,
    /// A Golden Trace: how a like problem was fixed (N9-1). Last in the capsule, so it is the first to be cut.
    pub example: Option<String>,
}

/// The capsule sections a packet adds, in order of importance: `(title, body)`. A skeleton of a file the policy keeps in is left out.
pub fn sections(packet: &Packet, may_leave: &dyn Fn(&str) -> bool) -> Vec<(String, String)> {
    let mut out = Vec::new();

    if !packet.requirements.is_empty() {
        out.push(("Requirements and what done means".to_string(), packet.requirements.iter().map(|(id, done)| format!("- {id}: {done}")).collect::<Vec<_>>().join("\n")));
    }

    if !packet.contracts.is_empty() {
        out.push(("Locked contracts (do not change their signatures)".to_string(), packet.contracts.iter().map(|c| format!("- {c}")).collect::<Vec<_>>().join("\n")));
    }

    if !packet.last_diff.trim().is_empty() {
        out.push(("The local model's last attempt (diff)".to_string(), format!("<diff>\n{}\n</diff>", packet.last_diff.trim_end())));
    }

    if !packet.new_tests.is_empty() {
        out.push((
            "Tests written for this step".to_string(),
            packet.new_tests.iter().filter(|(path, _)| may_leave(path)).map(|(path, text)| format!("<file path=\"{path}\">\n{}\n</file>", text.trim_end())).collect::<Vec<_>>().join("\n"),
        ));
    }

    for (path, skeleton) in packet.skeletons.iter().filter(|(path, _)| may_leave(path)) {
        out.push((format!("Skeleton of {path} (signatures only)"), format!("<file path=\"{path}\">\n{}\n</file>", skeleton.trim_end())));
    }

    if let Some(example) = packet.example.as_ref().filter(|example| !example.trim().is_empty()) {
        out.push(("A like problem that was solved before".to_string(), example.trim_end().to_string()));
    }

    out.retain(|(_, body)| !body.trim().is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rotation::capsule::{finish, Prepared, Step, DEFAULT_BUDGET_TOKENS};
    use crate::trust::policy::Policy;

    fn packet() -> Packet {
        Packet {
            requirements: vec![("R-004".into(), "total() returns the tax-inclusive sum".into())],
            contracts: vec!["pub fn total(items: &[Item]) -> u64".into()],
            skeletons: vec![("src/shop.rs".into(), "L3: pub struct Item\nL9: pub fn total(items: &[Item]) -> u64".into()), (".env".into(), "SECRET=1".into())],
            last_diff: "-    items.len()\n+    items.iter().sum()".into(),
            sliced_error: Some("rustc error: mismatched types\n at src/shop.rs:12".into()),
            new_tests: vec![("tests/total.rs".into(), "#[test] fn totals() { assert_eq!(total(&[]), 0); }".into())],
            example: Some("Error: E0308\nFix: -x +y".into()),
        }
    }

    fn step<'a>(packet: Option<&'a Packet>, error: &'a str) -> Step<'a> {
        Step { session_id: "packet-test", goal: "make total() right", done_when: &[], error, notes: &[], files: &[], packet }
    }

    fn prepared() -> Prepared {
        Prepared { stable: "RULES".into(), snippets: vec![], redactions: vec![] }
    }

    #[test]
    fn the_packet_adds_its_sections_to_the_same_capsule_in_the_same_budget() {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::default();
        let packet = packet();
        /* 3.2: the packet is on by default, so the plain capsule is asked for by switching it off. */
        let plain = crate::v3::ablating(&["nr.escalation_packet"], || finish(&prepared(), dir.path(), &policy, &step(Some(&packet), "raw log line\n".repeat(50).as_str()), DEFAULT_BUDGET_TOKENS));
        let with = crate::v3::forcing(&["nr.escalation_packet"], || finish(&prepared(), dir.path(), &policy, &step(Some(&packet), "raw log line\n".repeat(50).as_str()), DEFAULT_BUDGET_TOKENS));

        assert!(!plain.text.contains("R-004"), "switched off: a plain capsule");
        assert!(with.text.starts_with("RULES"), "the stable prefix is first");

        for expected in ["R-004: total() returns the tax-inclusive sum", "pub fn total(items: &[Item]) -> u64", "items.iter().sum()", "mismatched types", "tests/total.rs", "Skeleton of src/shop.rs"] {
            assert!(with.text.contains(expected), "{expected} missing from:\n{}", with.text);
        }

        assert!(!with.text.contains("raw log line"), "the sliced error replaced the raw log");
        assert!(!with.text.contains("SECRET=1"), "a file the policy keeps in is not sent");
        assert!(with.tokens <= DEFAULT_BUDGET_TOKENS);
    }

    #[test]
    fn a_packet_stays_under_the_budget_and_the_goal_survives_a_small_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut big = packet();

        big.last_diff = "+ added line\n".repeat(5_000);
        big.skeletons = (0..40).map(|n| (format!("src/f{n}.rs"), "L1: pub fn x()\n".repeat(100))).collect();

        for budget in [800, 1_500, DEFAULT_BUDGET_TOKENS] {
            let capsule = crate::v3::forcing(&["nr.escalation_packet"], || finish(&prepared(), dir.path(), &Policy::default(), &step(Some(&big), "e"), budget));

            assert!(capsule.tokens <= budget + 60, "{} tokens for a budget of {budget}", capsule.tokens);
            assert!(capsule.text.contains("make total() right"), "the goal is the last to go");
        }
    }

    #[test]
    fn a_secret_in_the_diff_or_a_test_never_leaves() {
        let dir = tempfile::tempdir().unwrap();
        let fake = format!("sk-{}", "a1B2c3D4".repeat(6));
        let mut leaky = packet();

        leaky.last_diff = format!("+let key = \"{fake}\";");
        leaky.new_tests = vec![("tests/t.rs".into(), format!("const KEY: &str = \"{fake}\";"))];

        let capsule = crate::v3::forcing(&["nr.escalation_packet"], || finish(&prepared(), dir.path(), &Policy::default(), &step(Some(&leaky), "e"), DEFAULT_BUDGET_TOKENS));

        assert!(!capsule.text.contains(&fake), "{}", capsule.text);
        assert!(capsule.redactions.iter().any(|rule| rule.starts_with("packet")), "{:?}", capsule.redactions);
    }

    #[test]
    fn sections_are_in_order_and_empty_ones_are_left_out() {
        let titles: Vec<String> = sections(&packet(), &|path| path != ".env").into_iter().map(|(title, _)| title).collect();

        assert_eq!(titles[0], "Requirements and what done means");
        assert!(titles.iter().any(|t| t.contains("Locked contracts")) && titles.iter().any(|t| t.contains("last attempt")));
        assert_eq!(titles.iter().filter(|t| t.starts_with("Skeleton of")).count(), 1);
        assert!(sections(&Packet::default(), &|_| true).is_empty());
    }
}
