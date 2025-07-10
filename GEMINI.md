# Gemini Project Notes for tace.chat

This file contains project-specific information and commands for the Gemini agent to reference.

## Build Commands

*   **Build all Rust crates:** `cargo build`
*   **Build specific Rust crate (e.g., tace_node):** `cargo build --package tace_node`
*   **Build webclient (WASM):** `wasm-pack build webclient --target web --release`

## Run Commands

*   **Run tace_node:** `cargo run --package tace_node`
*   **Run webclient (via Docker Compose):** `docker-compose up --build` (access at `http://localhost:8000`)

## Project Structure

*   **Cargo Workspace:** The project is a Cargo workspace.
*   **Rust Crates:**
    *   `lib/` (package: `tace_lib`)
    *   `node/` (package: `tace_node`)
    *   `webclient/` (package: `tace_webclient`)
*   **Web Client Serving:** The web client is served using Nginx via Docker Compose. Static files are in `public/` and `webclient/pkg/`.

## Common Issues & Resolutions

*   **Docker Build - `failed to read Cargo.toml`:** Ensure all workspace members (e.g., `node/`) are copied into the Docker build context.
*   **Nginx MIME Type Issues:** If the browser downloads files or fails to load module scripts due to incorrect MIME types, check `nginx.conf` and explicitly define `types` for `.wasm`, `.js`, and `.html` files.
