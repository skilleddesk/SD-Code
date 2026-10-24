//! The Proof Pack: what a turn did, proved, in one file (the pipeline's last step, "prove").
//!
//! A pack is built from the record, never from a summary a model wrote: the turn's own events (which
//! files, which commands, which checks), its Verify run (checks, secret and SAST scans, the second AI's
//! verdict), its Trust score with the reasons, its cost with where the number came from, its rollback
//! point, and the audit ledger's rows for the turn with the chain's state - so a client or a reviewer
//! can see that the record has not been edited since.
//!
//! Two forms: the JSON (for a machine, or another AI), and an HTML report in the person's language that
//! a client can open with no software at all.

use serde_json::{json, Value};

use crate::sdcp::events::StoredEvent;
use crate::store::sqlite::Store;

/// The pack for one turn (or, with `turn_id: None`, for every turn of a chat).
pub fn build(store: &Store, log: &[StoredEvent], session_id: &str, turn_id: Option<&str>) -> Value {
    let turns: Vec<String> = match turn_id {
        Some(turn) => vec![turn.to_string()],
        None => store
            .turns(session_id)
            .unwrap_or_default()
            .iter()
            .filter_map(|turn| turn["turnId"].as_str().map(str::to_string))
            .collect(),
    };
    let session = store.session(session_id).ok().flatten().unwrap_or(Value::Null);
    let root = store.session_project_root(session_id).ok().flatten();
    let policy = crate::trust::policy::Policy::load(root.as_deref(), None);
    let mut items = Vec::new();

    for turn in &turns {
        let row = store.turn_row(turn).ok().flatten().unwrap_or(Value::Null);
        let events: Vec<&StoredEvent> = log
            .iter()
            .filter(|entry| entry.turn_id.as_deref() == Some(turn.as_str()) || entry.event["turnId"] == turn.as_str())
            .collect();
        let mut files: Vec<Value> = Vec::new();
        let mut commands: Vec<Value> = Vec::new();
        let mut outcomes: std::collections::HashMap<String, (String, String)> = Default::default();

        for entry in &events {
            if entry.event["type"] == "ToolCallCompleted" {
                outcomes.insert(
                    entry.event["callId"].as_str().unwrap_or_default().to_string(),
                    (entry.event["status"].as_str().unwrap_or_default().to_string(), entry.event["meta"].as_str().unwrap_or_default().to_string()),
                );
            }
        }

        for entry in &events {
            if entry.event["type"] != "ToolCallStarted" {
                continue;
            }

            let call = entry.event["callId"].as_str().unwrap_or_default();
            let (status, meta) = outcomes.get(call).cloned().unwrap_or_default();
            let target = entry.event["target"].as_str().unwrap_or_default();

            match entry.event["tool"].as_str() {
                Some("edit") => files.push(json!({ "path": target, "action": entry.event["name"], "status": status, "meta": meta })),
                Some("run") => commands.push(json!({ "command": target, "status": status, "meta": meta })),
                _ => {}
            }
        }

        let verify = events
            .iter()
            .rev()
            .find(|entry| entry.event["type"] == "VerifyUpdated" && entry.event["state"] == "done")
            .map(|entry| entry.event.clone());
        let checkpoint = events
            .iter()
            .find(|entry| entry.event["type"] == "CheckpointSaved")
            .map(|entry| entry.event["checkpoint"].clone());
        let facts = crate::trust::score::facts(log, turn, &policy, root.as_deref());
        let (score, level, reasons) = crate::trust::score::turn(&facts, policy.max_files_per_turn);
        let audit: Vec<Value> = store
            .audit_rows(None, Some(turn), 500)
            .unwrap_or_default()
            .iter()
            .rev()
            .map(|row| json!({ "seq": row.seq, "ts": row.ts, "actor": row.actor, "kind": row.kind, "summary": row.summary, "hash": row.hash }))
            .collect();

        items.push(json!({
            "turnId": turn,
            "prompt": row["prompt"],
            "engine": row["engine"],
            "model": row["model"],
            "startedAt": row["startedAt"],
            "finishedAt": row["finishedAt"],
            "state": row["state"],
            "answer": row["answer"].as_str().map(|answer| answer.chars().take(4000).collect::<String>()),
            "files": files,
            "commands": commands,
            "verify": verify.map(|event| json!({
                "pass": event["pass"],
                "verdict": event["verdict"],
                "checks": event["checks"],
                "review": event["review"],
                "scans": event["scans"],
            })),
            "trust": { "score": score, "level": level, "reasons": reasons },
            "cost": store.usage_of(turn).ok().flatten(),
            /* M1-7: what went to the chain, from the turn's own relay events - null for a turn that never relayed or counted. */
            "relay": relay_of(&events),
            /* M4-3: the closing contract's three parts, filled from the record - not from what the model said it did. */
            "completion": if crate::v3::on("turn.contract") { completion_of(&files, &commands, verify_ref(&events), &row["state"]) } else { Value::Null },
            "rollbackPoint": checkpoint,
            "audit": audit,
        }));
    }

    json!({
        "format": "sdc-proof-pack/1",
        "generatedAt": chrono::Utc::now().to_rfc3339(),
        "sdcd": crate::VERSION,
        "session": { "id": session_id, "title": session["title"], "projectRoot": root },
        "policy": { "source": policy.source, "production": policy.production, "maxFilesPerTurn": policy.max_files_per_turn, "privacy": policy.privacy },
        "ledger": crate::trust::ledger::verify(store),
        "turns": items,
    })
}

fn verify_ref<'a>(events: &'a [&StoredEvent]) -> Option<&'a Value> {
    events.iter().rev().find(|entry| entry.event["type"] == "VerifyUpdated" && entry.event["state"] == "done").map(|entry| &entry.event)
}

/// What changed, what was verified and what was not done, from the pack's own record: the files and commands the tools reported, Verify's
/// checks and the turn's state. A part with nothing in it says so in words - "nothing" is a statement, an empty list is a silence.
pub fn completion_of(files: &[Value], commands: &[Value], verify: Option<&Value>, state: &Value) -> Value {
    let mut changed = Vec::new();
    let mut not_done = Vec::new();

    for file in files {
        let line = format!("{} ({}{})", file["path"].as_str().unwrap_or_default(), file["action"].as_str().unwrap_or("edit").to_lowercase(), file["meta"].as_str().filter(|meta| !meta.is_empty()).map(|meta| format!(", {meta}")).unwrap_or_default());

        if file["status"] == "done" || file["status"].is_null() {
            changed.push(line);
        } else {
            not_done.push(format!("{} was not changed ({})", file["path"].as_str().unwrap_or_default(), file["status"].as_str().unwrap_or("failed")));
        }
    }

    let mut verified = Vec::new();

    match verify {
        Some(event) => {
            for check in event["checks"].as_array().cloned().unwrap_or_default() {
                let name = check["name"].as_str().or_else(|| check["command"].as_str()).unwrap_or("a check");

                match check["status"].as_str() {
                    Some("pass") => verified.push(format!("{name} passed")),
                    Some("fail") => not_done.push(format!("{name} failed")),
                    _ => not_done.push(format!("{name} did not finish")),
                }
            }

            if event["pass"] == false && verified.is_empty() {
                not_done.push("Verify did not pass".to_string());
            }
        }
        None => {
            if !changed.is_empty() {
                not_done.push("Verify was not run on this turn".to_string());
            }
        }
    }

    let ran = commands.iter().filter(|command| command["status"] == "done").count();

    if ran > 0 && verified.is_empty() {
        verified.push(format!("{ran} command(s) ran; Verify did not check the change"));
    }

    if state != "done" && !state.is_null() && state != "completed" {
        not_done.push(format!("the turn ended as `{}`", state.as_str().unwrap_or("unfinished")));
    }

    json!({
        "changed": if changed.is_empty() { vec!["no files".to_string()] } else { changed },
        "verified": if verified.is_empty() { vec!["nothing ran".to_string()] } else { verified },
        "notDone": if not_done.is_empty() { vec!["nothing".to_string()] } else { not_done },
    })
}

/// The relay part of a turn's pack: its summary and every attempt, as the events recorded them. `Null` for a turn with neither.
fn relay_of(events: &[&StoredEvent]) -> Value {
    let summary = events.iter().rev().find(|entry| entry.event["type"] == "RelaySummary").map(|entry| entry.event.clone());
    let steps: Vec<Value> = events.iter().filter(|entry| entry.event["type"] == "RelayStep").map(|entry| entry.event.clone()).collect();

    match (summary, steps.is_empty()) {
        (None, true) => Value::Null,
        (summary, _) => json!({ "summary": summary, "steps": steps }),
    }
}

/// The report's words, in the languages SDC writes reports in; English for anything else.
fn words(lang: &str) -> [&'static str; 28] {
    match lang.split('-').next().unwrap_or("en") {
        "bn" => [
            "প্রমাণপত্র", "অনুরোধ", "ইঞ্জিন", "যা বদলেছে", "যে কমান্ড চলেছে", "যাচাই", "পাস", "পাস হয়নি", "যাচাই হয়নি",
            "বিশ্বাস স্কোর", "খরচ", "ফেরার বিন্দু", "অডিট লেজার", "অক্ষত — কেউ বদলায়নি", "ভাঙা — এই সারিতে বদল ধরা পড়েছে",
            "দ্বিতীয় AI-এর রায়", "গোপন তথ্য (secret)", "ঝুঁকিপূর্ণ কোড", "কিছু পাওয়া যায়নি", "আনুমানিক", "মাপা", "তৈরি",
            "সাইলেন্ট রিলে", "লোকাল ধাপ", "ক্লাউড চেইনের ধাপ",
            "যা বদলেছে (রেকর্ড থেকে)", "যা যাচাই হয়েছে", "যা করা হয়নি",
        ],
        "hi" => [
            "प्रमाण पत्र", "अनुरोध", "इंजन", "क्या बदला", "कौन से कमांड चले", "जाँच", "पास", "पास नहीं", "जाँच नहीं हुई",
            "भरोसा स्कोर", "खर्च", "वापसी बिंदु", "ऑडिट लेजर", "सुरक्षित — किसी ने नहीं बदला", "टूटा — इस पंक्ति में बदलाव मिला",
            "दूसरे AI का निर्णय", "गुप्त जानकारी", "जोखिम भरा कोड", "कुछ नहीं मिला", "अनुमानित", "मापा गया", "बनाया",
            "साइलेंट रिले", "लोकल चरण", "चेन के चरण",
            "क्या बदला (रिकॉर्ड से)", "क्या जाँचा गया", "क्या नहीं हुआ",
        ],
        "ar" => [
            "حزمة الإثبات", "الطلب", "المحرك", "ما الذي تغيّر", "الأوامر التي نُفّذت", "التحقق", "نجح", "لم ينجح", "لم يُتحقق",
            "درجة الثقة", "التكلفة", "نقطة الاستعادة", "سجل التدقيق", "سليم — لم يُعدَّل", "مكسور — تغيير في هذا الصف",
            "رأي الذكاء الثاني", "أسرار", "شيفرة خطرة", "لم يُعثر على شيء", "تقديري", "مُقاس", "أُنشئ",
            "التتابع الصامت", "خطوات محلية", "خطوات السلسلة",
            "ما تغيّر (من السجل)", "ما تم التحقق منه", "ما لم يُنجز",
        ],
        "es" => [
            "Paquete de prueba", "Solicitud", "Motor", "Qué cambió", "Comandos ejecutados", "Verificación", "Aprobado", "No aprobado", "Sin verificar",
            "Puntuación de confianza", "Costo", "Punto de restauración", "Registro de auditoría", "Intacto: nadie lo modificó", "Roto: cambio en esta fila",
            "Veredicto de la segunda IA", "Secretos", "Código riesgoso", "No se encontró nada", "estimado", "medido", "Generado",
            "Relevo silencioso", "pasos locales", "pasos de la cadena",
            "Qué cambió (según el registro)", "Qué se verificó", "Qué no se hizo",
        ],
        _ => [
            "Proof Pack", "Request", "Engine", "What changed", "Commands that ran", "Verification", "Passed", "Did not pass", "Not verified",
            "Trust score", "Cost", "Rollback point", "Audit ledger", "Intact - nobody has edited it", "Broken - a change was found at this row",
            "Second AI's verdict", "Secrets", "Risky code", "Nothing found", "estimated", "measured", "Generated",
            "Silent Relay", "local steps", "chain steps",
            "What changed (from the record)", "What was verified", "What was not done",
        ],
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The pack as a self-contained HTML page, in `lang`. Right-to-left for Arabic script.
pub fn html(pack: &Value, lang: &str) -> String {
    let w = words(lang);
    let rtl = matches!(lang.split('-').next(), Some("ar" | "ur" | "fa" | "he"));
    let mut body = String::new();

    for turn in pack["turns"].as_array().cloned().unwrap_or_default() {
        let trust = &turn["trust"];
        let level_color = match trust["level"].as_str() {
            Some("high") => "#1a7f37",
            Some("medium") => "#9a6700",
            _ => "#cf222e",
        };

        body.push_str(&format!(
            "<section><h2>{}</h2><p class=\"prompt\" dir=\"auto\">{}</p><p class=\"meta\">{} · {} · {}</p>",
            w[1],
            escape(turn["prompt"].as_str().unwrap_or_default()),
            escape(turn["engine"].as_str().unwrap_or_default()),
            escape(turn["model"].as_str().unwrap_or_default()),
            escape(turn["startedAt"].as_str().unwrap_or_default()),
        ));
        body.push_str(&format!(
            "<div class=\"score\" style=\"border-color:{level_color}\"><strong style=\"color:{level_color}\">{} {}/100</strong><ul>",
            w[9], trust["score"]
        ));

        for reason in trust["reasons"].as_array().cloned().unwrap_or_default() {
            let delta = reason["delta"].as_i64().unwrap_or(0);

            body.push_str(&format!(
                "<li>{}{}</li>",
                escape(reason["text"].as_str().unwrap_or_default()),
                if delta == 0 { String::new() } else { format!(" <span class=\"delta\">({delta})</span>") }
            ));
        }

        body.push_str("</ul></div>");
        body.push_str(&format!("<h3>{}</h3><ul class=\"mono\" dir=\"ltr\">", w[3]));

        let files = turn["files"].as_array().cloned().unwrap_or_default();

        if files.is_empty() {
            body.push_str(&format!("<li>{}</li>", w[18]));
        }

        for file in files {
            body.push_str(&format!(
                "<li>{} <b>{}</b> <span class=\"meta\">{}</span></li>",
                escape(file["action"].as_str().unwrap_or_default()),
                escape(file["path"].as_str().unwrap_or_default()),
                escape(file["meta"].as_str().unwrap_or_default())
            ));
        }

        body.push_str(&format!("</ul><h3>{}</h3><ul class=\"mono\" dir=\"ltr\">", w[4]));

        let commands = turn["commands"].as_array().cloned().unwrap_or_default();

        if commands.is_empty() {
            body.push_str(&format!("<li>{}</li>", w[18]));
        }

        for command in commands {
            body.push_str(&format!(
                "<li><code>{}</code> <span class=\"meta\">{}</span></li>",
                escape(command["command"].as_str().unwrap_or_default()),
                escape(command["meta"].as_str().unwrap_or_default())
            ));
        }

        body.push_str(&format!("</ul><h3>{}</h3>", w[5]));

        match turn["verify"].as_object() {
            None => body.push_str(&format!("<p class=\"warn\">{}</p>", w[8])),
            Some(verify) => {
                let passed = verify.get("pass").and_then(Value::as_bool).unwrap_or(false);

                body.push_str(&format!("<p class=\"{}\">{}</p><ul class=\"mono\" dir=\"ltr\">", if passed { "ok" } else { "bad" }, if passed { w[6] } else { w[7] }));

                for check in verify.get("checks").and_then(Value::as_array).cloned().unwrap_or_default() {
                    body.push_str(&format!(
                        "<li>{} — {}</li>",
                        escape(check["command"].as_str().or(check["name"].as_str()).unwrap_or_default()),
                        escape(check["status"].as_str().unwrap_or_default())
                    ));
                }

                body.push_str("</ul>");

                let scans = verify.get("scans").cloned().unwrap_or(Value::Null);
                let secrets = scans["secrets"].as_array().map(Vec::len).unwrap_or(0);
                let sast = scans["sast"].as_array().map(Vec::len).unwrap_or(0);

                body.push_str(&format!(
                    "<p>{}: {} · {}: {}</p>",
                    w[16],
                    if secrets == 0 { w[18].to_string() } else { secrets.to_string() },
                    w[17],
                    if sast == 0 { w[18].to_string() } else { sast.to_string() }
                ));

                if let Some(review) = verify.get("review").filter(|review| !review.is_null()) {
                    body.push_str(&format!(
                        "<p>{}: <b>{}</b> ({}) — {}</p>",
                        w[15],
                        escape(review["verdict"].as_str().unwrap_or("-")),
                        escape(review["model"].as_str().unwrap_or_default()),
                        escape(review["summary"].as_str().unwrap_or_default())
                    ));
                }
            }
        }

        if let Some(cost) = turn["cost"].as_object() {
            let source = cost.get("costSource").and_then(Value::as_str).unwrap_or("none");

            body.push_str(&format!(
                "<h3>{}</h3><p>${:.4} · {} · {} in / {} out</p>",
                w[10],
                cost.get("costUsd").and_then(Value::as_f64).unwrap_or(0.0),
                if source == "measured" || source == "priced" { w[20] } else { w[19] },
                cost.get("inputTokens").and_then(Value::as_u64).unwrap_or(0),
                cost.get("outputTokens").and_then(Value::as_u64).unwrap_or(0)
            ));
        }

        if let Some(completion) = turn["completion"].as_object() {
            for (title, key) in [(w[25], "changed"), (w[26], "verified"), (w[27], "notDone")] {
                body.push_str(&format!("<h3>{title}</h3><ul>"));

                for line in completion.get(key).and_then(Value::as_array).cloned().unwrap_or_default() {
                    body.push_str(&format!("<li>{}</li>", escape(line.as_str().unwrap_or_default())));
                }

                body.push_str("</ul>");
            }
        }

        if let Some(summary) = turn["relay"]["summary"].as_object().filter(|summary| summary.get("attempts").and_then(Value::as_u64).unwrap_or(0) > 0) {
            body.push_str(&format!(
                "<h3>{}</h3><p>{}: {} · {}: {} · ${:.4} ({})</p>",
                w[22],
                w[23],
                summary.get("stepsLocal").and_then(Value::as_u64).unwrap_or(0),
                w[24],
                summary.get("stepsCloud").and_then(Value::as_u64).unwrap_or(0),
                summary.get("costUsd").and_then(Value::as_f64).unwrap_or(0.0),
                escape(summary.get("costLabel").and_then(Value::as_str).unwrap_or("estimate")),
            ));

            for step in turn["relay"]["steps"].as_array().cloned().unwrap_or_default() {
                body.push_str(&format!(
                    "<p class=\"mono\" dir=\"ltr\">{} → {} · {} · {}</p>",
                    escape(step["fromModel"].as_str().unwrap_or_default()),
                    escape(step["toModel"].as_str().unwrap_or_default()),
                    escape(step["reason"].as_str().unwrap_or_default()),
                    escape(step["outcome"].as_str().unwrap_or_default()),
                ));
            }
        }

        if let Some(checkpoint) = turn["rollbackPoint"].as_object() {
            body.push_str(&format!(
                "<h3>{}</h3><p class=\"mono\" dir=\"ltr\">{} · {}</p>",
                w[11],
                escape(checkpoint.get("title").and_then(Value::as_str).unwrap_or_default()),
                escape(checkpoint.get("filesHash").and_then(Value::as_str).unwrap_or_default())
            ));
        }

        body.push_str("</section>");
    }

    let ledger = &pack["ledger"];
    let intact = ledger["intact"].as_bool().unwrap_or(false);

    format!(
        "<!doctype html><html lang=\"{lang}\" dir=\"{dir}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title} · {session}</title>\
<style>body{{font:15px/1.6 system-ui,'Noto Sans Bengali','Noto Sans Arabic',sans-serif;max-width:860px;margin:0 auto;padding:24px 16px;color:#1f2328;background:#fff}}\
h1{{font-size:22px;margin:0 0 4px}}h2{{font-size:17px;margin:28px 0 6px;border-top:1px solid #d0d7de;padding-top:18px}}h3{{font-size:14px;margin:16px 0 4px;color:#57606a}}\
.meta{{color:#57606a;font-size:13px}}.mono{{font-family:ui-monospace,Consolas,monospace;font-size:13px}}.prompt{{background:#f6f8fa;border-radius:8px;padding:10px 12px}}\
.score{{border-left:4px solid;padding:6px 12px;margin:10px 0;background:#f6f8fa;border-radius:6px}}.delta{{color:#57606a}}.ok{{color:#1a7f37;font-weight:600}}.bad{{color:#cf222e;font-weight:600}}.warn{{color:#9a6700;font-weight:600}}\
@media (prefers-color-scheme:dark){{body{{background:#0d1117;color:#e6edf3}}.prompt,.score{{background:#161b22}}h2{{border-color:#30363d}}.meta,h3,.delta{{color:#8b949e}}}}</style></head>\
<body><h1>{title}</h1><p class=\"meta\">{session} · {generated_label} {generated}</p>{body}\
<h2>{ledger_label}</h2><p class=\"{ledger_class}\">{ledger_text}</p><p class=\"meta mono\" dir=\"ltr\">{entries} · {head}</p></body></html>",
        lang = escape(lang),
        dir = if rtl { "rtl" } else { "ltr" },
        title = w[0],
        session = escape(pack["session"]["title"].as_str().unwrap_or("SDC")),
        generated_label = w[21],
        generated = escape(pack["generatedAt"].as_str().unwrap_or_default()),
        ledger_label = w[12],
        ledger_class = if intact { "ok" } else { "bad" },
        ledger_text = if intact { w[13].to_string() } else { format!("{} #{}", w[14], ledger["brokenAt"]) },
        entries = ledger["entries"],
        head = escape(ledger["head"].as_str().unwrap_or_default()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_proves_what_a_turn_did_and_renders_in_bengali() {
        let store = Store::in_memory().unwrap();

        store.upsert_host("local", "Local", "local", None, "connected", None).unwrap();
        store.insert_session("s1", "local", "Fix the form", "fix", None).unwrap();
        store.start_turn("turn-1", "s1", 1, "native_api", "claude-opus-5-5", "Deep", "ফর্ম ঠিক করো").unwrap();

        let log = vec![
            StoredEvent { seq: 1, ts: String::new(), session_id: Some("s1".into()), turn_id: Some("turn-1".into()), event: json!({ "type": "CheckpointSaved", "checkpoint": { "title": "Before Edit form.php", "filesHash": "abc" } }) },
            StoredEvent { seq: 2, ts: String::new(), session_id: Some("s1".into()), turn_id: Some("turn-1".into()), event: json!({ "type": "ToolCallStarted", "callId": "c1", "tool": "edit", "name": "Edit", "target": "form.php" }) },
            StoredEvent { seq: 3, ts: String::new(), session_id: Some("s1".into()), turn_id: Some("turn-1".into()), event: json!({ "type": "ToolCallCompleted", "callId": "c1", "status": "done", "meta": "+3 −1" }) },
        ];
        let pack = build(&store, &log, "s1", Some("turn-1"));

        assert_eq!(pack["turns"][0]["files"][0]["path"], "form.php");
        assert_eq!(pack["turns"][0]["trust"]["score"], 70, "changed, not verified: capped");
        assert_eq!(pack["ledger"]["intact"], true);

        let page = html(&pack, "bn");

        assert!(page.contains("প্রমাণপত্র"));
        assert!(page.contains("যাচাই হয়নি"));
        assert!(page.contains("form.php"));
        assert!(html(&pack, "ar").contains("dir=\"rtl\""));
    }

    #[test]
    fn a_turn_that_relayed_carries_its_attempts_and_split_into_the_pack_and_the_page() {
        let store = Store::in_memory().unwrap();

        store.upsert_host("local", "Local", "local", None, "connected", None).unwrap();
        store.insert_session("s1", "local", "Relay", "fix", None).unwrap();
        store.start_turn("turn-r", "s1", 1, "native_api", "qwen3.5:9b", "Fast", "fix it").unwrap();
        store.start_turn("turn-plain", "s1", 2, "native_api", "qwen3.5:9b", "Fast", "read it").unwrap();

        let step = json!({ "type": "RelayStep", "turnId": "turn-r", "fromModel": "ollama:qwen3.5:9b", "toModel": "deepseek:deepseek-chat", "reason": "loop", "outcome": "done" });
        let summary = json!({ "type": "RelaySummary", "turnId": "turn-r", "stepsLocal": 4, "stepsCloud": 1, "attempts": 1, "costUsd": 0.002, "costLabel": "priced" });
        let log = vec![
            StoredEvent { seq: 1, ts: String::new(), session_id: Some("s1".into()), turn_id: Some("turn-r".into()), event: step },
            StoredEvent { seq: 2, ts: String::new(), session_id: Some("s1".into()), turn_id: Some("turn-r".into()), event: summary },
        ];
        let pack = build(&store, &log, "s1", None);
        let relayed = pack["turns"].as_array().unwrap().iter().find(|turn| turn["turnId"] == "turn-r").unwrap();
        let plain = pack["turns"].as_array().unwrap().iter().find(|turn| turn["turnId"] == "turn-plain").unwrap();

        assert_eq!(relayed["relay"]["summary"]["stepsCloud"], 1);
        assert_eq!(relayed["relay"]["steps"][0]["toModel"], "deepseek:deepseek-chat");
        assert!(plain["relay"].is_null(), "a turn that never relayed has no relay part");

        let page = html(&pack, "bn");

        assert!(page.contains("সাইলেন্ট রিলে") && page.contains("deepseek:deepseek-chat") && page.contains("loop"), "{page}");
        assert!(html(&pack, "en").contains("Silent Relay"));
    }

    #[test]
    fn the_closing_contract_is_filled_from_the_record_and_not_from_what_the_model_said() {
        let files = vec![json!({ "path": "pay.py", "action": "Edit", "status": "done", "meta": "+3 −1" }), json!({ "path": "locked.py", "action": "Edit", "status": "declined", "meta": "" })];
        let commands = vec![json!({ "command": "python3 -m unittest", "status": "done", "meta": "" })];
        let verify = json!({ "pass": false, "checks": [{ "name": "unit tests", "status": "pass" }, { "name": "lint", "status": "fail" }] });
        let done = completion_of(&files, &commands, Some(&verify), &json!("done"));

        assert_eq!(done["changed"], json!(["pay.py (edit, +3 −1)"]));
        assert_eq!(done["verified"], json!(["unit tests passed"]));
        assert_eq!(done["notDone"], json!(["locked.py was not changed (declined)", "lint failed"]));

        let bare = completion_of(&[], &[], None, &json!("done"));

        assert_eq!((bare["changed"].clone(), bare["verified"].clone(), bare["notDone"].clone()), (json!(["no files"]), json!(["nothing ran"]), json!(["nothing"])));

        let unverified = completion_of(&files[..1], &commands, None, &json!("done"));

        assert_eq!(unverified["verified"], json!(["1 command(s) ran; Verify did not check the change"]));
        assert_eq!(unverified["notDone"], json!(["Verify was not run on this turn"]));
        assert_eq!(completion_of(&[], &[], None, &json!("stopped"))["notDone"], json!(["the turn ended as `stopped`"]));
    }

    #[test]
    fn the_pack_carries_the_completion_only_when_the_contract_switch_is_on() {
        let store = Store::in_memory().unwrap();

        store.upsert_host("local", "Local", "local", None, "connected", None).unwrap();
        store.insert_session("s1", "local", "T", "fix", None).unwrap();
        store.start_turn("turn-c", "s1", 1, "native_api", "m", "Fast", "do it").unwrap();

        let log = vec![
            StoredEvent { seq: 1, ts: String::new(), session_id: Some("s1".into()), turn_id: Some("turn-c".into()), event: json!({ "type": "ToolCallStarted", "callId": "c1", "tool": "edit", "name": "Edit", "target": "a.py" }) },
            StoredEvent { seq: 2, ts: String::new(), session_id: Some("s1".into()), turn_id: Some("turn-c".into()), event: json!({ "type": "ToolCallCompleted", "callId": "c1", "status": "done", "meta": "+1 −0" }) },
        ];

        assert!(build(&store, &log, "s1", Some("turn-c"))["turns"][0]["completion"].is_null());

        let on = crate::v3::forcing(&["turn.contract"], || build(&store, &log, "s1", Some("turn-c")));

        assert_eq!(on["turns"][0]["completion"]["changed"], json!(["a.py (edit, +1 −0)"]));

        let page = crate::v3::forcing(&["turn.contract"], || html(&on, "bn"));

        assert!(page.contains("যা বদলেছে (রেকর্ড থেকে)") && page.contains("a.py"));
    }
}
