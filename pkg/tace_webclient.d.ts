/* tslint:disable */
/* eslint-disable */
export function generate_keypair(): JsKeypair;
export function encrypt(my_private_key_hex: string, their_public_key_hex: string, plaintext: string): EncryptedMessage;
export function decrypt(my_private_key_hex: string, their_public_key_hex: string, ciphertext: Uint8Array, nonce: Uint8Array): string;
export function sign(private_key_hex: string, message: Uint8Array): Uint8Array;
export function main_js(): void;
export class EncryptedMessage {
  private constructor();
  free(): void;
  ciphertext: Uint8Array;
  nonce: Uint8Array;
}
export class JsKeypair {
  private constructor();
  free(): void;
  private_key: string;
  public_key: string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly __wbg_jskeypair_free: (a: number, b: number) => void;
  readonly __wbg_get_jskeypair_private_key: (a: number) => [number, number];
  readonly __wbg_get_jskeypair_public_key: (a: number) => [number, number];
  readonly generate_keypair: () => number;
  readonly __wbg_encryptedmessage_free: (a: number, b: number) => void;
  readonly __wbg_get_encryptedmessage_ciphertext: (a: number) => [number, number];
  readonly __wbg_set_encryptedmessage_ciphertext: (a: number, b: number, c: number) => void;
  readonly __wbg_get_encryptedmessage_nonce: (a: number) => [number, number];
  readonly __wbg_set_encryptedmessage_nonce: (a: number, b: number, c: number) => void;
  readonly encrypt: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number];
  readonly decrypt: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number, number, number];
  readonly sign: (a: number, b: number, c: number, d: number) => [number, number, number, number];
  readonly main_js: () => void;
  readonly __wbg_set_jskeypair_private_key: (a: number, b: number, c: number) => void;
  readonly __wbg_set_jskeypair_public_key: (a: number, b: number, c: number) => void;
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly __externref_table_alloc: () => number;
  readonly __wbindgen_export_2: WebAssembly.Table;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  readonly __externref_table_dealloc: (a: number) => void;
  readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;
/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
*
* @returns {InitOutput}
*/
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
