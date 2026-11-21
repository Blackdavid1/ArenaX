#![allow(dead_code)]

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;
use validator::Validate;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Tournament {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub game: String, // Changed from game_type for consistency
    pub max_participants: i32,
    pub entry_fee: i64, // Changed from Decimal for database compatibility
    pub entry_fee_currency: String,
    pub prize_pool: i64, // Changed from Decimal
    pub prize_pool_currency: String,
    pub status: TournamentStatus,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub registration_deadline: DateTime<Utc>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub bracket_type: BracketType,
    pub rules: Option<String>,
    pub min_skill_level: Option<i32>, // For skill-based matchmaking
    pub max_skill_level: Option<i32>,
    pub cleaned_up_at: Option<DateTime<Utc>>,
}

/// Immutable snapshot of the tournament rules a participant accepted at
/// registration. Lifecycle transitions and settlement MUST read from this
/// snapshot rather than the mutable `Tournament` row so that later edits to
/// tournament settings cannot retroactively change participant obligations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TournamentRulesSnapshot {
    /// Bumped whenever the snapshot schema or hashing changes.
    pub version: u32,
    /// Stable content hash over the canonical snapshot payload.
    pub hash: String,
    pub format: SnapshotFormatTerms,
    pub scoring: SnapshotScoringTerms,
    pub eligibility: SnapshotEligibilityTerms,
    pub timing: SnapshotTimingTerms,
    pub fee: SnapshotFeeTerms,
    pub payout: SnapshotPayoutTerms,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotFormatTerms {
    pub game: String,
    pub bracket_type: String,
    pub max_participants: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotScoringTerms {
    pub rules: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotEligibilityTerms {
    pub min_skill_level: Option<i32>,
    pub max_skill_level: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotTimingTerms {
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub registration_deadline: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotFeeTerms {
    pub entry_fee: i64,
    pub entry_fee_currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotPayoutTerms {
    pub prize_pool: i64,
    pub prize_pool_currency: String,
}

/// Current snapshot schema version.
pub const RULES_SNAPSHOT_VERSION: u32 = 1;

impl TournamentRulesSnapshot {
    /// Build a snapshot from the current tournament row, computing the stable
    /// content hash over the canonical payload.
    pub fn capture(tournament: &Tournament) -> Self {
        let mut snapshot = Self {
            version: RULES_SNAPSHOT_VERSION,
            hash: String::new(),
            format: SnapshotFormatTerms {
                game: tournament.game.clone(),
                bracket_type: tournament.bracket_type.to_string(),
                max_participants: tournament.max_participants,
            },
            scoring: SnapshotScoringTerms {
                rules: tournament.rules.clone(),
            },
            eligibility: SnapshotEligibilityTerms {
                min_skill_level: tournament.min_skill_level,
                max_skill_level: tournament.max_skill_level,
            },
            timing: SnapshotTimingTerms {
                start_time: tournament.start_time,
                end_time: tournament.end_time,
                registration_deadline: tournament.registration_deadline,
            },
            fee: SnapshotFeeTerms {
                entry_fee: tournament.entry_fee,
                entry_fee_currency: tournament.entry_fee_currency.clone(),
            },
            payout: SnapshotPayoutTerms {
                prize_pool: tournament.prize_pool,
                prize_pool_currency: tournament.prize_pool_currency.clone(),
            },
        };
        snapshot.hash = snapshot.compute_hash();
        snapshot
    }

    /// Deterministic hash over the canonical snapshot payload (excluding the
    /// hash field itself). Used to detect drift and to key re-consent flows.
    pub fn compute_hash(&self) -> String {
        let canonical = serde_json::json!({
            "version": self.version,
            "format": self.format,
            "scoring": self.scoring,
            "eligibility": self.eligibility,
            "timing": self.timing,
            "fee": self.fee,
            "payout": self.payout,
        });
        let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
        let digest = Sha256::digest(&bytes);
        format!("{:x}", digest)
    }

    /// Verify the stored hash still matches the payload (tamper/drift check).
    pub fn verify(&self) -> bool {
        self.hash == self.compute_hash()
    }

    /// True when the live tournament settings differ from the accepted
    /// snapshot, meaning a versioned re-consent flow is required.
    pub fn differs_from(&self, tournament: &Tournament) -> bool {
        let current = Self::capture(tournament);
        current.compute_hash() != self.compute_hash()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TournamentType {
    SingleElimination,
    DoubleElimination,
    RoundRobin,
    Swiss,
}

impl std::fmt::Display for TournamentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TournamentType::SingleElimination => write!(f, "single_elimination"),
            TournamentType::DoubleElimination => write!(f, "double_elimination"),
            TournamentType::RoundRobin => write!(f, "round_robin"),
            TournamentType::Swiss => write!(f, "swiss"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[repr(i32)]
pub enum TournamentStatus {
    Draft = 0,
    Upcoming = 1,
    RegistrationOpen = 2,
    RegistrationClosed = 3,
    InProgress = 4,
    Completed = 5,
    Cancelled = 6,
}

impl std::fmt::Display for TournamentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TournamentStatus::Draft => write!(f, "draft"),
            TournamentStatus::Upcoming => write!(f, "upcoming"),
            TournamentStatus::RegistrationOpen => write!(f, "registration_open"),
            TournamentStatus::RegistrationClosed => write!(f, "registration_closed"),
            TournamentStatus::InProgress => write!(f, "in_progress"),
            TournamentStatus::Completed => write!(f, "completed"),
            TournamentStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl From<i32> for TournamentStatus {
    fn from(v: i32) -> Self {
        match v {
            0 => TournamentStatus::Draft,
            1 => TournamentStatus::Upcoming,
            2 => TournamentStatus::RegistrationOpen,
            3 => TournamentStatus::RegistrationClosed,
            4 => TournamentStatus::InProgress,
            5 => TournamentStatus::Completed,
            6 => TournamentStatus::Cancelled,
            _ => TournamentStatus::Draft,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TournamentVisibility {
    Public,
    Private,
    InviteOnly,
}

impl std::fmt::Display for TournamentVisibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TournamentVisibility::Public => write!(f, "public"),
            TournamentVisibility::Private => write!(f, "private"),
            TournamentVisibility::InviteOnly => write!(f, "invite_only"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateTournamentRequest {
    #[validate(length(min = 3, max = 255))]
    pub name: String,
    #[validate(length(max = 1000))]
    pub description: Option<String>,
    #[validate(length(min = 1, max = 50))]
    pub game: String,
    pub bracket_type: BracketType,
    pub entry_fee: i64,
    pub entry_fee_currency: String,
    #[validate(range(min = 2, max = 1000))]
    pub max_participants: i32,
    pub start_time: DateTime<Utc>,
    pub registration_deadline: DateTime<Utc>,
    pub rules: Option<String>,
    pub min_skill_level: Option<i32>,
    pub max_skill_level: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct UpdateTournamentRequest {
    #[validate(length(min = 3, max = 255))]
    pub name: Option<String>,
    #[validate(length(max = 1000))]
    pub description: Option<String>,
    pub tournament_type: Option<TournamentType>,
    pub entry_fee: Option<Decimal>,
    #[validate(range(min = 2, max = 1000))]
    pub max_participants: Option<i32>,
    pub visibility: Option<TournamentVisibility>,
    pub registration_start: Option<DateTime<Utc>>,
    pub registration_end: Option<DateTime<Utc>>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub rules: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentResponse {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub game: String,
    pub max_participants: i32,
    pub current_participants: i32,
    pub entry_fee: i64,
    pub entry_fee_currency: String,
    pub prize_pool: i64,
    pub prize_pool_currency: String,
    pub status: TournamentStatus,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub registration_deadline: DateTime<Utc>,
    pub bracket_type: BracketType,
    pub can_join: bool,
    pub is_participant: bool,
    pub participant_status: Option<ParticipantStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TournamentParticipant {
    pub id: Uuid,
    pub tournament_id: Uuid,
    pub user_id: Uuid,
    pub registered_at: DateTime<Utc>,
    pub entry_fee_paid: bool,
    pub status: ParticipantStatus,
    pub seed_number: Option<i32>,
    pub current_round: Option<i32>,
    pub eliminated_at: Option<DateTime<Utc>>,
    pub final_rank: Option<i32>,
    pub prize_amount: Option<i64>,
    pub prize_currency: Option<String>,
    pub prize_tx_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[repr(i32)]
pub enum ParticipantStatus {
    Registered = 0,
    Paid = 1,
    Active = 2,
    Eliminated = 3,
    Disqualified = 4,
    Withdrawn = 5,
}

impl std::fmt::Display for ParticipantStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParticipantStatus::Registered => write!(f, "registered"),
            ParticipantStatus::Paid => write!(f, "paid"),
            ParticipantStatus::Active => write!(f, "active"),
            ParticipantStatus::Eliminated => write!(f, "eliminated"),
            ParticipantStatus::Disqualified => write!(f, "disqualified"),
            ParticipantStatus::Withdrawn => write!(f, "withdrawn"),
        }
    }
}

impl From<i32> for ParticipantStatus {
    fn from(v: i32) -> Self {
        match v {
            0 => ParticipantStatus::Registered,
            1 => ParticipantStatus::Paid,
            2 => ParticipantStatus::Active,
            3 => ParticipantStatus::Eliminated,
            4 => ParticipantStatus::Disqualified,
            5 => ParticipantStatus::Withdrawn,
            _ => ParticipantStatus::Registered,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TournamentStanding {
    pub tournament_id: Uuid,
    pub tournament_name: String,
    pub user_id: Uuid,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub participation_status: String,
    pub wins: Option<i64>,
    pub matches_played: Option<i64>,
    pub total_score: Option<Decimal>,
    pub current_rank: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PrizePool {
    pub id: Uuid,
    pub tournament_id: Uuid,
    pub total_amoun

/* … truncated 6211 chars — edit only what you need near the top … */
