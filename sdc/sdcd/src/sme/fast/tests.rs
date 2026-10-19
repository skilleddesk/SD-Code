use super::*;

#[test]
fn short_single_job_requests_are_fast() {
    for request in ["what does parse_args do in src/cli.rs?", "rename total to subtotal in billing.py", "ei function ta ki kore?", "এই ফাংশনটা কী করে?"] {
        assert!(is_fast(request), "{request}");
    }
}

#[test]
fn large_or_multi_job_requests_are_not() {
    let long = "x".repeat(MAX_CHARS + 1);

    for request in [
        long.as_str(),
        "",
        "fix src/a.rs and src/b.rs",
        "add a flag, then update the docs",
        "what is this? and why does it fail?",
        "line one\nline two\nline three",
        "login thik koro tarpor test lekho",
        "লগইন ঠিক করো এবং টেস্ট লেখো",
    ] {
        assert!(!is_fast(request), "{request:?}");
    }
}
