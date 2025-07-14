use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use ecdsa::signature::Verifier;
use hkdf::Hkdf;
use p256::ecdsa::signature::Signer;
use p256::{
    ecdsa::{Signature, SigningKey, VerifyingKey},
    PublicKey, SecretKey,
};
use rand_core::OsRng;
use sha2::Sha256;

pub struct Keypair {
    pub private_key: String,
    pub public_key: String,
}

pub fn generate_keypair() -> Keypair {
    let private_key = SigningKey::random(&mut OsRng);
    let public_key = VerifyingKey::from(&private_key);

    let private_key_hex = hex::encode(private_key.to_bytes());
    let public_key_hex = hex::encode(public_key.to_sec1_bytes());

    Keypair {
        private_key: private_key_hex,
        public_key: public_key_hex,
    }
}

fn derive_shared_secret(private_key_hex: &str, public_key_hex: &str) -> Result<[u8; 32], String> {
    let private_bytes = hex::decode(private_key_hex).map_err(|e| e.to_string())?;
    let public_bytes = hex::decode(public_key_hex).map_err(|e| e.to_string())?;

    let secret_key = SecretKey::from_slice(&private_bytes).map_err(|e| e.to_string())?;
    let public_key = PublicKey::from_sec1_bytes(&public_bytes).map_err(|e| e.to_string())?;

    let shared_secret =
        p256::ecdh::diffie_hellman(secret_key.to_nonzero_scalar(), public_key.as_affine());

    let hk = Hkdf::<Sha256>::new(None, shared_secret.raw_secret_bytes().as_ref());
    let mut okm = [0u8; 32];
    hk.expand(&[], &mut okm).map_err(|e| e.to_string())?;

    Ok(okm)
}

pub fn encrypt(
    private_key_hex: &str,
    public_key_hex: &str,
    plaintext: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let key_material = derive_shared_secret(private_key_hex, public_key_hex)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_material);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; 12];
    rand_core::RngCore::fill_bytes(&mut OsRng, &mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| e.to_string())?;

    Ok((ciphertext, nonce_bytes.to_vec()))
}

pub fn decrypt(
    private_key_hex: &str,
    public_key_hex: &str,
    ciphertext: &[u8],
    nonce: &[u8],
) -> Result<Vec<u8>, String> {
    let key_material = derive_shared_secret(private_key_hex, public_key_hex)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_material);
    let cipher = Aes256Gcm::new(key);

    let nonce_slice = Nonce::from_slice(nonce);
    let plaintext = cipher
        .decrypt(nonce_slice, ciphertext)
        .map_err(|e| e.to_string())?;

    Ok(plaintext)
}

pub fn sign(private_key_hex: &str, message: &[u8]) -> Result<Vec<u8>, String> {
    let private_bytes = hex::decode(private_key_hex).map_err(|e| e.to_string())?;
    let signing_key = SigningKey::from_slice(&private_bytes).map_err(|e| e.to_string())?;
    let signature: Signature = signing_key.sign(message);
    Ok(signature.to_vec())
}

pub fn verify(public_key_hex: &str, message: &[u8], signature_bytes: &[u8]) -> bool {
    let public_bytes = match hex::decode(public_key_hex) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };
    let signature = match Signature::from_slice(signature_bytes) {
        Ok(sig) => sig,
        Err(_) => return false,
    };
    let verifying_key = match VerifyingKey::from_sec1_bytes(&public_bytes) {
        Ok(key) => key,
        Err(_) => return false,
    };

    verifying_key.verify(message, &signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        // 1. Generate two keypairs
        let sender_keys = generate_keypair();
        let receiver_keys = generate_keypair();

        // 2. Define a message
        let original_plaintext = "This is a secret message for testing.";

        // 3. Encrypt the message from sender to receiver
        let (ciphertext, nonce) = encrypt(
            &sender_keys.private_key,
            &receiver_keys.public_key,
            original_plaintext.as_bytes(),
        )
        .expect("Encryption failed");

        // 4. Decrypt the message by the receiver
        let decrypted_bytes = decrypt(
            &receiver_keys.private_key,
            &sender_keys.public_key,
            &ciphertext,
            &nonce,
        )
        .expect("Decryption failed");

        let decrypted_plaintext =
            String::from_utf8(decrypted_bytes).expect("Failed to convert bytes to string");

        // 5. Assert that the decrypted message matches the original
        assert_eq!(original_plaintext, decrypted_plaintext);
    }

    #[test]
    fn test_decrypt_fails_with_wrong_key() {
        let sender_keys = generate_keypair();
        let receiver_keys = generate_keypair();
        let malicious_keys = generate_keypair(); // A third party

        let original_plaintext = "Another secret message.";

        // Encrypt from sender to receiver
        let (ciphertext, nonce) = encrypt(
            &sender_keys.private_key,
            &receiver_keys.public_key,
            original_plaintext.as_bytes(),
        )
        .expect("Encryption failed");

        // Attempt to decrypt with the wrong key (malicious user)
        let result = decrypt(
            &malicious_keys.private_key, // Wrong private key
            &sender_keys.public_key,
            &ciphertext,
            &nonce,
        );

        // Assert that decryption failed
        assert!(
            result.is_err(),
            "Decryption should have failed with the wrong key, but it succeeded."
        );
    }

    #[test]
    fn test_sign_verify_roundtrip() {
        let keys = generate_keypair();
        let message = b"this is a test message";

        // Sign the message
        let signature = sign(&keys.private_key, message).expect("Signing failed");

        // Verify the signature
        let is_valid = verify(&keys.public_key, message, &signature);
        assert!(
            is_valid,
            "Signature verification failed for a valid signature."
        );
    }

    #[test]
    fn test_verify_fails_with_wrong_key() {
        let keys1 = generate_keypair();
        let keys2 = generate_keypair(); // Different key
        let message = b"another test message";

        // Sign with key1
        let signature = sign(&keys1.private_key, message).expect("Signing failed");

        // Try to verify with key2
        let is_valid = verify(&keys2.public_key, message, &signature);
        assert!(
            !is_valid,
            "Signature verification should have failed with the wrong public key."
        );
    }

    #[test]
    fn test_verify_fails_with_tampered_message() {
        let keys = generate_keypair();
        let original_message = b"original message";
        let tampered_message = b"tampered message";

        // Sign the original message
        let signature = sign(&keys.private_key, original_message).expect("Signing failed");

        // Try to verify with the tampered message
        let is_valid = verify(&keys.public_key, tampered_message, &signature);
        assert!(
            !is_valid,
            "Signature verification should have failed for a tampered message."
        );
    }
}
