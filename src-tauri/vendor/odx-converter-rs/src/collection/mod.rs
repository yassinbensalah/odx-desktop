// src/collection/mod.rs
pub mod odx_collection;
pub mod odx_collection_group;
pub mod xml_parser;

pub use odx_collection::OdxCollection;
pub use odx_collection_group::OdxCollectionGroup;
pub use xml_parser::parse_odx_file;
