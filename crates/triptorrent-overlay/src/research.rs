//! Deterministic, no-I/O models used by the M3 discovery research.
//!
//! This is an architectural comparison tool, not a production DHT. It models
//! Kademlia-style XOR routing, replicated records, churn, selective dropping,
//! target-key Sybils, and the metadata visible under four discovery policies.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write;

const K_BUCKET_SIZE: usize = 8;
const ALPHA: usize = 3;
const MAX_LOOKUP_ROUNDS: usize = 64;

/// Discovery policies compared by the M3 research harness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryCandidate {
    /// BEP 5-like direct lookup and direct provider records under the content ID.
    VanillaKademlia,
    /// Direct lookup under a capability-derived key, with direct provider records.
    BlindedKademlia,
    /// Blinded records reached through one oblivious gateway and one rendezvous token.
    DistributedRendezvous,
    /// Three diverse oblivious lookup paths followed by separate relay coordination.
    SeparatedMultiStage,
}

impl DiscoveryCandidate {
    /// Stable label used in reports.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::VanillaKademlia => "vanilla",
            Self::BlindedKademlia => "blinded",
            Self::DistributedRendezvous => "rendezvous",
            Self::SeparatedMultiStage => "multi-stage",
        }
    }

    const fn policy(self) -> Policy {
        match self {
            Self::VanillaKademlia => Policy {
                key: LookupKey::Raw,
                origin: LookupOrigin::Direct,
                provider: ProviderRecord::Direct,
                routing: RoutingPolicy::Closest,
                lookup_paths: 1,
            },
            Self::BlindedKademlia => Policy {
                key: LookupKey::CapabilityDerived,
                origin: LookupOrigin::Direct,
                provider: ProviderRecord::Direct,
                routing: RoutingPolicy::Closest,
                lookup_paths: 1,
            },
            Self::DistributedRendezvous => Policy {
                key: LookupKey::CapabilityDerived,
                origin: LookupOrigin::Oblivious,
                provider: ProviderRecord::Indirect,
                routing: RoutingPolicy::PrefixDiverse,
                lookup_paths: 1,
            },
            Self::SeparatedMultiStage => Policy {
                key: LookupKey::CapabilityDerived,
                origin: LookupOrigin::Oblivious,
                provider: ProviderRecord::Indirect,
                routing: RoutingPolicy::PrefixDiverse,
                lookup_paths: 3,
            },
        }
    }
}

/// A deterministic topology and workload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResearchScenario {
    /// Stable scenario label.
    pub name: &'static str,
    /// Ordinary nodes created before targeted Sybils.
    pub ordinary_nodes: usize,
    /// Percentage of ordinary nodes that selectively drop or censor records.
    pub malicious_percent: u8,
    /// Target-key Sybil identities added to the topology.
    pub sybil_nodes: usize,
    /// Simulated network-prefix groups controlled by the Sybil operator.
    pub sybil_groups: usize,
    /// Percentage of ordinary nodes that fail after records are published.
    pub failed_percent: u8,
    /// Distinct content records exercised.
    pub contents: usize,
    /// Repeated lookups made by the same logical requester per content.
    pub lookups_per_content: usize,
}

/// Reproducible scenarios used in the M3 report.
pub const DEFAULT_SCENARIOS: [ResearchScenario; 4] = [
    ResearchScenario {
        name: "clean-100",
        ordinary_nodes: 100,
        malicious_percent: 0,
        sybil_nodes: 0,
        sybil_groups: 0,
        failed_percent: 0,
        contents: 20,
        lookups_per_content: 5,
    },
    ResearchScenario {
        name: "churn-1000",
        ordinary_nodes: 1_000,
        malicious_percent: 0,
        sybil_nodes: 0,
        sybil_groups: 0,
        failed_percent: 15,
        contents: 50,
        lookups_per_content: 2,
    },
    ResearchScenario {
        name: "malicious-1000",
        ordinary_nodes: 1_000,
        malicious_percent: 20,
        sybil_nodes: 0,
        sybil_groups: 0,
        failed_percent: 5,
        contents: 50,
        lookups_per_content: 2,
    },
    ResearchScenario {
        name: "target-sybil-1000",
        ordinary_nodes: 1_000,
        malicious_percent: 5,
        sybil_nodes: 250,
        sybil_groups: 4,
        failed_percent: 5,
        contents: 1,
        lookups_per_content: 100,
    },
];

/// Aggregated measurements for one candidate in one scenario.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResearchMetrics {
    /// Number of logical lookups.
    pub lookups: usize,
    /// Lookups that reached an honest, live record replica.
    pub successes: usize,
    /// Sum of the longest routing path, in iterative rounds, per lookup.
    pub routing_rounds: usize,
    /// Sum of distinct nodes learning the lookup key per lookup.
    pub lookup_key_observers: usize,
    /// Sum of nodes learning a raw content ID during lookup.
    pub raw_content_observers: usize,
    /// Sum of nodes learning requester network location and lookup key together.
    pub requester_key_observers: usize,
    /// Sum of nodes learning provider network location and lookup key at publication.
    pub provider_key_observers: usize,
    /// Lookups in which one malicious node observed requester and provider linkage.
    pub adversary_both_sides: usize,
    /// Lookups linkable by colluding malicious lookup and storage nodes.
    pub colluding_adversary_both_sides: usize,
    /// Failed lookups in which at least one malicious node occupied the observed path.
    pub eclipsed_lookups: usize,
    /// Content keys for which a malicious node observed at least two lookups.
    pub repeated_key_linkability: usize,
    /// Repeated keys linkable to the same requester network location.
    pub repeated_requester_linkability: usize,
    /// Logical control messages, including request/response legs and rendezvous stages.
    pub control_messages: usize,
    /// Total replicated records.
    pub stored_records: usize,
    /// Replicas placed on target-key Sybil nodes.
    pub sybil_records: usize,
    /// Total routing-table contacts across all nodes.
    pub routing_entries: usize,
    /// Total nodes in the simulated topology.
    pub nodes: usize,
    /// Distinct content records in the workload.
    pub contents: usize,
}

/// One row in the M3 comparison.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResearchResult {
    /// Input scenario.
    pub scenario: ResearchScenario,
    /// Evaluated discovery policy.
    pub candidate: DiscoveryCandidate,
    /// Collected metrics.
    pub metrics: ResearchMetrics,
}

/// Tunable parameters used by M8 seeded adversarial experiments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResearchTuning {
    /// Reproducible topology/workload seed.
    pub seed: u64,
    /// Record replicas selected around each lookup key.
    pub replica_count: usize,
    /// Independent paths used by the separated multi-stage candidate.
    pub separated_lookup_paths: usize,
    /// Percentage of each routing table deliberately replaced by attacker contacts.
    pub routing_contamination_percent: u8,
}

impl Default for ResearchTuning {
    fn default() -> Self {
        Self {
            seed: 0,
            replica_count: K_BUCKET_SIZE,
            separated_lookup_paths: 3,
            routing_contamination_percent: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct Policy {
    key: LookupKey,
    origin: LookupOrigin,
    provider: ProviderRecord,
    routing: RoutingPolicy,
    lookup_paths: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LookupKey {
    Raw,
    CapabilityDerived,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LookupOrigin {
    Direct,
    Oblivious,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ProviderRecord {
    Direct,
    Indirect,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RoutingPolicy {
    Closest,
    PrefixDiverse,
}

#[derive(Clone)]
struct Node {
    id: u64,
    group: u16,
    malicious: bool,
    failed: bool,
    sybil: bool,
    routing: Vec<usize>,
}

#[derive(Default)]
struct LookupTrace {
    success: bool,
    rounds: usize,
    observers: BTreeSet<usize>,
    queried: BTreeSet<usize>,
}

/// Runs every default scenario against every candidate.
#[must_use]
pub fn run_default_research_suite() -> Vec<ResearchResult> {
    DEFAULT_SCENARIOS
        .iter()
        .flat_map(|scenario| {
            [
                DiscoveryCandidate::VanillaKademlia,
                DiscoveryCandidate::BlindedKademlia,
                DiscoveryCandidate::DistributedRendezvous,
                DiscoveryCandidate::SeparatedMultiStage,
            ]
            .map(|candidate| simulate(*scenario, candidate))
        })
        .collect()
}

/// Runs one deterministic scenario.
///
/// # Panics
///
/// Panics when the scenario has too few nodes, no workload, or target Sybils without a network
/// group. The built-in scenarios satisfy these invariants.
#[must_use]
pub fn simulate(scenario: ResearchScenario, candidate: DiscoveryCandidate) -> ResearchResult {
    simulate_with_tuning(scenario, candidate, ResearchTuning::default())
}

/// Runs one reproducible M8 scenario with explicit topology and defense tuning.
///
/// # Panics
///
/// Panics for an unusable scenario or zero replica/path settings.
#[must_use]
#[allow(clippy::too_many_lines)] // Keeping one visible loop makes every accumulated metric auditable.
pub fn simulate_with_tuning(
    scenario: ResearchScenario,
    candidate: DiscoveryCandidate,
    tuning: ResearchTuning,
) -> ResearchResult {
    assert!(scenario.ordinary_nodes > K_BUCKET_SIZE);
    assert!(scenario.contents > 0);
    assert!(scenario.lookups_per_content > 0);
    assert!(scenario.sybil_nodes == 0 || scenario.sybil_groups > 0);
    assert!(tuning.replica_count > 0);
    assert!(tuning.separated_lookup_paths > 0);
    assert!(tuning.routing_contamination_percent <= 100);

    let mut policy = candidate.policy();
    if candidate == DiscoveryCandidate::SeparatedMultiStage {
        policy.lookup_paths = tuning.separated_lookup_paths;
    }
    let first_key = lookup_key(0, policy.key == LookupKey::CapabilityDerived, tuning.seed);
    let mut nodes = build_nodes(scenario, first_key, tuning.seed);
    build_routing_tables(&mut nodes);
    contaminate_routing_tables(
        &mut nodes,
        tuning.routing_contamination_percent,
        tuning.seed,
    );
    let routing_entries = nodes.iter().map(|node| node.routing.len()).sum();
    let mut metrics = empty_metrics(scenario, nodes.len(), routing_entries);

    for content in 0..scenario.contents {
        let target = lookup_key(
            content,
            policy.key == LookupKey::CapabilityDerived,
            tuning.seed,
        );
        let replicas = select_replicas(
            &nodes,
            target,
            policy.routing == RoutingPolicy::PrefixDiverse,
            tuning.replica_count,
        );
        metrics.stored_records += replicas.len();
        metrics.sybil_records += replicas.iter().filter(|&&index| nodes[index].sybil).count();
        let provider = select_node(
            &nodes,
            stable_hash(b"provider", content as u64 ^ tuning.seed),
            true,
        );
        let provider_observers = provider_observers(&nodes, policy, provider, target, &replicas);
        metrics.provider_key_observers += provider_observers.len();

        let requester = select_node(
            &nodes,
            stable_hash(b"requester", content as u64 ^ tuning.seed),
            true,
        );
        let mut malicious_repeat_counts: HashMap<usize, usize> = HashMap::new();
        let mut malicious_requester_repeat_counts: HashMap<usize, usize> = HashMap::new();

        for repetition in 0..scenario.lookups_per_content {
            let combined = combined_lookup(LookupInputs {
                nodes: &nodes,
                policy,
                requester,
                target,
                replicas: &replicas,
                content,
                repetition,
                seed: tuning.seed,
            });

            if combined.success {
                metrics.successes += 1;
            }
            metrics.routing_rounds += combined.rounds;
            metrics.lookup_key_observers += combined.observers.len();
            if policy.key == LookupKey::Raw {
                metrics.raw_content_observers += combined.observers.len();
            }
            if policy.origin == LookupOrigin::Direct {
                metrics.requester_key_observers += combined.queried.len();
            }

            let malicious_observers: BTreeSet<_> = combined
                .observers
                .iter()
                .copied()
                .filter(|&index| nodes[index].malicious)
                .collect();
            if !combined.success && !malicious_observers.is_empty() {
                metrics.eclipsed_lookups += 1;
            }
            for index in &malicious_observers {
                *malicious_repeat_counts.entry(*index).or_default() += 1;
                if policy.origin == LookupOrigin::Direct {
                    *malicious_requester_repeat_counts.entry(*index).or_default() += 1;
                }
            }
            if policy.provider == ProviderRecord::Direct
                && malicious_observers
                    .iter()
                    .any(|index| provider_observers.contains(index))
            {
                metrics.adversary_both_sides += 1;
            }
            if policy.provider == ProviderRecord::Direct
                && !malicious_observers.is_empty()
                && provider_observers
                    .iter()
                    .any(|&index| nodes[index].malicious)
            {
                metrics.colluding_adversary_both_sides += 1;
            }

            metrics.control_messages += combined.queried.len() * 2;
            if policy.origin == LookupOrigin::Oblivious {
                metrics.control_messages += policy.lookup_paths * 4;
            }
            if policy.provider == ProviderRecord::Indirect {
                metrics.control_messages += 2;
            }
        }

        if malicious_repeat_counts.values().any(|&count| count > 1) {
            metrics.repeated_key_linkability += 1;
        }
        if malicious_requester_repeat_counts
            .values()
            .any(|&count| count > 1)
        {
            metrics.repeated_requester_linkability += 1;
        }
    }

    ResearchResult {
        scenario,
        candidate,
        metrics,
    }
}

fn provider_observers(
    nodes: &[Node],
    policy: Policy,
    provider: usize,
    target: u64,
    replicas: &BTreeSet<usize>,
) -> BTreeSet<usize> {
    if policy.provider == ProviderRecord::Indirect {
        return BTreeSet::new();
    }
    let publication = route_lookup(nodes, provider, target, &BTreeSet::new(), false);
    publication.queried.union(replicas).copied().collect()
}

#[derive(Clone, Copy)]
struct LookupInputs<'a> {
    nodes: &'a [Node],
    policy: Policy,
    requester: usize,
    target: u64,
    replicas: &'a BTreeSet<usize>,
    content: usize,
    repetition: usize,
    seed: u64,
}

fn combined_lookup(inputs: LookupInputs<'_>) -> LookupTrace {
    let mut combined = LookupTrace::default();
    for path in 0..inputs.policy.lookup_paths {
        let start = if inputs.policy.origin == LookupOrigin::Oblivious {
            select_node(
                inputs.nodes,
                stable_hash(
                    b"gateway",
                    combine_seed(inputs.content, inputs.repetition, path) ^ inputs.seed,
                ),
                false,
            )
        } else {
            inputs.requester
        };
        let trace = route_lookup(
            inputs.nodes,
            start,
            inputs.target,
            inputs.replicas,
            inputs.policy.routing == RoutingPolicy::PrefixDiverse,
        );
        combined.success |= trace.success;
        combined.rounds = combined.rounds.max(trace.rounds);
        combined.observers.extend(trace.observers);
        combined.queried.extend(trace.queried);
    }
    if inputs.policy.origin == LookupOrigin::Direct {
        combined.observers.remove(&inputs.requester);
    }
    combined
}

fn empty_metrics(
    scenario: ResearchScenario,
    nodes: usize,
    routing_entries: usize,
) -> ResearchMetrics {
    ResearchMetrics {
        lookups: scenario.contents * scenario.lookups_per_content,
        successes: 0,
        routing_rounds: 0,
        lookup_key_observers: 0,
        raw_content_observers: 0,
        requester_key_observers: 0,
        provider_key_observers: 0,
        adversary_both_sides: 0,
        colluding_adversary_both_sides: 0,
        eclipsed_lookups: 0,
        repeated_key_linkability: 0,
        repeated_requester_linkability: 0,
        control_messages: 0,
        stored_records: 0,
        sybil_records: 0,
        routing_entries,
        nodes,
        contents: scenario.contents,
    }
}

/// Renders the reproducible metrics as Markdown tables.
#[must_use]
pub fn render_markdown(results: &[ResearchResult]) -> String {
    let mut output = String::from(
        "| Scenario | Candidate | Success | Modeled eclipse | Rounds | Key observers | Raw-ID observers | Requester+key | Provider+key | Same node both | Colluding both | Messages |\n\
         |---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n",
    );
    for result in results {
        let metrics = &result.metrics;
        writeln!(
            output,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            result.scenario.name,
            result.candidate.label(),
            ratio(metrics.successes, metrics.lookups),
            ratio(metrics.eclipsed_lookups, metrics.lookups),
            average(metrics.routing_rounds, metrics.lookups),
            average(metrics.lookup_key_observers, metrics.lookups),
            average(metrics.raw_content_observers, metrics.lookups),
            average(metrics.requester_key_observers, metrics.lookups),
            average(metrics.provider_key_observers, metrics.contents),
            ratio(metrics.adversary_both_sides, metrics.lookups),
            ratio(metrics.colluding_adversary_both_sides, metrics.lookups),
            average(metrics.control_messages, metrics.lookups),
        )
        .expect("writing to a String cannot fail");
    }

    output.push_str(
        "\n| Scenario | Candidate | Replicas/content | Sybil replicas | Routing contacts/node | Repeated key | Repeated requester |\n\
         |---|---|---:|---:|---:|---:|---:|\n",
    );
    for result in results {
        let metrics = &result.metrics;
        writeln!(
            output,
            "| {} | {} | {} | {} | {} | {} | {} |",
            result.scenario.name,
            result.candidate.label(),
            average(metrics.stored_records, metrics.contents),
            average(metrics.sybil_records, metrics.contents),
            average(metrics.routing_entries, metrics.nodes),
            ratio(metrics.repeated_key_linkability, metrics.contents),
            ratio(metrics.repeated_requester_linkability, metrics.contents),
        )
        .expect("writing to a String cannot fail");
    }
    output
}

fn build_nodes(scenario: ResearchScenario, sybil_target: u64, seed: u64) -> Vec<Node> {
    let malicious_count = scenario
        .ordinary_nodes
        .saturating_mul(usize::from(scenario.malicious_percent))
        / 100;
    let failed_count = scenario
        .ordinary_nodes
        .saturating_mul(usize::from(scenario.failed_percent))
        / 100;
    let malicious = ranked_indices(scenario.ordinary_nodes, malicious_count, b"malicious", seed);
    let failed = ranked_indices(scenario.ordinary_nodes, failed_count, b"failed", seed);
    let mut nodes: Vec<_> = (0..scenario.ordinary_nodes)
        .map(|index| Node {
            id: stable_hash(b"node", index as u64 ^ seed),
            group: u16::try_from(index % 256).expect("group is bounded"),
            malicious: malicious.contains(&index),
            failed: failed.contains(&index),
            sybil: false,
            routing: Vec::new(),
        })
        .collect();
    nodes.extend((0..scenario.sybil_nodes).map(|index| Node {
        id: sybil_target ^ (index as u64 + 1),
        group: u16::try_from(256 + index % scenario.sybil_groups).expect("group is bounded"),
        malicious: true,
        failed: false,
        sybil: true,
        routing: Vec::new(),
    }));
    nodes
}

fn ranked_indices(total: usize, count: usize, domain: &[u8], seed: u64) -> HashSet<usize> {
    let mut ranked: Vec<_> = (0..total)
        .map(|index| (stable_hash(domain, index as u64 ^ seed), index))
        .collect();
    ranked.sort_unstable();
    ranked
        .into_iter()
        .take(count)
        .map(|(_, index)| index)
        .collect()
}

fn build_routing_tables(nodes: &mut [Node]) {
    for current in 0..nodes.len() {
        let mut buckets: Vec<Vec<usize>> = (0..64).map(|_| Vec::new()).collect();
        for candidate in 0..nodes.len() {
            if current == candidate {
                continue;
            }
            let distance = nodes[current].id ^ nodes[candidate].id;
            let bucket = 63_usize.saturating_sub(distance.leading_zeros() as usize);
            buckets[bucket].push(candidate);
        }
        let mut routing = Vec::new();
        for bucket in &mut buckets {
            bucket.sort_unstable_by_key(|&index| nodes[index].id);
            routing.extend(bucket.iter().take(K_BUCKET_SIZE).copied());
        }
        nodes[current].routing = routing;
    }
}

fn contaminate_routing_tables(nodes: &mut [Node], percent: u8, seed: u64) {
    if percent == 0 {
        return;
    }
    let malicious: Vec<_> = nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| node.malicious.then_some(index))
        .collect();
    if malicious.is_empty() {
        return;
    }
    for (node_index, node) in nodes.iter_mut().enumerate() {
        let replacements = node.routing.len().saturating_mul(usize::from(percent)) / 100;
        for slot in 0..replacements {
            let choice_seed = stable_hash(
                b"routing-contamination",
                (node_index as u64).rotate_left(17) ^ slot as u64 ^ seed,
            );
            let attacker = malicious[usize::try_from(choice_seed % malicious.len() as u64)
                .expect("attacker index is bounded")];
            if attacker != node_index {
                node.routing[slot] = attacker;
            }
        }
        node.routing.sort_unstable();
        node.routing.dedup();
    }
}

fn select_replicas(
    nodes: &[Node],
    target: u64,
    diverse: bool,
    replica_count: usize,
) -> BTreeSet<usize> {
    let mut ordered: Vec<_> = (0..nodes.len()).collect();
    ordered.sort_unstable_by_key(|&index| nodes[index].id ^ target);
    let mut groups = HashSet::new();
    ordered
        .into_iter()
        .filter(|&index| !diverse || groups.insert(nodes[index].group))
        .take(replica_count)
        .collect()
}

fn route_lookup(
    nodes: &[Node],
    start: usize,
    target: u64,
    replicas: &BTreeSet<usize>,
    diverse: bool,
) -> LookupTrace {
    let mut trace = LookupTrace::default();
    trace.observers.insert(start);
    if nodes[start].failed || nodes[start].malicious {
        return trace;
    }

    let mut known: BTreeSet<_> = nodes[start].routing.iter().copied().collect();
    let mut queried_groups = HashSet::new();
    for round in 0..MAX_LOOKUP_ROUNDS {
        let mut candidates: Vec<_> = known
            .iter()
            .copied()
            .filter(|index| !trace.queried.contains(index))
            .collect();
        candidates.sort_unstable_by_key(|&index| nodes[index].id ^ target);
        let mut selected = Vec::new();
        for index in candidates {
            if diverse && !queried_groups.insert(nodes[index].group) {
                continue;
            }
            selected.push(index);
            if selected.len() == ALPHA {
                break;
            }
        }
        if selected.is_empty() {
            break;
        }
        trace.rounds = round + 1;

        for index in selected {
            trace.queried.insert(index);
            trace.observers.insert(index);
            let node = &nodes[index];
            if node.failed {
                continue;
            }
            if !node.malicious && replicas.contains(&index) {
                trace.success = true;
            }
            if !node.malicious {
                known.extend(node.routing.iter().copied());
            }
        }
        if trace.success || lookup_converged(nodes, target, &known, &trace.queried, diverse) {
            break;
        }
    }
    trace
}

fn lookup_converged(
    nodes: &[Node],
    target: u64,
    known: &BTreeSet<usize>,
    queried: &BTreeSet<usize>,
    diverse: bool,
) -> bool {
    let mut closest: Vec<_> = known.iter().copied().collect();
    closest.sort_unstable_by_key(|&index| nodes[index].id ^ target);
    let mut groups = HashSet::new();
    let closest: Vec<_> = closest
        .into_iter()
        .filter(|&index| !diverse || groups.insert(nodes[index].group))
        .take(K_BUCKET_SIZE)
        .collect();
    closest.len() == K_BUCKET_SIZE && closest.iter().all(|index| queried.contains(index))
}

fn select_node(nodes: &[Node], seed: u64, require_honest: bool) -> usize {
    let start = usize::try_from(seed % nodes.len() as u64).expect("index is bounded");
    (0..nodes.len())
        .map(|offset| (start + offset) % nodes.len())
        .find(|&index| !nodes[index].failed && (!require_honest || !nodes[index].malicious))
        .expect("scenario contains a usable node")
}

fn lookup_key(content: usize, blinded: bool, seed: u64) -> u64 {
    let raw = stable_hash(b"content", content as u64 ^ seed);
    if blinded {
        stable_hash(b"capability-key", raw)
    } else {
        raw
    }
}

fn combine_seed(content: usize, repetition: usize, path: usize) -> u64 {
    let content = content as u64;
    let repetition = repetition as u64;
    let path = path as u64;
    content.rotate_left(17) ^ repetition.rotate_left(33) ^ path.rotate_left(49)
}

fn stable_hash(domain: &[u8], value: u64) -> u64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&value.to_be_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&hasher.finalize().as_bytes()[..8]);
    u64::from_be_bytes(bytes)
}

fn ratio(part: usize, total: usize) -> String {
    let tenths = part.saturating_mul(1_000) / total.max(1);
    format!("{}.{:01}%", tenths / 10, tenths % 10)
}

fn average(total: usize, count: usize) -> String {
    let tenths = total.saturating_mul(10) / count.max(1);
    format!("{}.{:01}", tenths / 10, tenths % 10)
}

/// Distribution summary for the bounded seeded M8 Monte Carlo experiment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MonteCarloSummary {
    pub candidate: DiscoveryCandidate,
    pub trials: usize,
    pub success_min_permille: usize,
    pub success_median_permille: usize,
    pub success_max_permille: usize,
    pub eclipse_median_permille: usize,
    pub average_rounds_tenths: usize,
    pub average_messages_tenths: usize,
    pub honest_replicas_tenths: usize,
    pub malicious_replicas_tenths: usize,
    pub requester_key_observers_tenths: usize,
    pub provider_key_observers_tenths: usize,
    pub same_adversary_permille: usize,
    pub colluding_adversary_permille: usize,
    pub repeated_key_linkability_permille: usize,
}

/// Runs twelve fixed-seed, parameter-varying trials for every M3 candidate.
///
/// This is a comparative model, not an Internet-scale probability estimate.
#[must_use]
pub fn run_m8_monte_carlo() -> Vec<MonteCarloSummary> {
    let candidates = [
        DiscoveryCandidate::VanillaKademlia,
        DiscoveryCandidate::BlindedKademlia,
        DiscoveryCandidate::DistributedRendezvous,
        DiscoveryCandidate::SeparatedMultiStage,
    ];
    candidates
        .into_iter()
        .map(|candidate| summarize_trials(candidate, 12))
        .collect()
}

fn summarize_trials(candidate: DiscoveryCandidate, trial_count: usize) -> MonteCarloSummary {
    let mut success_rates = Vec::with_capacity(trial_count);
    let mut eclipse_rates = Vec::with_capacity(trial_count);
    let mut totals = ResearchMetrics {
        lookups: 0,
        successes: 0,
        routing_rounds: 0,
        lookup_key_observers: 0,
        raw_content_observers: 0,
        requester_key_observers: 0,
        provider_key_observers: 0,
        adversary_both_sides: 0,
        colluding_adversary_both_sides: 0,
        eclipsed_lookups: 0,
        repeated_key_linkability: 0,
        repeated_requester_linkability: 0,
        control_messages: 0,
        stored_records: 0,
        sybil_records: 0,
        routing_entries: 0,
        nodes: 0,
        contents: 0,
    };
    for trial in 0..trial_count {
        let scenario = ResearchScenario {
            name: "m8-seeded",
            ordinary_nodes: [100, 180, 260][trial % 3],
            malicious_percent: [5, 15, 30][trial % 3],
            sybil_nodes: [0, 24, 64][trial % 3],
            sybil_groups: [1, 4, 16][trial % 3],
            failed_percent: [0, 10, 20][(trial / 2) % 3],
            contents: 4,
            lookups_per_content: [4, 8][trial % 2],
        };
        let tuning = ResearchTuning {
            seed: [
                0x1020_3040,
                0x2233_4455,
                0x3141_5926,
                0x4242_4242,
                0x5566_7788,
                0x6172_8394,
                0x7000_0001,
                0x8123_4567,
                0x90ab_cdef,
                0xa1b2_c3d4,
                0xbad0_cafe,
                0xc001_d00d,
            ][trial],
            replica_count: [4, 8, 12][(trial / 2) % 3],
            separated_lookup_paths: [1, 3, 5][trial % 3],
            routing_contamination_percent: [0, 20, 50][(trial / 3) % 3],
        };
        let metrics = simulate_with_tuning(scenario, candidate, tuning).metrics;
        success_rates.push(permille(metrics.successes, metrics.lookups));
        eclipse_rates.push(permille(metrics.eclipsed_lookups, metrics.lookups));
        accumulate_metrics(&mut totals, &metrics);
    }
    success_rates.sort_unstable();
    eclipse_rates.sort_unstable();
    MonteCarloSummary {
        candidate,
        trials: trial_count,
        success_min_permille: success_rates[0],
        success_median_permille: success_rates[trial_count / 2],
        success_max_permille: success_rates[trial_count - 1],
        eclipse_median_permille: eclipse_rates[trial_count / 2],
        average_rounds_tenths: tenths(totals.routing_rounds, totals.lookups),
        average_messages_tenths: tenths(totals.control_messages, totals.lookups),
        honest_replicas_tenths: tenths(
            totals.stored_records.saturating_sub(totals.sybil_records),
            totals.contents,
        ),
        malicious_replicas_tenths: tenths(totals.sybil_records, totals.contents),
        requester_key_observers_tenths: tenths(totals.requester_key_observers, totals.lookups),
        provider_key_observers_tenths: tenths(totals.provider_key_observers, totals.contents),
        same_adversary_permille: permille(totals.adversary_both_sides, totals.lookups),
        colluding_adversary_permille: permille(
            totals.colluding_adversary_both_sides,
            totals.lookups,
        ),
        repeated_key_linkability_permille: permille(
            totals.repeated_key_linkability,
            totals.contents,
        ),
    }
}

fn accumulate_metrics(total: &mut ResearchMetrics, value: &ResearchMetrics) {
    total.lookups += value.lookups;
    total.successes += value.successes;
    total.routing_rounds += value.routing_rounds;
    total.lookup_key_observers += value.lookup_key_observers;
    total.raw_content_observers += value.raw_content_observers;
    total.requester_key_observers += value.requester_key_observers;
    total.provider_key_observers += value.provider_key_observers;
    total.adversary_both_sides += value.adversary_both_sides;
    total.colluding_adversary_both_sides += value.colluding_adversary_both_sides;
    total.eclipsed_lookups += value.eclipsed_lookups;
    total.repeated_key_linkability += value.repeated_key_linkability;
    total.repeated_requester_linkability += value.repeated_requester_linkability;
    total.control_messages += value.control_messages;
    total.stored_records += value.stored_records;
    total.sybil_records += value.sybil_records;
    total.routing_entries += value.routing_entries;
    total.nodes += value.nodes;
    total.contents += value.contents;
}

fn permille(part: usize, total: usize) -> usize {
    part.saturating_mul(1_000) / total.max(1)
}

fn tenths(total: usize, count: usize) -> usize {
    total.saturating_mul(10) / count.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: ResearchScenario = ResearchScenario {
        name: "small",
        ordinary_nodes: 100,
        malicious_percent: 10,
        sybil_nodes: 0,
        sybil_groups: 0,
        failed_percent: 5,
        contents: 4,
        lookups_per_content: 3,
    };

    #[test]
    fn research_simulation_is_deterministic() {
        let first = simulate(SMALL, DiscoveryCandidate::SeparatedMultiStage);
        let second = simulate(SMALL, DiscoveryCandidate::SeparatedMultiStage);
        assert_eq!(first, second);
    }

    #[test]
    fn blinded_indirect_lookup_reduces_modeled_metadata_linkage() {
        let vanilla = simulate(SMALL, DiscoveryCandidate::VanillaKademlia).metrics;
        let separated = simulate(SMALL, DiscoveryCandidate::SeparatedMultiStage).metrics;

        assert!(vanilla.raw_content_observers > 0);
        assert!(vanilla.requester_key_observers > 0);
        assert!(vanilla.provider_key_observers > 0);
        assert_eq!(separated.raw_content_observers, 0);
        assert_eq!(separated.requester_key_observers, 0);
        assert_eq!(separated.provider_key_observers, 0);
    }

    #[test]
    fn diversity_preserves_replicas_under_a_single_operator_sybil_cluster() {
        let scenario = ResearchScenario {
            name: "sybil",
            ordinary_nodes: 100,
            malicious_percent: 0,
            sybil_nodes: 32,
            sybil_groups: 1,
            failed_percent: 0,
            contents: 1,
            lookups_per_content: 10,
        };
        let vanilla = simulate(scenario, DiscoveryCandidate::VanillaKademlia).metrics;
        let separated = simulate(scenario, DiscoveryCandidate::SeparatedMultiStage).metrics;

        assert_eq!(vanilla.sybil_records, K_BUCKET_SIZE);
        assert!(separated.sybil_records < K_BUCKET_SIZE);
        assert!(separated.successes > vanilla.successes);
    }

    #[test]
    fn default_suite_covers_large_scale_and_targeted_sybil_failure() {
        let results = run_default_research_suite();
        assert_eq!(results.len(), DEFAULT_SCENARIOS.len() * 4);
        assert!(
            results
                .iter()
                .any(|result| result.scenario.ordinary_nodes == 1_000)
        );

        let targeted = |candidate| {
            &results
                .iter()
                .find(|result| {
                    result.scenario.name == "target-sybil-1000" && result.candidate == candidate
                })
                .expect("default suite should include the targeted-Sybil scenario")
                .metrics
        };
        let vanilla = targeted(DiscoveryCandidate::VanillaKademlia);
        let separated = targeted(DiscoveryCandidate::SeparatedMultiStage);

        assert_eq!(vanilla.successes, 0);
        assert_eq!(separated.successes, 99);
        assert!(separated.control_messages > vanilla.control_messages);
        assert_eq!(separated.requester_key_observers, 0);
    }

    #[test]
    fn m8_seeded_monte_carlo_is_reproducible_and_covers_all_candidates() {
        let first = run_m8_monte_carlo();
        let second = run_m8_monte_carlo();
        assert_eq!(first, second);
        assert_eq!(first.len(), 4);
        assert!(first.iter().all(|result| result.trials == 12));
        assert!(first.iter().all(|result| {
            result.success_min_permille <= result.success_median_permille
                && result.success_median_permille <= result.success_max_permille
        }));
    }
}
