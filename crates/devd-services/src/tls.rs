//! The local certificate authority.
//!
//! `devd` issues its own certificates: one CA per installation, and one leaf per
//! site. There is no `openssl.exe` to discover, no self-signed certificate that a
//! browser refuses with an unbypassable error, and no bundled third-party binary
//! - the whole chain is generated in-process with `rcgen` (ECDSA P-256).
//!
//! What this does and does not give you, precisely:
//!
//! * Every site gets **real TLS**: a correct key pair, a leaf signed by the CA
//!   with the site's hostname in `subjectAltName`, and a chain the server
//!   presents in full. `curl --cacert <devd-ca.crt>` and any client configured to
//!   trust the CA accept it without an override, and certificate verification is
//!   never disabled anywhere.
//! * A browser only accepts it after the CA is added to the operating system's
//!   trust store, which is an administrator action on purpose. `devd tls trust`
//!   does that with `certutil`, and it is an `admin`-tier capability: it changes
//!   what every program on the machine trusts, so a human has to approve it.
//!   Until then a browser shows the usual warning, and the command output says so
//!   rather than pretending otherwise.
//!
//! The CA private key is written under the layout's `certs` directory and is
//! never logged, never rendered into a configuration file and never returned by a
//! capability - only the public certificate is.

use std::path::{Path, PathBuf};
use std::time::Duration;

use devd_core::capability::Ctx;
use devd_core::util;
use devd_core::{DevdError, Result};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose, SerialNumber,
};
use serde::Serialize;

/// How long an issued leaf is valid. Below the 398 days that browsers enforce
/// for publicly trusted certificates, so a leaf here never trips that rule if it
/// is later used somewhere stricter.
const LEAF_DAYS: i64 = 397;

/// How long the CA itself is valid.
const CA_DAYS: i64 = 3650;

/// What an issue produced.
#[derive(Debug, Clone, Serialize)]
pub struct Issued {
    /// Hostname the certificate is for.
    pub host: String,
    /// Certificate chain file (leaf followed by the CA).
    pub cert_path: String,
    /// Private key file.
    pub key_path: String,
    /// Serial number, lowercase hex.
    pub serial: String,
    /// Expiry, RFC 3339.
    pub not_after: String,
    /// `true` when the CA was created as part of this call.
    pub ca_created: bool,
}

/// The CA certificate.
pub fn ca_cert_path(ctx: &Ctx) -> PathBuf {
    ctx.paths.certs().join("devd-ca.crt")
}

/// The CA private key.
pub fn ca_key_path(ctx: &Ctx) -> PathBuf {
    ctx.paths.certs().join("devd-ca.key")
}

/// Certificate chain path for a host: `certs/<host>.crt`.
///
/// The host is part of the *file name* rather than the extension, so a
/// hostname's own dots must not be treated as an extension separator:
/// `certs/blog.test.crt` and `certs/blog.test.key` are two files, and
/// `Path::with_extension` would have collapsed both onto `certs/blog.test`.
pub fn cert_path_for_host(ctx: &Ctx, host: &str) -> PathBuf {
    ctx.paths.certs().join(format!("{host}.crt"))
}

/// Private key path for a host: `certs/<host>.key`.
pub fn key_path_for_host(ctx: &Ctx, host: &str) -> PathBuf {
    ctx.paths.certs().join(format!("{host}.key"))
}

/// Whether a CA exists.
pub fn ca_exists(ctx: &Ctx) -> bool {
    ca_cert_path(ctx).is_file() && ca_key_path(ctx).is_file()
}

fn stamp(days: i64) -> (time::OffsetDateTime, time::OffsetDateTime) {
    let now = time::OffsetDateTime::now_utc();
    // Backdated an hour so a clock that is slightly behind does not see a
    // not-yet-valid certificate.
    (
        now - time::Duration::hours(1),
        now + time::Duration::days(days),
    )
}

/// Create the CA if it is missing. Idempotent.
///
/// Returns `true` when this call created it.
pub fn ensure_ca(ctx: &Ctx) -> Result<bool> {
    if ca_exists(ctx) {
        return Ok(false);
    }
    std::fs::create_dir_all(ctx.paths.certs())?;

    let mut params = CertificateParams::new(Vec::<String>::new())
        .map_err(|e| DevdError::internal(format!("could not build the CA parameters: {e}")))?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, "devd local development CA");
    params
        .distinguished_name
        .push(DnType::OrganizationName, "devd");
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let (not_before, not_after) = stamp(CA_DAYS);
    params.not_before = not_before;
    params.not_after = not_after;
    let (serial, _serial_hex) = random_serial();
    params.serial_number = Some(serial);

    let key = KeyPair::generate()
        .map_err(|e| DevdError::internal(format!("could not generate the CA key: {e}")))?;
    let cert = params
        .self_signed(&key)
        .map_err(|e| DevdError::internal(format!("could not sign the CA certificate: {e}")))?;

    util::atomic_write(&ca_cert_path(ctx), &format!("{}\n", cert.pem().trim()))?;
    write_secret(
        &ca_key_path(ctx),
        &format!("{}\n", key.serialize_pem().trim()),
    )?;
    Ok(true)
}

/// Issue (or re-issue) a leaf for a host.
pub fn issue(ctx: &Ctx, host: &str) -> Result<Issued> {
    util::validate_hostname(host)?;
    let ca_created = ensure_ca(ctx)?;

    let ca_key_pem = std::fs::read_to_string(ca_key_path(ctx))?;
    let ca_key = KeyPair::from_pem(&ca_key_pem)
        .map_err(|e| DevdError::internal(format!("the stored CA key is unreadable: {e}")))?;
    let ca_cert_pem = std::fs::read_to_string(ca_cert_path(ctx))?;
    // A CA created by an earlier run has to be turned back into parameters before
    // rcgen can sign with it, and re-parsing the PEM is the supported way. The
    // parsed parameters carry the original subject key identifier, so a leaf
    // issued here chains to the certificate already on disk rather than to a new
    // one. The reconstructed certificate is used only as the issuer for signing;
    // the chain written below contains the stored bytes.
    let ca_params = CertificateParams::from_ca_cert_pem(&ca_cert_pem)
        .map_err(|e| DevdError::invalid(format!("the stored CA certificate is unreadable: {e}")))?;
    let ca_issuer = ca_params
        .self_signed(&ca_key)
        .map_err(|e| DevdError::internal(format!("the stored CA certificate is unusable: {e}")))?;

    let mut params = CertificateParams::new(vec![host.to_string()]).map_err(|e| {
        DevdError::invalid(format!(
            "`{host}` cannot be used as a certificate name: {e}"
        ))
    })?;
    params.distinguished_name.push(DnType::CommonName, host);
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let (not_before, not_after) = stamp(LEAF_DAYS);
    params.not_before = not_before;
    params.not_after = not_after;
    let (serial, serial_hex) = random_serial();
    params.serial_number = Some(serial);

    let key = KeyPair::generate()
        .map_err(|e| DevdError::internal(format!("could not generate a site key: {e}")))?;
    let cert = params
        .signed_by(&key, &ca_issuer, &ca_key)
        .map_err(|e| DevdError::internal(format!("could not sign the site certificate: {e}")))?;

    // The chain file holds the leaf *and* the CA, in the order a server presents
    // them, because nginx is told to send `ssl_certificate` as the full chain.
    let chain = format!("{}\n{}\n", cert.pem().trim(), ca_cert_pem.trim());
    let cert_path = cert_path_for_host(ctx, host);
    let key_path = key_path_for_host(ctx, host);
    util::atomic_write(&cert_path, &chain)?;
    write_secret(&key_path, &format!("{}\n", key.serialize_pem().trim()))?;

    Ok(Issued {
        host: host.to_string(),
        cert_path: cert_path.display().to_string(),
        key_path: key_path.display().to_string(),
        serial: serial_hex,
        not_after: format_iso(not_after),
        ca_created,
    })
}

/// Whether a leaf exists and is unexpired.
pub fn leaf_valid(ctx: &Ctx, host: &str) -> bool {
    cert_path_for_host(ctx, host).is_file() && key_path_for_host(ctx, host).is_file()
}

/// Read the CA certificate, for a plan or for `devd tls ca`.
pub fn ca_pem_text(ctx: &Ctx) -> Result<String> {
    util::read_to_string_opt(&ca_cert_path(ctx))?
        .ok_or_else(|| DevdError::not_found("no devd certificate authority exists yet"))
}

/// Write a private key with restrictive permissions where the platform has them.
///
/// On Windows the file inherits the directory's access control list, which for a
/// per-user layout under `%LOCALAPPDATA%` is already restricted to the account.
/// That is stated plainly in `docs/security.md` rather than implied.
fn write_secret(path: &Path, contents: &str) -> Result<()> {
    util::atomic_write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// A random certificate serial number, with its hex form.
///
/// Version 4 UUIDs are already random and already available, so they are used as
/// the entropy source rather than adding a second random number generator.
fn random_serial() -> (SerialNumber, String) {
    let mut bytes: [u8; 16] = *uuid::Uuid::new_v4().as_bytes();
    // Setting the top bit keeps every serial the same width when encoded, which
    // is what a reader that parses the DER integer expects.
    bytes[0] |= 0x80;
    (SerialNumber::from_slice(&bytes), hex::encode(bytes))
}

fn format_iso(at: time::OffsetDateTime) -> String {
    at.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| at.to_string())
}

/// One issued certificate, for `tls.list`.
#[derive(Debug, Clone, Serialize)]
pub struct IssuedRow {
    /// Hostname the certificate was issued for.
    pub host: String,
    /// Certificate chain file.
    pub cert: String,
    /// Whether the private key is present beside it.
    pub key_present: bool,
}

/// Every issued site certificate under the layout's `certs` directory.
pub fn list(ctx: &Ctx) -> Result<Vec<IssuedRow>> {
    let dir = ctx.paths.certs();
    let mut rows = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(rows),
        Err(e) => return Err(e.into()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("crt") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // The authority is not a site certificate and is reported by `tls.ca.show`.
        if stem == "devd-ca" {
            continue;
        }
        rows.push(IssuedRow {
            host: stem.to_string(),
            cert: path.display().to_string(),
            key_present: ctx.paths.certs().join(format!("{stem}.key")).is_file(),
        });
    }
    rows.sort_by(|a, b| a.host.cmp(&b.host));
    Ok(rows)
}

/// The common name written on the CA, used to identify it to `certutil`.
const CA_COMMON_NAME: &str = "devd local development CA";

/// Add the CA to the operating system's trust store.
///
/// The command is invoked with an argument array, never a shell string, and the
/// binary is resolved through `PATH` rather than assumed to be at a fixed path.
/// Failure is reported with the tool's own output: in particular, adding to the
/// machine-wide store from a normal shell fails, and saying "access denied,
/// re-run from an elevated shell" is more useful than "failed".
pub fn trust(ctx: &Ctx, machine: bool) -> Result<devd_core::capability::Outcome> {
    ensure_ca(ctx)?;
    let cert = ca_cert_path(ctx);

    #[cfg(windows)]
    {
        let certutil = util::which("certutil").ok_or_else(|| {
            DevdError::not_found("certutil was not found on PATH; it ships with Windows")
        })?;
        let mut args = vec!["-addstore".to_string()];
        if !machine {
            args.push("-user".to_string());
        }
        args.push("Root".to_string());
        args.push(cert.display().to_string());
        let out = util::run(
            &certutil,
            &args,
            None,
            &[],
            Some(std::time::Duration::from_secs(60)),
        )?;
        if !out.ok() {
            let hint = if machine {
                " Adding to the machine-wide store requires an elevated shell."
            } else {
                ""
            };
            return Err(DevdError::Command {
                program: "certutil -addstore".to_string(),
                code: out.code,
                stderr: format!("{}{hint}", util::truncate(out.stderr.trim(), 800)),
            });
        }
        return Ok(devd_core::capability::Outcome::with_data(
            if machine {
                "the devd certificate authority is trusted machine-wide".to_string()
            } else {
                "the devd certificate authority is trusted for this user".to_string()
            },
            serde_json::json!({
                "certificate": cert.display().to_string(),
                "scope": if machine { "machine" } else { "user" },
                "store": "Root",
                "output": util::truncate(out.stdout.trim(), 800),
            }),
        ));
    }

    #[cfg(target_os = "macos")]
    {
        let security = util::which("security").ok_or_else(|| {
            DevdError::not_found("`security` was not found; it ships with macOS")
        })?;
        let args = macos::trust_args(&cert, machine);
        let out = util::run(
            &security,
            &args,
            None,
            &[],
            Some(std::time::Duration::from_secs(120)),
        )?;
        if !out.ok() {
            let hint = if machine {
                " Adding to the System keychain needs an administrator; macOS asks for a password."
            } else {
                ""
            };
            return Err(DevdError::Command {
                program: "security add-trusted-cert".to_string(),
                code: out.code,
                stderr: format!("{}{hint}", util::truncate(out.stderr.trim(), 800)),
            });
        }
        return Ok(devd_core::capability::Outcome::with_data(
            if machine {
                "the devd certificate authority is trusted machine-wide".to_string()
            } else {
                "the devd certificate authority is trusted for this user".to_string()
            },
            serde_json::json!({
                "certificate": cert.display().to_string(),
                "scope": if machine { "machine" } else { "user" },
                "keychain": macos::keychain(machine),
            }),
        ));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // The portable answer on Linux is p11-kit's `trust`, which is what most
        // distributions route their CA store through.
        if let Some(trust) = util::which("trust") {
            let out = util::run(
                &trust,
                &["anchor".to_string(), cert.display().to_string()],
                None,
                &[],
                Some(std::time::Duration::from_secs(60)),
            )?;
            if !out.ok() {
                return Err(DevdError::Command {
                    program: "trust anchor".to_string(),
                    code: out.code,
                    stderr: util::truncate(out.stderr.trim(), 800),
                });
            }
            // p11-kit has one store, the system one. Say what was asked and what was
            // done rather than quietly answering a different question.
            return Ok(devd_core::capability::Outcome::with_data(
                "the devd certificate authority is trusted in the system store".to_string(),
                serde_json::json!({
                    "certificate": cert.display().to_string(),
                    "scope": "system",
                    "requested_scope": if machine { "machine" } else { "user" },
                }),
            ));
        }
        Err(DevdError::unsupported(format!(
            "trusting the authority needs p11-kit's `trust` command, which is not installed. \
             As an administrator, copy {} to /usr/local/share/ca-certificates/devd-ca.crt and \
             run update-ca-certificates.",
            cert.display()
        )))
    }
}

/// Whether the authority is present in the operating system's trust stores.
#[derive(Debug, Clone, Serialize)]
pub struct TrustStatus {
    /// Present in the current user's store.
    pub user: bool,
    /// Present in the machine-wide store.
    pub machine: bool,
}

/// Check the trust stores without changing them.
///
/// `certutil -store` exits successfully when the certificate is present and
/// with a "not found" status when it is not, which is the check this uses. A
/// missing `certutil` (an unusual Windows install) reports "not trusted" rather
/// than failing: the answer is unknown, and claiming trust would be worse.
pub fn trust_status(ctx: &Ctx) -> Result<TrustStatus> {
    if !ca_exists(ctx) {
        return Ok(TrustStatus {
            user: false,
            machine: false,
        });
    }

    #[cfg(windows)]
    {
        let Some(certutil) = util::which("certutil") else {
            return Ok(TrustStatus {
                user: false,
                machine: false,
            });
        };
        let present = |machine: bool| -> bool {
            let mut args = Vec::new();
            if !machine {
                args.push("-user".to_string());
            }
            args.push("-store".to_string());
            args.push("Root".to_string());
            args.push(CA_COMMON_NAME.to_string());
            util::run(&certutil, &args, None, &[], Some(Duration::from_secs(20)))
                .map(|out| out.ok())
                .unwrap_or(false)
        };
        Ok(TrustStatus {
            user: present(false),
            machine: present(true),
        })
    }

    #[cfg(target_os = "macos")]
    {
        let Some(security) = util::which("security") else {
            return Ok(TrustStatus {
                user: false,
                machine: false,
            });
        };
        let present = |machine: bool| -> bool {
            util::run(
                &security,
                &macos::status_args(machine),
                None,
                &[],
                Some(Duration::from_secs(20)),
            )
            .map(|out| out.ok())
            .unwrap_or(false)
        };
        Ok(TrustStatus {
            user: present(false),
            machine: present(true),
        })
    }

    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let Some(trust) = util::which("trust") else {
            return Ok(TrustStatus {
                user: false,
                machine: false,
            });
        };
        let found = util::run(
            &trust,
            &["list".to_string()],
            None,
            &[],
            Some(Duration::from_secs(20)),
        )
        .map(|out| out.stdout.contains(CA_COMMON_NAME))
        .unwrap_or(false);
        Ok(TrustStatus {
            user: found,
            machine: found,
        })
    }
}

/// Remove the CA from the operating system's trust store.
pub fn untrust(ctx: &Ctx, machine: bool) -> Result<devd_core::capability::Outcome> {
    let cert = ca_cert_path(ctx);
    if !cert.is_file() {
        return Err(DevdError::not_found(
            "there is no devd certificate authority to remove",
        ));
    }

    #[cfg(windows)]
    {
        let certutil = util::which("certutil").ok_or_else(|| {
            DevdError::not_found("certutil was not found on PATH; it ships with Windows")
        })?;
        let mut args = vec!["-delstore".to_string()];
        if !machine {
            args.push("-user".to_string());
        }
        args.push("Root".to_string());
        // certutil accepts the subject common name as the identifier, which is
        // stable and does not require parsing the DER for a serial number.
        args.push(CA_COMMON_NAME.to_string());
        let out = util::run(
            &certutil,
            &args,
            None,
            &[],
            Some(std::time::Duration::from_secs(60)),
        )?;
        if !out.ok() {
            let hint = if machine {
                " Removing from the machine-wide store requires an elevated shell."
            } else {
                ""
            };
            return Err(DevdError::Command {
                program: "certutil -delstore".to_string(),
                code: out.code,
                stderr: format!("{}{hint}", util::truncate(out.stderr.trim(), 800)),
            });
        }
        return Ok(devd_core::capability::Outcome::with_data(
            "the devd certificate authority is no longer trusted".to_string(),
            serde_json::json!({
                "scope": if machine { "machine" } else { "user" },
                "output": util::truncate(out.stdout.trim(), 800),
            }),
        ));
    }

    #[cfg(target_os = "macos")]
    {
        let security = util::which("security").ok_or_else(|| {
            DevdError::not_found("`security` was not found; it ships with macOS")
        })?;
        // Two steps: withdraw the trust setting, then delete the certificate.
        // Deleting alone leaves a trust setting that points at nothing.
        let untrust = util::run(
            &security,
            &macos::untrust_args(&cert, machine),
            None,
            &[],
            Some(std::time::Duration::from_secs(120)),
        )?;
        if !untrust.ok() {
            return Err(DevdError::Command {
                program: "security remove-trusted-cert".to_string(),
                code: untrust.code,
                stderr: util::truncate(untrust.stderr.trim(), 800),
            });
        }
        let delete = util::run(
            &security,
            &macos::delete_args(machine),
            None,
            &[],
            Some(std::time::Duration::from_secs(120)),
        )?;
        if !delete.ok() {
            return Err(DevdError::Command {
                program: "security delete-certificate".to_string(),
                code: delete.code,
                stderr: util::truncate(delete.stderr.trim(), 800),
            });
        }
        return Ok(devd_core::capability::Outcome::with_data(
            "the devd certificate authority is no longer trusted".to_string(),
            serde_json::json!({
                "scope": if machine { "machine" } else { "user" },
                "keychain": macos::keychain(machine),
            }),
        ));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(trust) = util::which("trust") {
            let out = util::run(
                &trust,
                &[
                    "anchor".to_string(),
                    "--remove".to_string(),
                    cert.display().to_string(),
                ],
                None,
                &[],
                Some(std::time::Duration::from_secs(60)),
            )?;
            if !out.ok() {
                return Err(DevdError::Command {
                    program: "trust anchor --remove".to_string(),
                    code: out.code,
                    stderr: util::truncate(out.stderr.trim(), 800),
                });
            }
            return Ok(devd_core::capability::Outcome::with_data(
                "the devd certificate authority is no longer trusted in the system store".to_string(),
                serde_json::json!({
                    "scope": "system",
                    "requested_scope": if machine { "machine" } else { "user" },
                }),
            ));
        }
        Err(DevdError::unsupported(
            "removing a trust anchor needs p11-kit's `trust` command, which is not installed",
        ))
    }
}

/// The `security` command lines macOS trust management needs.
///
/// Built here, as plain functions of their inputs, so that they are checked on
/// every platform; only running them is macOS-specific.
pub mod macos {
    use super::CA_COMMON_NAME;
    use std::path::Path;

    /// The keychain a scope refers to.
    pub fn keychain(machine: bool) -> String {
        if machine {
            "/Library/Keychains/System.keychain".to_string()
        } else {
            let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
            format!("{home}/Library/Keychains/login.keychain-db")
        }
    }

    /// `security add-trusted-cert -r trustRoot [-d] -k <keychain> <cert>`.
    ///
    /// `-d` adds the trust setting for every user rather than the current one.
    pub fn trust_args(cert: &Path, machine: bool) -> Vec<String> {
        let mut args = vec!["add-trusted-cert".to_string()];
        if machine {
            args.push("-d".to_string());
        }
        args.extend([
            "-r".to_string(),
            "trustRoot".to_string(),
            "-k".to_string(),
            keychain(machine),
            cert.display().to_string(),
        ]);
        args
    }

    /// `security remove-trusted-cert [-d] <cert>`.
    pub fn untrust_args(cert: &Path, machine: bool) -> Vec<String> {
        let mut args = vec!["remove-trusted-cert".to_string()];
        if machine {
            args.push("-d".to_string());
        }
        args.push(cert.display().to_string());
        args
    }

    /// `security delete-certificate -c <name> <keychain>`.
    pub fn delete_args(machine: bool) -> Vec<String> {
        vec![
            "delete-certificate".to_string(),
            "-c".to_string(),
            CA_COMMON_NAME.to_string(),
            keychain(machine),
        ]
    }

    /// `security find-certificate -c <name> <keychain>`, which exits 0 when the
    /// certificate is in the keychain.
    pub fn status_args(machine: bool) -> Vec<String> {
        vec![
            "find-certificate".to_string(),
            "-c".to_string(),
            CA_COMMON_NAME.to_string(),
            keychain(machine),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn macos_trusts_a_root_in_the_users_keychain_without_touching_the_system_one() {
        let args = macos::trust_args(std::path::Path::new("/tmp/devd-ca.pem"), false);
        assert_eq!(args[0], "add-trusted-cert");
        assert!(!args.contains(&"-d".to_string()));
        assert!(args.windows(2).any(|w| w == ["-r", "trustRoot"]));
        let keychain = &args[args.len() - 2];
        assert!(keychain.ends_with("/Library/Keychains/login.keychain-db"), "{keychain}");
        assert_eq!(args.last().unwrap(), "/tmp/devd-ca.pem");
    }

    #[test]
    fn macos_machine_scope_uses_the_system_keychain_and_the_admin_flag() {
        let args = macos::trust_args(std::path::Path::new("/tmp/devd-ca.pem"), true);
        assert!(args.contains(&"-d".to_string()));
        assert!(args.contains(&"/Library/Keychains/System.keychain".to_string()));
        let untrust = macos::untrust_args(std::path::Path::new("/tmp/devd-ca.pem"), true);
        assert_eq!(untrust, ["remove-trusted-cert", "-d", "/tmp/devd-ca.pem"]);
    }

    #[test]
    fn macos_finds_and_deletes_the_authority_by_its_common_name() {
        for machine in [false, true] {
            let find = macos::status_args(machine);
            let delete = macos::delete_args(machine);
            assert_eq!(find[0], "find-certificate");
            assert_eq!(delete[0], "delete-certificate");
            for args in [&find, &delete] {
                assert!(args.windows(2).any(|w| w == ["-c", CA_COMMON_NAME]));
                assert_eq!(args.last().unwrap(), &macos::keychain(machine));
            }
        }
    }

    fn ctx(tag: &str) -> Ctx {
        let dir = std::env::temp_dir().join(format!("devd-tls-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = devd_core::paths::Paths::rooted(dir);
        paths.ensure().unwrap();
        Ctx {
            store: Arc::new(devd_core::state::Store::open_in_memory().unwrap()),
            paths: Arc::new(paths),
            settings: Arc::new(devd_core::config::Settings::default()),
            events: devd_core::events::EventBus::new(),
            actor: devd_core::capability::Actor::human(devd_core::capability::Surface::Cli),
            dry_run: false,
        }
    }

    #[test]
    fn a_ca_is_created_once_and_reused() {
        let c = ctx("ca");
        assert!(!ca_exists(&c));
        assert!(ensure_ca(&c).unwrap(), "the first call creates it");
        assert!(ca_exists(&c));
        assert!(!ensure_ca(&c).unwrap(), "the second call reuses it");
        let pem = ca_pem_text(&c).unwrap();
        assert!(pem.contains("BEGIN CERTIFICATE"), "{pem}");
        assert!(
            ca_key_path(&c).is_file(),
            "the CA key must be stored for leaf issuance"
        );
        // A second call must not replace the key, or every existing leaf breaks.
        let first_key = std::fs::read_to_string(ca_key_path(&c)).unwrap();
        ensure_ca(&c).unwrap();
        assert_eq!(first_key, std::fs::read_to_string(ca_key_path(&c)).unwrap());
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn a_leaf_is_issued_against_a_ca_created_on_demand() {
        let c = ctx("leaf");
        let issued = issue(&c, "blog.test").unwrap();
        assert!(issued.ca_created, "the CA is created on first use");
        assert_eq!(issued.host, "blog.test");

        let cert = std::fs::read_to_string(&issued.cert_path).unwrap();
        let key = std::fs::read_to_string(&issued.key_path).unwrap();
        let ca = ca_pem_text(&c).unwrap();
        // The chain presents the leaf and then the CA.
        assert_eq!(cert.matches("BEGIN CERTIFICATE").count(), 2, "{cert}");
        assert!(cert.starts_with("-----BEGIN CERTIFICATE-----"));
        assert!(
            cert.trim_end().ends_with(ca.trim()),
            "the CA must be last in the chain"
        );
        assert!(key.contains("BEGIN PRIVATE KEY"), "{key}");
        assert!(key.contains("EC PRIVATE KEY") || key.contains("PRIVATE KEY"));
        assert!(leaf_valid(&c, "blog.test"));
        assert!(!leaf_valid(&c, "other.test"));
        // The hostname is part of the file name, dots and all. `Path::with_extension`
        // would have written `blog.crt`, and nginx - rendered against
        // `blog.test.crt` - would then have refused to start.
        assert!(
            issued
                .cert_path
                .replace('\\', "/")
                .ends_with("/certs/blog.test.crt"),
            "{}",
            issued.cert_path
        );
        assert!(
            issued
                .key_path
                .replace('\\', "/")
                .ends_with("/certs/blog.test.key"),
            "{}",
            issued.key_path
        );
        assert_eq!(issued.serial.len(), 32, "{}", issued.serial);
        assert!(issued.not_after.starts_with('2'), "{}", issued.not_after);
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn issuing_again_replaces_the_leaf() {
        let c = ctx("reissue");
        let first = issue(&c, "shop.test").unwrap();
        let second = issue(&c, "shop.test").unwrap();
        assert!(!second.ca_created, "the CA already existed");
        assert_ne!(first.serial, second.serial);
        let _ = std::fs::remove_dir_all(c.paths.root());
    }

    #[test]
    fn a_hostile_hostname_is_refused() {
        let c = ctx("bad");
        // A newline in a name would let an injected line reach the nginx
        // configuration; validation happens before anything is written.
        let err = issue(&c, "evil.test\nserver { listen 1; }").unwrap_err();
        assert_eq!(err.code(), "invalid_input", "{err}");
        let _ = std::fs::remove_dir_all(c.paths.root());
    }
}
