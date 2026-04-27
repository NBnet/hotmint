use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

use hotmint_types::Height;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// WAL (Write-Ahead Log) entry types.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum WalEntry {
    /// Logged before starting a commit batch. If the node crashes after this
    /// entry but before `CommitDone`, recovery knows a mid-commit crash occurred.
    CommitIntent {
        /// The target height (highest block being committed).
        target_height: Height,
    },
    /// Logged after commit + persist_state succeeds.
    CommitDone { target_height: Height },
}

const WAL_FILE: &str = "consensus.wal";
/// Magic bytes at the start of each entry for corruption detection.
const ENTRY_MAGIC: [u8; 4] = [0x57, 0x41, 0x4C, 0x31]; // "WAL1"

/// Write-Ahead Log for consensus commit operations.
///
/// Provides crash recovery by recording commit intent before executing blocks
/// and commit-done after persisting state. On restart, if a `CommitIntent`
/// without a matching `CommitDone` is found, the node knows a mid-commit
/// crash occurred and can re-execute from the block store.
pub struct ConsensusWal {
    path: PathBuf,
    file: File,
}

/// Result of WAL recovery check on startup.
#[derive(Debug, Clone, PartialEq)]
pub enum WalRecovery {
    /// No recovery needed — last commit completed successfully or WAL is empty.
    Clean,
    /// A commit was in progress when the node crashed. The application should
    /// re-execute blocks from `last_committed_height + 1` to `target_height`.
    NeedsReplay { target_height: Height },
}

impl ConsensusWal {
    /// Open or create the WAL file in the given data directory.
    pub fn open(data_dir: &Path) -> io::Result<Self> {
        let path = data_dir.join(WAL_FILE);
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .read(true)
            .open(&path)?;
        // Seek to end so new entries append after any existing content.
        // Unlike `.append(true)`, this allows truncate+seek(0) to work
        // correctly — subsequent writes go to position 0, not the old EOF.
        file.seek(io::SeekFrom::End(0))?;
        Ok(Self { path, file })
    }

    /// Check the WAL for incomplete commits (called on startup).
    pub fn check_recovery(data_dir: &Path) -> io::Result<WalRecovery> {
        let path = data_dir.join(WAL_FILE);
        Self::check_recovery_path(&path, None)
    }

    /// Check the WAL and clear stale intents that are already covered by the
    /// durable consensus checkpoint.
    pub fn check_recovery_with_committed_height(
        data_dir: &Path,
        last_committed_height: Height,
    ) -> io::Result<WalRecovery> {
        let path = data_dir.join(WAL_FILE);
        Self::check_recovery_path(&path, Some(last_committed_height))
    }

    /// Reconcile this open WAL handle with the durable consensus checkpoint.
    pub fn reconcile_with_committed_height(
        &mut self,
        last_committed_height: Height,
    ) -> io::Result<WalRecovery> {
        let recovery = Self::check_recovery_path(&self.path, Some(last_committed_height))?;
        self.file.seek(io::SeekFrom::End(0))?;
        Ok(recovery)
    }

    fn check_recovery_path(
        path: &Path,
        last_committed_height: Option<Height>,
    ) -> io::Result<WalRecovery> {
        if !path.exists() {
            return Ok(WalRecovery::Clean);
        }

        let mut file = File::open(path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        let mut last_entry = None;
        let mut offset = 0;

        while offset < buf.len() {
            match decode_entry(&buf[offset..]) {
                DecodeResult::Entry(entry, consumed) => {
                    last_entry = Some(entry);
                    offset += consumed;
                }
                DecodeResult::Truncated => {
                    warn!(offset, "WAL: ignoring truncated, unsynced EOF tail");
                    truncate_path(path, offset as u64)?;
                    break;
                }
                DecodeResult::Invalid => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("corrupt or truncated WAL entry at byte offset {offset}"),
                    ));
                }
            }
        }

        match last_entry {
            Some(WalEntry::CommitIntent { target_height }) => {
                if let Some(committed_height) = last_committed_height
                    && committed_height >= target_height
                {
                    warn!(
                        target_height = target_height.as_u64(),
                        committed_height = committed_height.as_u64(),
                        "WAL: clearing stale commit intent already covered by persisted state"
                    );
                    truncate_path(path, 0)?;
                    return Ok(WalRecovery::Clean);
                }
                warn!(
                    target_height = target_height.as_u64(),
                    "WAL: incomplete commit detected, replay needed"
                );
                Ok(WalRecovery::NeedsReplay { target_height })
            }
            Some(WalEntry::CommitDone { .. }) => {
                truncate_path(path, 0)?;
                Ok(WalRecovery::Clean)
            }
            None => Ok(WalRecovery::Clean),
        }
    }

    /// Record that a commit batch is about to start.
    pub fn log_commit_intent(&mut self, target_height: Height) -> io::Result<()> {
        let entry = WalEntry::CommitIntent { target_height };
        self.write_entry(&entry)?;
        self.file.sync_all()
    }

    /// Record that a commit batch completed successfully.
    pub fn log_commit_done(&mut self, target_height: Height) -> io::Result<()> {
        let entry = WalEntry::CommitDone { target_height };
        self.write_entry(&entry)?;
        // Truncate the WAL after a successful commit to keep it small.
        // The WAL only needs to survive between intent and done.
        self.file.sync_all()?;
        self.truncate()
    }

    /// Truncate the WAL file (called after successful commit).
    fn truncate(&mut self) -> io::Result<()> {
        self.file.set_len(0)?;
        self.file.seek(io::SeekFrom::Start(0))?;
        self.file.sync_all()
    }

    fn write_entry(&mut self, entry: &WalEntry) -> io::Result<()> {
        let payload = postcard::to_allocvec(entry).map_err(io::Error::other)?;
        let len = payload.len() as u32;
        self.file.write_all(&ENTRY_MAGIC)?;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&payload)?;
        Ok(())
    }
}

enum DecodeResult {
    Entry(WalEntry, usize),
    Truncated,
    Invalid,
}

/// Decode one WAL entry from a byte slice. Returns the entry and bytes consumed.
fn decode_entry(buf: &[u8]) -> DecodeResult {
    if buf.len() < 4 {
        return if ENTRY_MAGIC.starts_with(buf) {
            DecodeResult::Truncated
        } else {
            DecodeResult::Invalid
        };
    }
    if buf[..4] != ENTRY_MAGIC {
        return DecodeResult::Invalid;
    }
    if buf.len() < 8 {
        return DecodeResult::Truncated;
    }
    let len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    if buf.len() < 8 + len {
        return DecodeResult::Truncated;
    }
    let Ok(entry) = postcard::from_bytes(&buf[8..8 + len]) else {
        return DecodeResult::Invalid;
    };
    DecodeResult::Entry(entry, 8 + len)
}

fn truncate_path(path: &Path, len: u64) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).open(path)?;
    file.set_len(len)?;
    file.seek(io::SeekFrom::Start(len))?;
    file.sync_all()
}

/// The WAL struct implements `hotmint_consensus::Wal`.
impl hotmint_consensus::Wal for ConsensusWal {
    fn log_commit_intent(&mut self, target_height: Height) -> io::Result<()> {
        self.log_commit_intent(target_height)
    }
    fn log_commit_done(&mut self, target_height: Height) -> io::Result<()> {
        self.log_commit_done(target_height)
    }
}

/// No-op WAL for testing or when WAL is disabled.
pub struct NoopWal;

impl hotmint_consensus::Wal for NoopWal {
    fn log_commit_intent(&mut self, _target_height: Height) -> io::Result<()> {
        Ok(())
    }
    fn log_commit_done(&mut self, _target_height: Height) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wal_clean_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let recovery = ConsensusWal::check_recovery(dir.path()).unwrap();
        assert_eq!(recovery, WalRecovery::Clean);
    }

    #[test]
    fn wal_clean_after_done() {
        let dir = tempfile::tempdir().unwrap();
        let mut wal = ConsensusWal::open(dir.path()).unwrap();
        wal.log_commit_intent(Height(5)).unwrap();
        wal.log_commit_done(Height(5)).unwrap();

        let recovery = ConsensusWal::check_recovery(dir.path()).unwrap();
        assert_eq!(recovery, WalRecovery::Clean);
    }

    #[test]
    fn wal_needs_replay_after_intent() {
        let dir = tempfile::tempdir().unwrap();
        let mut wal = ConsensusWal::open(dir.path()).unwrap();
        wal.log_commit_intent(Height(10)).unwrap();
        // Simulate crash — no log_commit_done
        drop(wal);

        let recovery = ConsensusWal::check_recovery(dir.path()).unwrap();
        assert_eq!(
            recovery,
            WalRecovery::NeedsReplay {
                target_height: Height(10),
            }
        );
    }

    #[test]
    fn wal_recovery_ignores_unsynced_partial_tail() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(WAL_FILE), [ENTRY_MAGIC[0]]).unwrap();

        let recovery = ConsensusWal::check_recovery(dir.path()).unwrap();
        assert_eq!(recovery, WalRecovery::Clean);
        assert_eq!(
            std::fs::metadata(dir.path().join(WAL_FILE)).unwrap().len(),
            0
        );
    }

    #[test]
    fn wal_recovery_rejects_corrupt_magic() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(WAL_FILE), [0xff]).unwrap();

        let err = ConsensusWal::check_recovery(dir.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn wal_recovery_clears_stale_intent_at_committed_height() {
        let dir = tempfile::tempdir().unwrap();
        let mut wal = ConsensusWal::open(dir.path()).unwrap();
        wal.log_commit_intent(Height(10)).unwrap();
        drop(wal);

        let recovery =
            ConsensusWal::check_recovery_with_committed_height(dir.path(), Height(10)).unwrap();
        assert_eq!(recovery, WalRecovery::Clean);
        assert_eq!(
            std::fs::metadata(dir.path().join(WAL_FILE)).unwrap().len(),
            0
        );
    }

    #[test]
    fn wal_truncated_after_done() {
        let dir = tempfile::tempdir().unwrap();
        let mut wal = ConsensusWal::open(dir.path()).unwrap();

        // Multiple commit cycles
        for h in 1..=5 {
            wal.log_commit_intent(Height(h)).unwrap();
            wal.log_commit_done(Height(h)).unwrap();
        }

        // WAL should be truncated/clean
        let path = dir.path().join(WAL_FILE);
        let size = std::fs::metadata(&path).unwrap().len();
        assert_eq!(size, 0, "WAL should be truncated after successful commits");
    }
}
