//! Bounded image resources and terminal graphics. Expensive third-party work is
//! executed only in the owned worker process, never a layout callback.
//!
//! Foundation APIs: the CLI does not yet discover/load current document images
//! or attach Resources/Scene to its native frame. Its optional `--images` mode
//! only probes terminal capabilities; the default retains clickable alt text.
pub mod bytes;
pub mod worker;
pub const MAX_URI: usize = 192 * 1024;
pub const MAX_ENCODED: usize = 8 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;
pub const MAX_CANVAS_PIXELS: usize = 1024 * 1024;
pub const MAX_CANVAS: usize = MAX_CANVAS_PIXELS * 4;
pub const CACHE_BYTES: usize = 32 * 1024 * 1024;

pub mod cache;

pub mod input;

pub mod protocol;

pub mod session;

pub mod layout;

pub mod resources;

pub mod scene;
