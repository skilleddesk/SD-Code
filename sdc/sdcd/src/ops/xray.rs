//! **Takeover X-ray** (2.0 in the plan): an agency inherits a server nobody documented. One read-only
//! scan - `ssh` commands that only read, nothing installed, nothing changed (P1) - becomes:
//!
//! * a **map**: the machine, its services, the sites its web server serves (with their folders and
//!   certificates), its databases, its scheduled jobs, its containers and its open ports;
//! * a **document** in Markdown, for the handover folder;
//! * a **risk list**, each risk with why it matters and what to do.
//!
//! Every command is guarded with `2>/dev/null` and `|| true`: a machine without `docker` or without
//! permission to read `nginx -T` gives a shorter map, not a failed scan. Sections are marked `@@NAME`, so
//! the parser reads structure rather than guessing at output (P3).

use serde_json::{json, Value};

/// The scan: one script, read-only.
pub const SCAN: &str = r#"
echo '@@OS'; (. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME") || uname -sr; uname -m
echo '@@UPTIME'; uptime -p 2>/dev/null || uptime
echo '@@CPU'; nproc 2>/dev/null; free -m 2>/dev/null | awk '/Mem:/ {print $2" "$3}'
echo '@@DISK'; df -hP -x tmpfs -x devtmpfs 2>/dev/null | tail -n +2
echo '@@SERVICES'; systemctl list-units --type=service --state=running --no-legend --no-pager 2>/dev/null | awk '{print $1}' | head -80
echo '@@PORTS'; (ss -tlnH 2>/dev/null || netstat -tln 2>/dev/null | tail -n +3) | awk '{print $4}' | sort -u | head -60
echo '@@NGINX'; (nginx -T 2>/dev/null || cat /etc/nginx/sites-enabled/* /etc/nginx/conf.d/*.conf 2>/dev/null) | grep -E '^\s*(server_name|root|listen|ssl_certificate)\s' | sed 's/;.*//' | head -200
echo '@@APACHE'; (apache2ctl -S 2>/dev/null || httpd -S 2>/dev/null) | grep -E 'namevhost|alias|DocumentRoot' | head -80; grep -rhE '^\s*(ServerName|DocumentRoot)' /etc/apache2/sites-enabled /etc/httpd/conf.d 2>/dev/null | head -80
echo '@@WORDPRESS'; find /var/www /home /srv -maxdepth 5 -name wp-config.php 2>/dev/null | head -40
echo '@@DATABASES'; ps -eo comm 2>/dev/null | grep -E '^(mysqld|mariadbd|postgres|redis-server|mongod)$' | sort -u
echo '@@DOCKER'; docker ps --format '{{.Names}} {{.Image}} {{.Ports}}' 2>/dev/null | head -40
echo '@@CRON'; crontab -l 2>/dev/null | grep -v '^#' | grep -v '^$' | head -40; ls /etc/cron.d 2>/dev/null | sed 's/^/cron.d: /'
echo '@@RUNTIMES'; php -v 2>/dev/null | head -1; node -v 2>/dev/null | sed 's/^/node /'; python3 -V 2>/dev/null; ruby -v 2>/dev/null | cut -d' ' -f1-2; java -version 2>&1 | head -1 | grep -i version
echo '@@CERTS'; for c in /etc/letsencrypt/live/*/cert.pem; do [ -r "$c" ] && echo "$(basename "$(dirname "$c")") $(openssl x509 -noout -enddate -in "$c" 2>/dev/null | cut -d= -f2)"; done 2>/dev/null
echo '@@SSHD'; (sshd -T 2>/dev/null || cat /etc/ssh/sshd_config 2>/dev/null) | grep -iE '^\s*(permitrootlogin|passwordauthentication)\s' | head -4
echo '@@FIREWALL'; (ufw status 2>/dev/null | head -1) || true; iptables -S 2>/dev/null | wc -l | sed 's/^/iptables-rules: /'
echo '@@UPDATES'; (apt list --upgradable 2>/dev/null | tail -n +2 | wc -l | sed 's/^/apt: /') ; (yum -q check-update 2>/dev/null | grep -c '^[a-z]' | sed 's/^/yum: /')
echo '@@BACKUPS'; ls -d /var/backups/* /backup* /home/*/backup* 2>/dev/null | head -20
echo '@@END'
"#;

/// The scan's output, section by section.
pub fn sections(output: &str) -> std::collections::HashMap<String, Vec<String>> {
    let mut map: std::collections::HashMap<String, Vec<String>> = Default::default();
    let mut current: Option<String> = None;

    for line in output.lines() {
        if let Some(name) = line.strip_prefix("@@") {
            current = (name != "END").then(|| name.trim().to_string());

            if let Some(name) = &current {
                map.entry(name.clone()).or_default();
            }

            continue;
        }

        if let (Some(name), false) = (&current, line.trim().is_empty()) {
            map.entry(name.clone()).or_default().push(line.trim().to_string());
        }
    }

    map
}

fn version_number(text: &str) -> Option<(u32, u32)> {
    let digits: String = text.chars().skip_while(|character| !character.is_ascii_digit()).collect();
    let mut parts = digits.split(|character: char| !character.is_ascii_digit()).filter(|part| !part.is_empty());

    Some((parts.next()?.parse().ok()?, parts.next().and_then(|minor| minor.parse().ok()).unwrap_or(0)))
}

/// The map and the risks, from the scan.
pub fn analyse(host: &str, output: &str) -> Value {
    let map = sections(output);
    let get = |name: &str| map.get(name).cloned().unwrap_or_default();
    let mut risks: Vec<Value> = Vec::new();
    let mut risk = |level: &str, title: String, why: &str, fix: &str| {
        risks.push(json!({ "level": level, "title": title, "why": why, "fix": fix }));
    };

    /* Sites, from nginx: server_name / root / ssl_certificate lines, grouped per server block. */
    let mut sites: Vec<Value> = Vec::new();
    let mut current = json!({ "names": [], "root": Value::Null, "ssl": false, "listen": [] });

    for line in get("NGINX") {
        let mut words = line.split_whitespace();
        let key = words.next().unwrap_or_default();
        let rest: Vec<String> = words.map(str::to_string).collect();

        match key {
            "server_name" => {
                if !current["names"].as_array().map(Vec::is_empty).unwrap_or(true) {
                    sites.push(current.clone());
                    current = json!({ "names": [], "root": Value::Null, "ssl": false, "listen": [] });
                }

                current["names"] = json!(rest.iter().filter(|name| name.as_str() != "_").cloned().collect::<Vec<_>>());
            }
            "root" => current["root"] = json!(rest.first()),
            "ssl_certificate" => current["ssl"] = json!(true),
            "listen" => {
                if let Some(array) = current["listen"].as_array_mut() {
                    array.push(json!(rest.join(" ")));
                }
            }
            _ => {}
        }
    }

    if !current["names"].as_array().map(Vec::is_empty).unwrap_or(true) {
        sites.push(current);
    }

    let apache: Vec<String> = get("APACHE");

    for line in &apache {
        if let Some(name) = line.strip_prefix("ServerName").map(str::trim) {
            sites.push(json!({ "names": [name], "root": Value::Null, "ssl": false, "server": "apache" }));
        }
    }

    let wordpress = get("WORDPRESS");

    /* Certificates and their days. */
    let certs: Vec<Value> = get("CERTS")
        .iter()
        .filter_map(|line| {
            let (name, date) = line.split_once(' ')?;
            let days = super::health::days_until(date);

            Some(json!({ "name": name, "expires": date, "days": days }))
        })
        .collect();

    for cert in &certs {
        if let Some(days) = cert["days"].as_i64() {
            if days < 0 {
                risk("critical", format!("Certificate for {} has expired", cert["name"].as_str().unwrap_or("?")), "Browsers refuse the site.", "Renew it (certbot renew) and check the renewal timer.");
            } else if days < 14 {
                risk("high", format!("Certificate for {} expires in {days} days", cert["name"].as_str().unwrap_or("?")), "The site will stop loading in browsers when it expires.", "Check that certbot's timer runs; renew now.");
            }
        }
    }

    /* SSH. */
    for line in get("SSHD") {
        let lowered = line.to_lowercase();

        if lowered.starts_with("permitrootlogin") && lowered.ends_with(" yes") {
            risk("high", "Root can log in over SSH".into(), "A leaked or guessed root password is the whole machine.", "Set PermitRootLogin to no (or prohibit-password) and use a normal user with sudo.");
        }

        if lowered.starts_with("passwordauthentication") && lowered.ends_with(" yes") {
            risk("medium", "SSH accepts passwords".into(), "Passwords can be guessed by bots that try all day.", "Use keys only (PasswordAuthentication no), or at least a second factor.");
        }
    }

    /* Firewall. */
    let firewall = get("FIREWALL");
    let ufw_inactive = firewall.iter().any(|line| line.to_lowercase().contains("status: inactive"));
    let no_iptables = firewall.iter().any(|line| line.trim() == "iptables-rules: 0" || line.trim() == "iptables-rules: 3");

    if ufw_inactive || (firewall.iter().all(|line| !line.to_lowercase().contains("status: active")) && no_iptables) {
        risk("medium", "No firewall is active".into(), "Every service listening on the network is reachable from the internet.", "Enable ufw (allow ssh, http, https) or configure iptables/nftables.");
    }

    /* Databases listening on every interface. */
    let ports = get("PORTS");

    for (port, name) in [("3306", "MySQL"), ("5432", "PostgreSQL"), ("6379", "Redis"), ("27017", "MongoDB")] {
        if ports.iter().any(|address| (address.starts_with("0.0.0.0:") || address.starts_with("*:") || address.starts_with("[::]:")) && address.ends_with(&format!(":{port}"))) {
            risk("high", format!("{name} listens on every network interface (port {port})"), "A database reachable from the internet is attacked constantly.", "Bind it to 127.0.0.1, or firewall the port to the hosts that need it.");
        }
    }

    /* Disk. */
    for line in get("DISK") {
        let columns: Vec<&str> = line.split_whitespace().collect();

        if let (Some(percent), Some(mount)) = (columns.get(4).and_then(|value| value.trim_end_matches('%').parse::<u32>().ok()), columns.get(5)) {
            if percent >= 90 {
                risk("high", format!("{mount} is {percent}% full"), "A full disk stops databases, logs and uploads.", "Clear old logs and backups, or grow the disk.");
            }
        }
    }

    /* Updates, OS age, runtimes. */
    let pending: u32 = get("UPDATES").iter().filter_map(|line| line.split(':').nth(1)).filter_map(|count| count.trim().parse::<u32>().ok()).sum();

    if pending >= 30 {
        risk("medium", format!("{pending} package updates are waiting"), "Security fixes are among them.", "Plan an update window: apt upgrade (after a backup).");
    }

    let os = get("OS").first().cloned().unwrap_or_default();
    let os_lower = os.to_lowercase();
    let eol = (os_lower.contains("ubuntu") && version_number(&os).is_some_and(|(major, _)| major < 20))
        || (os_lower.contains("debian") && version_number(&os).is_some_and(|(major, _)| major < 11))
        || os_lower.contains("centos linux 7")
        || os_lower.contains("centos 7");

    if eol {
        risk("high", format!("{os} no longer gets security updates"), "Known holes stay open.", "Plan a migration to a supported release.");
    }

    let runtimes = get("RUNTIMES");

    for line in &runtimes {
        let lowered = line.to_lowercase();

        if lowered.starts_with("php ") && version_number(line).is_some_and(|(major, _)| major < 8) {
            risk("high", format!("{line} is past its end of life"), "No security fixes; newer plugins stop supporting it.", "Upgrade PHP to 8.1+ (test the sites on staging first).");
        }

        if lowered.starts_with("node ") && version_number(line).is_some_and(|(major, _)| major < 18) {
            risk("medium", format!("{line} is past its end of life"), "No security fixes.", "Upgrade Node.js to an LTS release.");
        }
    }

    /* Backups. */
    let backups = get("BACKUPS");

    if backups.is_empty() {
        risk("high", "No backup folders were found".into(), "If the disk fails or a site is broken, there may be nothing to go back to.", "Add the sites to SDC and turn on Safe Deploy's backups, or set up a nightly off-site backup.");
    }

    let order = |level: &str| match level {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        _ => 3,
    };

    risks.sort_by_key(|risk| order(risk["level"].as_str().unwrap_or("low")));

    let cpu = get("CPU");

    json!({
        "host": host,
        "scannedAt": chrono::Utc::now().to_rfc3339(),
        "os": os,
        "arch": get("OS").get(1).cloned(),
        "uptime": get("UPTIME").first().cloned(),
        "cpus": cpu.first().cloned(),
        "memoryMb": cpu.get(1).cloned(),
        "disks": get("DISK"),
        "services": get("SERVICES"),
        "ports": ports,
        "sites": sites,
        "wordpress": wordpress,
        "databases": get("DATABASES"),
        "docker": get("DOCKER"),
        "cron": get("CRON"),
        "runtimes": runtimes,
        "certificates": certs,
        "backups": backups,
        "risks": risks,
    })
}

/// The handover document, in Markdown.
pub fn document(map: &Value) -> String {
    let list = |key: &str| -> String {
        let items = map[key].as_array().cloned().unwrap_or_default();

        if items.is_empty() {
            "- (none found)\n".to_string()
        } else {
            items.iter().map(|item| format!("- {}\n", item.as_str().map(str::to_string).unwrap_or_else(|| item.to_string()))).collect()
        }
    };
    let mut text = format!(
        "# Server X-ray: {host}\n\nScanned {at} by SDC, read-only.\n\n## Machine\n\n- {os} ({arch})\n- Up: {uptime}\n- CPUs: {cpus} · Memory (total/used MB): {memory}\n\n## Risks\n\n",
        host = map["host"].as_str().unwrap_or("?"),
        at = map["scannedAt"].as_str().unwrap_or_default(),
        os = map["os"].as_str().unwrap_or("?"),
        arch = map["arch"].as_str().unwrap_or("?"),
        uptime = map["uptime"].as_str().unwrap_or("?"),
        cpus = map["cpus"].as_str().unwrap_or("?"),
        memory = map["memoryMb"].as_str().unwrap_or("?"),
    );

    let risks = map["risks"].as_array().cloned().unwrap_or_default();

    if risks.is_empty() {
        text.push_str("- No risks found by this scan.\n");
    }

    for risk in risks {
        text.push_str(&format!(
            "- **[{}] {}** - {} *Fix:* {}\n",
            risk["level"].as_str().unwrap_or("?").to_uppercase(),
            risk["title"].as_str().unwrap_or_default(),
            risk["why"].as_str().unwrap_or_default(),
            risk["fix"].as_str().unwrap_or_default()
        ));
    }

    text.push_str("\n## Sites\n\n");

    for site in map["sites"].as_array().cloned().unwrap_or_default() {
        text.push_str(&format!(
            "- {} → {}{}\n",
            site["names"].as_array().map(|names| names.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
            site["root"].as_str().unwrap_or("(root not read)"),
            if site["ssl"] == true { " · HTTPS" } else { "" }
        ));
    }

    for (title, key) in [
        ("WordPress installs", "wordpress"),
        ("Databases", "databases"),
        ("Services", "services"),
        ("Open ports", "ports"),
        ("Scheduled jobs", "cron"),
        ("Containers", "docker"),
        ("Runtimes", "runtimes"),
        ("Backups", "backups"),
        ("Disks", "disks"),
    ] {
        text.push_str(&format!("\n## {title}\n\n{}", list(key)));
    }

    text.push_str("\n## Certificates\n\n");

    for cert in map["certificates"].as_array().cloned().unwrap_or_default() {
        text.push_str(&format!("- {} - expires {} ({} days)\n", cert["name"].as_str().unwrap_or("?"), cert["expires"].as_str().unwrap_or("?"), cert["days"]));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "@@OS\nUbuntu 18.04.6 LTS\nx86_64\n@@UPTIME\nup 3 weeks\n@@CPU\n2\n3936 2100\n@@DISK\n/dev/sda1 40G 37G 3G 93% /\n\
@@SERVICES\nnginx.service\nmysql.service\n@@PORTS\n0.0.0.0:22\n0.0.0.0:80\n0.0.0.0:3306\n\
@@NGINX\nserver_name shop.test www.shop.test\nroot /var/www/shop\nlisten 443 ssl\nssl_certificate /etc/letsencrypt/live/shop.test/fullchain.pem\nserver_name blog.test\nroot /var/www/blog\n\
@@APACHE\n@@WORDPRESS\n/var/www/blog/wp-config.php\n@@DATABASES\nmysqld\n@@DOCKER\n@@CRON\n0 3 * * * /usr/local/bin/backup.sh\n\
@@RUNTIMES\nPHP 7.4.3 (cli)\nnode v16.20.0\n@@CERTS\n@@SSHD\npermitrootlogin yes\npasswordauthentication yes\n@@FIREWALL\nStatus: inactive\niptables-rules: 3\n@@UPDATES\napt: 57\n@@BACKUPS\n@@END\n";

    #[test]
    fn a_scan_becomes_a_map_of_sites_and_a_ranked_list_of_risks() {
        let map = analyse("vps-1", OUTPUT);

        assert_eq!(map["sites"].as_array().unwrap().len(), 2);
        assert_eq!(map["sites"][0]["root"], "/var/www/shop");
        assert_eq!(map["sites"][0]["ssl"], true);

        let titles: Vec<String> = map["risks"].as_array().unwrap().iter().map(|risk| risk["title"].as_str().unwrap().to_string()).collect();

        for expected in ["Root can log in", "MySQL listens", "93% full", "no longer gets security updates", "PHP 7.4.3", "No firewall", "57 package updates", "No backup folders"] {
            assert!(titles.iter().any(|title| title.contains(expected)), "missing {expected}: {titles:?}");
        }

        assert_eq!(map["risks"][0]["level"], "high", "ranked, most severe first");
    }

    #[test]
    fn the_document_is_markdown_a_person_can_hand_over() {
        let text = document(&analyse("vps-1", OUTPUT));

        assert!(text.starts_with("# Server X-ray: vps-1"));
        assert!(text.contains("shop.test, www.shop.test → /var/www/shop · HTTPS"));
        assert!(text.contains("## Scheduled jobs"));
    }

    #[test]
    fn the_scan_only_reads() {
        for word in [" rm ", "apt install", "apt-get install", " > /etc", "systemctl restart", "systemctl stop", "chmod", "useradd"] {
            assert!(!SCAN.contains(word), "the scan must not change anything: {word}");
        }
    }
}
