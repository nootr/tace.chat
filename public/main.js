import init from './wisp_webclient.js';

async function run() {
  await init();
  // You can call functions from your Rust WASM module here
  // For example: init().then(module => module.greet("World"));
  console.log("Wisp WebClient loaded!");
}
run();