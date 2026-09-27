//! **Micro-staging and the client's approval page** (1.0 in the plan).
//!
//! A copy of the site on the same server - a folder next to it, served on a port (or at a staging URL the
//! owner already points at it) - that a client can open, look at, and approve before the change reaches
//! production. The page is written in the client's language, with the summary, the changes, and the before
//! and after screenshots SDC took.
//!
//! Approving works without anything installed (P1): on a PHP host the page posts to a small
//! `sdc-approve.php` next to it, which records the answer in `.sdc-approval.json` - but only with the
//! page's own random token - and SDC reads that file back (`approval.poll`). Without PHP, the page's
//! buttons open an email to the agency instead.
//!
//! A WordPress copy needs a database of its own - a staging copy writing into the production database is
//! production - so a WordPress site stages only with `staging.db` set, and the copy's `wp-config.php` is
//! pointed at it.

use serde_json::{json, Value};

use super::{quote, Site};

/// The staging folder: the configured one, or `~/.sdc/staging/<site>`.
pub fn folder(site: &Site) -> String {
    site.config["staging"]["path"].as_str().filter(|path| !path.trim().is_empty()).map(str::to_string).unwrap_or_else(|| format!("~/.sdc/staging/{}", site.id))
}

/// The script that makes (or refreshes) the copy and, when no staging URL is configured, serves it on a
/// port. Prints `url:<url>` for the page.
pub fn create_script(site: &Site, host: &str) -> Result<String, String> {
    let target = super::deploy::dir_expr(&folder(site));
    let port = site.config["staging"]["port"].as_u64().unwrap_or(8089);
    let configured_url = site.config["staging"]["url"].as_str().filter(|url| !url.trim().is_empty()).map(str::to_string);
    let url = configured_url.clone().unwrap_or_else(|| format!("http://{host}:{port}/"));
    let excludes = site.backup_excludes().iter().filter(|item| item.as_str() != "node_modules").map(|item| format!("--exclude={}", quote(item))).collect::<Vec<_>>().join(" ");
    let kind = site.config["kind"].as_str().unwrap_or("other");
    let mut script = format!(
        "set -e; S={target}; mkdir -p \"$S\"; \
         if command -v rsync >/dev/null 2>&1; then rsync -a --delete {excludes} ./ \"$S\"/; else cp -a ./. \"$S\"/; fi; echo copied"
    );

    if kind == "wordpress" {
        let staging_db = site.config["staging"]["db"].as_str().filter(|name| !name.trim().is_empty()).ok_or(
            "A WordPress site stages with a database of its own: set Staging → Database (an empty database the site's user can write), so the copy never writes into production.",
        )?;
        let (dump, _) = super::db_commands(site).ok_or("The site's database is not configured.")?;
        let read = |name: &str| format!("$(sed -n \"s/.*define( *['\\\"]{name}['\\\"] *, *['\\\"]\\([^'\\\"]*\\)['\\\"].*/\\1/p\" wp-config.php | head -1)");

        script.push_str(&format!(
            "; {dump} | mysql -h\"{host_db}\" -u\"{user}\" -p\"{password}\" {staging}; \
             sed -i \"s/define( *['\\\"]DB_NAME['\\\"] *, *['\\\"][^'\\\"]*['\\\"] *)/define('DB_NAME', '{staging_plain}')/\" \"$S/wp-config.php\"; \
             grep -q 'SDC staging' \"$S/wp-config.php\" || sed -i \"1a /* SDC staging */ define('WP_HOME', '{url_plain}'); define('WP_SITEURL', '{url_plain}');\" \"$S/wp-config.php\"; echo db-copied",
            host_db = read("DB_HOST"),
            user = read("DB_USER"),
            password = read("DB_PASSWORD"),
            staging = quote(staging_db),
            staging_plain = staging_db.replace(['\'', '"', '/', '\\'], ""),
            url_plain = url.trim_end_matches('/').replace(['\'', '"', '\\'], "").replace('/', "\\/"),
        ));
    }

    if configured_url.is_none() {
        let server = match kind {
            "wordpress" | "php" | "laravel" => format!("php -S 0.0.0.0:{port} -t \"$S\""),
            "node" => format!("sh -c 'cd \"$S\" && PORT={port} npm start'"),
            _ => format!("python3 -m http.server {port} --directory \"$S\""),
        };

        script.push_str(&format!(
            "; [ -f \"$S/.sdc-server.pid\" ] && kill \"$(cat \"$S/.sdc-server.pid\")\" 2>/dev/null || true; \
             nohup {server} > \"$S/.sdc-server.log\" 2>&1 & echo $! > \"$S/.sdc-server.pid\"; sleep 1"
        ));
    }

    script.push_str(&format!("; echo url:{}", quote(&url)));

    Ok(script)
}

/// Stops the temporary server and, when asked, removes the copy.
pub fn stop_script(site: &Site, remove: bool) -> String {
    let target = super::deploy::dir_expr(&folder(site));
    let mut script = format!("S={target}; [ -f \"$S/.sdc-server.pid\" ] && kill \"$(cat \"$S/.sdc-server.pid\")\" 2>/dev/null; rm -f \"$S/.sdc-server.pid\"");

    if remove {
        /* `find -delete`, depth-first, only inside the staging folder SDC made. */
        script.push_str("; [ -d \"$S\" ] && find \"$S\" -depth -delete");
    }

    script.push_str("; echo stopped");

    script
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The approval page's words.
fn words(lang: &str) -> [&'static str; 9] {
    match lang.split('-').next().unwrap_or("en") {
        "bn" => ["অনুমোদনের জন্য পরিবর্তন", "কী বদলেছে", "আগে", "পরে", "অনুমোদন দিন", "প্রশ্ন আছে", "আপনার প্রশ্ন বা মন্তব্য", "ধন্যবাদ — আপনার উত্তর পৌঁছেছে।", "এটা একটা staging কপি; আসল সাইট এখনো বদলায়নি।"],
        "hi" => ["अनुमोदन के लिए बदलाव", "क्या बदला", "पहले", "बाद में", "मंज़ूरी दें", "सवाल है", "आपका सवाल या टिप्पणी", "धन्यवाद — आपका जवाब पहुँच गया।", "यह staging कॉपी है; असली साइट अभी नहीं बदली।"],
        "ar" => ["تغييرات بانتظار موافقتك", "ما الذي تغيّر", "قبل", "بعد", "موافقة", "لدي سؤال", "سؤالك أو ملاحظتك", "شكرًا — وصل ردّك.", "هذه نسخة تجريبية؛ الموقع الحقيقي لم يتغير بعد."],
        "es" => ["Cambios para aprobar", "Qué cambió", "Antes", "Después", "Aprobar", "Tengo una pregunta", "Su pregunta o comentario", "Gracias: su respuesta llegó.", "Esta es una copia de prueba; el sitio real aún no cambió."],
        _ => ["Changes for your approval", "What changed", "Before", "After", "Approve", "I have a question", "Your question or comment", "Thank you - your answer has arrived.", "This is a staging copy; the real site has not changed yet."],
    }
}

/// The page itself. `action` is `php` (post to `sdc-approve.php`) or a `mailto:` address.
#[allow(clippy::too_many_arguments)]
pub fn page(site: &Site, lang: &str, summary: &str, changes: &[String], before: Option<&str>, after: Option<&str>, token: &str, email: Option<&str>, php: bool) -> String {
    let w = words(lang);
    let rtl = matches!(lang.split('-').next(), Some("ar" | "ur" | "fa" | "he"));
    let shots = match (before, after) {
        (Some(before), Some(after)) => format!(
            "<div class=\"shots\"><figure><figcaption>{}</figcaption><img src=\"{}\" alt=\"\"></figure><figure><figcaption>{}</figcaption><img src=\"{}\" alt=\"\"></figure></div>",
            w[2],
            escape(before),
            w[3],
            escape(after)
        ),
        _ => String::new(),
    };
    let list: String = changes.iter().map(|change| format!("<li>{}</li>", escape(change))).collect();
    let actions = if php {
        format!(
            "<form method=\"post\" action=\"sdc-approve.php\"><input type=\"hidden\" name=\"token\" value=\"{token}\"><label>{}<textarea name=\"note\" rows=\"3\"></textarea></label>\
             <div class=\"row\"><button name=\"decision\" value=\"approved\" class=\"primary\">{}</button><button name=\"decision\" value=\"question\">{}</button></div></form>",
            w[6], w[4], w[5]
        )
    } else {
        let to = email.unwrap_or("");

        format!(
            "<div class=\"row\"><a class=\"button primary\" href=\"mailto:{to}?subject={subject}%20-%20approved&body=Approved%20({token})\">{}</a>\
             <a class=\"button\" href=\"mailto:{to}?subject={subject}%20-%20question&body=({token})%20\">{}</a></div>",
            w[4],
            w[5],
            subject = escape(&site.name).replace(' ', "%20")
        )
    };

    format!(
        "<!doctype html><html lang=\"{lang}\" dir=\"{dir}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title} · {name}</title>\
<style>body{{font:16px/1.6 system-ui,'Noto Sans Bengali','Noto Sans Arabic',sans-serif;max-width:900px;margin:0 auto;padding:24px 16px;color:#1f2328;background:#fff}}\
h1{{font-size:22px}}.note{{background:#fff8c5;border-radius:8px;padding:8px 12px}}.summary{{background:#f6f8fa;border-radius:8px;padding:12px}}\
.shots{{display:grid;grid-template-columns:repeat(auto-fit,minmax(280px,1fr));gap:12px}}figure{{margin:0}}img{{width:100%;border:1px solid #d0d7de;border-radius:8px}}\
.row{{display:flex;gap:10px;flex-wrap:wrap;margin-top:12px}}button,.button{{font:inherit;padding:10px 18px;border-radius:8px;border:1px solid #d0d7de;background:#f6f8fa;cursor:pointer;text-decoration:none;color:inherit}}\
.primary{{background:#1a7f37;border-color:#1a7f37;color:#fff}}textarea{{width:100%;box-sizing:border-box;font:inherit;margin-top:6px}}\
@media (prefers-color-scheme:dark){{body{{background:#0d1117;color:#e6edf3}}.summary{{background:#161b22}}.note{{background:#3b2e00}}button,.button{{background:#21262d;border-color:#30363d}}}}</style></head>\
<body><h1>{title}: {name}</h1><p class=\"note\">{note}</p><div class=\"summary\" dir=\"auto\">{summary}</div><h2>{what}</h2><ul dir=\"auto\">{list}</ul>{shots}{actions}</body></html>",
        lang = escape(lang),
        dir = if rtl { "rtl" } else { "ltr" },
        title = w[0],
        name = escape(&site.name),
        note = w[8],
        summary = escape(summary),
        what = w[1],
    )
}

/// The receiver on a PHP host: it accepts one answer, only with the page's token, and writes it where
/// SDC reads it. It runs no command and includes nothing.
pub fn receiver(token: &str, lang: &str) -> String {
    let thanks = words(lang)[7].replace('\'', "&#39;");

    format!(
        "<?php\n// SDC approval receiver: records one answer for this staging copy. Nothing else.\n\
         if (($_POST['token'] ?? '') !== '{token}') {{ http_response_code(403); exit('forbidden'); }}\n\
         $decision = ($_POST['decision'] ?? '') === 'approved' ? 'approved' : 'question';\n\
         $note = substr(strip_tags((string)($_POST['note'] ?? '')), 0, 2000);\n\
         file_put_contents(__DIR__ . '/.sdc-approval.json', json_encode(['token' => '{token}', 'decision' => $decision, 'note' => $note, 'at' => gmdate('c')]));\n\
         header('Content-Type: text/html; charset=utf-8');\necho '<!doctype html><meta charset=\"utf-8\"><p style=\"font:18px system-ui;padding:24px\">{thanks}</p>';\n"
    )
}

/// Reads `.sdc-approval.json` back: the decision, when it carries the right token.
pub fn read_answer(text: &str, token: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(text.trim()).ok()?;

    (value["token"] == token).then(|| json!({ "decision": value["decision"], "note": value["note"], "at": value["at"] }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(kind: &str, staging: Value) -> Site {
        Site { id: "s1".into(), name: "Shop".into(), host_id: "vps".into(), root: "/srv/shop".into(), url: "https://shop.test".into(), config: json!({ "kind": kind, "staging": staging, "backup": { "db": { "kind": "wordpress" } } }) }
    }

    #[test]
    fn a_wordpress_copy_refuses_to_share_the_production_database() {
        assert!(create_script(&site("wordpress", json!({})), "1.2.3.4").unwrap_err().contains("database of its own"));

        let script = create_script(&site("wordpress", json!({ "db": "shop_staging" })), "1.2.3.4").unwrap();

        assert!(script.contains("define('DB_NAME', 'shop_staging')"));
        assert!(script.contains("php -S 0.0.0.0:8089"));
        assert!(script.ends_with("echo url:'http://1.2.3.4:8089/'"));
    }

    #[test]
    fn a_static_site_is_served_by_python_and_a_configured_url_is_used_as_is() {
        assert!(create_script(&site("static", json!({})), "h").unwrap().contains("python3 -m http.server 8089"));

        let configured = create_script(&site("static", json!({ "url": "https://staging.shop.test" })), "h").unwrap();

        assert!(!configured.contains("nohup"));
        assert!(configured.contains("url:'https://staging.shop.test'"));
    }

    #[test]
    fn the_page_speaks_the_clients_language_and_the_receiver_checks_the_token() {
        let page = page(&site("static", json!({})), "bn", "ফর্ম ঠিক করা হয়েছে", &["contact.php".into()], Some("data:image/png;base64,AA"), Some("data:image/png;base64,BB"), "tok123", None, true);

        assert!(page.contains("অনুমোদন দিন"));
        assert!(page.contains("value=\"tok123\""));
        assert!(page.contains("data:image/png;base64,BB"));

        let php = receiver("tok123", "bn");

        assert!(php.contains("!== 'tok123'"));
        assert!(!php.contains("exec(") && !php.contains("system(") && !php.contains("include"));
        assert_eq!(read_answer(r#"{"token":"tok123","decision":"approved","note":"ok","at":"x"}"#, "tok123").unwrap()["decision"], "approved");
        assert!(read_answer(r#"{"token":"other","decision":"approved"}"#, "tok123").is_none());
    }

    #[test]
    fn stopping_removes_only_the_staging_folder() {
        let script = stop_script(&site("static", json!({})), true);

        assert!(script.contains("find \"$S\" -depth -delete"));
        assert!(crate::pty::denied_reason_line(&script).is_none());
    }
}
