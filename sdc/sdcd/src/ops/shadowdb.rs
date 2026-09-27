//! **Shadow database migration** (2.0 in the plan): the migration a deploy would run is rehearsed first
//! on a throwaway copy of the real database, on the same server, and the copy is dropped afterwards.
//!
//! ```text
//!   create sdc_shadow_<id> ─► copy the real database into it ─► schema before ─► run the migration
//!   against the copy ─► schema after ─► the difference, the exit code, the output ─► drop the copy
//! ```
//!
//! The real database is only ever **read** (dumped). The migration command names its database with
//! `{db}` - `DB_DATABASE={db} php artisan migrate --force`, `psql {db} -f migration.sql` - which is what
//! points it at the copy. Credentials are read on the host (wp-config.php, ~/.my.cnf, ~/.pgpass) and never
//! reach SDC.

use serde_json::{json, Value};

use super::{quote, Site};

/// The shell functions that talk to a site's database engine: `(engine, definitions)`.
fn definitions(site: &Site) -> Result<(&'static str, String), String> {
    let db = &site.config["backup"]["db"];
    let read = |name: &str| format!("$(sed -n \"s/.*define( *['\\\"]{name}['\\\"] *, *['\\\"]\\([^'\\\"]*\\)['\\\"].*/\\1/p\" wp-config.php | head -1)");

    match db["kind"].as_str() {
        Some("wordpress") => Ok((
            "mysql",
            format!(
                "DBH=\"{}\"; DBU=\"{}\"; DBP=\"{}\"; DBN=\"{}\"; m() {{ mysql -h\"$DBH\" -u\"$DBU\" -p\"$DBP\" \"$@\"; }}; d() {{ mysqldump -h\"$DBH\" -u\"$DBU\" -p\"$DBP\" \"$@\"; }};",
                read("DB_HOST"),
                read("DB_USER"),
                read("DB_PASSWORD"),
                read("DB_NAME")
            ),
        )),
        Some("mysql") => {
            let name = db["name"].as_str().ok_or("the site's database name is not set")?;

            Ok(("mysql", format!("DBN={}; m() {{ mysql \"$@\"; }}; d() {{ mysqldump \"$@\"; }};", quote(name))))
        }
        Some("postgres") => {
            let name = db["name"].as_str().ok_or("the site's database name is not set")?;

            Ok(("postgres", format!("DBN={}; p() {{ psql -q -v ON_ERROR_STOP=1 \"$@\"; }};", quote(name))))
        }
        _ => Err("This site has no database configured (Backup → Database), so there is nothing to rehearse a migration on.".to_string()),
    }
}

/// The rehearsal script. `command` may use `{db}` for the copy's name.
pub fn script(site: &Site, command: &str, shadow: &str) -> Result<String, String> {
    let (engine, defs) = definitions(site)?;
    let command = command.replace("{db}", shadow);

    Ok(match engine {
        "mysql" => format!(
            "{defs} S={shadow}; T=\"$(mktemp -d)\"; \
             m -e \"CREATE DATABASE \\`$S\\`\" || {{ echo '@@ERROR cannot-create'; exit 5; }}; \
             trap 'm -e \"DROP DATABASE IF EXISTS \\`$S\\`\" >/dev/null 2>&1; rm -rf \"$T\"' EXIT; \
             d --single-transaction --quick \"$DBN\" | m \"$S\" || {{ echo '@@ERROR copy-failed'; exit 6; }}; \
             d --no-data --skip-comments \"$S\" > \"$T/before.sql\"; \
             echo '@@MIGRATION'; ( {command} ) 2>&1; code=$?; echo \"@@EXIT $code\"; \
             d --no-data --skip-comments \"$S\" > \"$T/after.sql\"; \
             echo '@@SCHEMA'; diff \"$T/before.sql\" \"$T/after.sql\" | grep -E '^[<>]' | head -300; echo '@@END'"
        ),
        _ => format!(
            "{defs} S={shadow}; T=\"$(mktemp -d)\"; \
             createdb \"$S\" || {{ echo '@@ERROR cannot-create'; exit 5; }}; \
             trap 'dropdb --if-exists \"$S\" >/dev/null 2>&1; rm -rf \"$T\"' EXIT; \
             pg_dump --no-owner \"$DBN\" | p \"$S\" >/dev/null || {{ echo '@@ERROR copy-failed'; exit 6; }}; \
             pg_dump --schema-only --no-owner \"$S\" > \"$T/before.sql\"; \
             echo '@@MIGRATION'; ( {command} ) 2>&1; code=$?; echo \"@@EXIT $code\"; \
             pg_dump --schema-only --no-owner \"$S\" > \"$T/after.sql\"; \
             echo '@@SCHEMA'; diff \"$T/before.sql\" \"$T/after.sql\" | grep -E '^[<>]' | head -300; echo '@@END'"
        ),
    })
}

/// The rehearsal's output as a result: passed or not, the migration's own output, and the schema change.
pub fn read(output: &str) -> Value {
    let mut section = "";
    let (mut migration, mut schema) = (Vec::new(), Vec::new());
    let mut exit: Option<i64> = None;
    let mut error: Option<String> = None;

    for line in output.lines() {
        if let Some(rest) = line.strip_prefix("@@ERROR ") {
            error = Some(match rest.trim() {
                "cannot-create" => "The database user cannot create a database, so no copy could be made. Give it CREATE rights, or rehearse on a staging server.".to_string(),
                "copy-failed" => "The real database could not be copied into the rehearsal copy.".to_string(),
                other => other.to_string(),
            });
            continue;
        }

        if let Some(code) = line.strip_prefix("@@EXIT ") {
            exit = code.trim().parse().ok();
            continue;
        }

        match line {
            "@@MIGRATION" => section = "migration",
            "@@SCHEMA" => section = "schema",
            "@@END" => section = "",
            _ if section == "migration" => migration.push(line.to_string()),
            _ if section == "schema" => schema.push(line.to_string()),
            _ => {}
        }
    }

    let passed = error.is_none() && exit == Some(0);

    json!({
        "passed": passed,
        "exitCode": exit,
        "error": error,
        "output": migration.iter().rev().take(80).rev().cloned().collect::<Vec<_>>(),
        "schemaChanges": schema,
        "sentence": match (&error, passed) {
            (Some(error), _) => error.clone(),
            (None, true) => format!("The migration ran cleanly on a copy of the real database ({} schema line(s) changed). The copy has been dropped.", schema.len()),
            (None, false) => format!("The migration FAILED on the copy (exit {}). The real database was not touched.", exit.map(|code| code.to_string()).unwrap_or_else(|| "unknown".into())),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(kind: &str) -> Site {
        Site { id: "s".into(), name: "Shop".into(), host_id: "vps".into(), root: "/srv/shop".into(), url: String::new(), config: json!({ "backup": { "db": { "kind": kind, "name": "shop" } } }) }
    }

    #[test]
    fn the_rehearsal_only_reads_the_real_database_and_always_drops_the_copy() {
        let text = script(&site("mysql"), "DB_DATABASE={db} php artisan migrate --force", "sdc_shadow_x").unwrap();

        assert!(text.contains("DB_DATABASE=sdc_shadow_x php artisan migrate"));
        assert!(text.contains("trap 'm -e \"DROP DATABASE IF EXISTS"), "the copy is dropped on any exit");
        assert!(text.contains("d --single-transaction --quick \"$DBN\" | m \"$S\""), "the real database is only dumped");
        assert!(!text.contains("DROP DATABASE IF EXISTS \\`$DBN"), "the real database is never dropped");
        assert!(script(&site("none"), "x", "s").is_err());
    }

    #[test]
    fn a_result_says_pass_or_fail_with_the_schema_change() {
        let passed = read("@@MIGRATION\nMigrating: 2026_09_add_orders\n@@EXIT 0\n@@SCHEMA\n> CREATE TABLE `orders` (\n@@END\n");

        assert_eq!(passed["passed"], true);
        assert_eq!(passed["schemaChanges"].as_array().unwrap().len(), 1);

        let failed = read("@@MIGRATION\nSQLSTATE[42S01]: table exists\n@@EXIT 1\n@@SCHEMA\n@@END\n");

        assert_eq!(failed["passed"], false);
        assert!(failed["sentence"].as_str().unwrap().contains("not touched"));
        assert_eq!(read("@@ERROR cannot-create\n")["passed"], false);
    }
}
