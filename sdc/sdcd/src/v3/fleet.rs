//! **Fleet fan-out** (v3 I7): one command on many servers, and what comes back is *clustered* before a model sees
//! it - "17 identical (sample from web-03); 3 differ: …" instead of twenty copies of the same output.
//!
//! Outputs are compared after normalising what always differs between machines and runs: timestamps and clock
//! times, PIDs, UUIDs and long hex ids, IP addresses, durations, the host's own name. Normalising is for *grouping*
//! only: every group shows a real (un-normalised) sample, and every output that differs from the majority is shown
//! as a diff against it - so no difference that matters is hidden (A-16).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;

/// One server's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub host: String,
    pub exit: i64,
    pub output: String,
}

fn rules() -> &'static [(Regex, &'static str)] {
    static RULES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();

    RULES.get_or_init(|| {
        [
            (r"\b\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})?\b", "<time>"),
            (r"\b(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s+\d{1,2}\s+\d{2}:\d{2}(:\d{2})?\b", "<time>"),
            (r"\b\d{1,2}:\d{2}:\d{2}\b", "<clock>"),
            (r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b", "<uuid>"),
            (r"\b[0-9a-f]{12,64}\b", "<hex>"),
            (r"\b\d{1,3}(\.\d{1,3}){3}(:\d+)?\b", "<ip>"),
            (r"(?i)\b(pid[ =:]?|process )\d+\b", "pid <n>"),
            (r"\[\d{2,7}\]", "[<n>]"),
            (r"\b\d+(\.\d+)?\s?(ms|s|µs|us|ns|min)\b", "<duration>"),
            (r"(?i)\bup\s+\d+\s+(days?|hours?|min(utes)?)(,\s*\d+:\d+)?", "up <uptime>"),
            (r"load average: [\d., ]+", "load average: <load>"),
        ]
        .iter()
        .map(|(pattern, with)| (Regex::new(pattern).expect("a valid fleet rule"), *with))
        .collect()
    })
}

/// The output with what always differs between machines replaced, for grouping.
pub fn normalise(output: &str, host: &str) -> String {
    let mut text = output.replace("\r\n", "\n");

    if !host.is_empty() {
        text = text.replace(host, "<host>");

        if let Some(short) = host.split('.').next().filter(|short| short.len() >= 3) {
            text = text.replace(short, "<host>");
        }
    }

    for (rule, with) in rules() {
        text = rule.replace_all(&text, *with).to_string();
    }

    text.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// A group of servers whose outputs are the same after normalising.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub hosts: Vec<String>,
    pub exit: i64,
    pub key: String,
    pub sample: String,
}

/// The groups, largest first (the majority is the reference the others are compared with).
pub fn cluster(answers: &[Answer]) -> Vec<Group> {
    let mut groups: BTreeMap<(i64, String), Group> = BTreeMap::new();

    for answer in answers {
        let key = normalise(&answer.output, &answer.host);

        groups
            .entry((answer.exit, key.clone()))
            .or_insert_with(|| Group { hosts: Vec::new(), exit: answer.exit, key, sample: answer.output.clone() })
            .hosts
            .push(answer.host.clone());
    }

    let mut groups: Vec<Group> = groups.into_values().collect();

    groups.sort_by(|a, b| b.hosts.len().cmp(&a.hosts.len()).then(a.hosts.cmp(&b.hosts)));
    groups
}

fn clip(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();

    if all.len() <= lines {
        return text.trim_end().to_string();
    }

    format!("{}\n… {} more line(s)", all[..lines].join("\n"), all.len() - lines)
}

/// The report a model reads.
pub fn render(command: &str, answers: &[Answer]) -> String {
    let groups = cluster(answers);
    let Some(majority) = groups.first() else {
        return format!("`{command}` ran on no server.");
    };
    let mut out = format!("`{command}` on {} server(s): {} distinct result(s) after normalising times, PIDs, ids, IPs and host names.\n", answers.len(), groups.len());

    out.push_str(&format!(
        "\n{} identical (exit {}): {}\nsample from {}:\n{}\n",
        majority.hosts.len(),
        majority.exit,
        majority.hosts.join(", "),
        majority.hosts[0],
        clip(&majority.sample, 40)
    ));

    for group in &groups[1..] {
        let diff = similar::TextDiff::from_lines(majority.key.as_str(), group.key.as_str()).unified_diff().context_radius(1).header("majority", &group.hosts[0]).to_string();

        out.push_str(&format!(
            "\n{} differ{} (exit {}): {}\ndiff against the majority (normalised):\n{}",
            group.hosts.len(),
            if group.hosts.len() == 1 { "s" } else { "" },
            group.exit,
            group.hosts.join(", "),
            clip(&diff, 40)
        ));
    }

    out
}

/// The tool the agent is offered when `v3.fleet` is on.
pub fn spec() -> crate::agent::dialect::ToolSpec {
    crate::agent::dialect::ToolSpec {
        name: "fleet_run",
        description: "Run one shell command on several of the person's servers at once and get the answers clustered: identical outputs (after normalising times, PIDs, ids, IPs and host names) are reported once with a sample, and every server that differs is shown as a diff. Use it to check or fix many servers. Set canary to run on the first server alone first and stop there if it fails. Commands that change anything ask the person first.",
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The shell command, as it would be typed on each server." },
                "hosts": { "type": "array", "items": { "type": "string" }, "description": "Server names or ids, or [\"all\"] for every connected server." },
                "canary": { "type": "boolean", "description": "Run on the first server alone first; continue on the rest only if it exits 0." },
            },
            "required": ["command", "hosts"],
            "additionalProperties": false,
        }),
    }
}

/// The servers a selector names, from the daemon's hosts: `(name, ssh)` - `all`, or names/ids.
pub fn select_hosts(selector: &[String]) -> Vec<(String, crate::ssh::Ssh)> {
    let Some(store) = crate::v3::journal::store() else {
        return Vec::new();
    };
    let all = selector.iter().any(|name| name.eq_ignore_ascii_case("all"));

    store
        .hosts()
        .unwrap_or_default()
        .into_iter()
        .filter(|host| host["hostType"] != "local")
        .filter(|host| all || selector.iter().any(|name| host["hostId"] == name.as_str() || host["name"] == name.as_str()))
        .filter_map(|host| {
            let id = host["hostId"].as_str()?.to_string();
            let (Some(target), port) = store.host_address(&id).ok()?? else {
                return None;
            };

            Some((host["name"].as_str().unwrap_or(&id).to_string(), crate::ssh::Ssh::new(crate::auth::remote::SshTarget { user_host: target, port })))
        })
        .collect()
}

/// Runs a command on servers in parallel.
pub fn run_on(servers: &[(String, crate::ssh::Ssh)], command: &str) -> Vec<Answer> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = servers
            .iter()
            .map(|(name, ssh)| {
                scope.spawn(move || match ssh.run(command, std::time::Duration::from_secs(120)) {
                    Ok(output) => Answer { host: name.clone(), exit: output.code.unwrap_or(-1) as i64, output: format!("{}{}", output.stdout, output.stderr) },
                    Err(error) => Answer { host: name.clone(), exit: -1, output: format!("SDC could not run it there: {}", error.message) },
                })
            })
            .collect();

        handles.into_iter().filter_map(|handle| handle.join().ok()).collect()
    })
}

/// What the fleet tool says per server when nothing is clustered (for the reduction measurement): every output whole.
pub fn render_plain(command: &str, answers: &[Answer]) -> String {
    let mut out = format!("`{command}` on {} server(s):\n", answers.len());

    for answer in answers {
        out.push_str(&format!("\n== {} (exit {})\n{}\n", answer.host, answer.exit, answer.output.trim_end()));
    }

    out
}

/// Commands that only look, run on a fleet without asking (anything else asks the person first).
pub fn read_only(command: &str) -> bool {
    let command = command.trim();
    let safe = ["uptime", "df ", "df", "free", "uname", "hostname", "cat /etc/os-release", "systemctl status", "systemctl is-active", "docker ps", "docker version", "nproc", "whoami", "date", "lsb_release", "journalctl --no-pager"];

    !command.contains(['>', ';', '&', '|', '`', '$']) && safe.iter().any(|start| command == start.trim() || command.starts_with(start))
}

/// A synthetic fleet for the A-16 measurement: `servers` machines answering `uptime && docker ps`, all the same but
/// for times, PIDs, ids and names - except `different` of them, which really differ (a stopped container).
pub fn synthetic(servers: usize, different: usize) -> Vec<Answer> {
    (0..servers)
        .map(|index| {
            let host = format!("web-{index:02}.fleet.example");
            let broken = index >= servers - different;
            let mut output = format!(
                " {:02}:{:02}:{:02} up {} days,  3:{:02},  0 users,  load average: 0.{}, 0.1{}, 0.05\nCONTAINER ID   IMAGE          STATUS          NAMES\n{:012x}   shop-api:2.1   Up {} hours     api\n{:012x}   postgres:16    Up {} hours     db\n",
                10 + index % 12, index % 60, (index * 7) % 60, 3 + index, index % 60, index % 10, index % 10,
                0xa1b2_c3d4_e5f6u64 + index as u64, 2 + index,
                0x0f1e_2d3c_4b5au64 + index as u64, 2 + index
            );

            if broken {
                output = output.replace(&format!("Up {} hours     api", 2 + index), "Exited (1) 5 minutes ago     api");
            }

            output.push_str(&format!("[{}] healthcheck ok on {host}\n", 1000 + index * 13));

            Answer { host, exit: 0, output }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_always_differs_is_normalised_away() {
        let one = normalise("2026-10-14T02:13:13Z web-01 sshd[4121]: accepted from 10.0.0.7:5521 in 12ms", "web-01");
        let two = normalise("2026-10-15T09:01:44Z web-02 sshd[88]: accepted from 10.0.0.9:6000 in 7ms", "web-02");

        assert_eq!(one, two);
        assert_ne!(normalise("disk 91% full", "a"), normalise("disk 12% full", "b"), "a real difference stays");
    }

    #[test]
    fn twenty_servers_with_three_real_differences() {
        let answers = synthetic(20, 3);
        let groups = cluster(&answers);
        let report = render("uptime && docker ps", &answers);

        assert_eq!(groups.len(), 2, "{groups:?}");
        assert_eq!(groups[0].hosts.len(), 17);
        assert_eq!(groups[1].hosts.len(), 3);
        assert!(report.contains("17 identical") && report.contains("3 differ") && report.contains("Exited (1)"), "{report}");

        let (plain, clustered) = (crate::context::tokens_of(&render_plain("x", &answers)), crate::context::tokens_of(&report));

        assert!(clustered * 100 <= plain * 25, "at least 75% smaller: {clustered} vs {plain}");
    }

    #[test]
    fn only_looking_commands_run_without_asking() {
        assert!(read_only("uptime") && read_only("df -h") && read_only("systemctl status nginx"));
        assert!(!read_only("rm -rf /tmp/x") && !read_only("uptime; reboot") && !read_only("docker ps | xargs docker rm"));
    }
}
