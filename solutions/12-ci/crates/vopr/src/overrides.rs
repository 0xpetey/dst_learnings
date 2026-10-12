//! Overrides: the knobs a run can be given on top of what its seed drew.
//!
//! A seed names a configuration. An override changes one thing about it
//! and leaves the rest: the same dice, the same workload, one fault
//! family off, or fewer requests. Every override is a command-line flag,
//! so a seed plus its overrides is a line that reproduces the run:
//!
//! ```text
//!   vopr --seed 562 --requests 80 --nodes 3 --no-disk --no-pauses --bugs forgotten_promise
//! ```
//!
//! The shrinker in [`crate::shrink`] searches this space. Module 11's
//! lesson says why it searches *this* space and not the seed's: a seed
//! has no neighbours, and the configuration has.

use crate::RunConfig;
use determinism::Chance;
use kv_core::{Bug, Duration};
use kv_env::fake::WorldConfig;
use sim::{DiskConfig, NetConfig};
use std::collections::BTreeSet;
use std::fmt;

/// The requests a run gets unless a flag says otherwise.
pub const DEFAULT_REQUESTS: u64 = 150;

/// One thing that can be turned off. A fault family, a rate inside one,
/// or the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Knob {
    /// The network model: reliable, constant latency.
    Net,
    /// The disk model: reliable, constant latency.
    Disk,
    Partitions,
    Crashes,
    Pauses,
    /// Every clock reads true time.
    Clocks,
    Buggify,
    SlowHost,
    /// No core: every host can be crashed, paused and cut.
    Core,
    // Inside the network model.
    Loss,
    Dup,
    Reorder,
    // Inside the disk model.
    Lost,
    Torn,
    Misdirect,
    Rot,
    ReadFault,
    // Inside the clock model.
    Jumps,
    Drift,
    Offset,
}

impl Knob {
    /// Coarse to fine: the families first, the rates inside them after.
    /// The shrinker tries them in this order.
    pub const ALL: [Knob; 20] = [
        Knob::Net,
        Knob::Disk,
        Knob::Partitions,
        Knob::Crashes,
        Knob::Pauses,
        Knob::Clocks,
        Knob::Buggify,
        Knob::SlowHost,
        Knob::Core,
        Knob::Loss,
        Knob::Dup,
        Knob::Reorder,
        Knob::Lost,
        Knob::Torn,
        Knob::Misdirect,
        Knob::Rot,
        Knob::ReadFault,
        Knob::Jumps,
        Knob::Drift,
        Knob::Offset,
    ];

    /// The flag that turns this knob off.
    pub const fn flag(self) -> &'static str {
        match self {
            Knob::Net => "--no-net",
            Knob::Disk => "--no-disk",
            Knob::Partitions => "--no-partitions",
            Knob::Crashes => "--no-crashes",
            Knob::Pauses => "--no-pauses",
            Knob::Clocks => "--no-clocks",
            Knob::Buggify => "--no-buggify",
            Knob::SlowHost => "--no-slow-host",
            Knob::Core => "--no-core",
            Knob::Loss => "--no-loss",
            Knob::Dup => "--no-dup",
            Knob::Reorder => "--no-reorder",
            Knob::Lost => "--no-lost",
            Knob::Torn => "--no-torn",
            Knob::Misdirect => "--no-misdirect",
            Knob::Rot => "--no-rot",
            Knob::ReadFault => "--no-read-fault",
            Knob::Jumps => "--no-jumps",
            Knob::Drift => "--no-drift",
            Knob::Offset => "--no-offset",
        }
    }

    pub fn from_flag(flag: &str) -> Option<Knob> {
        Knob::ALL.into_iter().find(|k| k.flag() == flag)
    }

    /// Is there anything for this knob to turn off in `config`? A knob
    /// that changes nothing is not worth a run.
    pub fn is_live(self, config: &RunConfig) -> bool {
        let w = &config.world;
        match self {
            Knob::Net => !w.net.is_reliable(),
            Knob::Disk => !w.disk.is_reliable(),
            Knob::Partitions => w.partitions.is_some(),
            Knob::Crashes => w.crashes.is_some(),
            Knob::Pauses => w.pauses.is_some(),
            Knob::Clocks => w.clocks.is_some(),
            Knob::Buggify => w.buggify.is_some(),
            Knob::SlowHost => w.slow_host.is_some(),
            Knob::Core => !w.core.is_empty(),
            Knob::Loss => !w.net.loss.is_never(),
            Knob::Dup => !w.net.duplicate.is_never(),
            Knob::Reorder => !w.net.reorder.is_never(),
            Knob::Lost => !w.disk.lost_write.is_never(),
            Knob::Torn => !w.disk.torn_write.is_never(),
            Knob::Misdirect => !w.disk.misdirect_write.is_never(),
            Knob::Rot => !w.disk.corrupt_write.is_never(),
            Knob::ReadFault => !w.disk.corrupt_read.is_never(),
            Knob::Jumps => w.clocks.as_ref().is_some_and(|c| c.has_jumps()),
            Knob::Drift => w.clocks.as_ref().is_some_and(|c| c.drift_ppm_max != 0),
            Knob::Offset => w.clocks.as_ref().is_some_and(|c| !c.offset_max.is_zero()),
        }
    }

    /// Turn it off. The family's "off" is what the swarm draws when its
    /// coin says off, so a shrunk configuration is one the swarm can
    /// reach.
    pub fn apply(self, w: &mut WorldConfig) {
        match self {
            Knob::Net => w.net = NetConfig::default(),
            Knob::Disk => w.disk = DiskConfig::default(),
            Knob::Partitions => w.partitions = None,
            Knob::Crashes => w.crashes = None,
            Knob::Pauses => w.pauses = None,
            Knob::Clocks => w.clocks = None,
            Knob::Buggify => w.buggify = None,
            Knob::SlowHost => w.slow_host = None,
            Knob::Core => w.core.clear(),
            Knob::Loss => w.net.loss = Chance::NEVER,
            Knob::Dup => w.net.duplicate = Chance::NEVER,
            Knob::Reorder => {
                w.net.reorder = Chance::NEVER;
                w.net.reorder_delay_max = Duration::ZERO;
            }
            Knob::Lost => w.disk.lost_write = Chance::NEVER,
            Knob::Torn => w.disk.torn_write = Chance::NEVER,
            Knob::Misdirect => w.disk.misdirect_write = Chance::NEVER,
            Knob::Rot => w.disk.corrupt_write = Chance::NEVER,
            Knob::ReadFault => w.disk.corrupt_read = Chance::NEVER,
            Knob::Jumps => {
                if let Some(c) = &mut w.clocks {
                    c.jump_max = Duration::ZERO;
                    c.jump_gap_max = Duration::ZERO;
                }
            }
            Knob::Drift => {
                if let Some(c) = &mut w.clocks {
                    c.drift_ppm_max = 0;
                }
            }
            Knob::Offset => {
                if let Some(c) = &mut w.clocks {
                    c.offset_max = Duration::ZERO;
                }
            }
        }
    }
}

impl fmt::Display for Knob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.flag())
    }
}

/// Everything a command line can change about a seed's run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Overrides {
    /// Requests in the chaos phase. `None` is [`DEFAULT_REQUESTS`].
    pub requests: Option<u64>,
    /// Hosts in the cluster. Three or five.
    pub nodes: Option<u8>,
    pub clients_per_node: Option<u64>,
    /// Knobs turned off.
    pub off: BTreeSet<Knob>,
    /// Bugs to plant. A test fixture, never drawn by a seed.
    pub bugs: Vec<Bug>,
}

impl Overrides {
    /// The configuration `seed` means under these overrides.
    pub fn configure(&self, seed: u64) -> RunConfig {
        let mut config = RunConfig::from_seed(seed, self.requests.unwrap_or(DEFAULT_REQUESTS));
        self.apply(&mut config);
        config
    }

    pub fn apply(&self, config: &mut RunConfig) {
        if let Some(n) = self.nodes {
            config.nodes = n;
            config.world.core.retain(|&id| id < n);
            if !config.world.core.is_empty() && config.world.core.len() < usize::from(n / 2 + 1) {
                // A core is a majority or nothing.
                config.world.core.clear();
            }
        }
        if let Some(c) = self.clients_per_node {
            config.clients_per_node = c;
        }
        for knob in &self.off {
            knob.apply(&mut config.world);
        }
        config.world.bugs.clone_from(&self.bugs);
    }

    /// Nothing overridden: the seed as drawn.
    pub fn is_empty(&self) -> bool {
        *self == Overrides::default()
    }

    /// The flags, in a fixed order, so that two equal overrides print
    /// the same line.
    pub fn to_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(r) = self.requests {
            args.push("--requests".into());
            args.push(r.to_string());
        }
        if let Some(n) = self.nodes {
            args.push("--nodes".into());
            args.push(n.to_string());
        }
        if let Some(c) = self.clients_per_node {
            args.push("--clients".into());
            args.push(c.to_string());
        }
        for knob in &self.off {
            args.push(knob.flag().into());
        }
        if !self.bugs.is_empty() {
            args.push("--bugs".into());
            args.push(self.bugs.iter().map(|b| b.name()).collect::<Vec<_>>().join(","));
        }
        args
    }

    /// Read the override flags out of `args`, leaving everything else.
    /// Returns what was not understood, if anything was malformed.
    pub fn parse(args: &[String]) -> Result<(Overrides, Vec<String>), String> {
        let mut o = Overrides::default();
        let mut rest = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let a = args[i].as_str();
            let value = |i: usize| -> Result<&str, String> {
                args.get(i + 1)
                    .map(String::as_str)
                    .ok_or_else(|| format!("{a} takes a value"))
            };
            match a {
                "--requests" => {
                    o.requests = Some(value(i)?.parse().map_err(|_| format!("{a}: not a number"))?);
                    i += 2;
                }
                "--nodes" => {
                    let n: u8 = value(i)?.parse().map_err(|_| format!("{a}: not a number"))?;
                    if n != 3 && n != 5 {
                        return Err("--nodes takes 3 or 5".into());
                    }
                    o.nodes = Some(n);
                    i += 2;
                }
                "--clients" => {
                    let c: u64 = value(i)?.parse().map_err(|_| format!("{a}: not a number"))?;
                    if c == 0 {
                        return Err("--clients takes at least 1".into());
                    }
                    o.clients_per_node = Some(c);
                    i += 2;
                }
                "--bugs" => {
                    for name in value(i)?.split(',') {
                        let bug = Bug::from_name(name).ok_or_else(|| format!("unknown bug {name:?}"))?;
                        if !o.bugs.contains(&bug) {
                            o.bugs.push(bug);
                        }
                    }
                    i += 2;
                }
                _ => {
                    if let Some(knob) = Knob::from_flag(a) {
                        o.off.insert(knob);
                    } else {
                        rest.push(args[i].clone());
                    }
                    i += 1;
                }
            }
        }
        Ok((o, rest))
    }
}

impl fmt::Display for Overrides {
    /// The flags, space separated. Empty for no overrides.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_args().join(" "))
    }
}

/// The command line that reproduces `seed` under `overrides`. With the
/// commit hash, a complete bug report.
pub fn reproduction(seed: u64, overrides: &Overrides) -> String {
    let mut line = format!("cargo run --release -p vopr -- --seed {seed}");
    if !overrides.is_empty() {
        line.push(' ');
        line.push_str(&overrides.to_string());
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_round_trip() {
        let mut o = Overrides {
            requests: Some(80),
            nodes: Some(3),
            clients_per_node: Some(1),
            off: BTreeSet::new(),
            bugs: vec![Bug::ForgottenPromise, Bug::LeaseRead],
        };
        o.off.insert(Knob::Disk);
        o.off.insert(Knob::Loss);
        let args = o.to_args();
        let (back, rest) = Overrides::parse(&args).unwrap();
        assert_eq!(back, o);
        assert!(rest.is_empty());
        assert_eq!(
            o.to_string(),
            "--requests 80 --nodes 3 --clients 1 --no-disk --no-loss --bugs forgotten_promise,lease_read"
        );
    }

    #[test]
    fn unknown_flags_are_left_over() {
        let args: Vec<String> = ["--seed", "7", "--no-crashes", "--trace"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (o, rest) = Overrides::parse(&args).unwrap();
        assert_eq!(o.off.into_iter().collect::<Vec<_>>(), vec![Knob::Crashes]);
        assert_eq!(rest, ["--seed", "7", "--trace"]);
        assert!(Overrides::parse(&["--nodes".to_string(), "4".to_string()]).is_err());
        assert!(Overrides::parse(&["--bugs".to_string(), "nope".to_string()]).is_err());
    }

    #[test]
    fn every_knob_turns_something_off() {
        // Across a few seeds every knob is live somewhere, and after it
        // is applied it is live nowhere.
        let mut seen = BTreeSet::new();
        for seed in 0..64 {
            let config = RunConfig::from_seed(seed, 10);
            for knob in Knob::ALL {
                if knob.is_live(&config) {
                    seen.insert(knob);
                    let mut c = config.clone();
                    knob.apply(&mut c.world);
                    assert!(!knob.is_live(&c), "{knob} on seed {seed}");
                }
            }
        }
        assert_eq!(
            seen.len(),
            Knob::ALL.len(),
            "never live: {:?}",
            Knob::ALL.iter().filter(|k| !seen.contains(k)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn nodes_three_drops_a_core_of_five() {
        let seed = (0..200)
            .find(|&s| {
                let c = RunConfig::from_seed(s, 10);
                c.nodes == 5 && c.world.core.len() == 3
            })
            .expect("a five-node seed with a core");
        let o = Overrides {
            nodes: Some(3),
            ..Overrides::default()
        };
        let c = o.configure(seed);
        assert_eq!(c.nodes, 3);
        assert!(
            c.world.core.is_empty() || c.world.core.len() == 2,
            "{:?}",
            c.world.core
        );
        assert!(c.world.core.iter().all(|&n| n < 3));
    }
}
