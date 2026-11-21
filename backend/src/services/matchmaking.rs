//! Canonical matchmaking service boundary.
//!
//! Defines the deterministic queue-exit state machine for queued, matched,
//! and reserved players (issue #193). All join/cancel/timeout/reconnect
//! operations funnel through this module so that retries are idempotent and
//! reservation expiry restores eligible players without skipping fairness
//! rules.

use std::collections::HashMap;
use std::sync::Mutex;

/// Lifecycle of a player inside the matchmaking queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueState {
    /// Waiting in the queue, eligible to be matched.
    Queued,
    /// Placed into a match; awaiting reservation confirmation.
    Matched,
    /// Holds a reservation slot for a specific match.
    Reserved,
    /// Left the queue voluntarily (cancel).
    Cancelled,
    /// Reservation or queue slot expired (timeout).
    Expired,
}

impl QueueState {
    /// Terminal states cannot transition further without a fresh join.
    pub fn is_terminal(self) -> bool {
        matches!(self, QueueState::Cancelled | QueueState::Expired)
    }
}

/// Outcome surfaced to clients for every queue-exit operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueOutcome {
    /// Player is queued (join accepted or already queued).
    Queued { position: usize },
    /// Player was matched into a match.
    Matched { match_id: String },
    /// Player holds a reservation for a match.
    Reserved { match_id: String },
    /// Player cancelled; `already` is true when the cancel was a no-op retry.
    Cancelled { already: bool },
    /// Reservation/queue slot expired.
    Expired { already: bool },
    /// Operation rejected because the player is in a terminal state.
    Rejected { reason: RejectReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// Cancel attempted after the player was already matched.
    AlreadyMatched,
    /// Reservation confirmation attempted without a reservation.
    NoReservation,
    /// Player is not known to the queue.
    UnknownPlayer,
}

/// A single queue entry keyed by player id.
#[derive(Debug, Clone)]
struct Entry {
    state: QueueState,
    match_id: Option<String>,
    /// Monotonic sequence used to preserve fairness ordering.
    seq: u64,
}

/// Deterministic matchmaking queue with idempotent operations.
#[derive(Debug, Default)]
pub struct MatchmakingQueue {
    inner: Mutex<QueueInner>,
}

#[derive(Debug, Default)]
struct QueueInner {
    entries: HashMap<String, Entry>,
    next_seq: u64,
}

impl MatchmakingQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Join the queue. Retries are idempotent: an already-queued player keeps
    /// their original position and sequence.
    pub fn join(&self, player_id: &str) -> QueueOutcome {
        let mut inner = self.inner.lock().expect("matchmaking lock poisoned");
        if let Some(entry) = inner.entries.get(player_id) {
            match entry.state {
                QueueState::Queued => {
                    let position = inner.position_of(player_id);
                    return QueueOutcome::Queued { position };
                }
                QueueState::Matched => {
                    return QueueOutcome::Matched {
                        match_id: entry.match_id.clone().unwrap_or_default(),
                    };
                }
                QueueState::Reserved => {
                    return QueueOutcome::Reserved {
                        match_id: entry.match_id.clone().unwrap_or_default(),
                    };
                }
                QueueState::Cancelled | QueueState::Expired => {
                    // A fresh join after a terminal exit re-enters the queue
                    // with a new sequence, preserving fairness ordering.
                }
            }
        }
        let seq = inner.next_seq;
        inner.next_seq += 1;
        inner.entries.insert(
            player_id.to_string(),
            Entry {
                state: QueueState::Queued,
                match_id: None,
                seq,
            },
        );
        let position = inner.position_of(player_id);
        QueueOutcome::Queued { position }
    }

    /// Cancel a queued player. Cancelling a matched or reserved player is
    /// rejected so finalized matches stay immutable. Retries are idempotent.
    pub fn cancel(&self, player_id: &str) -> QueueOutcome {
        let mut inner = self.inner.lock().expect("matchmaking lock poisoned");
        match inner.entries.get_mut(player_id) {
            None => QueueOutcome::Rejected {
                reason: RejectReason::UnknownPlayer,
            },
            Some(entry) => match entry.state {
                QueueState::Queued => {
                    entry.state = QueueState::Cancelled;
                    QueueOutcome::Cancelled { already: false }
                }
                QueueState::Cancelled | QueueState::Expired => {
                    QueueOutcome::Cancelled { already: true }
                }
                QueueState::Matched | QueueState::Reserved => QueueOutcome::Rejected {
                    reason: RejectReason::AlreadyMatched,
                },
            },
        }
    }

    /// Expire a queued or reserved player. Expiry of a reserved player frees
    /// the reservation and returns the player to the queue tail so fairness
    /// ordering is preserved. Retries are idempotent.
    pub fn expire(&self, player_id: &str) -> QueueOutcome {
        let mut inner = self.inner.lock().expect("matchmaking lock poisoned");
        let seq = inner.next_seq;
        match inner.entries.get_mut(player_id) {
            None => QueueOutcome::Rejected {
                reason: RejectReason::UnknownPlayer,
            },
            Some(entry) => match entry.state {
                QueueState::Queued => {
                    entry.state = QueueState::Expired;
                    QueueOutcome::Expired { already: false }
                }
                QueueState::Reserved => {
                    // Reservation cleanup: restore eligibility at the tail.
                    entry.state = QueueState::Queued;
                    entry.match_id = None;
                    entry.seq = seq;
                    inner.next_seq += 1;
                    let position = inner.position_of(player_id);
                    QueueOutcome::Queued { position }
                }
                QueueState::Matched => QueueOutcome::Rejected {
                    reason: RejectReason::AlreadyMatched,
                },
                QueueState::Cancelled | QueueState::Expired => {
                    QueueOutcome::Expired { already: true }
                }
            },
        }
    }

    /// Mark a queued player as matched. Idempotent for the same match id.
    pub fn mark_matched(&self, player_id: &str, match_id: &str) -> QueueOutcome {
        let mut inner = self.inner.lock().expect("matchmaking lock poisoned");
        match inner.entries.get_mut(player_id) {
            None => QueueOutcome::Rejected {
                reason: RejectReason::UnknownPlayer,
            },
            Some(entry) => match entry.state {
                QueueState::Queued => {
                    entry.state = QueueState::Matched;
                    entry.match_id = Some(match_id.to_string());
                    QueueOutcome::Matched {
                        match_id: match_id.to_string(),
                    }
                }
                QueueState::Matched | QueueState::Reserved => QueueOutcome::Matched {
                    match_id: entry.match_id.clone().unwrap_or_default(),
                },
                QueueState::Cancelled | QueueState::Expired => QueueOutcome::Rejected {
                    reason: RejectReason::UnknownPlayer,
                },
            },
        }
    }

    /// Confirm a reservation for a matched player. Idempotent.
    pub fn reserve(&self, player_id: &str) -> QueueOutcome {
        let mut inner = self.inner.lock().expect("matchmaking lock poisoned");
        match inner.entries.get_mut(player_id) {
            None => QueueOutcome::Rejected {
                reason: RejectReason::UnknownPlayer,
            },
            Some(entry) => match entry.state {
                QueueState::Matched => {
                    entry.state = QueueState::Reserved;
                    QueueOutcome::Reserved {
                        match_id: entry.match_id.clone().unwrap_or_default(),
                    }
                }
                QueueState::Reserved => QueueOutcome::Reserved {
                    match_id: entry.match_id.clone().unwrap_or_default(),
                },
                _ => QueueOutcome::Rejected {
                    reason: RejectReason::NoReservation,
                },
            },
        }
    }

    /// Current state for a player, if known.
    pub fn state_of(&self, player_id: &str) -> Option<QueueState> {
        let inner = self.inner.lock().expect("matchmaking lock poisoned");
        inner.entries.get(player_id).map(|e| e.state)
    }
}

impl QueueInner {
    /// 1-based position among queued players ordered by fairness sequence.
    fn position_of(&self, player_id: &str) -> usize {
        let mut queued: Vec<(&String, u64)> = self
            .entries
            .iter()
            .filter(|(_, e)| e.state == QueueState::Queued)
            .map(|(id, e)| (id, e.seq))
            .collect();
        queued.sort_by_key(|(_, seq)| *seq);
        queued
            .iter()
            .position(|(id, _)| id.as_str() == player_id)
            .map(|idx| idx + 1)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_is_idempotent() {
        let q = MatchmakingQueue::new();
        assert_eq!(q.join("a"), QueueOutcome::Queued { position: 1 });
        assert_eq!(q.join("a"), QueueOutcome::Queued { position: 1 });
        assert_eq!(q.join("b"), QueueOutcome::Queued { position: 2 });
    }

    #[test]
    fn cancel_is_idempotent_and_rejects_matched() {
        let q = MatchmakingQueue::new();
        q.join("a");
        assert_eq!(q.cancel("a"), QueueOutcome::Cancelled { already: false });
        assert_eq!(q.cancel("a"), QueueOutcome::Cancelled { already: true });

        q.join("b");
        q.mark_matched("b", "m1");
        assert_eq!(
            q.cancel("b"),
            QueueOutcome::Rejected {
                reason: RejectReason::AlreadyMatched
            }
        );
    }

    #[test]
    fn reservation_expiry_restores_to_tail() {
        let q = MatchmakingQueue::new();
        q.join("a");
        q.join("b");
        q.mark_matched("a", "m1");
        q.reserve("a");
        // b is now first in line; a rejoins at the tail after expiry.
        assert_eq!(q.expire("a"), QueueOutcome::Queued { position: 2 });
        assert_eq!(q.state_of("a"), Some(QueueState::Queued));
        assert_eq!(q.expire("a"), QueueOutcome::Expired { already: false });
        assert_eq!(q.expire("a"), QueueOutcome::Expired { already: true });
    }

    #[test]
    fn rejoin_after_terminal_gets_new_sequence() {
        let q = MatchmakingQueue::new();
        q.join("a");
        q.join("b");
        q.cancel("a");
        assert_eq!(q.join("a"), QueueOutcome::Queued { position: 2 });
    }
}
