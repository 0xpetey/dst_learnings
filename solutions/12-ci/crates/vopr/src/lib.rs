//! The VOPR: one seed in, one verdict out.
//!
//! ```text
//!   seed ──► RunConfig::from_seed ──► World::cluster + Workload
//!                                          │
//!              chaos phase: run_workload(requests) under every fault the seed chose
//!                           (a core of f + 1 replicas, if the seed drew one, is spared
//!                            and its clients must be answered throughout)
//!                                          │
//!              stop_faults ──► run until converged, or time out (liveness)
//!                                          │
//!              quiet phase: run_workload(quiet_requests), every one must complete
//!                                          │
//!              checks: linearizable history, logs are prefixes, map is log
//!                                          │
//!                                       Report
//! ```
//!
//! Everything the earlier modules built is here and nothing is new: the
//! scheduler, the network and disk models, the fault schedules, the
//! fault points, the workload, the history and the checker. The module's
//! contribution is the *shape* of a run, and the discipline that every
//! number in it comes from the seed.
//!
//! A run that fails panics. With `panic = "abort"` in the workspace
//! profile the process stops at the assertion, after the configuration
//! line was printed, and the message says which check fired. That is the
//! report a failing seed gets: the seed, the configuration, the trace if
//! asked for, and the assertion.

pub mod batch;
pub mod inspect;
pub mod overrides;
pub mod shrink;

use checker::kv::{self, KvModel};
use checker::linearize::Verdict;
use checker::{Workload, WorkloadConfig};
use determinism::Prng;
use kv_core::client;
use kv_core::log;
use kv_core::replica;
use kv_core::{Duration, NodeId};
use kv_env::fake::{WorkloadRun, World, WorldConfig, WorldStats};
use std::fmt;

/// Candidates per key for the linearizability search.
pub const CHECK_LIMIT: u64 = 2_000_000;

/// Everything one run is given, all drawn from the seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunConfig {
    pub seed: u64,
    pub nodes: u8,
    pub clients_per_node: u64,
    /// Requests in the chaos phase.
    pub requests: u64,
    /// Requests in the quiet phase. Every one must complete.
    pub quiet_requests: u64,
    pub world: WorldConfig,
    pub workload: WorkloadConfig,
    /// The workload's own dice, forked from the seed.
    pub workload_seed: u64,
    pub client: client::Config,
    /// How long the cluster has to converge once the faults stop.
    pub convergence_timeout: Duration,
}

impl RunConfig {
    /// The configuration seed `seed` means: the fault swarm, the workload
    /// and the workload's dice, in that order, from one `Prng`. Adding a
    /// draw at the end keeps every earlier seed's meaning.
    pub fn from_seed(seed: u64, requests: u64) -> RunConfig {
        let mut prng = Prng::from_seed(seed);
        let mut world = WorldConfig::swarm(&mut prng);
        world.keep_trace_lines = false;
        let workload = WorkloadConfig::random(&mut prng);
        let workload_seed = prng.next_u64();
        // Module 10: a cluster of five one time in four, and a core one
        // time in two. Drawn last, so that every earlier seed means the
        // same faults it did.
        let nodes: u8 = if prng.chance(1, 4) { 5 } else { 3 };
        if prng.chance(1, 2) {
            let mut ids: Vec<NodeId> = (0..nodes).collect();
            prng.shuffle(&mut ids);
            ids.truncate(usize::from(nodes / 2 + 1));
            ids.sort_unstable();
            world.core = ids;
        }
        RunConfig {
            seed,
            nodes,
            clients_per_node: 2,
            requests,
            quiet_requests: 30,
            world,
            workload,
            workload_seed,
            client: client::Config {
                initial_timeout: Duration::from_millis(100),
                max_timeout: Duration::from_millis(1000),
                max_attempts: 50,
            },
            convergence_timeout: Duration::from_secs(10),
        }
    }

    pub fn clients(&self) -> u64 {
        u64::from(self.nodes) * self.clients_per_node
    }
}

impl fmt::Display for RunConfig {
    /// One line: the head of the report and of the trace.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "seed={} nodes={} clients={} requests={} quiet={} {} workload=[{}]",
            self.seed,
            self.nodes,
            self.clients(),
            self.requests,
            self.quiet_requests,
            self.world,
            self.workload
        )
    }
}

/// What a run that passed has to say for itself.
#[derive(Clone, Debug)]
pub struct Report {
    pub config: RunConfig,
    pub chaos: WorkloadRun,
    pub quiet: WorkloadRun,
    /// Virtual time from the faults stopping to the cluster converging.
    pub converged_in: Duration,
    /// Virtual time the whole run took.
    pub virtual_time: Duration,
    pub stats: WorldStats,
    /// Per replica, in node order, as the run ended.
    pub replicas: Vec<replica::Stats>,
    /// The primary at the end.
    pub primary: NodeId,
    /// Operations in the history, and the search that checked them.
    pub history: usize,
    pub keys: usize,
    pub check: checker::Stats,
    /// The trace's hash. Two runs of one seed must agree on it.
    pub hash: u64,
    /// Scheduler events fired. The run's cost in a unit the seed decides
    /// and the machine does not. Module 12's budget is in this unit.
    pub events: u64,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = &self.stats;
        let views: u64 = self.replicas.iter().map(|r| r.view_changes).sum();
        let promotions: u64 = self.replicas.iter().map(|r| r.promotions).sum();
        let repairs: u64 = self.replicas.iter().map(|r| r.repairs).sum();
        let pulls: u64 = self.replicas.iter().map(|r| r.pulls_sent).sum();
        writeln!(
            f,
            "chaos: submitted={} completed={} gave_up={} in {:?}",
            self.chaos.submitted,
            self.chaos.completed,
            self.chaos.gave_up,
            self.virtual_time - self.converged_in
        )?;
        writeln!(
            f,
            "faults: partitions={} crashes={} pauses={} clock_jumps={} buggify_fires={} net_dropped={}",
            s.partitions, s.heals, s.pauses, s.clock_jumps, s.buggify_fires, s.dropped_at_down_hosts
        )?;
        writeln!(
            f,
            "cluster: view_changes={views} promotions={promotions} pulls={pulls} repairs={repairs} primary=n{} converged_in={:?}",
            self.primary, self.converged_in
        )?;
        writeln!(
            f,
            "quiet: submitted={} completed={} gave_up={}",
            self.quiet.submitted, self.quiet.completed, self.quiet.gave_up
        )?;
        writeln!(
            f,
            "check: history={} keys={} steps={} cached={} depth={}",
            self.history, self.keys, self.check.steps, self.check.cached, self.check.depth
        )?;
        write!(
            f,
            "cost: events={} requests={} per_request={}",
            self.events,
            self.config.requests,
            self.events / self.config.requests.max(1)
        )
    }
}

/// Run one seed to its verdict. Panics with the failing check.
pub fn run(config: &RunConfig) -> Report {
    let mut w = World::cluster(config.nodes, config.world.clone());
    for node in 0..config.nodes {
        for k in 0..config.clients_per_node {
            let id = 1 + u64::from(node) * config.clients_per_node + k;
            w.add_client(id, config.client, node);
        }
    }
    let mut workload = Workload::new(config.workload.clone(), Prng::from_seed(config.workload_seed));

    // 1. Chaos. Every fault family the seed turned on is running. If the
    //    seed drew a core, the core's clients must still be answered: a
    //    majority that is up and connected is a cluster, whatever the
    //    rest is doing. TigerBeetle's liveness mode.
    let chaos = w.run_workload(&mut workload, config.requests);
    if !config.world.core.is_empty() {
        let core_gave_up: Vec<&(u64, u64)> = w
            .gave_up
            .iter()
            .filter(|(client, _)| config.world.core.contains(&w.target(*client)))
            .collect();
        assert!(
            core_gave_up.is_empty(),
            "liveness: a client of the core gave up during the chaos: {core_gave_up:?}: {}",
            describe_cluster(&w)
        );
    }

    // 2. Quiet. The faults stop, and the cluster has a bounded time to
    //    come back. This is the liveness check.
    w.stop_faults();
    let stopped_at = w.now();
    let hb = config.world.heartbeat;
    w.run_until_idle();
    w.run_for(hb.suspect_after + hb.interval + Duration::from_millis(1));
    assert!(
        w.run_until_converged(config.convergence_timeout),
        "liveness: the cluster did not converge within {:?} of the faults stopping: {}",
        config.convergence_timeout,
        describe_cluster(&w)
    );
    let converged_in = w.now().saturating_duration_since(stopped_at);

    // 3. Everything asked for after the faults stop is answered.
    let quiet = w.run_workload(&mut workload, config.quiet_requests);
    assert_eq!(
        quiet.gave_up, 0,
        "liveness: a request after the faults stopped was not answered"
    );
    assert_eq!(quiet.submitted, quiet.completed);
    assert!(
        w.run_until_converged(config.convergence_timeout),
        "liveness: the cluster did not settle after the quiet phase: {}",
        describe_cluster(&w)
    );

    // 4. Safety. The history is linearizable, the logs are prefixes of
    //    one another, and the primary's map is its log replayed.
    let report = kv::check(&w.history, CHECK_LIMIT);
    if let Some(failed) = &report.failed {
        let mut ops = String::new();
        for (i, op) in failed.history.ops().iter().enumerate() {
            use std::fmt::Write as _;
            let _ = writeln!(
                ops,
                "  {i:3} c{} id={} {:?} call={} ret={:?} -> {:?}",
                op.client, op.request_id, op.request, op.call, op.ret, op.response
            );
        }
        panic!(
            "safety: key {:?} is not linearizable: {}\n{ops}",
            String::from_utf8_lossy(&failed.key),
            match &failed.verdict {
                Verdict::Illegal(i) => i.to_string(),
                v => format!("{v:?}"),
            }
        );
    }
    w.check_logs()
        .unwrap_or_else(|(a, b)| panic!("safety: the logs of n{a} and n{b} are not prefixes of one another"));
    let primary = w.primary().expect("converged");
    // The white box: the primary's map is the log it holds, replayed.
    let records = w.replica(primary).records();
    let replayed: KvModel = kv::replay(&records);
    let map: Vec<(&[u8], &[u8])> = w.node(primary).entries().collect();
    let log: Vec<(&[u8], &[u8])> = replayed.entries().collect();
    assert_eq!(map, log, "safety: the primary's map is not its log replayed");
    // And the log it holds is the log on its disk, on a disk that keeps
    // what it is given. Seed 12784 found a misdirected write under the
    // first version of this check, which compared the map with the disk:
    // the disk had a hole the replica had not restarted to find yet.
    let disk = &config.world.disk;
    let can_damage = !disk.lost_write.is_never()
        || !disk.corrupt_write.is_never()
        || !disk.corrupt_read.is_never()
        || !disk.misdirect_write.is_never();
    if !can_damage {
        let on_disk = log::recover(w.log_on_disk(primary));
        assert_eq!(
            on_disk.records, records,
            "safety: the primary's disk does not hold the log it serves"
        );
    }

    let replicas = (0..config.nodes).map(|n| w.replica(n).stats()).collect();
    let hash = w.trace().hash();
    let events = w.scheduler.stats().fired;
    Report {
        config: config.clone(),
        chaos,
        quiet,
        converged_in,
        virtual_time: w.now().saturating_duration_since(kv_core::Instant::ZERO),
        stats: w.stats,
        replicas,
        primary,
        history: w.history.len(),
        keys: report.keys,
        check: report.stats,
        hash,
        events,
    }
}

/// One line per host, for a liveness failure, and the first point where
/// two logs disagree, if they do.
fn describe_cluster(w: &World) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    if let Err((a, b)) = w.check_logs() {
        let (la, lb) = (w.replica(a).log_bytes(), w.replica(b).log_bytes());
        let at = la
            .iter()
            .zip(lb)
            .position(|(x, y)| x != y)
            .unwrap_or(la.len().min(lb.len()));
        let _ = write!(
            s,
            "\n  the logs of n{a} and n{b} differ from offset {at}, in record {}",
            at / log::SECTOR_SIZE + 1
        );
    }
    for host in &w.hosts {
        if host.is_up() {
            let r = host.replica();
            let _ = write!(
                s,
                "\n  n{} {} view={} len={} durable={}{}",
                host.id,
                r.status(),
                r.view(),
                r.len(),
                r.durable(),
                if host.is_paused() { " paused" } else { "" }
            );
        } else {
            let _ = write!(s, "\n  n{} down", host.id);
        }
    }
    s
}
