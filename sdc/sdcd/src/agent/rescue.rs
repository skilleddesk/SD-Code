//! **Tool-call Rescue (P1-6, switch `sme.toolcall_rescue`)**: a small local model often *prints* the tool call it means to make - as
//! `<tool_call>{"name": ...}</tool_call>`, as a fenced JSON block, as a bare object, or as Qwen's `<function=...><parameter=...>` - instead
//! of making it through the runtime's tool-call channel. The turn would then end with the call sitting in the answer and nothing done.
//!
//! The rescue reads such a reply, repairs near-valid JSON (trailing commas, single quotes, unquoted keys, `True`/`None`, comments, a
//! missing closing bracket), and turns each call into a real one **only if it validates against the tool's schema**: a known tool name,
//! an object of arguments, every required argument present, every value of the declared type, no argument the schema does not
//! list. Nothing is coerced and nothing is guessed - an invalid call is rejected, with the reason, and the model is told.
//!
//! It acts only when the reply has no real tool call (it never second-guesses a model that used the channel) and only for local models
//! (where this failure lives); a model explaining a tool in prose is protected by requiring the call to be the reply's last substantial
//! content.

use serde_json::{json, Map, Value};

use super::dialect::{ToolSpec, ToolUse};

/// What a reply held.
#[derive(Debug, Default, PartialEq)]
pub struct Rescued {
    pub calls: Vec<ToolUse>,
    /// Calls that were printed but are not valid: why each was refused.
    pub rejected: Vec<String>,
    /// The reply's text with the rescued calls taken out.
    pub text: String,
}

/// Near-valid JSON made valid, or `None`. Applies the repairs in order and stops at the first that parses.
pub fn repair_json(raw: &str) -> Option<Value> {
    let trimmed = raw.trim();

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }

    let mut text = trimmed.replace(['\u{201c}', '\u{201d}'], "\"").replace(['\u{2018}', '\u{2019}'], "'");

    /* line and block comments outside strings */
    text = strip_comments(&text);

    for attempt in [text.clone(), single_to_double(&text), unquoted_keys(&single_to_double(&text))] {
        let mut candidate = attempt;

        candidate = candidate.replace(": True", ": true").replace(": False", ": false").replace(": None", ": null").replace(":True", ":true").replace(":False", ":false").replace(":None", ":null");
        candidate = trailing_commas(&candidate);

        if let Ok(value) = serde_json::from_str::<Value>(&candidate) {
            return Some(value);
        }

        /* a missing closing bracket or brace at the end (the model stopped one short) */
        if let Some(closed) = close_brackets(&candidate) {
            if let Ok(value) = serde_json::from_str::<Value>(&closed) {
                return Some(value);
            }
        }
    }

    None
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let (mut index, mut in_string, mut quote) = (0, false, '"');

    while index < chars.len() {
        let c = chars[index];

        if in_string {
            out.push(c);

            if c == '\\' && index + 1 < chars.len() {
                out.push(chars[index + 1]);
                index += 1;
            } else if c == quote {
                in_string = false;
            }
        } else if c == '"' || c == '\'' {
            in_string = true;
            quote = c;
            out.push(c);
        } else if c == '/' && chars.get(index + 1) == Some(&'/') {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }

            continue;
        } else if c == '/' && chars.get(index + 1) == Some(&'*') {
            index += 2;

            while index + 1 < chars.len() && !(chars[index] == '*' && chars[index + 1] == '/') {
                index += 1;
            }

            index += 2;
            continue;
        } else {
            out.push(c);
        }

        index += 1;
    }

    out
}

/// `{'a': 'b'}` -> `{"a": "b"}` - only when the text has no double quotes at all (otherwise it is a different problem).
fn single_to_double(text: &str) -> String {
    if text.contains('"') || !text.contains('\'') {
        return text.to_string();
    }

    text.replace('\'', "\"")
}

/// `{name: "x"}` -> `{"name": "x"}`.
fn unquoted_keys(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    let (mut index, mut in_string) = (0, false);

    while index < chars.len() {
        let c = chars[index];

        if in_string {
            out.push(c);

            if c == '\\' && index + 1 < chars.len() {
                out.push(chars[index + 1]);
                index += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if (c.is_alphabetic() || c == '_') && matches!(out.trim_end().chars().last(), Some('{' | ',')) {
            let mut end = index;

            while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
                end += 1;
            }

            let mut after = end;

            while after < chars.len() && chars[after].is_whitespace() {
                after += 1;
            }

            if chars.get(after) == Some(&':') {
                out.push('"');
                out.extend(&chars[index..end]);
                out.push('"');
                index = end;
                continue;
            }

            out.push(c);
        } else {
            out.push(c);
        }

        index += 1;
    }

    out
}

/// `[1, 2,]` and `{"a": 1,}` -> without the last comma, outside strings.
fn trailing_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut index, mut in_string) = (0, false);

    while index < chars.len() {
        let c = chars[index];

        if in_string {
            out.push(c);

            if c == '\\' && index + 1 < chars.len() {
                out.push(chars[index + 1]);
                index += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == ',' {
            let mut next = index + 1;

            while next < chars.len() && chars[next].is_whitespace() {
                next += 1;
            }

            if !matches!(chars.get(next), Some('}' | ']')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }

        index += 1;
    }

    out
}

/// Appends the closing brackets a text is missing, in the right order; `None` when it is not just "cut short".
fn close_brackets(text: &str) -> Option<String> {
    let mut stack = Vec::new();
    let (mut in_string, mut escaped) = (false, false);

    for c in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else {
            match c {
                '"' => in_string = true,
                '{' => stack.push('}'),
                '[' => stack.push(']'),
                '}' | ']' if stack.pop() != Some(c) => return None,
                _ => {}
            }
        }
    }

    if in_string || stack.is_empty() || stack.len() > 3 {
        return None;
    }

    let mut closed = text.trim_end().trim_end_matches(',').to_string();

    closed.extend(stack.iter().rev());
    Some(closed)
}

/// Finds balanced `{…}` objects in a text (outside nothing special: strings are respected).
fn objects(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut found = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        if chars[index].1 != '{' {
            index += 1;
            continue;
        }

        let (mut depth, mut in_string, mut quote, mut escaped, mut end) = (0i32, false, '"', false, None);

        for (position, (_, c)) in chars.iter().enumerate().skip(index) {
            if in_string {
                if escaped {
                    escaped = false;
                } else if *c == '\\' {
                    escaped = true;
                } else if *c == quote {
                    in_string = false;
                }
            } else if *c == '"' || *c == '\'' {
                in_string = true;
                quote = *c;
            } else if *c == '{' {
                depth += 1;
            } else if *c == '}' {
                depth -= 1;

                if depth == 0 {
                    end = Some(position);
                    break;
                }
            }
        }

        match end {
            Some(position) => {
                found.push((chars[index].0, chars[position].0 + 1));
                index = position + 1;
            }
            None => {
                /* never closed: the model was cut off or stopped short; the repair may close it */
                found.push((chars[index].0, text.len()));
                break;
            }
        }
    }

    found
}

/// A parsed JSON value as a tool call, in any of the shapes models print: `{name, arguments}`, `{tool, parameters}`, `{function: {name, arguments}}`.
fn as_call(value: &Value) -> Option<(String, Value)> {
    let object = value.as_object()?;
    let inner = object.get("function").and_then(Value::as_object).unwrap_or(object);
    let name = ["name", "tool", "tool_name", "function_name"].iter().find_map(|key| inner.get(*key).and_then(Value::as_str))?.to_string();
    let arguments = ["arguments", "parameters", "input", "args"].iter().find_map(|key| inner.get(*key)).cloned().unwrap_or_else(|| json!({}));
    /* arguments may arrive as a JSON string */
    let arguments = match arguments {
        Value::String(text) => repair_json(&text).unwrap_or(Value::String(text)),
        other => other,
    };

    Some((name, arguments))
}

/// `<function=name><parameter=key>value</parameter>...</function>` (Qwen3-Coder).
fn xml_calls(text: &str) -> Vec<(String, Value, (usize, usize))> {
    let mut found = Vec::new();
    let mut from = 0;

    while let Some(start) = text[from..].find("<function=") {
        let start = from + start;
        let Some(name_end) = text[start..].find('>') else {
            break;
        };
        let name = text[start + 10..start + name_end].trim().to_string();
        let body_from = start + name_end + 1;
        let end = text[body_from..].find("</function>").map(|at| body_from + at);
        let body = &text[body_from..end.unwrap_or(text.len())];
        let mut arguments = Map::new();
        let mut cursor = 0;

        while let Some(open) = body[cursor..].find("<parameter=") {
            let open = cursor + open;
            let Some(key_end) = body[open..].find('>') else {
                break;
            };
            let key = body[open + 11..open + key_end].trim().to_string();
            let value_from = open + key_end + 1;
            let close = body[value_from..].find("</parameter>").map(|at| value_from + at).unwrap_or(body.len());
            let raw = body[value_from..close].trim_matches('\n');

            /* a value that parses as a number, boolean, array or object is that; anything else is the text */
            arguments.insert(key, serde_json::from_str::<Value>(raw).ok().filter(|value| !value.is_string()).unwrap_or_else(|| Value::String(raw.to_string())));
            cursor = close + 12;
        }

        let stop = end.map_or(text.len(), |at| at + 11);

        found.push((name, Value::Object(arguments), (start, stop)));
        from = stop;
    }

    found
}

/// Validates arguments against a tool's schema. `Err` says what is wrong. No coercion.
pub fn validate(spec: &ToolSpec, arguments: &Value) -> Result<(), String> {
    let Some(given) = arguments.as_object() else {
        return Err(format!("{}: the arguments are not an object", spec.name));
    };
    let properties = spec.schema["properties"].as_object();

    for required in spec.schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if !given.contains_key(required) {
            return Err(format!("{}: the required argument `{required}` is missing", spec.name));
        }
    }

    for (key, value) in given {
        let Some(declared) = properties.and_then(|properties| properties.get(key)) else {
            if spec.schema["additionalProperties"] == false {
                return Err(format!("{}: `{key}` is not an argument of this tool", spec.name));
            }

            continue;
        };
        let ok = match declared["type"].as_str() {
            Some("string") => value.is_string() && declared["enum"].as_array().is_none_or(|options| options.contains(value)),
            Some("integer") => value.is_i64() || value.is_u64(),
            Some("number") => value.is_number(),
            Some("boolean") => value.is_boolean(),
            Some("array") => value.is_array(),
            Some("object") => value.is_object(),
            _ => true,
        };

        if !ok {
            return Err(format!("{}: `{key}` should be a {} (got {})", spec.name, declared["type"].as_str().unwrap_or("value"), value));
        }
    }

    Ok(())
}

/// Reads a reply's text for printed tool calls.
pub fn rescue(text: &str, specs: &[ToolSpec]) -> Rescued {
    rescue_with(text, specs, true)
}

/// `rescue`, optionally only the tagged forms (`<tool_call>` JSON and Qwen XML) and not fenced or bare JSON objects (PADMA P0-2 tries those last).
pub fn rescue_with(text: &str, specs: &[ToolSpec], bare_objects: bool) -> Rescued {
    let mut spans: Vec<((usize, usize), String, Value)> = Vec::new();
    let mut seen_spans: Vec<(usize, usize)> = Vec::new();
    let overlaps = |spans: &[(usize, usize)], candidate: (usize, usize)| spans.iter().any(|span| candidate.0 < span.1 && span.0 < candidate.1);

    /* 1. <tool_call>…</tool_call> (also unclosed at the end of the reply) */
    let mut from = 0;

    while let Some(start) = text[from..].find("<tool_call>") {
        let start = from + start;
        let body_from = start + 11;
        let close = text[body_from..].find("</tool_call>");
        let body_to = close.map_or(text.len(), |at| body_from + at);
        let stop = close.map_or(text.len(), |at| body_from + at + 12);

        if let Some((name, arguments)) = repair_json(&text[body_from..body_to]).as_ref().and_then(as_call) {
            spans.push(((start, stop), name, arguments));
            seen_spans.push((start, stop));
        }

        from = stop.max(start + 1);
    }

    /* 2. Qwen3-Coder's XML */
    for (name, arguments, span) in xml_calls(text) {
        if !overlaps(&seen_spans, span) {
            seen_spans.push(span);
            spans.push((span, name, arguments));
        }
    }

    /* 3. fenced blocks and bare objects: every balanced object that reads as a call */
    for (start, stop) in if bare_objects { objects(text) } else { Vec::new() } {
        if overlaps(&seen_spans, (start, stop)) {
            continue;
        }

        if let Some((name, arguments)) = repair_json(&text[start..stop]).as_ref().and_then(as_call) {
            /* extend over a surrounding ``` fence, so the fence is taken out of the text too */
            let before = text[..start].trim_end();
            let fence_start = before.rfind("```").filter(|at| before[*at + 3..].lines().count() <= 2).unwrap_or(start);
            let after = &text[stop..];
            let fence_stop = after.trim_start().strip_prefix("```").map_or(stop, |rest| text.len() - rest.len());
            let span = (fence_start.min(start), fence_stop.max(stop));

            seen_spans.push(span);
            spans.push((span, name, arguments));
        }
    }

    spans.sort_by_key(|(span, ..)| span.0);

    let mut result = Rescued::default();

    for (index, (_, name, arguments)) in spans.iter().enumerate() {
        match specs.iter().find(|spec| spec.name == name) {
            None => result.rejected.push(format!("`{name}` is not a tool this turn has")),
            Some(spec) => match validate(spec, arguments) {
                Ok(()) => result.calls.push(ToolUse { id: format!("rescued-{}", index + 1), name: name.clone(), input: arguments.clone() }),
                Err(reason) => result.rejected.push(reason),
            },
        }
    }

    let mut remaining = String::new();
    let mut cursor = 0;

    for ((start, stop), ..) in &spans {
        if *start >= cursor {
            remaining.push_str(&text[cursor..*start]);
            cursor = *stop;
        }
    }

    remaining.push_str(&text[cursor.min(text.len())..]);
    result.text = remaining.trim().to_string();

    /* Prose that merely shows a call: a lot of explanation around the call and the call is not last. Nothing is run. */
    if !result.calls.is_empty() && !spans.is_empty() {
        let last_end = spans.iter().map(|((_, stop), ..)| *stop).max().unwrap_or(0);
        let trailing = text[last_end.min(text.len())..].trim();

        if trailing.chars().count() > 400 {
            result.rejected.push("the call is followed by a long explanation, so it reads as an example and was not run".to_string());
            result.calls.clear();
        }
    }

    result
}

/// **Constrained output (P1-6, switch `sme.constrained`)**: the JSON Schema of one tool call - `{name, arguments}` with `name` one of the
/// tools and `arguments` that tool's own schema. Sent as Ollama's `format` on the retry after a printed call was refused, the
/// server can only produce a call of this shape; the reply is then read by `rescue` like any printed call, and still validated.
pub fn call_schema(specs: &[ToolSpec]) -> Value {
    let any_of: Vec<Value> = specs
        .iter()
        .map(|spec| json!({ "type": "object", "properties": { "name": { "type": "string", "enum": [spec.name] }, "arguments": spec.schema }, "required": ["name", "arguments"] }))
        .collect();

    json!({ "anyOf": any_of })
}

thread_local! {
    static CONSTRAIN_NEXT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Asks for the next request of this turn to be constrained to a tool call.
pub fn constrain_next(on: bool) {
    CONSTRAIN_NEXT.with(|flag| flag.set(on));
}

/// Takes the request (once): whether it is to be constrained.
pub fn take_constrained() -> bool {
    CONSTRAIN_NEXT.with(|flag| flag.replace(false))
}

/// Puts rescued calls into a reply the way the channel would have: `tool_uses`, and the assistant message in OpenAI shape.
/// Returns what was rescued and what was refused, for the note to the model. Does nothing for a reply that already has tool calls.
pub fn apply(reply: &mut super::dialect::Reply, specs: &[ToolSpec]) -> Rescued {
    if !reply.tool_uses.is_empty() || reply.text.trim().is_empty() {
        return Rescued::default();
    }

    let result = rescue(&reply.text, specs);

    if result.calls.is_empty() {
        return result;
    }

    reply.tool_uses = result.calls.clone();
    reply.stop = "tool_calls".to_string();
    reply.text = result.text.clone();

    if reply.message.get("role").is_some_and(|role| role == "assistant") {
        reply.message["content"] = Value::String(result.text.clone());
        reply.message["tool_calls"] = Value::Array(result.calls.iter().map(|call| json!({ "id": call.id, "type": "function", "function": { "name": call.name, "arguments": call.input.to_string() } })).collect());
    }

    result
}

#[cfg(test)]
mod tests;
