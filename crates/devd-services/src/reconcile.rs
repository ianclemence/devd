//! Reconcile what devd recorded about its services with what is running.
//!
//! A daemon that restarts, or devd being killed, leaves the records and the
//! processes out of step. This pass compares them and says what it finds. It
//! decides ownership from the pid record (process id, start time and image), and
//! never from a port: a program that happens to listen on nginx's port is not
//! nginx, and devd neither adopts nor stops it. It reports it.

use serde::Serialize;

use devd_core::capability::Ctx;
use devd_core::process;

use crate::{local_tcp_open, running_pid};

/// What a pass found about one service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Finding {
    /// devd started it and it is still running; devd manages it.
    Adopted,
    /// A pid record exists but no such devd-started process does; it was removed.
    StaleRecord,
    /// Something listens on the service's port, but devd cannot show it started
    /// it. It is left alone.
    AmbiguousOwner,
    /// The service should be running and nothing is.
    Down,
    /// The service is running although it was meant to be stopped.
    UnexpectedlyRunning,
}

/// One service and what was found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Service name.
    pub service: String,
    /// What was found.
    pub finding: Finding,
    /// A sentence saying so.
    pub detail: String,
}

/// Compare every recorded service with the processes devd can prove are its own.
/// A healthy service that is meant to be stopped and is stopped reports nothing.
pub fn reconcile(ctx: &Ctx) -> Vec<Report> {
    let services = ctx.store.list_services().unwrap_or_default();
    let mut reports = Vec::new();
    for service in services {
        let wants_running = service.desired_state == "running";
        let owned = running_pid(ctx, &service);
        let listening = service.port.is_some_and(|p| p != 0 && local_tcp_open(p));
        let record = crate::pid_file(ctx, &service.name);

        let (finding, detail) = match (owned, wants_running) {
            (Some(pid), true) => (
                Some(Finding::Adopted),
                format!("{} is running as pid {pid}, started by devd", service.name),
            ),
            (Some(pid), false) => (
                Some(Finding::UnexpectedlyRunning),
                format!(
                    "{} is running as pid {pid} although it is meant to be stopped",
                    service.name
                ),
            ),
            (None, _) => {
                if process::read_pid_record(&record).is_some() {
                    let _ = std::fs::remove_file(&record);
                    (
                        Some(Finding::StaleRecord),
                        format!(
                            "{} had a pid record for a process that is gone or is not the one devd started; the record was removed",
                            service.name
                        ),
                    )
                } else if listening {
                    (
                        Some(Finding::AmbiguousOwner),
                        format!(
                            "port {} is in use, but devd cannot show that it started the process there; it was left alone",
                            service.port.unwrap_or(0)
                        ),
                    )
                } else if wants_running {
                    (
                        Some(Finding::Down),
                        format!("{} should be running and is not", service.name),
                    )
                } else {
                    (None, String::new())
                }
            }
        };
        if let Some(finding) = finding {
            reports.push(Report {
                service: service.name.clone(),
                finding,
                detail,
            });
        }
    }
    reports
}

#[cfg(test)]
mod tests {
    use super::*;
    use devd_core::state::{Service, Store};
    use devd_core::{util, Paths, Settings};
    use std::sync::Arc;

    fn ctx(label: &str) -> (Ctx, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::rooted(dir.path().join(label));
        paths.ensure().unwrap();
        let store = Arc::new(Store::open_in_memory().unwrap());
        store.migrate().unwrap();
        let ctx = Ctx {
            store,
            paths: Arc::new(paths),
            settings: Arc::new(Settings::default()),
            events: devd_core::events::EventBus::new(),
            actor: devd_core::capability::Actor::system(),
            run: None,
            dry_run: false,
        };
        (ctx, dir)
    }

    fn service(name: &str, port: Option<u16>, desired: &str) -> Service {
        Service {
            id: util::new_id(),
            name: name.into(),
            kind: "nginx".into(),
            runtime_id: None,
            argv: serde_json::to_string(&vec!["definitely-not-running.exe"]).unwrap(),
            port,
            health: None,
            desired_state: desired.into(),
            restart_policy: "never".into(),
            enabled: true,
            created_at: util::now(),
        }
    }

    #[test]
    fn a_stopped_service_that_should_be_stopped_reports_nothing() {
        let (c, _d) = ctx("quiet");
        c.store
            .upsert_service(&service("nginx", None, "stopped"))
            .unwrap();
        assert!(reconcile(&c).is_empty());
    }

    #[test]
    fn a_service_that_should_run_and_does_not_is_down() {
        let (c, _d) = ctx("down");
        c.store
            .upsert_service(&service("nginx", None, "running"))
            .unwrap();
        let reports = reconcile(&c);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].finding, Finding::Down);
    }

    #[test]
    fn a_pid_record_for_a_process_devd_did_not_start_is_removed_not_adopted() {
        let (c, _d) = ctx("stale");
        c.store
            .upsert_service(&service("nginx", None, "stopped"))
            .unwrap();
        let record = crate::pid_file(&c, "nginx");
        std::fs::create_dir_all(record.parent().unwrap()).unwrap();
        // This process is alive but is not the service's program.
        process::write_pid_record(&record, std::process::id()).unwrap();
        let reports = reconcile(&c);
        assert_eq!(reports[0].finding, Finding::StaleRecord);
        assert!(!record.exists(), "the stale record is cleaned up");
    }

    #[test]
    fn a_stranger_on_the_port_is_reported_and_left_alone() {
        let (c, _d) = ctx("stranger");
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        c.store
            .upsert_service(&service("nginx", Some(port), "running"))
            .unwrap();
        let reports = reconcile(&c);
        assert_eq!(reports[0].finding, Finding::AmbiguousOwner);
        assert!(reports[0].detail.contains("left alone"));
        // Still listening: nothing was stopped on the strength of the port.
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
    }
}
