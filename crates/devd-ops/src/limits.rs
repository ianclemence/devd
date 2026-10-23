//! PHP limits here against the limits of your server.
//!
//! A limit that is looser here than on the server is a bug that only shows after the deploy: an upload
//! that works locally and fails in production. Ploi's API does not expose a server's PHP limits, so devd
//! is told what the server has (read them from the PHP tab in Ploi) with `ploi.limits_set`; until then it
//! assumes the `production` profile, which is what a default Ploi server runs, and says so.

use std::collections::BTreeMap;

use devd_core::capability::Ctx;
use devd_core::{DevdError, Result};
use devd_runtime::phpconf;

/// The kind under which the server's limits are stored.
pub const KIND: &str = "server_limits";

/// The limits that are compared.
pub const KEYS: &[&str] = &["memory_limit", "upload_max_filesize", "post_max_size", "max_execution_time"];

/// A PHP size (`2G`, `128M`, `512K`, `1048576`) in bytes. `-1` and `0` for a time are unlimited.
pub fn bytes(value: &str) -> Option<u128> {
    let v = value.trim();
    if v == "-1" {
        return Some(u128::MAX);
    }
    let (digits, unit) = match v.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((i, _)) => v.split_at(i),
        None => (v, ""),
    };
    let n: u128 = digits.parse().ok()?;
    let factor: u128 = match unit.trim().to_ascii_uppercase().as_str() {
        "" | "B" => 1,
        "K" | "KB" => 1 << 10,
        "M" | "MB" => 1 << 20,
        "G" | "GB" => 1 << 30,
        _ => return None,
    };
    Some(n.saturating_mul(factor))
}

fn seconds(value: &str) -> Option<u128> {
    let n: i128 = value.trim().parse().ok()?;
    Some(if n <= 0 { u128::MAX } else { n as u128 })
}

/// What the site's PHP allows here: its own profile when it has one, else the values in `php.ini`.
pub fn local(ctx: &Ctx, site: &str) -> Result<BTreeMap<String, String>> {
    let s = crate::site_named(ctx, site)?;
    if let Some(profile) = devd_services::site_profile(ctx, site) {
        if devd_services::alt_sites(ctx).contains(site) {
            return phpconf::profile_values(&profile);
        }
    }
    let php = crate::php_for_site(ctx, &s)?;
    let ini = php.parent().map(|d| d.join("php.ini"));
    // What PHP will really use: the ini's last assignments. A setting the ini never assigns is PHP's own default.
    let mut out = ini.filter(|p| p.is_file()).and_then(|p| phpconf::effective_values(&p, KEYS).ok()).unwrap_or_default();
    for (k, v) in [("memory_limit", "128M"), ("upload_max_filesize", "2M"), ("post_max_size", "8M"), ("max_execution_time", "30")] {
        out.entry(k.to_string()).or_insert_with(|| v.to_string());
    }
    Ok(out)
}

/// The server's limits and whether devd only assumes them.
pub fn server(ctx: &Ctx) -> Result<(BTreeMap<String, String>, bool)> {
    match ctx.store.ops_get(KIND, "ploi")? {
        Some(i) => {
            let mut out = phpconf::profile_values("production")?;
            for k in KEYS {
                if let Some(v) = i.data[k].as_str() {
                    out.insert(k.to_string(), v.to_string());
                }
            }
            Ok((out, false))
        }
        None => Ok((phpconf::profile_values("production")?, true)),
    }
}

/// Store what the server runs. Only the named limits change.
pub fn set_server(ctx: &Ctx, set: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>> {
    let (mut current, _) = server(ctx)?;
    for (k, v) in set {
        if !KEYS.contains(&k.as_str()) {
            return Err(DevdError::invalid(format!("`{k}` is not a limit devd compares; use {}", KEYS.join(", "))));
        }
        let ok = if k == "max_execution_time" { seconds(v).is_some() } else { bytes(v).is_some() };
        if !ok {
            return Err(DevdError::invalid(format!("`{v}` is not a usable value for {k} (like 128M, 2G, 30)")));
        }
        current.insert(k.clone(), v.trim().to_string());
    }
    let doc: serde_json::Map<String, serde_json::Value> = current.iter().map(|(k, v)| (k.clone(), serde_json::json!(v))).collect();
    ctx.store.ops_put(KIND, "ploi", "", &serde_json::Value::Object(doc))?;
    Ok(current)
}

/// The differences that matter: every limit that is looser here than on the server.
pub fn compare(local: &BTreeMap<String, String>, server: &BTreeMap<String, String>, assumed: bool) -> Vec<String> {
    let mut out = Vec::new();
    for k in KEYS {
        let (Some(l), Some(s)) = (local.get(*k), server.get(*k)) else { continue };
        let (lv, sv) = if *k == "max_execution_time" { (seconds(l), seconds(s)) } else { (bytes(l), bytes(s)) };
        if let (Some(lv), Some(sv)) = (lv, sv) {
            if lv > sv {
                out.push(format!(
                    "{k} is {l} here but {s} on {}: {}",
                    if assumed { "a default Ploi server (assumed; tell devd yours with ploi.limits_set)" } else { "your server" },
                    match *k {
                        "memory_limit" => "a request that fits here can run out of memory there",
                        "upload_max_filesize" | "post_max_size" => "an upload that works here will be refused there",
                        _ => "a request that finishes here can be cut off there",
                    }
                ));
            }
        }
    }
    out
}

/// The warnings for a site: its limits here against the server's.
pub fn warnings(ctx: &Ctx, site: &str) -> Vec<String> {
    let (Ok(here), Ok((there, assumed))) = (local(ctx, site), server(ctx)) else { return Vec::new() };
    compare(&here, &there, assumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn sizes_and_times_are_read_the_way_php_reads_them() {
        assert_eq!(bytes("128M"), Some(128 << 20));
        assert_eq!(bytes("2G"), Some(2 << 30));
        assert_eq!(bytes("512k"), Some(512 << 10));
        assert_eq!(bytes("1048576"), Some(1 << 20));
        assert_eq!(bytes("-1"), Some(u128::MAX));
        assert_eq!(bytes("lots"), None);
        assert_eq!(seconds("30"), Some(30));
        assert_eq!(seconds("0"), Some(u128::MAX));
    }

    #[test]
    fn only_limits_that_are_looser_here_are_reported() {
        let here = m(&[("memory_limit", "2G"), ("upload_max_filesize", "5M"), ("post_max_size", "8M"), ("max_execution_time", "36000")]);
        let there = m(&[("memory_limit", "128M"), ("upload_max_filesize", "5M"), ("post_max_size", "8M"), ("max_execution_time", "30")]);
        let w = compare(&here, &there, false);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].starts_with("memory_limit is 2G here but 128M on your server"), "{w:?}");
        assert!(w[1].starts_with("max_execution_time is 36000"), "{w:?}");
        assert!(compare(&there, &here, false).is_empty(), "tighter here is fine");
        assert!(compare(&here, &there, true)[0].contains("assumed"));
    }

    #[test]
    fn unlimited_counts_as_the_loosest() {
        let here = m(&[("memory_limit", "-1")]);
        let there = m(&[("memory_limit", "4G")]);
        assert_eq!(compare(&here, &there, false).len(), 1);
        assert!(compare(&there, &here, false).is_empty());
    }
}
