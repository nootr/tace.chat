# Wisp: WhatsApp-Web-like P2P Messaging App (WASM + Rust + Chord DHT)

A lightweight, privacy-focused messaging application inspired by WhatsApp Web.

## 🧠 Overview

This project consists of:

* A **static web client (SPA)** written in Rust, compiled to WebAssembly (WASM).
* A **distributed P2P network** using a **Chord DHT** implementation in Rust.
* A shared **Rust library** for cryptographic logic and message handling, used by both frontend and backend.
* All messages are **end-to-end encrypted** and **expire after 24 hours**.
* The architecture is optimized for **small binary size** and **low memory usage**.

---

## 📦 Project Structure

```
project-root/
├── lib/              # Shared Rust library (crypto, message types, utils)
├── node/             # Lightweight P2P node binary
├── webclient/        # SPA client (Rust + WASM)
├── bin/run           # Runs both SPA and test nodes locally
└── INSTRUCTIONS.md   # This file
```

---

## ✅ Requirements

* Written entirely in **Rust** (both node and frontend via WASM)
* Very small binary + minimal memory usage
* Fully testable with unit tests at each step
* Avoid unnecessary comments
* Use **early returns** to reduce nesting
* One **hardcoded bootstrap node** in SPA
* Messages use **TTL = 24h**; after expiry, **NACK** is sent back to sender
* Sender **polls for NACKs** the same way recipients poll for messages

---

## 🧱 Incremental Development Steps

### 1. **Setup Project Structure**

* [x] Initialize Cargo workspace with `lib`, `node`, and `webclient`
* [x] Setup shared crate in lib/ for:
  * Message struct definitions
  * Cryptography helpers (encryption, signing)
  * DHT protocol messages
* [x] Setup node/ crate as a binary

### 2. **Minimal Chord DHT Node**

* [x] Implement barebones Chord DHT node:

  * Join network via bootstrap node (env variable)
  * Start new network if no bootstrap provided
* [x] Minimal network operations (store/retrieve keys)

### 3. **Local Test Harness**

* [x] Add a script to spin up N local nodes (e.g., `bin/run`)
* [x] Use local ports and logs for testing
* [x] Ensure nodes discover each other and stabilize the ring

### 4. **Messaging Endpoints**

* [x] Add DHT message handlers to the node:
  * `Store` — Accepts encrypted message from client
  * `Retrieve` — Polls for new messages or NACKs
* [x] Messages are stored in the DHT with TTL metadata

### 5. **Message Expiry + NACK Handling**

* [ ] Implement background task for pruning expired messages
* [ ] On expiry, trigger NACK message to sender node
* [ ] Polling client can check for NACKs using `Retrieve`

### 6. **WebClient (SPA) Setup**

* [x] Rust + WASM + Tailwind or minimal UI
* [x] Use `wasm-bindgen` and `web-sys` for DOM bindings
* [x] Integrate shared `lib/` for encryption/decryption
* [x] Compile to `pkg/` and serve via `bin/run`

### 7. **Send/Receive Messages in WebClient**

* [ ] Encrypt messages using public key of recipient
* [ ] Send via `Store` message
* [ ] Poll for incoming messages and NACKs via `Retrieve`
* [ ] Decrypt and display messages

### 8. **Dev Webserver and Bootstrap Node**

* [x] Expand `bin/run`:
  * Serves the compiled `webclient` as static assets
  * Starts 1 or more P2P nodes (optional for local testing)

---

## 🧪 Testing Strategy

* Unit tests for:

  * Message serialization/deserialization
  * Encryption/decryption
  * Chord DHT ring logic (joining, storing, retrieving)
* Integration tests using `mockall` to mock network clients.
* End-to-end: send message → store in DHT → retrieve/decrypt

---

## 🔐 Cryptography

* All messages are **end-to-end encrypted**:

  * **Sender encrypts with recipient’s public key**
  * **Recipient decrypts with private key**
* Use `x25519` for key exchange
* Use `chacha20poly1305` for symmetric encryption
* Use `ed25519` for signing

---

## 🌐 P2P Design Notes

* Each node runs a small TCP server
* All routing happens via Chord DHT
* Node keeps only routing table + recent messages
* Messages are not stored permanently — TTL enforced

---

## 🔄 Polling Model

* Clients **poll** their connected node for new messages every X seconds
* Same polling is used to check for **NACKs** (messages that expired undelivered)

---

## 🧼 Goals & Philosophy

* **Minimalism**: Every component should be as simple and lightweight as possible
* **Security by Design**: Messages encrypted and signed by default
* **Testability**: Each step should be small and verifiable with tests
* **No comments unless essential**: Prefer expressive code and clear naming
* **Avoid nested code**: Use early returns

---

Let me know if you want a real `Cargo.toml` + project bootstrapped, and I’ll generate the files too.