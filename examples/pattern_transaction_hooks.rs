//! Pattern: **transaction state-change hooks** (DISTRO-style).
//!
//! A transaction transitions through `Begun → Prepared → Committed |
//! Aborted | RecoveredFromCrash`. Each transition needs to fire hooks
//! — replicated index updates, journal flushes, downstream cache
//! invalidations — without coupling the transaction manager to them.
//!
//! Priority ordering matters here: the WAL flush must happen *before*
//! the in-memory cache invalidates *before* the rate-limiter increments.
//! `register_with_priority` provides this for free.
//!
//! Run with: `cargo run --example pattern_transaction_hooks`

use std::sync::Arc;
use std::sync::Mutex;

use registry_io::SyncRegistry;

#[derive(Debug, Clone)]
struct TransactionEvent {
    txid: u64,
    transition: Transition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // RecoveredFromCrash is part of the public schema
// but not exercised by this short example.
enum Transition {
    Begun,
    Prepared,
    Committed,
    Aborted,
    RecoveredFromCrash,
}

fn main() {
    let bus: SyncRegistry<TransactionEvent> = SyncRegistry::new();
    let order: Arc<Mutex<Vec<(u64, &'static str)>>> = Arc::new(Mutex::new(Vec::new()));

    // Highest priority: journal flush. Must complete before anything
    // observes the commit.
    {
        let order = Arc::clone(&order);
        let _ = bus.register_with_priority(1000, move |evt: &TransactionEvent| {
            if matches!(evt.transition, Transition::Committed) {
                order.lock().unwrap().push((evt.txid, "wal:flush"));
            }
        });
    }

    // Medium priority: in-memory cache invalidator.
    {
        let order = Arc::clone(&order);
        let _ =
            bus.register_with_priority(500, move |evt: &TransactionEvent| match evt.transition {
                Transition::Committed | Transition::Aborted => {
                    order.lock().unwrap().push((evt.txid, "cache:invalidate"));
                }
                _ => {}
            });
    }

    // Default priority (0): replication shipper.
    {
        let order = Arc::clone(&order);
        let _ = bus.register(move |evt: &TransactionEvent| {
            if matches!(evt.transition, Transition::Committed) {
                order.lock().unwrap().push((evt.txid, "replication:send"));
            }
        });
    }

    // Low priority: metrics increment.
    {
        let order = Arc::clone(&order);
        let _ = bus.register_with_priority(-100, move |evt: &TransactionEvent| {
            let tag = match evt.transition {
                Transition::Begun => "metrics:tx_begun",
                Transition::Prepared => "metrics:tx_prepared",
                Transition::Committed => "metrics:tx_committed",
                Transition::Aborted => "metrics:tx_aborted",
                Transition::RecoveredFromCrash => "metrics:tx_recovered",
            };
            order.lock().unwrap().push((evt.txid, tag));
        });
    }

    // Drive a small transaction lifecycle.
    let transitions = [
        Transition::Begun,
        Transition::Prepared,
        Transition::Committed,
    ];

    for transition in transitions {
        bus.notify(&TransactionEvent {
            txid: 42,
            transition,
        });
    }

    // ... and one that aborts.
    bus.notify(&TransactionEvent {
        txid: 43,
        transition: Transition::Begun,
    });
    bus.notify(&TransactionEvent {
        txid: 43,
        transition: Transition::Aborted,
    });

    let log = order.lock().unwrap();
    println!("hook execution order (priority desc, then registration order):");
    for (txid, tag) in log.iter() {
        println!("  txid={txid:>3} hook={tag}");
    }
}
