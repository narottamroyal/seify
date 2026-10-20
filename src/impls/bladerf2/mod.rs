//! bladeRF 2.0 micro backend.

mod common;

#[cfg(any(target_arch = "wasm32", feature = "smol", feature = "tokio"))]
mod asynchronous;
#[cfg(any(target_arch = "wasm32", feature = "smol", feature = "tokio"))]
pub use asynchronous::{AsyncBladeRf2, AsyncBladeRf2RxStreamer, AsyncBladeRf2TxStreamer};

#[cfg(not(target_arch = "wasm32"))]
mod sync;
#[cfg(not(target_arch = "wasm32"))]
pub use sync::{BladeRf2, RxStreamer, TxStreamer};
