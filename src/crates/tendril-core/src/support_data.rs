//! The support manager's read-only window on real data: MongoDB finds, counts and aggregations, and
//! `GET`s against an API, each logged on the ticket it is for (`support_cases::log_lookup`).
//!
//! Read-only is enforced twice. The source should be a database user with only the `read` role (the
//! Support page says so), and this module only ever issues reads: there is no write verb, an
//! aggregation that writes (`$out`, `$merge`) or runs code (`$function`, `$accumulator`, `$where`) is
//! refused, and an API call is always a `GET` to a path under the source's own base URL.
//!
//! Secrets in what comes back (passwords, hashes, tokens, keys) are redacted before the manager sees
//! them, and output is capped so one lookup cannot flood its context.

use crate::error::{Result, TendrilError};
use crate::support_cases::{self, DataSource, SourceKind};
use mongodb::bson::{self, Bson, Document};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

const MAX_DOCS: i64 = 50;
const MAX_OUTPUT: usize = 30_000;
const TIMEOUT: Duration = Duration::from_secs(15);

const BANNED_STAGES: &[&str] = &["$out", "$merge", "$function", "$accumulator", "$where"];

/// A field whose value never reaches the manager.
fn sensitive(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    ["password", "passwd", "hash", "secret", "token", "apikey", "api_key", "otp", "totp", "mfa", "pin", "ssn", "cvv", "cardnumber", "iban", "privatekey", "private_key", "salt"]
        .iter()
        .any(|s| k.contains(s))
}

pub(crate) fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if sensitive(k) && !v.is_null() {
                    *v = Value::String("[redacted]".into());
                } else {
                    redact(v);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(redact),
        _ => {}
    }
}

pub(crate) fn cap(mut s: String) -> String {
    if s.len() > MAX_OUTPUT {
        let mut end = MAX_OUTPUT;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push_str("\n… (cut off: narrow the query or project fewer fields)");
    }
    s
}

fn bad(msg: impl Into<String>) -> TendrilError {
    TendrilError::Validation(msg.into())
}

fn parse_doc(json: &str, what: &str) -> Result<Document> {
    if json.trim().is_empty() {
        return Ok(Document::new());
    }
    let value: Value = serde_json::from_str(json).map_err(|e| bad(format!("{what} is not JSON: {e}")))?;
    match Bson::try_from(value).map_err(|e| bad(format!("{what}: {e}")))? {
        Bson::Document(d) => Ok(d),
        _ => Err(bad(format!("{what} must be a JSON object"))),
    }
}

fn walks_banned(value: &Bson) -> Option<&'static str> {
    match value {
        Bson::Document(d) => d.iter().find_map(|(k, v)| BANNED_STAGES.iter().find(|b| **b == k).copied().or_else(|| walks_banned(v))),
        Bson::Array(a) => a.iter().find_map(walks_banned),
        _ => None,
    }
}

/// A MongoDB read.
#[derive(Debug, Clone)]
pub enum MongoRead {
    Collections,
    Find { collection: String, filter: String, projection: String, sort: String, limit: i64 },
    Count { collection: String, filter: String },
    Aggregate { collection: String, pipeline: String },
}

impl MongoRead {
    pub(crate) fn describe(&self) -> String {
        match self {
            MongoRead::Collections => "collections".into(),
            MongoRead::Find { collection, filter, projection, sort, limit } => {
                format!("find {collection} filter={filter} projection={projection} sort={sort} limit={limit}")
            }
            MongoRead::Count { collection, filter } => format!("count {collection} filter={filter}"),
            MongoRead::Aggregate { collection, pipeline } => format!("aggregate {collection} {pipeline}"),
        }
    }
}

pub(crate) async fn mongo(source: &DataSource, database: Option<&str>, read: &MongoRead) -> Result<(String, String)> {
    use futures_util::TryStreamExt;
    use mongodb::options::{ClientOptions, ReadPreference, SelectionCriteria};

    let mut options = ClientOptions::parse(&source.url)
        .await
        .map_err(|e| TendrilError::Other(format!("Could not read the connection string for '{}': {e}", source.name)))?;
    options.server_selection_timeout = Some(TIMEOUT);
    options.connect_timeout = Some(TIMEOUT);
    options.app_name = Some("tendril-support".into());
    // A secondary when there is one, so a support lookup never loads the primary.
    options.selection_criteria = Some(SelectionCriteria::ReadPreference(ReadPreference::SecondaryPreferred { options: None }));
    let db_name = database
        .map(str::to_string)
        .or_else(|| options.default_database.clone())
        .ok_or_else(|| bad(format!("'{}' names no database; pass --db", source.name)))?;
    let client = mongodb::Client::with_options(options).map_err(|e| TendrilError::Other(e.to_string()))?;
    let db = client.database(&db_name);

    let to_json = |docs: Vec<Document>| -> String {
        let mut values: Vec<Value> = docs.into_iter().map(|d| Bson::Document(d).into_relaxed_extjson()).collect();
        values.iter_mut().for_each(redact);
        serde_json::to_string_pretty(&values).unwrap_or_default()
    };

    match read {
        MongoRead::Collections => {
            let mut names = db.list_collection_names().await.map_err(|e| TendrilError::Other(e.to_string()))?;
            names.sort();
            Ok((format!("{} collections", names.len()), names.join("\n")))
        }
        MongoRead::Find { collection, filter, projection, sort, limit } => {
            let filter = parse_doc(filter, "--filter")?;
            if let Some(stage) = walks_banned(&Bson::Document(filter.clone())) {
                return Err(bad(format!("{stage} is not allowed in a support lookup")));
            }
            let coll = db.collection::<Document>(collection);
            let mut find = coll
                .find(filter)
                .limit((*limit).clamp(1, MAX_DOCS))
                .max_time(TIMEOUT);
            let projection = parse_doc(projection, "--projection")?;
            if !projection.is_empty() {
                find = find.projection(projection);
            }
            let sort = parse_doc(sort, "--sort")?;
            if !sort.is_empty() {
                find = find.sort(sort);
            }
            let docs: Vec<Document> = find
                .await
                .map_err(|e| TendrilError::Other(e.to_string()))?
                .try_collect()
                .await
                .map_err(|e| TendrilError::Other(e.to_string()))?;
            Ok((format!("{} documents", docs.len()), to_json(docs)))
        }
        MongoRead::Count { collection, filter } => {
            let filter = parse_doc(filter, "--filter")?;
            if let Some(stage) = walks_banned(&Bson::Document(filter.clone())) {
                return Err(bad(format!("{stage} is not allowed in a support lookup")));
            }
            let n = db
                .collection::<Document>(collection)
                .count_documents(filter)
                .max_time(TIMEOUT)
                .await
                .map_err(|e| TendrilError::Other(e.to_string()))?;
            Ok((format!("{n} matching"), n.to_string()))
        }
        MongoRead::Aggregate { collection, pipeline } => {
            let value: Value = serde_json::from_str(pipeline).map_err(|e| bad(format!("--pipeline is not JSON: {e}")))?;
            let Bson::Array(stages) = Bson::try_from(value).map_err(|e| bad(e.to_string()))? else {
                return Err(bad("--pipeline must be a JSON array of stages"));
            };
            if let Some(stage) = walks_banned(&Bson::Array(stages.clone())) {
                return Err(bad(format!("{stage} is not allowed in a support lookup: lookups only read")));
            }
            let mut docs: Vec<Document> = stages
                .into_iter()
                .map(|s| match s {
                    Bson::Document(d) => Ok(d),
                    _ => Err(bad("each pipeline stage must be an object")),
                })
                .collect::<Result<_>>()?;
            docs.push(bson::doc! { "$limit": MAX_DOCS });
            let out: Vec<Document> = db
                .collection::<Document>(collection)
                .aggregate(docs)
                .max_time(TIMEOUT)
                .await
                .map_err(|e| TendrilError::Other(e.to_string()))?
                .try_collect()
                .await
                .map_err(|e| TendrilError::Other(e.to_string()))?;
            Ok((format!("{} documents", out.len()), to_json(out)))
        }
    }
}

/// Runs `read` against the Mongo source `source_name` for `ticket`, logging it on the ticket.
pub async fn mongo_read(home: &Path, ticket: &str, source_name: &str, database: Option<&str>, read: MongoRead) -> Result<String> {
    let source = support_cases::source(home, source_name)?;
    if source.kind != SourceKind::Mongo {
        return Err(bad(format!("'{}' is an API; use `tendril support api`", source.name)));
    }
    let described = read.describe();
    let result = tokio::time::timeout(TIMEOUT * 2, mongo(&source, database, &read))
        .await
        .map_err(|_| TendrilError::Other("The lookup timed out".into()))
        .and_then(|r| r);
    let summary = match &result {
        Ok((summary, _)) => summary.clone(),
        Err(e) => format!("failed: {e}"),
    };
    support_cases::log_lookup(home, ticket, &source.name, &described, &summary)?;
    result.map(|(_, body)| cap(body))
}

/// A `GET` of `path` (relative to the source's base URL) for `ticket`, logged on the ticket.
pub async fn api_get(home: &Path, ticket: &str, source_name: &str, path: &str) -> Result<String> {
    let source = support_cases::source(home, source_name)?;
    if source.kind != SourceKind::Http {
        return Err(bad(format!("'{}' is a database; use `tendril support db`", source.name)));
    }
    let path = path.trim();
    if path.contains("://") || path.starts_with("//") || path.contains("..") {
        return Err(bad("Pass a path under the source's base URL, like /api/accounts/123"));
    }
    let url = format!("{}/{}", source.url.trim_end_matches('/'), path.trim_start_matches('/'));
    let mut req = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .unwrap_or_default()
        .get(&url);
    if let Some((name, value)) = source.header.as_deref().and_then(|h| h.split_once(':')) {
        req = req.header(name.trim(), value.trim());
    }
    let result = async {
        let resp = req.send().await.map_err(|e| TendrilError::Other(format!("Could not reach {}: {e}", source.name)))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let body = match serde_json::from_str::<Value>(&text) {
            Ok(mut v) => {
                redact(&mut v);
                serde_json::to_string_pretty(&v).unwrap_or(text)
            }
            Err(_) => text,
        };
        Ok::<_, TendrilError>((format!("{} · {} bytes", status.as_u16(), body.len()), format!("HTTP {}\n{body}", status.as_u16())))
    }
    .await;
    let summary = match &result {
        Ok((s, _)) => s.clone(),
        Err(e) => format!("failed: {e}"),
    };
    support_cases::log_lookup(home, ticket, &source.name, &format!("GET {path}"), &summary)?;
    result.map(|(_, body)| cap(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_and_code_running_stages_are_refused() {
        let pipeline: Value = serde_json::from_str(r#"[{"$match":{"a":1}},{"$merge":{"into":"x"}}]"#).unwrap();
        assert_eq!(walks_banned(&Bson::try_from(pipeline).unwrap()), Some("$merge"));
        let filter = parse_doc(r#"{"$where":"sleep(1000)"}"#, "f").unwrap();
        assert_eq!(walks_banned(&Bson::Document(filter)), Some("$where"));
        let fine = parse_doc(r#"{"email":"jo@acme.com","status":{"$in":["A","B"]}}"#, "f").unwrap();
        assert_eq!(walks_banned(&Bson::Document(fine)), None);
    }

    #[test]
    fn secrets_are_redacted_at_any_depth() {
        let mut v: Value = serde_json::from_str(
            r#"{"email":"jo@acme.com","passwordHash":"x","profile":{"apiKey":"k","name":"Jo"},"sessions":[{"token":"t"}]}"#,
        )
        .unwrap();
        redact(&mut v);
        assert_eq!(v["passwordHash"], "[redacted]");
        assert_eq!(v["profile"]["apiKey"], "[redacted]");
        assert_eq!(v["profile"]["name"], "Jo");
        assert_eq!(v["sessions"][0]["token"], "[redacted]");
    }
}
