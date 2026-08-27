// src/model/odx.rs – Top-level ODX envelope and shared primitive types.
//
// Each `.odx` entry inside a `.pdx` archive maps to one `OdxFile`.
// The top-level element can be one of several container types.

use std::collections::HashMap;

// ─── Cross-file reference ──────────────────────────────────────────────────

/// ODXLINK – an `ID-REF` / `DOC-REF` pair that points to an element in
/// (possibly another) ODX file.
#[derive(Debug, Clone, Default)]
pub struct OdxLink {
    pub id_ref: String,
    pub doc_ref: Option<String>,
    pub doc_type: Option<String>,
}

impl OdxLink {
    pub fn new(id_ref: impl Into<String>) -> Self {
        Self {
            id_ref: id_ref.into(),
            doc_ref: None,
            doc_type: None,
        }
    }

    pub fn with_doc_ref(mut self, doc_ref: impl Into<String>) -> Self {
        self.doc_ref = Some(doc_ref.into());
        self
    }
}

// ─── Short-name reference ─────────────────────────────────────────────────

/// SNREF – short-name-based reference (local scope only).
#[derive(Debug, Clone)]
pub struct SnRef {
    pub short_name: String,
}

/// SNPATHREF – dotted short-name path reference.
#[derive(Debug, Clone)]
pub struct SnPathRef {
    pub short_name_path: String,
}

// ─── Localised text (SD, SDG) ─────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct Sd {
    pub value: Option<String>,
    pub si: Option<String>,
    pub ti: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SdOrSdg {
    Sd(Sd),
    Sdg(Sdg),
}

#[derive(Debug, Clone, Default)]
pub struct Sdg {
    pub caption_sn: Option<String>,
    pub si: Option<String>,
    pub items: Vec<SdOrSdg>,
}

#[derive(Debug, Clone, Default)]
pub struct Sdgs {
    pub sdg: Vec<Sdg>,
}

/// Long name with optional translation identifier.
#[derive(Debug, Clone)]
pub struct LongName {
    pub value: Option<String>,
    pub ti: Option<String>,
}

/// Generic text with optional translation identifier.
#[derive(Debug, Clone)]
pub struct Text {
    pub value: Option<String>,
    pub ti: Option<String>,
}

// ─── Audience ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct AdditionalAudience {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
}

#[derive(Debug, Clone, Default)]
pub struct Audience {
    pub enabled_audience_refs: Vec<OdxLink>,
    pub disabled_audience_refs: Vec<OdxLink>,
    pub is_supplier: bool,
    pub is_development: bool,
    pub is_manufacturing: bool,
    pub is_after_sales: bool,
    pub is_after_market: bool,
}

// ─── Functional class ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FunctClass {
    pub id: String,
    pub short_name: String,
}

// ─── Top-level ODX file container ─────────────────────────────────────────

use crate::model::diag_layer::{DiagLayerContainer, ComParamSubset, ComParamSpec};

/// Parsed content of one `.odx` file inside a `.pdx` archive.
#[derive(Debug)]
pub struct OdxFile {
    /// The filename (relative path inside the ZIP) this was parsed from.
    pub source_file: String,

    /// Top-level container variant present in this file.
    pub content: OdxContent,
}

/// One `.odx` file contains exactly one top-level container.
#[derive(Debug)]
pub enum OdxContent {
    DiagLayerContainer(DiagLayerContainer),
    ComParamSubset(ComParamSubset),
    ComParamSpec(ComParamSpec),
    /// Other / not-yet-supported container types.
    Other { short_name: String },
}

impl OdxContent {
    /// Returns the short-name of the top-level container.
    pub fn container_key(&self) -> &str {
        match self {
            OdxContent::DiagLayerContainer(c) => &c.short_name,
            OdxContent::ComParamSubset(c) => &c.short_name,
            OdxContent::ComParamSpec(c) => &c.short_name,
            OdxContent::Other { short_name } => short_name,
        }
    }
}
