use crate::models::tournament::{Tournament, TournamentConfig, TournamentStatus};
use crate::services::bracket_service::{self, BracketShape};
use crate::services::fee_service;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug)]
pub enum TournamentError {
    NotFound,
    InvalidConfig(Vec<ConfigIssue>),
    Conflict(String),
    Internal(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigIssue {
    pub field: String,
    pub message: String,
}

/// Read-only result of validating a proposed tournament configuration.
/// Mirrors exactly what the create path would compute, without persisting anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TournamentConfigPreview {
    pub valid: bool,
    pub issues: Vec<ConfigIssue>,
    pub bracket: Option<BracketShape>,
    pub fee_cents: Option<i64>,
    pub capacity: Option<i64>,
}

pub struct TournamentService {
    pool: PgPool,
}

impl TournamentService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Shared validation used by both preview and create so they cannot diverge.
    fn validate_config(config: &TournamentConfig) -> Vec<ConfigIssue> {
        let mut issues = Vec::new();

        if config.max_participants <= 0 {
            issues.push(ConfigIssue {
                field: "max_participants".into(),
                message: "must be greater than zero".into(),
            });
        }

        if config.min_participants <= 0 {
            issues.push(ConfigIssue {
                field: "min_participants".into(),
                message: "must be greater than zero".into(),
            });
        }

        if config.min_participants > config.max_participants {
            issues.push(ConfigIssue {
                field: "min_participants".into(),
                message: "cannot exceed max_participants".into(),
            });
        }

        if config.entry_fee_cents < 0 {
            issues.push(ConfigIssue {
                field: "entry_fee_cents".into(),
                message: "cannot be negative".into(),
            });
        }

        if config.starts_at >= config.ends_at {
            issues.push(ConfigIssue {
                field: "starts_at".into(),
                message: "must be before ends_at".into(),
            });
        }

        issues
    }

    /// Dry-run preview for organizers. Performs no reservation, payment,
    /// notification, or persistent mutation.
    pub async fn preview_config(
        &self,
        config: &TournamentConfig,
    ) -> Result<TournamentConfigPreview, TournamentError> {
        let issues = Self::validate_config(config);

        if !issues.is_empty() {
            return Ok(TournamentConfigPreview {
                valid: false,
                issues,
                bracket: None,
                fee_cents: None,
                capacity: None,
            });
        }

        let bracket = bracket_service::compute_shape(config.max_participants);
        let fee_cents = fee_service::calculate_entry_fee(config);

        Ok(TournamentConfigPreview {
            valid: true,
            issues: Vec::new(),
            bracket: Some(bracket),
            fee_cents: Some(fee_cents),
            capacity: Some(config.max_participants),
        })
    }

    pub async fn create_tournament(
        &self,
        organizer_id: Uuid,
        config: &TournamentConfig,
    ) -> Result<Tournament, TournamentError> {
        let issues = Self::validate_config(config);
        if !issues.is_empty() {
            return Err(TournamentError::InvalidConfig(issues));
        }

        let bracket = bracket_service::compute_shape(config.max_participants);
        let fee_cents = fee_service::calculate_entry_fee(config);

        let now: DateTime<Utc> = Utc::now();
        let tournament = sqlx::query_as::<_, Tournament>(
            r#"
            INSERT INTO tournaments
                (id, organizer_id, status, max_participants, min_participants,
                 entry_fee_cents, bracket_size, starts_at, ends_at, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)
            RETURNING *
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(organizer_id)
        .bind(TournamentStatus::Draft)
        .bind(config.max_participants)
        .bind(config.min_participants)
        .bind(fee_cents)
        .bind(bracket.size)
        .bind(config.starts_at)
        .bind(config.ends_at)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| TournamentError::Internal(e.to_string()))?;

        Ok(tournament)
    }
}
