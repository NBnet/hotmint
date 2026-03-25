use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

use hotmint_types::Height;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

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
    CommitDone {
        target_height: Height,
    },
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
        if !path.exists() {
            return Ok(WalRecovery::Clean);
        }

        let mut file = File::open(&path)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;

        let mut last_entry = None;
        let mut offset = 0;

        while offset < buf.len() {
            match decode_entry(&buf[offset..]) {
                Some((entry, consumed)) => {
                    last_entry = Some(entry);
                    offset += consumed;
                }
                None => break, // corrupt or truncated tail
            }
        }

        match last_entry {
            Some(WalEntry::CommitIntent { target_height }) => {
                warn!(
                    target_height = target_height.as_u64(),
                    "WAL: incomplete commit detected, replay needed"
                );
                Ok(WalRecovery::NeedsReplay { target_height })
            }
            _ => Ok(WalRecovery::Clean),
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
        let payload =
            postcard::to_allocvec(entry).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        let len = payload.len() as u32;
        self.file.write_all(&ENTRY_MAGIC)?;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(&payload)?;
        Ok(())
    }
}

/// Decode one WAL entry from a byte slice. Returns the entry and bytes consumed.
fn decode_entry(buf: &[u8]) -> Option<(WalEntry, usize)> {
    if buf.len() < 8 {
        return None;
    }
    if buf[..4] != ENTRY_MAGIC {
        return None;
    }
    let len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    if buf.len() < 8 + len {
        return None;
    }
    let entry: WalEntry = postcard::from_bytes(&buf[8..8 + len]).ok()?;
    Some((entry, 8 + len))
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
        assert_eq!(recovery, WalRecovery::NeedsReplay {
            target_height: Height(10),
        });
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
