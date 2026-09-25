use hotmint_light::{LightClient, MptProof, SmtProof};
use std::{env, fs, path::Path, process::Command};
use vsdb::{MptCalc, SmtCalc};

fn verify_transmitted_proofs(dir: &Path) {
    for state in ["empty", "populated"] {
        let dir = dir.join(state);
        let mpt_root: [u8; 32] = fs::read(dir.join("mpt-root")).unwrap().try_into().unwrap();
        let smt_root: [u8; 32] = fs::read(dir.join("smt-root")).unwrap().try_into().unwrap();
        let mut wrong_mpt_root = mpt_root;
        let mut wrong_smt_root = smt_root;
        wrong_mpt_root[0] ^= 1;
        wrong_smt_root[0] ^= 1;
        for name in ["account", "missing"] {
            let mpt_bytes = fs::read(dir.join(format!("mpt-{name}"))).unwrap();
            let smt_bytes = fs::read(dir.join(format!("smt-{name}"))).unwrap();
            let mpt = MptProof::from_bytes(&mpt_bytes).unwrap();
            let smt = SmtProof::from_bytes(&smt_bytes).unwrap();
            assert!(LightClient::verify_state_proof(&mpt_root, name.as_bytes(), &mpt).unwrap());
            assert!(LightClient::verify_smt_state_proof(&smt_root, name.as_bytes(), &smt).unwrap());
            let expected = (state == "populated" && name == "account").then_some(b"10".as_slice());
            assert_eq!(mpt.value(), expected);
            assert_eq!(smt.value(), expected);
            assert!(!LightClient::verify_state_proof(&mpt_root, b"wrong-key", &mpt).unwrap());
            assert!(!LightClient::verify_smt_state_proof(&smt_root, b"wrong-key", &smt).unwrap());
            assert!(!matches!(
                LightClient::verify_state_proof(&wrong_mpt_root, name.as_bytes(), &mpt),
                Ok(true)
            ));
            assert!(!matches!(
                LightClient::verify_smt_state_proof(&wrong_smt_root, name.as_bytes(), &smt),
                Ok(true)
            ));
            assert!(MptProof::from_bytes(&smt_bytes).is_err());
            assert!(SmtProof::from_bytes(&mpt_bytes).is_err());
        }
    }
}

#[test]
fn both_proof_kinds_reach_the_light_client_across_processes() {
    if let Some(dir) = env::var_os("HOTMINT_PROOF_TRANSPORT_DIR") {
        verify_transmitted_proofs(Path::new(&dir));
        return;
    }
    let dir = env::temp_dir().join(format!("hotmint-proof-{}", rand::random::<u128>()));
    fs::create_dir(&dir).unwrap();
    let mut mpt = MptCalc::new();
    let mut smt = SmtCalc::new();
    for state in ["empty", "populated"] {
        if state == "populated" {
            mpt.insert(b"account", b"10").unwrap();
            smt.insert(b"account", b"10").unwrap();
        }
        let state_dir = dir.join(state);
        fs::create_dir(&state_dir).unwrap();
        fs::write(state_dir.join("mpt-root"), mpt.root_hash().unwrap()).unwrap();
        fs::write(state_dir.join("smt-root"), smt.root_hash().unwrap()).unwrap();
        for name in ["account", "missing"] {
            fs::write(
                state_dir.join(format!("mpt-{name}")),
                mpt.prove(name.as_bytes()).unwrap().to_bytes().unwrap(),
            )
            .unwrap();
            fs::write(
                state_dir.join(format!("smt-{name}")),
                smt.prove(name.as_bytes()).unwrap().to_bytes().unwrap(),
            )
            .unwrap();
        }
    }
    let child = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "both_proof_kinds_reach_the_light_client_across_processes",
            "--nocapture",
        ])
        .env("HOTMINT_PROOF_TRANSPORT_DIR", &dir)
        .output()
        .unwrap();
    assert!(child.status.success(), "{child:?}");
    fs::remove_dir_all(dir).unwrap();
}
