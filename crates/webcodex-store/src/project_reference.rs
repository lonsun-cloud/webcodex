use super::{Database, StoreDomain};
use rusqlite::{params, OptionalExtension, TransactionBehavior};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectReferenceRecord {
    pub ref_index: u64,
    pub canonical_project_id: String,
    pub root_fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectReferenceStoreError {
    InvalidInput,
    Unavailable,
}

impl std::fmt::Display for ProjectReferenceStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInput => "invalid project reference input",
            Self::Unavailable => "project reference store unavailable",
        })
    }
}

impl std::error::Error for ProjectReferenceStoreError {}

fn store_error(error: rusqlite::Error) -> ProjectReferenceStoreError {
    tracing::warn!(error = %error, "durable project reference store operation failed");
    ProjectReferenceStoreError::Unavailable
}

fn valid_input(principal_key: &str, canonical_project_id: &str, root_fingerprint: &str) -> bool {
    !principal_key.is_empty()
        && principal_key.len() <= 128
        && !canonical_project_id.is_empty()
        && canonical_project_id.len() <= 512
        && root_fingerprint
            .strip_prefix("wc_projroot_")
            .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

impl Database {
    pub fn get_or_create_project_reference(
        &self,
        principal_key: &str,
        canonical_project_id: &str,
        root_fingerprint: &str,
        created_at: i64,
    ) -> Result<ProjectReferenceRecord, ProjectReferenceStoreError> {
        if !valid_input(principal_key, canonical_project_id, root_fingerprint) {
            return Err(ProjectReferenceStoreError::InvalidInput);
        }
        let mut conn = self.lock_connection(StoreDomain::ProjectReference);
        let transaction = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let existing = transaction
            .query_row(
                "SELECT ref_index, canonical_project_id, root_fingerprint
                 FROM project_references
                 WHERE principal_key = ?1
                   AND canonical_project_id = ?2
                   AND root_fingerprint = ?3",
                params![principal_key, canonical_project_id, root_fingerprint],
                |row| {
                    let ref_index: i64 = row.get(0)?;
                    Ok(ProjectReferenceRecord {
                        ref_index: ref_index as u64,
                        canonical_project_id: row.get(1)?,
                        root_fingerprint: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(store_error)?;
        if let Some(existing) = existing {
            transaction.commit().map_err(store_error)?;
            return Ok(existing);
        }

        let next_index: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(ref_index), 0) + 1
                 FROM project_references
                 WHERE principal_key = ?1",
                params![principal_key],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        if next_index <= 0 {
            return Err(ProjectReferenceStoreError::Unavailable);
        }
        transaction
            .execute(
                "INSERT INTO project_references (
                    principal_key, ref_index, canonical_project_id, root_fingerprint, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    principal_key,
                    next_index,
                    canonical_project_id,
                    root_fingerprint,
                    created_at
                ],
            )
            .map_err(store_error)?;
        transaction.commit().map_err(store_error)?;
        Ok(ProjectReferenceRecord {
            ref_index: next_index as u64,
            canonical_project_id: canonical_project_id.to_string(),
            root_fingerprint: root_fingerprint.to_string(),
        })
    }

    pub fn lookup_project_reference(
        &self,
        principal_key: &str,
        ref_index: u64,
    ) -> Result<Option<ProjectReferenceRecord>, ProjectReferenceStoreError> {
        if principal_key.is_empty() || principal_key.len() > 128 || ref_index == 0 {
            return Err(ProjectReferenceStoreError::InvalidInput);
        }
        let ref_index =
            i64::try_from(ref_index).map_err(|_| ProjectReferenceStoreError::InvalidInput)?;
        let conn = self.lock_connection(StoreDomain::ProjectReference);
        conn.query_row(
            "SELECT ref_index, canonical_project_id, root_fingerprint
             FROM project_references
             WHERE principal_key = ?1 AND ref_index = ?2",
            params![principal_key, ref_index],
            |row| {
                let ref_index: i64 = row.get(0)?;
                Ok(ProjectReferenceRecord {
                    ref_index: ref_index as u64,
                    canonical_project_id: row.get(1)?,
                    root_fingerprint: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(store_error)
    }
}
