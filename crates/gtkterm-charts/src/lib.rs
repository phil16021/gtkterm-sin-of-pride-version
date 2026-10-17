//! Chart models and cairo renderers for the Sunshine visualization.
#![forbid(unsafe_code)]

pub mod model;
pub mod palette;
#[cfg(feature = "render")]
pub mod render;
pub mod scale;
