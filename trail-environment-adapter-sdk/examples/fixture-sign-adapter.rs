//! Deterministic conformance-fixture signer. Do not use embedded seed material
//! from tests for real publisher keys.

use std::env;
use std::fs;
use std::path::PathBuf;

use ed25519_dalek::{Signer, SigningKey};
use trail_environment_adapter_sdk::{
    AdapterPackageSignature, AdapterPublisherKey, PACKAGE_SIGNATURE_SCHEMA_V1,
    TRUSTED_PUBLISHER_KEY_SCHEMA_V1,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("fixture-sign-adapter: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let publisher = args.next().ok_or("missing publisher")?;
    let seed = args.next().ok_or("missing 32-byte seed hex")?;
    let payload_digest = args.next().ok_or("missing payload digest")?;
    let signature_path = PathBuf::from(args.next().ok_or("missing signature path")?);
    let key_path = PathBuf::from(args.next().ok_or("missing key path")?);
    if args.next().is_some() {
        return Err("unexpected extra arguments".to_string());
    }
    let seed = hex::decode(seed).map_err(|error| format!("invalid seed hex: {error}"))?;
    let seed: [u8; 32] = seed
        .try_into()
        .map_err(|_| "seed must decode to exactly 32 bytes".to_string())?;
    let signing_key = SigningKey::from_bytes(&seed);
    let public_key = signing_key.verifying_key().to_bytes();
    let key_id = format!("sha256:{}", sha256_hex(&public_key));
    let message = format!("{PACKAGE_SIGNATURE_SCHEMA_V1}\0{payload_digest}");
    let signature = signing_key.sign(message.as_bytes());
    let signature_document = AdapterPackageSignature {
        schema: PACKAGE_SIGNATURE_SCHEMA_V1.to_string(),
        publisher: publisher.clone(),
        key_id,
        payload_digest,
        signature: hex::encode(signature.to_bytes()),
    };
    let key_document = AdapterPublisherKey {
        schema: TRUSTED_PUBLISHER_KEY_SCHEMA_V1.to_string(),
        publisher,
        public_key: hex::encode(public_key),
    };
    fs::write(
        signature_path,
        toml::to_string(&signature_document).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        key_path,
        toml::to_string(&key_document).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    hex::encode(Sha256::digest(bytes))
}
