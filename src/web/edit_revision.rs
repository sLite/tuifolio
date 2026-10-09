use std::hash::{DefaultHasher, Hash, Hasher};

/// A content fingerprint for optimistic editing, not an authentication token.
/// Only the edited record participates, so unrelated saves do not cause conflicts.
pub(super) fn record_revision(record: &impl Hash) -> String {
    let mut hasher = DefaultHasher::new();
    record.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[derive(Debug, thiserror::Error)]
#[error(
    "This {record} changed after you opened the form, or its edit version is missing. Your changes were not saved. Copy your edits, then reload the latest record before trying again."
)]
pub(super) struct EditConflict {
    record: &'static str,
}

pub(super) fn check_revision(
    record: &impl Hash,
    expected: &str,
    label: &'static str,
) -> anyhow::Result<()> {
    if expected != record_revision(record) {
        return Err(EditConflict { record: label }.into());
    }
    Ok(())
}
