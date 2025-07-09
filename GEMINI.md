# Gemini Project Notes for Wisp

This file contains project-specific information and commands for the Gemini agent to reference.

## Build Commands

*   **Build all Rust crates:** `cargo build`
*   **Build specific Rust crate (e.g., wisp_node):** `cargo build --package wisp_node`
*   **Build webclient (WASM):** `wasm-pack build webclient --target web --release`

## Run Commands

*   **Run wisp_node:** `cargo run --package wisp_node`
*   **Run webclient (via Docker Compose):** `docker-compose up --build` (access at `http://localhost:8000`)

## Project Structure

*   **Cargo Workspace:** The project is a Cargo workspace.
*   **Rust Crates:**
    *   `lib/` (package: `wisp_lib`)
    *   `node/` (package: `wisp_node`)
    *   `webclient/` (package: `wisp_webclient`)
*   **Web Client Serving:** The web client is served using Nginx via Docker Compose. Static files are in `public/` and `webclient/pkg/`.

## Common Issues & Resolutions

*   **Docker Build - `failed to read Cargo.toml`:** Ensure all workspace members (e.g., `node/`) are copied into the Docker build context.
*   **Nginx MIME Type Issues:** If the browser downloads files or fails to load module scripts due to incorrect MIME types, check `nginx.conf` and explicitly define `types` for `.wasm`, `.js`, and `.html` files.
