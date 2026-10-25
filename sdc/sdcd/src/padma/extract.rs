//! **ToolCallExtractor (P0-2, switch `padma.tolerant_parse`)**: no tool call is lost because of its format. In order:
//! 1. the reply's native `tool_calls`;
//! 2. `<tool_call>` blocks in the content (JSON body, or Qwen3-Coder's `<function=...><parameter=...>`);
//! 3. the same inside the thinking;
//! 4. one fenced (or bare) JSON object that names a known tool.
//!
//! Each call is checked against the tool's schema; one that fails is refused with a short schema error for the model. Thinking is never
//! sent back, and an unclosed `<think>` is closed before the text is sent again.

use serde_json::{json, Value};

use crate::agent::dialect::{Reply, ToolSpec};
use crate::agent::rescue;

/// Where the calls of a reply came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Native,
    Content,
    Thinking,
    Fenced,
    Nothing,
}

#[derive(Debug)]
pub struct Extraction {
    pub source: Source,
    /// Calls that were printed but are not valid: why each was refused.
    pub rejected: Vec<String>,
}

/// Splits `<think>...</think>` blocks (and an unclosed `<think>` to the end) out of a text: `(visible, thought)`.
pub fn split_think(text: &str) -> (String, String) {
    let mut visible = String::new();
    let mut thought = String::new();
    let mut rest = text;

    while let Some(open) = rest.find("<think>") {
        visible.push_str(&rest[..open]);

        let body = &rest[open + 7..];

        match body.find("</think>") {
            Some(close) => {
                thought.push_str(&body[..close]);
                thought.push('\n');
                rest = &body[close + 8..];
            }
            None => {
                thought.push_str(body);
                rest = "";
            }
        }
    }

    visible.push_str(rest);

    /* a stray closing tag with no opening one: everything before it was thinking */
    if let Some(close) = visible.find("</think>") {
        thought.push_str(&visible[..close]);
        visible = visible[close + 8..].to_string();
    }

    (visible.trim().to_string(), thought.trim().to_string())
}

fn adopt(reply: &mut Reply, found: rescue::Rescued) {
    reply.tool_uses = found.calls.clone();
    reply.stop = "tool_calls".to_string();
    reply.text = found.text.clone();

    if reply.message.get("role").is_some_and(|role| role == "assistant") {
        reply.message["content"] = Value::String(found.text);
        reply.message["tool_calls"] = Value::Array(found.calls.iter().map(|call| json!({ "id": call.id, "type": "function", "function": { "name": call.name, "arguments": call.input.to_string() } })).collect());
    }
}

/// Reads a reply for the tool calls it holds and puts them in `tool_uses` the way the channel would have. A reply that already has native
/// calls is left alone. Whatever the model wrote inside `<think>` in its content is moved out of the text, so it is never sent back.
pub fn extract(reply: &mut Reply, specs: &[ToolSpec]) -> Extraction {
    let mut rejected: Vec<String> = Vec::new();

    if !reply.tool_uses.is_empty() {
        return Extraction { source: Source::Native, rejected };
    }

    /* an unclosed or closed <think> in the content: its text is the model's thinking, the rest is the answer */
    if reply.text.contains("<think>") || reply.text.contains("</think>") {
        let (visible, thought) = split_think(&reply.text);

        if !thought.is_empty() {
            if !reply.thinking.is_empty() {
                reply.thinking.push('\n');
            }

            reply.thinking.push_str(&thought);
        }

        reply.text = visible;

        if reply.message.get("role").is_some_and(|role| role == "assistant") && reply.message["content"].is_string() {
            reply.message["content"] = Value::String(reply.text.clone());
        }
    }

    let content = reply.text.clone();
    let thinking = reply.thinking.clone();
    let attempts: [(&str, Source, bool); 3] = [(&content, Source::Content, false), (&thinking, Source::Thinking, false), (&content, Source::Fenced, true)];

    for (text, source, bare) in attempts {
        if text.trim().is_empty() {
            continue;
        }

        let found = rescue::rescue_with(text, specs, bare);

        for reason in &found.rejected {
            if !rejected.contains(reason) {
                rejected.push(reason.clone());
            }
        }

        if !found.calls.is_empty() {
            /* a call found in the thinking leaves the content as it was */
            let found = if source == Source::Thinking { rescue::Rescued { text: reply.text.clone(), ..found } } else { found };

            adopt(reply, found);

            return Extraction { source, rejected: Vec::new() };
        }
    }

    Extraction { source: Source::Nothing, rejected }
}

/// The note to the model when a printed call was refused.
pub fn refusal_note(rejected: &[String]) -> String {
    format!("A tool call you printed was not run: {}. Make the call again with the tool-call channel and the exact arguments the tool lists.", rejected.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn specs() -> Vec<ToolSpec> {
        vec![ToolSpec {
            name: "read_file",
            description: "read",
            schema: json!({ "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"], "additionalProperties": false }),
        }]
    }

    fn reply(text: &str, thinking: &str) -> Reply {
        Reply { message: json!({ "role": "assistant", "content": text }), text: text.to_string(), thinking: thinking.to_string(), stop: "stop".to_string(), ..Reply::default() }
    }

    #[test]
    fn a_native_call_is_left_alone() {
        let mut native = reply("", "");

        native.tool_uses = vec![crate::agent::dialect::ToolUse { id: "1".into(), name: "read_file".into(), input: json!({ "path": "a" }) }];

        assert_eq!(extract(&mut native, &specs()).source, Source::Native);
    }

    #[test]
    fn a_json_call_in_the_content_is_found() {
        let mut printed = reply("<tool_call>{\"name\":\"read_file\",\"arguments\":{\"path\":\"src/a b.rs\"}}</tool_call>", "");
        let got = extract(&mut printed, &specs());

        assert_eq!(got.source, Source::Content);
        assert_eq!(printed.tool_uses[0].input, json!({ "path": "src/a b.rs" }));
        assert_eq!(printed.stop, "tool_calls");
        assert_eq!(printed.message["tool_calls"][0]["function"]["name"], "read_file");
    }

    #[test]
    fn qwen_xml_in_the_content_is_found() {
        let mut printed = reply("<function=read_file>\n<parameter=path>\nREADME.md\n</parameter>\n</function>", "");

        assert_eq!(extract(&mut printed, &specs()).source, Source::Content);
        assert_eq!(printed.tool_uses[0].input, json!({ "path": "README.md" }));
    }

    #[test]
    fn a_call_hidden_in_the_thinking_is_found_and_the_thinking_is_not_sent_back() {
        let mut hidden = reply("", "I should read it. <tool_call>{\"name\":\"read_file\",\"arguments\":{\"path\":\"x\"}}</tool_call>");
        let got = extract(&mut hidden, &specs());

        assert_eq!(got.source, Source::Thinking);
        assert_eq!(hidden.tool_uses.len(), 1);
        assert_eq!(hidden.message["content"], "");
        assert!(!hidden.message.to_string().contains("I should read it"));
    }

    #[test]
    fn a_fenced_json_object_is_the_last_resort() {
        let mut fenced = reply("```json\n{\"name\":\"read_file\",\"arguments\":{\"path\":\"x\"}}\n```", "");

        assert_eq!(extract(&mut fenced, &specs()).source, Source::Fenced);
    }

    #[test]
    fn an_invalid_call_is_refused_with_a_reason() {
        let mut wrong = reply("<tool_call>{\"name\":\"read_file\",\"arguments\":{\"file\":\"x\"}}</tool_call>", "");
        let got = extract(&mut wrong, &specs());

        assert_eq!(got.source, Source::Nothing);
        assert!(wrong.tool_uses.is_empty());
        assert!(got.rejected[0].contains("path"), "{:?}", got.rejected);
        assert!(refusal_note(&got.rejected).contains("was not run"));
    }

    #[test]
    fn think_blocks_are_split_out_and_an_unclosed_one_is_closed() {
        assert_eq!(split_think("<think>plan</think>The answer."), ("The answer.".to_string(), "plan".to_string()));
        assert_eq!(split_think("Hello <think>never closed"), ("Hello".to_string(), "never closed".to_string()));
        assert_eq!(split_think("hidden</think>shown"), ("shown".to_string(), "hidden".to_string()));

        let mut unclosed = reply("<think>I will read the file", "");

        extract(&mut unclosed, &specs());

        assert_eq!(unclosed.text, "");
        assert_eq!(unclosed.message["content"], "");
        assert_eq!(unclosed.thinking, "I will read the file");
    }

    #[test]
    fn a_plain_answer_has_no_calls() {
        let mut plain = reply("The answer is 4.", "");

        assert_eq!(extract(&mut plain, &specs()).source, Source::Nothing);
        assert!(plain.tool_uses.is_empty());
    }
}
