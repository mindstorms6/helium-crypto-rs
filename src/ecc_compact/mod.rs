use crate::{error, keypair, public_key, IntoBytes, KeyTag, KeyType, Network};
use p256::{
    ecdsa,
    elliptic_curve::{sec1::ToCompactEncodedPoint, weierstrass::DecompactPoint},
    FieldBytes,
};
use std::convert::TryFrom;

#[derive(Debug, PartialEq, Clone)]
pub struct PublicKey(p256::PublicKey);

#[derive(Debug, PartialEq, Clone)]
pub struct Signature(ecdsa::Signature);

pub type Keypair = keypair::Keypair<p256::SecretKey>;

pub const KEYPAIR_LENGTH: usize = 33;

impl keypair::Sign for Keypair {
    fn sign(&self, msg: &[u8]) -> error::Result<Vec<u8>> {
        use signature::Signer;
        let signature = self.try_sign(msg)?;
        Ok(signature.0.to_der().as_bytes().to_vec())
    }
}

impl TryFrom<&[u8]> for Keypair {
    type Error = error::Error;
    fn try_from(input: &[u8]) -> error::Result<Self> {
        let network = Network::try_from(input[0])?;
        let inner = p256::SecretKey::from_bytes(&input[1..])?;
        let public_key = public_key::PublicKey::for_network(network, PublicKey(inner.public_key()));
        Ok(Keypair {
            network,
            public_key,
            inner,
        })
    }
}

impl IntoBytes for Keypair {
    fn bytes_into(&self, output: &mut [u8]) {
        output[0] = u8::from(KeyTag {
            network: self.network,
            key_type: KeyType::EccCompact,
        });
        output[1..].copy_from_slice(&self.inner.to_bytes());
    }
}

impl Keypair {
    pub fn generate<R>(network: Network, csprng: &mut R) -> Keypair
    where
        R: rand_core::CryptoRng + rand_core::RngCore,
    {
        let mut inner = p256::SecretKey::random(&mut *csprng);
        let mut public_key = inner.public_key();
        while !bool::from(public_key.as_affine().is_compactable()) {
            inner = p256::SecretKey::random(&mut *csprng);
            public_key = inner.public_key();
        }
        Keypair {
            network,
            public_key: public_key::PublicKey::for_network(network, PublicKey(public_key)),
            inner,
        }
    }

    pub fn generate_from_entropy(network: Network, entropy: &[u8]) -> error::Result<Keypair> {
        let inner = p256::SecretKey::from_bytes(entropy)?;
        let public_key = inner.public_key();
        if !bool::from(public_key.as_affine().is_compactable()) {
            return Err(error::not_compact());
        }
        Ok(Keypair {
            network,
            public_key: public_key::PublicKey::for_network(network, PublicKey(public_key)),
            inner,
        })
    }

    pub fn to_bytes(&self) -> [u8; KEYPAIR_LENGTH] {
        let mut result = [0u8; KEYPAIR_LENGTH];
        self.bytes_into(&mut result);
        result
    }
}

impl signature::Signature for Signature {
    fn from_bytes(input: &[u8]) -> std::result::Result<Self, signature::Error> {
        Ok(Signature(signature::Signature::from_bytes(input)?))
    }

    fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl AsRef<[u8]> for Signature {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl signature::Signer<Signature> for Keypair {
    fn try_sign(&self, msg: &[u8]) -> std::result::Result<Signature, signature::Error> {
        // TODO: Thre has to be a way to avoid cloning for every signature?
        Ok(Signature(
            p256::ecdsa::SigningKey::from(self.inner.clone()).sign(msg),
        ))
    }
}

impl public_key::Verify for PublicKey {
    fn verify(&self, msg: &[u8], signature: &[u8]) -> error::Result {
        use signature::Verifier;
        let signature = p256::ecdsa::Signature::from_der(signature).map_err(error::Error::from)?;
        Ok(p256::ecdsa::VerifyingKey::from(self.0).verify(msg, &signature)?)
    }
}

impl TryFrom<&[u8]> for PublicKey {
    type Error = error::Error;

    fn try_from(input: &[u8]) -> error::Result<Self> {
        match p256::AffinePoint::decompact(&FieldBytes::from_slice(&input[1..])).into() {
            Some(point) => Ok(PublicKey(
                p256::PublicKey::from_affine(point).map_err(error::Error::from)?,
            )),
            None => Err(error::not_compact()),
        }
    }
}

impl IntoBytes for PublicKey {
    fn bytes_into(&self, output: &mut [u8]) {
        let encoded = self
            .0
            .as_affine()
            .to_compact_encoded_point()
            .expect("compact point");
        output.copy_from_slice(&encoded.as_bytes()[1..])
    }
}

#[cfg(test)]
mod tests {
    use super::{Keypair, PublicKey, TryFrom};
    use crate::{Network, Sign, Verify};
    use hex_literal::hex;
    use rand::rngs::OsRng;

    #[test]
    fn sign_roundtrip() {
        let keypair = Keypair::generate(Network::MainNet, &mut OsRng);
        let signature = keypair.sign(b"hello world").expect("signature");
        assert!(keypair
            .public_key
            .verify(b"hello world", &signature)
            .is_ok())
    }

    #[test]
    fn bytes_roundtrip() {
        use rand::rngs::OsRng;
        let keypair = Keypair::generate(Network::MainNet, &mut OsRng);
        let bytes = keypair.to_bytes();
        assert_eq!(
            keypair,
            super::Keypair::try_from(&bytes[..]).expect("keypair")
        );
    }

    #[test]
    fn verify() {
        // Test a msg signed and verified with a keypair generated with erlang libp2p_crypto
        const MSG: &[u8] = b"hello world";
        const PUBKEY: &str = "11nYr7TBMbpGiQadiCxGCPZFZ8ENo1JNtbS7aB5U7UXn4a8Dvb3";
        const SIG: &[u8] =
            &hex!("304402206d791eb96bcc7d0ef403bc7a653fd99a6906374ec9e4aff1d5907d4890e8dd3302204b4c93c7637b22565b944201df9c806d684165802b8a1cd91d4d7799c950e466");

        let public_key: crate::PublicKey = PUBKEY.parse().expect("b58 public key");
        assert!(public_key.verify(MSG, SIG).is_ok());
    }

    #[test]
    fn b58_roundtrip() {
        const B58: &str = "112jXiCTi9DpLC5nLdSZ2zccRVEtZizRJMizziCebaNbRDi8k6wR";
        let decoded: crate::PublicKey = B58.parse().expect("b58 public key");
        assert_eq!(B58, decoded.to_string());
    }

    #[test]
    fn non_compact_key() {
        const NON_COMPACT_KEY: &[u8] =
            &hex!("003ca9d8667de0c07aa71d98b3c8065d2e97ab7bb9cb8776bcc0577a7ac58acd4e");
        assert!(PublicKey::try_from(NON_COMPACT_KEY).is_err());
    }
}
