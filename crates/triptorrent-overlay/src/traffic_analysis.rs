//! Deterministic M8 model of metadata visible to a transfer relay.
//!
//! The model uses the current encoded M4 messages only to obtain ciphertext
//! lengths. It records direction, framed length and logical time; it never
//! exposes message plaintext to the modeled observer.

use triptorrent_core::{CHUNK_SIZE, Chunk, ChunkId, ContentId, Manifest};
use triptorrent_protocol::{Message, PieceAvailability};

const NOISE_TAG_BYTES: usize = 16;
const FRAME_PREFIX_BYTES: usize = 4;
const DISCOVERY_TO_RELAY_MS: u64 = 100;
const PAUSE_GAP_MS: u64 = 5_000;

/// Controlled transfer inputs used to generate one observer trace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrafficScenario {
    pub name: &'static str,
    pub content_bytes: usize,
    pub providers: usize,
    pub bytes_per_second: u64,
    pub striped_availability: bool,
    pub retries: usize,
    pub pauses: usize,
}

/// Frame direction visible to the relay.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameDirection {
    RequesterToProvider,
    ProviderToRequester,
}

/// Metadata visible for one opaque relay frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameObservation {
    pub logical_time_ms: u64,
    pub direction: FrameDirection,
    pub framed_bytes: usize,
}

/// Measurements calculated from one controlled trace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrafficMetrics {
    pub scenario: TrafficScenario,
    pub frame_count: usize,
    pub total_framed_bytes: usize,
    pub duration_ms: u64,
    pub bootstrap_to_relay_ms: u64,
    pub distinct_frame_sizes: usize,
    pub size_fingerprint: String,
}

/// Aggregate transparent-classifier results for the fixed M8 experiment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrafficAnalysisResult {
    pub traces: Vec<TrafficMetrics>,
    pub size_rank_pairs: usize,
    pub size_rank_agreements: usize,
    pub nearest_size_correct: usize,
    pub nearest_size_trials: usize,
    pub repeated_shape_fingerprint_equal: bool,
}

/// Runs the fixed deterministic metadata-only traffic experiment.
#[must_use]
pub fn run_m8_traffic_analysis() -> TrafficAnalysisResult {
    let scenarios = [
        scenario("small-single", 64 * 1024, 1, 0, false, 0, 0),
        scenario("small-paused", 64 * 1024, 2, 32 * 1024, true, 1, 1),
        scenario("medium-single", 1024 * 1024, 1, 0, false, 0, 0),
        scenario("medium-swarm", 1024 * 1024, 4, 256 * 1024, true, 2, 0),
        scenario("large-single", 4 * 1024 * 1024, 1, 0, false, 0, 0),
        scenario("large-paused", 4 * 1024 * 1024, 4, 512 * 1024, true, 3, 1),
    ];
    let traces: Vec<_> = scenarios.iter().copied().map(trace_metrics).collect();
    let mut size_rank_pairs = 0;
    let mut size_rank_agreements = 0;
    for left in 0..traces.len() {
        for right in left + 1..traces.len() {
            if traces[left].scenario.content_bytes == traces[right].scenario.content_bytes {
                continue;
            }
            size_rank_pairs += 1;
            if traces[left]
                .scenario
                .content_bytes
                .cmp(&traces[right].scenario.content_bytes)
                == traces[left]
                    .total_framed_bytes
                    .cmp(&traces[right].total_framed_bytes)
            {
                size_rank_agreements += 1;
            }
        }
    }
    let canonical: Vec<_> = [64 * 1024, 1024 * 1024, 4 * 1024 * 1024]
        .into_iter()
        .map(|size| trace_metrics(scenario("canonical", size, 1, 0, false, 0, 0)))
        .collect();
    let nearest_size_correct = traces
        .iter()
        .filter(|trace| {
            canonical
                .iter()
                .min_by_key(|candidate| {
                    candidate
                        .total_framed_bytes
                        .abs_diff(trace.total_framed_bytes)
                })
                .is_some_and(|candidate| {
                    candidate.scenario.content_bytes == trace.scenario.content_bytes
                })
        })
        .count();
    let repeated = trace_metrics(scenario(
        "medium-repeat",
        1024 * 1024,
        1,
        64 * 1024,
        false,
        0,
        1,
    ));
    TrafficAnalysisResult {
        repeated_shape_fingerprint_equal: repeated.size_fingerprint == traces[2].size_fingerprint,
        nearest_size_correct,
        nearest_size_trials: traces.len(),
        size_rank_pairs,
        size_rank_agreements,
        traces,
    }
}

const fn scenario(
    name: &'static str,
    content_bytes: usize,
    providers: usize,
    bytes_per_second: u64,
    striped_availability: bool,
    retries: usize,
    pauses: usize,
) -> TrafficScenario {
    TrafficScenario {
        name,
        content_bytes,
        providers,
        bytes_per_second,
        striped_availability,
        retries,
        pauses,
    }
}

fn trace_metrics(scenario: TrafficScenario) -> TrafficMetrics {
    let observations = simulate_observer_trace(scenario);
    let total_framed_bytes = observations.iter().map(|event| event.framed_bytes).sum();
    let duration_ms = observations
        .last()
        .map_or(0, |event| event.logical_time_ms)
        .saturating_sub(DISCOVERY_TO_RELAY_MS);
    let mut sizes: Vec<_> = observations
        .iter()
        .map(|event| event.framed_bytes)
        .collect();
    sizes.sort_unstable();
    sizes.dedup();
    let mut fingerprint = blake3::Hasher::new();
    for event in &observations {
        fingerprint.update(&event.framed_bytes.to_be_bytes());
        fingerprint.update(&[match event.direction {
            FrameDirection::RequesterToProvider => 0,
            FrameDirection::ProviderToRequester => 1,
        }]);
    }
    TrafficMetrics {
        scenario,
        frame_count: observations.len(),
        total_framed_bytes,
        duration_ms,
        bootstrap_to_relay_ms: DISCOVERY_TO_RELAY_MS,
        distinct_frame_sizes: sizes.len(),
        size_fingerprint: fingerprint.finalize().to_hex().to_string(),
    }
}

#[allow(clippy::too_many_lines)] // The linear trace generator mirrors the visible wire sequence.
fn simulate_observer_trace(scenario: TrafficScenario) -> Vec<FrameObservation> {
    assert!(scenario.providers > 0);
    let chunk_count = scenario.content_bytes.div_ceil(CHUNK_SIZE);
    let mut chunk_ids = Vec::with_capacity(chunk_count);
    for index in 0..chunk_count {
        let length = if index + 1 == chunk_count {
            scenario.content_bytes - index * CHUNK_SIZE
        } else {
            CHUNK_SIZE
        };
        chunk_ids.push(ChunkId::digest(&vec![0_u8; length]));
    }
    let manifest = Manifest {
        content_id: ContentId::digest(scenario.name.as_bytes()),
        length: u64::try_from(scenario.content_bytes).expect("scenario size fits u64"),
        chunk_size: u32::try_from(CHUNK_SIZE).expect("chunk size fits u32"),
        chunks: chunk_ids.clone(),
    };
    let mut events = Vec::new();
    let mut now = DISCOVERY_TO_RELAY_MS;
    for provider in 0..scenario.providers {
        push_event(&mut events, now, FrameDirection::RequesterToProvider, 48);
        push_event(
            &mut events,
            now + 1,
            FrameDirection::ProviderToRequester,
            48,
        );
        push_message(
            &mut events,
            now + 2,
            FrameDirection::RequesterToProvider,
            Message::SwarmRequest {
                content_id: manifest.content_id,
            },
        );
        let indices = (0..chunk_count)
            .filter(|index| {
                !scenario.striped_availability || index % scenario.providers == provider
            })
            .map(|index| u32::try_from(index).expect("scenario chunk index fits u32"));
        push_message(
            &mut events,
            now + 3,
            FrameDirection::ProviderToRequester,
            Message::SwarmManifest {
                manifest: manifest.clone(),
                availability: PieceAvailability::from_indices(
                    u32::try_from(chunk_count).expect("scenario chunk count fits u32"),
                    indices,
                ),
            },
        );
        now += 4;
    }
    let cadence = if scenario.bytes_per_second == 0 {
        1
    } else {
        u64::try_from(CHUNK_SIZE)
            .expect("chunk size fits u64")
            .saturating_mul(1_000)
            .div_ceil(scenario.bytes_per_second)
            .max(1)
    };
    for (index, chunk_id) in chunk_ids.into_iter().enumerate() {
        let index = u32::try_from(index).expect("scenario chunk index fits u32");
        push_message(
            &mut events,
            now,
            FrameDirection::RequesterToProvider,
            Message::ChunkRequest { index },
        );
        let length = expected_length(scenario.content_bytes, usize::try_from(index).unwrap());
        push_message(
            &mut events,
            now + cadence,
            FrameDirection::ProviderToRequester,
            Message::Chunk(Chunk {
                index,
                id: chunk_id,
                data: vec![0; length],
            }),
        );
        now = now.saturating_add(cadence + 1);
        if scenario.pauses > 0 && usize::try_from(index).unwrap() == chunk_count / 2 {
            now = now.saturating_add(PAUSE_GAP_MS);
        }
    }
    for retry in 0..scenario.retries {
        let index = u32::try_from(retry % chunk_count.max(1)).unwrap();
        push_message(
            &mut events,
            now,
            FrameDirection::RequesterToProvider,
            Message::ChunkRequest { index },
        );
        push_message(
            &mut events,
            now + cadence,
            FrameDirection::ProviderToRequester,
            Message::ChunkUnavailable { index },
        );
        now = now.saturating_add(cadence + 1);
    }
    events
}

fn expected_length(total: usize, index: usize) -> usize {
    (total - index * CHUNK_SIZE).min(CHUNK_SIZE)
}

fn push_message(
    events: &mut Vec<FrameObservation>,
    logical_time_ms: u64,
    direction: FrameDirection,
    message: Message,
) {
    let encoded = triptorrent_protocol::encode(message).expect("controlled message is valid");
    push_event(
        events,
        logical_time_ms,
        direction,
        encoded.len() + NOISE_TAG_BYTES,
    );
}

fn push_event(
    events: &mut Vec<FrameObservation>,
    logical_time_ms: u64,
    direction: FrameDirection,
    payload_bytes: usize,
) {
    events.push(FrameObservation {
        logical_time_ms,
        direction,
        framed_bytes: payload_bytes + FRAME_PREFIX_BYTES,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m8_traffic_experiment_is_reproducible_and_metadata_only() {
        let first = run_m8_traffic_analysis();
        let second = run_m8_traffic_analysis();
        assert_eq!(first, second);
        assert_eq!(first.size_rank_agreements, first.size_rank_pairs);
        assert!(first.nearest_size_correct >= 5);
        assert!(first.repeated_shape_fingerprint_equal);
        assert!(
            first
                .traces
                .iter()
                .all(|trace| trace.bootstrap_to_relay_ms == 100)
        );
    }
}
