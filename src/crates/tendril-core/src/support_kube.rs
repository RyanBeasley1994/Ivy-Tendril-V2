//! The support manager's view of the cluster, through the daemon machine's own kubeconfig.
//!
//! Turned on with `SupportSettings::kubernetes`. Three kinds of access, each logged on the ticket:
//!
//! * **Reading resources** (`tendril support kube ...`): only the read verbs (`get`, `describe`,
//!   `logs`, `top`, `events`, ...). Anything that changes the cluster is refused, so is `exec`, and so is
//!   printing a Secret.
//! * **A database from a Secret** (`tendril support kube-db`): the connection string is read from the
//!   Secret here and used here, the database is reached through a `kubectl port-forward` when it is
//!   in-cluster, and the same read-only lookups as a configured source run against it. The string, and
//!   its password, never reach the manager.
//! * **An in-cluster API** (`tendril support kube-api`): a `GET` through a port-forward to a service,
//!   optionally with a bearer token taken from a Secret, again without showing the token.

use crate::error::{Result, TendrilError};
use crate::support_cases::{self, DataSource, SourceKind};
use crate::support_data::{self, MongoRead};
use base64::Engine;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

const TIMEOUT: Duration = Duration::from_secs(20);

/// The verbs that only read.
const READ_VERBS: &[&str] = &["get", "describe", "logs", "top", "events", "explain", "api-resources", "api-versions", "version", "cluster-info"];

fn bad(msg: impl Into<String>) -> TendrilError {
    TendrilError::Validation(msg.into())
}

fn enabled(home: &Path) -> Result<Option<String>> {
    let settings = support_cases::load_settings(home);
    if !settings.kubernetes {
        return Err(bad("Cluster access is off for support. The operator turns it on in Support → Triage."));
    }
    Ok(settings.kube_context)
}

fn kubectl(context: Option<&str>) -> Command {
    let mut cmd = Command::new("kubectl");
    if let Some(c) = context {
        cmd.arg("--context").arg(c);
    }
    cmd.kill_on_drop(true);
    cmd
}

/// Why `args` may not run as a support lookup, if it may not.
pub fn refuse_reason(args: &[String]) -> Option<String> {
    let verb = args.iter().find(|a| !a.starts_with('-'))?.to_ascii_lowercase();
    if !READ_VERBS.contains(&verb.as_str()) {
        return Some(format!(
            "`kubectl {verb}` is not a read. Support lookups may only {}.",
            READ_VERBS.join(", ")
        ));
    }
    let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    // `describe secret` shows only key names and sizes, which is fine; anything that prints values is not.
    let names_secret = lower.iter().any(|a| {
        a == "secret" || a == "secrets" || a.starts_with("secret/") || a.starts_with("secrets/") || a.split(',').any(|p| p == "secret" || p == "secrets")
    });
    if names_secret && verb != "describe" {
        return Some("Secrets are never printed. Use `tendril support kube-secret` to see their keys, and `kube-db` / `kube-api` to use them.".into());
    }
    if lower.iter().any(|a| a == "--raw" || a.starts_with("--raw=")) && lower.iter().any(|a| a.contains("secret")) {
        return Some("Secrets are never printed.".into());
    }
    None
}

async fn output(mut cmd: Command) -> Result<String> {
    let out = tokio::time::timeout(TIMEOUT, cmd.stdin(Stdio::null()).output())
        .await
        .map_err(|_| TendrilError::Other("kubectl timed out".into()))?
        .map_err(|e| TendrilError::Other(format!("Could not run kubectl: {e}")))?;
    if !out.status.success() {
        return Err(TendrilError::Other(format!("kubectl: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// `kubectl <args>` for `ticket`, read verbs only.
pub async fn read(home: &Path, ticket: &str, args: Vec<String>) -> Result<String> {
    let context = enabled(home)?;
    if let Some(why) = refuse_reason(&args) {
        return Err(bad(why));
    }
    let mut cmd = kubectl(context.as_deref());
    cmd.args(&args);
    // A log tail is capped so one pod cannot fill the manager's context.
    if args.first().is_some_and(|v| v == "logs") && !args.iter().any(|a| a.starts_with("--tail")) {
        cmd.arg("--tail=300");
    }
    let result = output(cmd).await;
    let summary = match &result {
        Ok(text) => format!("{} lines", text.lines().count()),
        Err(e) => format!("failed: {e}"),
    };
    support_cases::log_lookup(home, ticket, "kubernetes", &format!("kubectl {}", args.join(" ")), &summary)?;
    result.map(support_data::cap)
}

async fn secret_data(context: Option<&str>, namespace: &str, name: &str) -> Result<serde_json::Map<String, Value>> {
    let mut cmd = kubectl(context);
    cmd.args(["get", "secret", name, "-n", namespace, "-o", "json"]);
    let json: Value = serde_json::from_str(&output(cmd).await?).map_err(|e| TendrilError::Other(e.to_string()))?;
    Ok(json["data"].as_object().cloned().unwrap_or_default())
}

async fn secret_value(context: Option<&str>, namespace: &str, name: &str, key: &str) -> Result<String> {
    let data = secret_data(context, namespace, name).await?;
    let raw = data
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| bad(format!("Secret {namespace}/{name} has no key '{key}'. Its keys: {}", data.keys().cloned().collect::<Vec<_>>().join(", "))))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|e| TendrilError::Other(format!("Secret {namespace}/{name}:{key} is not base64: {e}")))?;
    Ok(String::from_utf8_lossy(&bytes).trim().to_string())
}

/// A Secret's key names and sizes: what is in it, never what it says.
pub async fn secret_keys(home: &Path, ticket: &str, namespace: &str, name: &str) -> Result<String> {
    let context = enabled(home)?;
    let result = secret_data(context.as_deref(), namespace, name).await.map(|data| {
        data.iter()
            .map(|(k, v)| {
                let len = v.as_str().and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok()).map_or(0, |b| b.len());
                format!("{k}  ({len} bytes)")
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    let summary = match &result {
        Ok(keys) => format!("{} keys", keys.lines().count()),
        Err(e) => format!("failed: {e}"),
    };
    support_cases::log_lookup(home, ticket, "kubernetes", &format!("secret keys {namespace}/{name}"), &summary)?;
    result
}

/// A running `kubectl port-forward`, stopped when dropped.
struct Forward {
    _child: Child,
    port: u16,
}

async fn port_forward(context: Option<&str>, namespace: &str, target: &str, remote: u16) -> Result<Forward> {
    let mut cmd = kubectl(context);
    cmd.args(["port-forward", "-n", namespace, target, &format!(":{remote}"), "--address", "127.0.0.1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| TendrilError::Other(format!("Could not run kubectl: {e}")))?;
    let stdout = child.stdout.take().ok_or_else(|| TendrilError::Other("port-forward has no output".into()))?;
    let mut lines = BufReader::new(stdout).lines();
    let found = tokio::time::timeout(TIMEOUT, async {
        while let Ok(Some(line)) = lines.next_line().await {
            // "Forwarding from 127.0.0.1:53114 -> 27017"
            if let Some(port) = line
                .split("127.0.0.1:")
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|p| p.parse::<u16>().ok())
            {
                return Some(port);
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    match found {
        Some(port) => Ok(Forward { _child: child, port }),
        None => {
            let mut err = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = tokio::time::timeout(Duration::from_secs(2), stderr.read_to_string(&mut err)).await;
            }
            Err(TendrilError::Other(format!("Could not port-forward to {namespace}/{target}: {}", err.trim())))
        }
    }
}

/// The in-cluster host a connection string points at, as `(namespace, name, port)`, or `None` for one
/// reachable as it is. `name` is the first label: a service, or a pod of a headless service.
pub fn in_cluster_host(uri: &str) -> Option<(String, String, u16)> {
    let after = uri.split("://").nth(1)?;
    let hosts = after.rsplit_once('@').map_or(after, |(_, h)| h);
    let first = hosts.split(['/', '?']).next()?.split(',').next()?;
    let (host, port) = match first.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (first, 27017),
    };
    let labels: Vec<&str> = host.split('.').collect();
    if let Some(i) = labels.iter().position(|l| *l == "svc") {
        // name[.service].namespace.svc[.cluster.local]
        let namespace = labels.get(i.checked_sub(1)?)?.to_string();
        return Some((namespace, labels[0].to_string(), port));
    }
    // A bare `service` or `service.namespace` only resolves inside the cluster.
    match labels.len() {
        1 if host != "localhost" => Some((String::new(), labels[0].to_string(), port)),
        2 if !labels[1].contains(char::is_numeric) && !host.contains("localhost") => {
            Some((labels[1].to_string(), labels[0].to_string(), port))
        }
        _ => None,
    }
}

/// `uri` pointed at `127.0.0.1:port`, talking to that one member directly.
fn through(uri: &str, port: u16) -> String {
    let (scheme, rest) = uri.split_once("://").unwrap_or(("mongodb", uri));
    let scheme = scheme.replace("+srv", "");
    let (creds, hosts_and_rest) = match rest.rsplit_once('@') {
        Some((c, h)) => (format!("{c}@"), h),
        None => (String::new(), rest),
    };
    let tail_start = hosts_and_rest.find(['/', '?']).unwrap_or(hosts_and_rest.len());
    let tail = &hosts_and_rest[tail_start..];
    let (path, query) = tail.split_once('?').unwrap_or((tail, ""));
    let path = if path.is_empty() { "/" } else { path };
    // Replica-set discovery would hand back in-cluster names; talk to the forwarded member only.
    let mut params: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty() && !p.starts_with("replicaSet=") && !p.starts_with("directConnection="))
        .collect();
    params.push("directConnection=true");
    let tail = format!("{path}?{}", params.join("&"));
    format!("{scheme}://{creds}127.0.0.1:{port}{tail}")
}

/// A read-only MongoDB lookup against the database whose connection string is in a Secret.
pub async fn database(
    home: &Path,
    ticket: &str,
    secret: (&str, &str, &str),
    database: Option<&str>,
    read: MongoRead,
) -> Result<String> {
    let context = enabled(home)?;
    let (namespace, name, key) = secret;
    let described = format!("{} (db from secret {namespace}/{name}:{key})", read.describe());
    let result = async {
        let uri = secret_value(context.as_deref(), namespace, name, key).await?;
        let mut _forward = None;
        let uri = match in_cluster_host(&uri) {
            Some((ns, target, port)) => {
                let ns = if ns.is_empty() { namespace.to_string() } else { ns };
                let forward = match port_forward(context.as_deref(), &ns, &format!("svc/{target}"), port).await {
                    Ok(f) => f,
                    Err(_) => port_forward(context.as_deref(), &ns, &format!("pod/{target}"), port).await?,
                };
                let local = through(&uri, forward.port);
                _forward = Some(forward);
                local
            }
            None => uri,
        };
        let source = DataSource { name: format!("{namespace}/{name}"), kind: SourceKind::Mongo, url: uri, header: None, notes: String::new() };
        support_data::mongo(&source, database, &read).await
    }
    .await;
    let summary = match &result {
        Ok((s, _)) => s.clone(),
        // The error can quote the connection string; only the shape of the failure is logged and shown.
        Err(e) => format!("failed: {}", scrub(&e.to_string())),
    };
    support_cases::log_lookup(home, ticket, "kubernetes", &described, &summary)?;
    match result {
        Ok((_, body)) => Ok(support_data::cap(body)),
        Err(e) => Err(TendrilError::Other(scrub(&e.to_string()))),
    }
}

/// Removes anything credential-shaped from an error message.
fn scrub(text: &str) -> String {
    regex::Regex::new(r"(://)[^@\s/]+@")
        .map(|re| re.replace_all(text, "$1••••@").to_string())
        .unwrap_or_else(|_| text.to_string())
}

/// A `GET` to an in-cluster service through a port-forward: `target` is `svc/name` (or `pod/name`)
/// and `port` the port it serves on. `bearer` names a Secret key whose value is sent as a bearer token.
pub async fn api(
    home: &Path,
    ticket: &str,
    namespace: &str,
    target: &str,
    port: u16,
    path: &str,
    bearer: Option<(&str, &str)>,
) -> Result<String> {
    let context = enabled(home)?;
    let path = path.trim();
    if path.contains("://") || path.contains("..") {
        return Err(bad("Pass a path, like /api/accounts/123"));
    }
    let target = if target.contains('/') { target.to_string() } else { format!("svc/{target}") };
    let result = async {
        let token = match bearer {
            Some((name, key)) => Some(secret_value(context.as_deref(), namespace, name, key).await?),
            None => None,
        };
        let forward = port_forward(context.as_deref(), namespace, &target, port).await?;
        let url = format!("http://127.0.0.1:{}/{}", forward.port, path.trim_start_matches('/'));
        let mut req = reqwest::Client::builder().timeout(TIMEOUT).build().unwrap_or_default().get(&url);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.map_err(|e| TendrilError::Other(format!("Could not reach {target}: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let body = match serde_json::from_str::<Value>(&text) {
            Ok(mut v) => {
                support_data::redact(&mut v);
                serde_json::to_string_pretty(&v).unwrap_or(text)
            }
            Err(_) => text,
        };
        drop(forward);
        Ok::<_, TendrilError>((format!("{status} · {} bytes", body.len()), format!("HTTP {status}\n{body}")))
    }
    .await;
    let summary = match &result {
        Ok((s, _)) => s.clone(),
        Err(e) => format!("failed: {e}"),
    };
    support_cases::log_lookup(home, ticket, "kubernetes", &format!("GET {namespace}/{target}:{port}{path}"), &summary)?;
    result.map(|(_, body)| support_data::cap(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn only_reads_run_and_secrets_are_never_printed() {
        assert!(refuse_reason(&args("get pods -n prod")).is_none());
        assert!(refuse_reason(&args("logs deploy/server -n prod --since=1h")).is_none());
        assert!(refuse_reason(&args("describe secret mongo-creds -n mongodb")).is_none());
        for refused in [
            "delete pod x",
            "apply -f x.yaml",
            "exec -it pod/x -- sh",
            "scale deploy/x --replicas=0",
            "rollout restart deploy/x",
            "edit deploy/x",
            "patch deploy/x -p {}",
            "get secret mongo-creds -o yaml",
            "get secrets -A",
            "get pods,secrets -n x",
            "get --raw /api/v1/namespaces/x/secrets",
        ] {
            assert!(refuse_reason(&args(refused)).is_some(), "{refused} should be refused");
        }
    }

    #[test]
    fn in_cluster_hosts_are_found_and_rewritten_to_the_forward() {
        let rs = "mongodb://reader:pw@prop-rs0-0.prop-rs0.mongodb.svc.cluster.local:27017,prop-rs0-1.prop-rs0.mongodb.svc.cluster.local:27017/prop?replicaSet=rs0&authSource=admin";
        assert_eq!(in_cluster_host(rs), Some(("mongodb".into(), "prop-rs0-0".into(), 27017)));
        let local = through(rs, 50000);
        assert_eq!(local, "mongodb://reader:pw@127.0.0.1:50000/prop?authSource=admin&directConnection=true");
        assert_eq!(in_cluster_host("mongodb://u:p@mongo.prod:27018/db"), Some(("prod".into(), "mongo".into(), 27018)));
        assert_eq!(in_cluster_host("mongodb+srv://u:p@cluster0.abcde.mongodb.net/db"), None);
        assert!(through("mongodb://u:p@mongo:27017", 5).ends_with("127.0.0.1:5/?directConnection=true"));
    }

    #[test]
    fn credentials_are_scrubbed_from_errors() {
        assert_eq!(scrub("failed to connect to mongodb://reader:hunter2@127.0.0.1:5/db"), "failed to connect to mongodb://••••@127.0.0.1:5/db");
    }
}
