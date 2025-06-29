use anyhow::{Context, anyhow};
use libsecp256k1::{PublicKey, SecretKey};
use metis_primitives::Address;
use rand_chacha::{ChaCha20Rng, rand_core::SeedableRng};
use serde_json::json;
use std::path::{Path, PathBuf};

use super::{from_b64, to_b64};
use crate::cmd;
use metis_app_options::key::{
    KeyAddressArgs, KeyArgs, KeyCommands, KeyGenArgs, KeyIntoTendermintArgs,
};

cmd! {
  KeyArgs(self) {
    match &self.command {
        KeyCommands::Gen(args) => args.exec(()).await,
        KeyCommands::IntoTendermint(args) => args.exec(()).await,
        KeyCommands::Address(args) => args.exec(()).await,
    }
  }
}

cmd! {
  KeyGenArgs(self) {
    let mut rng = ChaCha20Rng::from_entropy();
    let sk = SecretKey::random(&mut rng);
    let pk = PublicKey::from_secret_key(&sk);

    export(&self.out_dir, &self.name, "sk", &secret_to_b64(&sk))?;
    export(&self.out_dir, &self.name, "pk", &public_to_b64(&pk))?;

    Ok(())
  }
}

cmd! {
  KeyIntoTendermintArgs(self) {
    let sk = read_secret_key(&self.secret_key)?;
    let pk = PublicKey::from_secret_key(&sk);
    let vk = tendermint::crypto::default::ecdsa_secp256k1::VerifyingKey::from_sec1_bytes(&pk.serialize())
      .map_err(|e| anyhow!("failed to convert public key: {e}"))?;
    let pub_key = tendermint::PublicKey::Secp256k1(vk);
    let address = tendermint::account::Id::from(pub_key);

    // tendermint-rs doesn't seem to handle Secp256k1 private keys;
    // if it did, we could use tendermint_config::PrivateValidatorKey
    // to encode the data structure. Tendermint should be okay with it
    // though, as long as we match the expected keys in the JSON.
    let priv_validator_key = json! ({
        "address": address,
        "pub_key": pub_key,
        "priv_key": {
            "type": "tendermint/PrivKeySecp256k1",
            "value": secret_to_b64(&sk)
        }
    });
    let json = serde_json::to_string_pretty(&priv_validator_key)?;

    std::fs::write(&self.out, json)?;

    Ok(())
  }
}

cmd! {
  KeyAddressArgs(self) {
    let pk = read_public_key(&self.public_key)?;
    let addr = Address::from_raw_public_key(&pk.serialize());
    // let addr = Address::new_secp256k1(&pk.serialize())?;
    println!("{}", addr);
    Ok(())
  }
}

#[allow(dead_code, unreachable_pub)]
fn secret_to_b64(sk: &SecretKey) -> String {
    to_b64(&sk.serialize())
}
#[allow(dead_code, unreachable_pub)]
fn public_to_b64(pk: &PublicKey) -> String {
    to_b64(&pk.serialize_compressed())
}
#[allow(dead_code, unreachable_pub)]
fn b64_to_public(b64: &str) -> anyhow::Result<PublicKey> {
    let json = json!(b64);
    let pk: PublicKey = serde_json::from_value(json)?;
    Ok(pk)
}
#[allow(dead_code, unreachable_pub)]
fn b64_to_secret(b64: &str) -> anyhow::Result<SecretKey> {
    let bz = from_b64(b64)?;
    let sk = SecretKey::parse_slice(&bz)?;
    Ok(sk)
}
#[allow(dead_code, unreachable_pub)]
pub fn read_public_key(public_key: &PathBuf) -> anyhow::Result<PublicKey> {
    let b64 = std::fs::read_to_string(public_key).context("failed to read public key")?;
    let pk = b64_to_public(&b64).context("failed to parse public key")?;
    Ok(pk)
}
#[allow(dead_code, unreachable_pub)]
pub fn read_secret_key(secret_key: &PathBuf) -> anyhow::Result<SecretKey> {
    let b64 = std::fs::read_to_string(secret_key).context("failed to read secret key")?;
    let sk = b64_to_secret(&b64).context("failed to parse secret key")?;
    Ok(sk)
}
#[allow(dead_code, unreachable_pub)]
fn export(output_dir: &Path, name: &str, ext: &str, b64: &str) -> anyhow::Result<()> {
    let output_path = output_dir.join(format!("{name}.{ext}"));
    std::fs::write(output_path, b64)?;
    Ok(())
}

// #[cfg(test)]
// mod tests {
//     use fendermint_vm_genesis::ValidatorKey;
//     use quickcheck_macros::quickcheck;
//
//     use crate::cmd::key::b64_to_public;
//
//     use super::public_to_b64;
//
//     #[quickcheck]
//     fn prop_public_key_deserialize_to_genesis(vk: ValidatorKey) {
//         let b64 = public_to_b64(&vk.0);
//         let pk = b64_to_public(&b64).unwrap();
//         assert_eq!(pk, vk.0)
//     }
// }
