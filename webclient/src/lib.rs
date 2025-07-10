#[wasm_bindgen]
pub fn webclient_test() -> String {
    "Hello from tace_webclient!".to_string()
}

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn main_js() -> Result<(), JsValue> {
    // This is a placeholder for the webclient's main entry point.
    // In a real application, you would initialize your UI here.
    web_sys::console::log_1(&JsValue::from_str(&webclient_test()));
    Ok(())
}
