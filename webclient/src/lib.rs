use tace_lib::keys;
use wasm_bindgen::prelude::*;

// --- Key Generation ---

#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct JsKeypair {
    #[wasm_bindgen(getter_with_clone)]
    pub private_key: String,
    #[wasm_bindgen(getter_with_clone)]
    pub public_key: String,
}

#[wasm_bindgen]
pub fn generate_keypair() -> JsKeypair {
    let keypair = keys::generate_keypair();
    JsKeypair {
        private_key: keypair.private_key,
        public_key: keypair.public_key,
    }
}

// --- Encryption & Decryption ---

#[wasm_bindgen]
pub struct EncryptedMessage {
    #[wasm_bindgen(getter_with_clone)]
    pub ciphertext: Vec<u8>,
    #[wasm_bindgen(getter_with_clone)]
    pub nonce: Vec<u8>,
}

#[wasm_bindgen]
pub fn encrypt(
    my_private_key_hex: &str,
    their_public_key_hex: &str,
    plaintext: &str,
) -> Result<EncryptedMessage, JsValue> {
    match keys::encrypt(
        my_private_key_hex,
        their_public_key_hex,
        plaintext.as_bytes(),
    ) {
        Ok((ciphertext, nonce)) => Ok(EncryptedMessage { ciphertext, nonce }),
        Err(e) => Err(JsValue::from_str(&e)),
    }
}

#[wasm_bindgen]
pub fn decrypt(
    my_private_key_hex: &str,
    their_public_key_hex: &str,
    ciphertext: &[u8],
    nonce: &[u8],
) -> Result<String, JsValue> {
    match keys::decrypt(my_private_key_hex, their_public_key_hex, ciphertext, nonce) {
        Ok(plaintext_bytes) => Ok(String::from_utf8_lossy(&plaintext_bytes).to_string()),
        Err(e) => Err(JsValue::from_str(&e)),
    }
}

#[wasm_bindgen(start)]
pub fn main_js() -> Result<(), JsValue> {
    // This function is called when the WASM module is loaded.
    // It's a good place for initialization if needed.
    web_sys::console::log_1(&JsValue::from_str("tace_webclient with encryption loaded."));
    Ok(())
}
