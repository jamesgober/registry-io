//! Pattern: **audit-log fan-out**.
//!
//! One audit event must be persisted to multiple sinks (stdout, file,
//! database, remote SIEM) without coupling the producer to any of them.
//! `SyncRegistry<AuditEvent>` is exactly the right primitive: register
//! once per sink, dispatch with one `notify` call, get reliable
//! sub-microsecond fan-out plus panic isolation between sinks.
//!
//! `register_guard` ties the sink's lifetime to its owning scope — if
//! the sink drops, its registration goes with it without explicit
//! cleanup.
//!
//! Run with: `cargo run --example pattern_audit_fanout`

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use registry_io::{HandlerGuard, SyncRegistry};

#[derive(Debug, Clone)]
struct AuditEvent {
    actor: &'static str,
    action: &'static str,
    resource: String,
    severity: Severity,
}

#[derive(Debug, Clone, Copy)]
enum Severity {
    Info,
    Warn,
    Critical,
}

/// A sink is anything that owns a `HandlerGuard`. When the sink drops,
/// the registration drops with it.
struct FileSink {
    _guard: HandlerGuard<AuditEvent>,
    captured: Arc<Mutex<Vec<String>>>,
}

impl FileSink {
    fn attach(bus: &Arc<SyncRegistry<AuditEvent>>) -> Self {
        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&captured);
        let _guard = bus.register_guard(move |evt: &AuditEvent| {
            // In a real implementation this would `write!` to an
            // `OpenOptions::append` file handle. Here we tee into memory.
            let line = format!(
                "{} action={} resource={} severity={:?}",
                evt.actor, evt.action, evt.resource, evt.severity
            );
            log.lock().unwrap().push(line);
        });
        Self { _guard, captured }
    }

    fn lines(&self) -> Vec<String> {
        self.captured.lock().unwrap().clone()
    }
}

struct StdoutSink {
    _guard: HandlerGuard<AuditEvent>,
}

impl StdoutSink {
    fn attach(bus: &Arc<SyncRegistry<AuditEvent>>) -> Self {
        let _guard = bus.register_guard(|evt: &AuditEvent| {
            println!(
                "[stdout] {} {} {} ({:?})",
                evt.actor, evt.action, evt.resource, evt.severity
            );
        });
        Self { _guard }
    }
}

struct CriticalAlertSink {
    _guard: HandlerGuard<AuditEvent>,
    triggered: Arc<AtomicUsize>,
}

impl CriticalAlertSink {
    fn attach(bus: &Arc<SyncRegistry<AuditEvent>>) -> Self {
        let triggered = Arc::new(AtomicUsize::new(0));
        let sink = Arc::clone(&triggered);
        let _guard = bus.register_guard(move |evt: &AuditEvent| {
            if matches!(evt.severity, Severity::Critical) {
                let _ = sink.fetch_add(1, Ordering::Relaxed);
            }
        });
        Self { _guard, triggered }
    }

    fn triggered(&self) -> usize {
        self.triggered.load(Ordering::Relaxed)
    }
}

fn main() {
    let bus = Arc::new(SyncRegistry::<AuditEvent>::new());

    let _stdout_sink = StdoutSink::attach(&bus);
    let file_sink = FileSink::attach(&bus);
    let alert_sink = CriticalAlertSink::attach(&bus);

    let events = [
        AuditEvent {
            actor: "alice",
            action: "login",
            resource: "session#42".into(),
            severity: Severity::Info,
        },
        AuditEvent {
            actor: "bob",
            action: "delete",
            resource: "user#17".into(),
            severity: Severity::Warn,
        },
        AuditEvent {
            actor: "system",
            action: "key-rotation-failed",
            resource: "kms#main".into(),
            severity: Severity::Critical,
        },
    ];

    for evt in &events {
        bus.notify(evt);
    }

    println!("\nfile sink captured {} lines:", file_sink.lines().len());
    for line in file_sink.lines() {
        println!("  {line}");
    }
    println!("critical alerts triggered: {}", alert_sink.triggered());

    drop(file_sink); // file sink stops listening
    drop(alert_sink); // alert sink stops listening
    drop(_stdout_sink); // stdout sink stops listening
    assert!(bus.is_empty(), "all guards dropped → registry empty");
    println!("\nall sinks detached cleanly; registry is empty");
}
