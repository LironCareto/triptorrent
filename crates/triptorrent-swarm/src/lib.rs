#![doc = "Experimental M4 swarm scheduling and bandwidth controls."]

use std::time::Duration;
use thiserror::Error;
use triptorrent_protocol::PieceAvailability;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ChunkState {
    #[default]
    Missing,
    InFlight(usize),
    Complete,
}

#[derive(Clone, Debug)]
struct ProviderState {
    availability: PieceAvailability,
    active: bool,
    busy: bool,
    accepted: usize,
    rejected: usize,
}

/// Per-provider counters scoped to one transfer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderStats {
    /// Accepted, verified chunks.
    pub accepted: usize,
    /// Rejected or failed chunk requests.
    pub rejected: usize,
    /// Whether this provider remains eligible for new work.
    pub active: bool,
}

/// Deterministic rarest-first scheduler with at most one request per provider.
pub struct Scheduler {
    chunks: Vec<ChunkState>,
    providers: Vec<ProviderState>,
    retries: usize,
}

impl Scheduler {
    /// Creates a scheduler and marks resume-validated chunks complete.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed availability or mismatched state lengths.
    pub fn new(
        chunk_count: usize,
        availability: Vec<PieceAvailability>,
        completed: &[bool],
    ) -> Result<Self, ScheduleError> {
        if completed.len() != chunk_count {
            return Err(ScheduleError::InvalidCompletedLength);
        }
        let expected = u32::try_from(chunk_count).map_err(|_| ScheduleError::TooManyChunks)?;
        if availability
            .iter()
            .any(|pieces| pieces.chunk_count != expected || !pieces.is_valid())
        {
            return Err(ScheduleError::InvalidAvailability);
        }
        Ok(Self {
            chunks: completed
                .iter()
                .map(|done| {
                    if *done {
                        ChunkState::Complete
                    } else {
                        ChunkState::Missing
                    }
                })
                .collect(),
            providers: availability
                .into_iter()
                .map(|pieces| ProviderState {
                    availability: pieces,
                    active: true,
                    busy: false,
                    accepted: 0,
                    rejected: 0,
                })
                .collect(),
            retries: 0,
        })
    }

    /// Assigns the rarest available unclaimed chunk to an idle provider.
    #[must_use]
    pub fn assign(&mut self, provider: usize) -> Option<u32> {
        let state = self.providers.get(provider)?;
        if !state.active || state.busy {
            return None;
        }
        let choice = self
            .chunks
            .iter()
            .enumerate()
            .filter(|(index, state)| {
                **state == ChunkState::Missing
                    && self.providers[provider]
                        .availability
                        .contains(u32::try_from(*index).unwrap_or(u32::MAX))
            })
            .map(|(index, _)| (self.source_count(index), index))
            .min()?;
        self.chunks[choice.1] = ChunkState::InFlight(provider);
        self.providers[provider].busy = true;
        u32::try_from(choice.1).ok()
    }

    /// Records a verified chunk from its assigned provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the response does not match the outstanding request.
    pub fn accept(&mut self, provider: usize, index: u32) -> Result<(), ScheduleError> {
        let index = usize::try_from(index).map_err(|_| ScheduleError::UnexpectedChunk)?;
        if self.chunks.get(index) != Some(&ChunkState::InFlight(provider)) {
            return Err(ScheduleError::UnexpectedChunk);
        }
        self.chunks[index] = ChunkState::Complete;
        let state = self
            .providers
            .get_mut(provider)
            .ok_or(ScheduleError::UnknownProvider)?;
        state.busy = false;
        state.accepted += 1;
        Ok(())
    }

    /// Penalizes a provider for a failed or invalid response and releases its chunk.
    pub fn reject(&mut self, provider: usize) {
        for chunk in &mut self.chunks {
            if *chunk == ChunkState::InFlight(provider) {
                *chunk = ChunkState::Missing;
                self.retries += 1;
            }
        }
        if let Some(state) = self.providers.get_mut(provider) {
            state.busy = false;
            state.active = false;
            state.rejected += 1;
        }
    }

    /// Returns whether every chunk is complete.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.chunks
            .iter()
            .all(|chunk| *chunk == ChunkState::Complete)
    }

    /// Returns whether at least one request is currently outstanding.
    #[must_use]
    pub fn has_in_flight(&self) -> bool {
        self.chunks
            .iter()
            .any(|chunk| matches!(chunk, ChunkState::InFlight(_)))
    }

    /// Returns whether every missing chunk still has an active source.
    #[must_use]
    pub fn can_finish(&self) -> bool {
        self.chunks
            .iter()
            .enumerate()
            .all(|(index, state)| *state != ChunkState::Missing || self.source_count(index) > 0)
    }

    /// Returns provider-local counters.
    #[must_use]
    pub fn provider_stats(&self, provider: usize) -> Option<ProviderStats> {
        self.providers.get(provider).map(|state| ProviderStats {
            accepted: state.accepted,
            rejected: state.rejected,
            active: state.active,
        })
    }

    /// Returns the number of requests reassigned after failures.
    #[must_use]
    pub const fn retries(&self) -> usize {
        self.retries
    }

    fn source_count(&self, index: usize) -> usize {
        let index = u32::try_from(index).unwrap_or(u32::MAX);
        self.providers
            .iter()
            .filter(|provider| provider.active && provider.availability.contains(index))
            .count()
    }
}

/// Deterministic leaky-bucket reservation state used by runtime rate limiters.
#[derive(Clone, Debug)]
pub struct RateSchedule {
    bytes_per_second: u64,
    next_available_ns: u128,
}

impl RateSchedule {
    /// Creates an enabled rate schedule.
    #[must_use]
    pub const fn new(bytes_per_second: u64) -> Self {
        Self {
            bytes_per_second,
            next_available_ns: 0,
        }
    }

    /// Reserves `bytes` at monotonic `now`, returning how long the caller must wait.
    #[must_use]
    pub fn reserve(&mut self, now: Duration, bytes: u64) -> Duration {
        if self.bytes_per_second == 0 || bytes == 0 {
            return Duration::ZERO;
        }
        let now_ns = now.as_nanos();
        let start = self.next_available_ns.max(now_ns);
        let service_ns = u128::from(bytes)
            .saturating_mul(1_000_000_000)
            .div_ceil(u128::from(self.bytes_per_second));
        self.next_available_ns = start.saturating_add(service_ns);
        duration_from_nanos(start.saturating_sub(now_ns))
    }
}

fn duration_from_nanos(nanos: u128) -> Duration {
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// Invalid scheduler input or provider response.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum ScheduleError {
    /// Resume completion flags must exactly match the manifest.
    #[error("resume completion state has the wrong length")]
    InvalidCompletedLength,
    /// A provider supplied a malformed or differently sized bitfield.
    #[error("provider supplied invalid piece availability")]
    InvalidAvailability,
    /// This prototype uses 32-bit chunk indices.
    #[error("manifest has more than u32::MAX chunks")]
    TooManyChunks,
    /// A response came from an unknown provider slot.
    #[error("unknown provider")]
    UnknownProvider,
    /// A provider returned an index that was not assigned to it.
    #[error("provider returned an unexpected chunk")]
    UnexpectedChunk,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_prefers_rare_chunks_and_avoids_duplicates() {
        let availability = vec![
            PieceAvailability::from_indices(4, [0, 1, 2, 3]),
            PieceAvailability::from_indices(4, [1, 2, 3]),
        ];
        let mut scheduler = Scheduler::new(4, availability, &[false; 4]).unwrap();
        assert_eq!(scheduler.assign(0), Some(0));
        assert_eq!(scheduler.assign(1), Some(1));
        scheduler.accept(0, 0).unwrap();
        scheduler.accept(1, 1).unwrap();
        assert_eq!(scheduler.assign(0), Some(2));
        assert_eq!(scheduler.assign(1), Some(3));
    }

    #[test]
    fn rejected_work_is_reassigned_to_another_source() {
        let availability = vec![PieceAvailability::all(2), PieceAvailability::all(2)];
        let mut scheduler = Scheduler::new(2, availability, &[false; 2]).unwrap();
        assert_eq!(scheduler.assign(0), Some(0));
        scheduler.reject(0);
        assert_eq!(scheduler.assign(1), Some(0));
        assert_eq!(scheduler.retries(), 1);
    }

    #[test]
    fn rate_schedule_is_deterministic() {
        let mut limiter = RateSchedule::new(1_000);
        assert_eq!(limiter.reserve(Duration::ZERO, 250), Duration::ZERO);
        assert_eq!(
            limiter.reserve(Duration::from_millis(100), 250),
            Duration::from_millis(150)
        );
        assert_eq!(
            limiter.reserve(Duration::from_millis(500), 500),
            Duration::ZERO
        );
    }

    #[test]
    fn replayed_or_wrong_provider_chunk_cannot_change_verified_state() {
        let availability = vec![PieceAvailability::all(1), PieceAvailability::all(1)];
        let mut scheduler = Scheduler::new(1, availability, &[false]).unwrap();
        assert_eq!(scheduler.assign(0), Some(0));
        assert_eq!(scheduler.accept(1, 0), Err(ScheduleError::UnexpectedChunk));
        assert!(!scheduler.is_complete());
        scheduler.accept(0, 0).unwrap();
        assert!(scheduler.is_complete());
        assert_eq!(scheduler.accept(0, 0), Err(ScheduleError::UnexpectedChunk));
        assert!(scheduler.is_complete());
    }

    #[test]
    fn hostile_provider_retries_are_bounded_by_provider_count() {
        let availability = vec![PieceAvailability::all(1); 4];
        let mut scheduler = Scheduler::new(1, availability, &[false]).unwrap();
        for provider in 0..4 {
            assert_eq!(scheduler.assign(provider), Some(0));
            scheduler.reject(provider);
            assert!(
                scheduler
                    .provider_stats(provider)
                    .is_some_and(|state| !state.active)
            );
        }
        assert_eq!(scheduler.retries(), 4);
        assert!(!scheduler.can_finish());
        assert_eq!(scheduler.assign(0), None);
    }
}
