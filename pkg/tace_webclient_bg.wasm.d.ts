/* tslint:disable */
/* eslint-disable */
export const memory: WebAssembly.Memory;
export const __wbg_jskeypair_free: (a: number, b: number) => void;
export const __wbg_get_jskeypair_private_key: (a: number) => [number, number];
export const __wbg_get_jskeypair_public_key: (a: number) => [number, number];
export const generate_keypair: () => number;
export const __wbg_encryptedmessage_free: (a: number, b: number) => void;
export const __wbg_get_encryptedmessage_ciphertext: (a: number) => [number, number];
export const __wbg_set_encryptedmessage_ciphertext: (a: number, b: number, c: number) => void;
export const __wbg_get_encryptedmessage_nonce: (a: number) => [number, number];
export const __wbg_set_encryptedmessage_nonce: (a: number, b: number, c: number) => void;
export const encrypt: (a: number, b: number, c: number, d: number, e: number, f: number) => [number, number, number];
export const decrypt: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => [number, number, number, number];
export const sign: (a: number, b: number, c: number, d: number) => [number, number, number, number];
export const main_js: () => void;
export const __wbg_set_jskeypair_private_key: (a: number, b: number, c: number) => void;
export const __wbg_set_jskeypair_public_key: (a: number, b: number, c: number) => void;
export const __wbindgen_exn_store: (a: number) => void;
export const __externref_table_alloc: () => number;
export const __wbindgen_export_2: WebAssembly.Table;
export const __wbindgen_free: (a: number, b: number, c: number) => void;
export const __wbindgen_malloc: (a: number, b: number) => number;
export const __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
export const __externref_table_dealloc: (a: number) => void;
export const __wbindgen_start: () => void;
