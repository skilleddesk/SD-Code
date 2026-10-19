use super::*;

fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read_file",
            description: "read",
            schema: json!({ "type": "object", "properties": { "path": { "type": "string" }, "offset": { "type": "integer" } }, "required": ["path"], "additionalProperties": false }),
        },
        ToolSpec {
            name: "run_command",
            description: "run",
            schema: json!({ "type": "object", "properties": { "command": { "type": "string" }, "timeout_seconds": { "type": "integer" } }, "required": ["command"], "additionalProperties": false }),
        },
        ToolSpec {
            name: "write_file",
            description: "write",
            schema: json!({ "type": "object", "properties": { "path": { "type": "string" }, "content": { "type": "string" }, "overwrite": { "type": "boolean" } }, "required": ["path", "content"], "additionalProperties": false }),
        },
    ]
}

fn names(result: &Rescued) -> Vec<String> {
    result.calls.iter().map(|call| format!("{}:{}", call.name, call.input)).collect()
}

/// The golden corpus: printed calls in every shape local models produce. Each must come out as one valid call.
#[test]
fn every_printed_shape_in_the_corpus_is_rescued_as_a_valid_call() {
    let corpus: &[(&str, &str, &str)] = &[
        ("hermes tags", "I'll read it.\n<tool_call>\n{\"name\": \"read_file\", \"arguments\": {\"path\": \"src/a.rs\"}}\n</tool_call>", "read_file"),
        ("hermes, unclosed at the end", "<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.rs\"}}", "read_file"),
        ("fenced json", "Let me run it:\n```json\n{\"name\": \"run_command\", \"arguments\": {\"command\": \"cargo test\"}}\n```", "run_command"),
        ("fenced without a language", "```\n{\"name\": \"run_command\", \"arguments\": {\"command\": \"ls\"}}\n```", "run_command"),
        ("bare object", "{\"name\": \"read_file\", \"arguments\": {\"path\": \"README.md\"}}", "read_file"),
        ("openai function shape", "{\"function\": {\"name\": \"read_file\", \"arguments\": \"{\\\"path\\\": \\\"x.py\\\"}\"}}", "read_file"),
        ("tool/parameters keys", "{\"tool\": \"run_command\", \"parameters\": {\"command\": \"pwd\"}}", "run_command"),
        ("trailing comma", "<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.rs\",},}</tool_call>", "read_file"),
        ("single quotes", "<tool_call>{'name': 'read_file', 'arguments': {'path': 'a.rs'}}</tool_call>", "read_file"),
        ("unquoted keys", "<tool_call>{name: \"read_file\", arguments: {path: \"a.rs\"}}</tool_call>", "read_file"),
        ("python literals", "<tool_call>{\"name\": \"write_file\", \"arguments\": {\"path\": \"a.txt\", \"content\": \"x\", \"overwrite\": True}}</tool_call>", "write_file"),
        ("a missing closing brace", "<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.rs\"}</tool_call>", "read_file"),
        ("comments", "<tool_call>{\n // read it\n \"name\": \"read_file\", /* path */ \"arguments\": {\"path\": \"a.rs\"}}\n</tool_call>", "read_file"),
        ("smart quotes", "<tool_call>{\u{201c}name\u{201d}: \u{201c}read_file\u{201d}, \u{201c}arguments\u{201d}: {\u{201c}path\u{201d}: \u{201c}a.rs\u{201d}}}</tool_call>", "read_file"),
        ("qwen3 coder xml", "<function=read_file>\n<parameter=path>\nsrc/lib.rs\n</parameter>\n</function>", "read_file"),
        ("xml with an integer", "<function=read_file><parameter=path>a.rs</parameter><parameter=offset>40</parameter></function>", "read_file"),
    ];

    for (label, text, tool) in corpus {
        let result = rescue(text, &specs());

        assert_eq!(result.calls.len(), 1, "{label}: {result:?}");
        assert_eq!(result.calls[0].name, *tool, "{label}");
        assert!(result.rejected.is_empty(), "{label}: {:?}", result.rejected);
        assert!(validate(specs().iter().find(|spec| spec.name == *tool).unwrap(), &result.calls[0].input).is_ok(), "{label}: a rescued call is schema-valid");
    }
}

#[test]
fn rescued_values_keep_their_types_and_their_strings() {
    let result = rescue("<function=read_file><parameter=path>a b/c.rs</parameter><parameter=offset>40</parameter></function>", &specs());

    assert_eq!(result.calls[0].input, json!({ "path": "a b/c.rs", "offset": 40 }));

    let result = rescue("<tool_call>{\"name\": \"write_file\", \"arguments\": {\"path\": \"a.json\", \"content\": \"{\\\"k\\\": [1, 2,]}\"}}</tool_call>", &specs());

    assert_eq!(result.calls[0].input["content"], "{\"k\": [1, 2,]}", "a string that looks broken inside a string is left alone");
}

/// Invalid calls are rejected - not guessed.
#[test]
fn invalid_printed_calls_are_rejected_with_the_reason_and_nothing_is_run() {
    let cases: &[(&str, &str)] = &[
        ("<tool_call>{\"name\": \"delete_everything\", \"arguments\": {}}</tool_call>", "is not a tool"),
        ("<tool_call>{\"name\": \"read_file\", \"arguments\": {}}</tool_call>", "required argument `path` is missing"),
        ("<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": 42}}</tool_call>", "should be a string"),
        ("<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"a\", \"mode\": \"rw\"}}</tool_call>", "is not an argument of this tool"),
        ("<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"a\", \"offset\": \"40\"}}</tool_call>", "should be a integer"),
        ("<tool_call>{\"name\": \"read_file\", \"arguments\": [\"a.rs\"]}</tool_call>", "not an object"),
    ];

    for (text, why) in cases {
        let result = rescue(text, &specs());

        assert!(result.calls.is_empty(), "{text}: {result:?}");
        assert!(result.rejected.iter().any(|reason| reason.contains(why)), "{text}: wanted `{why}` in {:?}", result.rejected);
    }
}

#[test]
fn prose_and_examples_are_not_run() {
    assert_eq!(rescue("I will look at the file and then fix the bug.", &specs()), Rescued { text: "I will look at the file and then fix the bug.".into(), ..Default::default() });
    assert!(rescue("The function returns {\"ok\": true}.", &specs()).calls.is_empty(), "an object that is not a call");

    let example = format!("To read a file you could call it like this: {{\"name\": \"read_file\", \"arguments\": {{\"path\": \"a.rs\"}}}}. {}", "That tool returns the file's text with line numbers, which is useful when you want to look closely. ".repeat(6));
    let result = rescue(&example, &specs());

    assert!(result.calls.is_empty(), "a call inside a long explanation is an example: {result:?}");
    assert!(result.rejected.iter().any(|reason| reason.contains("example")));
}

#[test]
fn two_calls_in_one_reply_keep_their_order_and_the_text_around_them_is_kept() {
    let result = rescue("First:\n<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.rs\"}}</tool_call>\nthen:\n<tool_call>{\"name\": \"run_command\", \"arguments\": {\"command\": \"ls\"}}</tool_call>", &specs());

    assert_eq!(names(&result), ["read_file:{\"path\":\"a.rs\"}", "run_command:{\"command\":\"ls\"}"]);
    assert_eq!(result.text, "First:\n\nthen:");
    assert_eq!(result.calls[0].id, "rescued-1");
}

#[test]
fn applying_a_rescue_makes_the_reply_look_as_if_the_channel_had_carried_the_call() {
    let mut reply = super::super::dialect::Reply {
        message: json!({ "role": "assistant", "content": "<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.rs\"}}</tool_call>" }),
        text: "<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.rs\"}}</tool_call>".into(),
        stop: "stop".into(),
        ..Default::default()
    };
    let result = apply(&mut reply, &specs());

    assert_eq!(result.calls.len(), 1);
    assert_eq!(reply.tool_uses.len(), 1);
    assert_eq!(reply.stop, "tool_calls");
    assert_eq!(reply.message["tool_calls"][0]["function"]["name"], "read_file");
    assert_eq!(reply.message["tool_calls"][0]["id"], "rescued-1");
    assert_eq!(reply.message["content"], "");

    /* a reply that already used the channel is never second-guessed */
    let mut real = super::super::dialect::Reply { text: "<tool_call>{\"name\": \"read_file\", \"arguments\": {\"path\": \"x\"}}</tool_call>".into(), tool_uses: vec![ToolUse { id: "c1".into(), name: "run_command".into(), input: json!({ "command": "ls" }) }], ..Default::default() };

    assert_eq!(apply(&mut real, &specs()), Rescued::default());
    assert_eq!(real.tool_uses.len(), 1);
}

#[test]
fn json_repair_fixes_the_usual_slips_and_gives_up_on_the_rest() {
    assert_eq!(repair_json("{'a': [1, 2,], b: None}"), Some(json!({ "a": [1, 2], "b": null })));
    assert_eq!(repair_json("{\"a\": 1"), Some(json!({ "a": 1 })));
    assert_eq!(repair_json("not json at all"), None);
    assert_eq!(repair_json("{\"a\": \"unterminated"), None);
    assert_eq!(repair_json("{\"a\": 1}}"), None, "an extra closer is not guessed away");
}

#[test]
fn the_constraint_allows_exactly_one_call_of_one_known_tool() {
    let schema = call_schema(&specs());
    let options = schema["anyOf"].as_array().unwrap();

    assert_eq!(options.len(), 3);
    assert_eq!(options[0]["properties"]["name"]["enum"], json!(["read_file"]));
    assert_eq!(options[0]["properties"]["arguments"]["required"], json!(["path"]));
    assert_eq!(options[1]["required"], json!(["name", "arguments"]));

    /* what the server would produce under it is read back as a valid call */
    let produced = r#"{"name": "run_command", "arguments": {"command": "ls"}}"#;

    assert_eq!(rescue(produced, &specs()).calls.len(), 1);

    /* the flag is taken once */
    constrain_next(true);
    assert!(take_constrained());
    assert!(!take_constrained());
}
