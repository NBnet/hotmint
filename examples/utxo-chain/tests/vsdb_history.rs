//! Versioned UTXOs and SMT roots survive independent process restarts.
use std::{path::PathBuf, process::Command};
use utxo_chain::{OutPoint, TxOutput};
use vsdb::{SmtCalc, VerMap, VerMapWithProof};

fn key(n: u8) -> [u8; 36] {
    OutPoint {
        txid: [n; 32],
        vout: n as u32,
    }
    .to_key()
}
fn phase(dir: PathBuf, phase: &str) {
    let mut proof = if phase == "seed" {
        VerMapWithProof::<[u8; 36], TxOutput, SmtCalc>::new()
    } else {
        let bytes = std::fs::read(dir.join("map-id")).unwrap();
        let id = u64::from_le_bytes(bytes.try_into().unwrap());
        VerMapWithProof::from_map(VerMap::from_meta(id).unwrap())
    };
    let branch = proof.map().main_branch();
    if phase == "seed" {
        for n in 0..64 {
            proof
                .map_mut()
                .insert(
                    branch,
                    &key(n),
                    &TxOutput {
                        value: n as u64 + 100,
                        pubkey_hash: [n % 4; 32],
                    },
                )
                .unwrap();
        }
        proof.map_mut().commit(branch).unwrap();
        let id = proof.map().save_meta().unwrap().map_id;
        std::fs::write(dir.join("map-id"), id.to_le_bytes()).unwrap();
    }
    if phase == "append" {
        proof.map_mut().remove(branch, &key(0)).unwrap();
        proof
            .map_mut()
            .insert(
                branch,
                &key(64),
                &TxOutput {
                    value: 100,
                    pubkey_hash: [0; 32],
                },
            )
            .unwrap();
        proof.map_mut().commit(branch).unwrap();
    }
    let added = phase == "append" || phase == "read-appended";
    let mut oracle = SmtCalc::from_entries((u8::from(added)..64 + u8::from(added)).map(|n| {
        (
            key(n).to_vec(),
            postcard::to_allocvec(&TxOutput {
                value: if n == 64 { 100 } else { n as u64 + 100 },
                pubkey_hash: [n % 4; 32],
            })
            .unwrap(),
        )
    }))
    .unwrap();
    let root: [u8; 32] = proof.merkle_root(branch).unwrap().try_into().unwrap();
    assert_eq!(root.as_slice(), oracle.root_hash().unwrap());
    for n in 0..=64 {
        let p = proof.prove(&key(n)).unwrap();
        assert!(SmtCalc::verify_proof(&root, &key(n), &p).unwrap());
        assert_eq!(p.value().is_some(), if added { n > 0 } else { n < 64 });
    }
    let root_file = dir.join(if added { "root-after" } else { "root-before" });
    if phase == "seed" || phase == "append" {
        std::fs::write(root_file, root).unwrap();
    } else {
        assert_eq!(std::fs::read(root_file).unwrap(), root);
    }
    vsdb::vsdb_flush();
    println!("HISTORY phase={phase} root={root:02x?}");
    std::process::exit(0);
}
#[test]
fn versioned_utxo_process_boundary() {
    if let Ok(action) = std::env::var("HOTMINT_VSDB_PHASE") {
        phase(
            PathBuf::from(std::env::var("HOTMINT_VSDB_DIR").unwrap()),
            &action,
        );
    }
    let dir = std::env::temp_dir().join(format!("hotmint-vsdb-history-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    for action in ["seed", "read", "append", "read-appended"] {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "versioned_utxo_process_boundary", "--nocapture"])
            .env("HOTMINT_VSDB_PHASE", action)
            .env("HOTMINT_VSDB_DIR", &dir)
            .env("VSDB_BASE_DIR", dir.join("db"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{action}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
