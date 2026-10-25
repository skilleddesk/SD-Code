//! **Evidence (P0-7, switch `padma.evidence_trust`)**: whether an answer *claims* that work was done. Trust compares this with what the log
//! can prove (an edit or a command was recorded); a claim with no proof is never "high".

/// First-person statements of finished work, in English and Bengali. A question or a plan ("I will fix", "should I fix") is not a claim.
const CLAIMS: &[&str] = &[
    "i fixed", "i've fixed", "i have fixed", "i changed", "i've changed", "i have changed", "i edited", "i updated", "i've updated", "i have updated", "i created", "i've created", "i added", "i've added",
    "i have added", "i wrote", "i implemented", "i refactored", "i removed", "i deleted", "i renamed", "i ran the", "i installed", "i built", "i deployed", "all tests pass", "tests pass", "tests passed",
    "ঠিক করেছি", "করে দিয়েছি",
];

const CLAIMS_BN: &[&str] = &["যোগ করেছি", "পরিবর্তন করেছি", "চালিয়েছি", "লিখেছি"];

/// True when the text says, in the first person, that something was changed, built, run or tested.
pub fn claims_work(text: &str) -> bool {
    let lower = text.to_lowercase();

    CLAIMS.iter().chain(CLAIMS_BN.iter()).any(|claim| lower.contains(claim))
}

#[cfg(test)]
mod tests {
    use super::claims_work;

    #[test]
    fn work_claims_are_found_and_plans_and_questions_are_not() {
        assert!(claims_work("I fixed the bug in src/main.rs and ran the tests - all of them pass."));
        assert!(claims_work("ফাইলটা ঠিক করেছি।"));
        assert!(claims_work("Done. All tests pass."));
        assert!(!claims_work("It says hi."));
        assert!(!claims_work("I will fix it next. Should I fix it now?"));
    }
}
