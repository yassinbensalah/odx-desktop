// src/lib.rs – Library root.

pub mod build_info;
pub mod collection;
pub mod converter;
pub mod error;
pub mod model;
pub mod options;
pub mod parser;
pub mod plugin;
pub mod timestamp;
pub mod writer;

// Re-export the prost-generated protobuf types at crate::mdd.
pub mod mdd {
    pub mod fileformat {
        include!(concat!(env!("OUT_DIR"), "/fileformat.rs"));
    }
}

pub use converter::{Converter, FileConverter, ConversionStats};
pub use error::OdxError;
pub use options::{ConverterOptions, CompressionConfig};
pub use plugin::PluginChain;
