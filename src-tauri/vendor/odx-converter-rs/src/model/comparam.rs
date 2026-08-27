// src/model/comparam.rs – Communication parameter types (COMPARAM, COMPLEXCOMPARAM,
// COMPARAMREF, COMPARAMSPEC, PROTSTACK, etc.).

use crate::model::odx::{OdxLink, LongName};

// ─── COMPARAM (simple) ────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ComParam {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub param_class: Option<String>,
    pub cp_type: Option<CpType>,
    pub cp_usage: Option<CpUsage>,
    pub display_level: Option<u32>,
    pub physical_default_value: Option<String>,
    pub data_object_prop_ref: Option<OdxLink>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpType {
    Standard,
    OemSpecific,
    Optional,
    OemOptional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpUsage {
    EcuSoftware,
    EcuComm,
    Application,
    Tester,
}

// ─── COMPLEXCOMPARAM ──────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ComplexComParam {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub param_class: Option<String>,
    pub cp_type: Option<CpType>,
    pub cp_usage: Option<CpUsage>,
    pub display_level: Option<u32>,
    pub allow_multiple_values: bool,
    pub sub_params: Vec<ComParamOrComplex>,
    pub complex_physical_default_values: Vec<ComplexValue>,
}

#[derive(Debug)]
pub enum ComParamOrComplex {
    Simple(ComParam),
    Complex(ComplexComParam),
}

// ─── SIMPLE-VALUE / COMPLEX-VALUE ─────────────────────────────────────────

#[derive(Debug)]
pub struct SimpleValue {
    pub value: Option<String>,
}

#[derive(Debug, Default)]
pub struct ComplexValue {
    pub entries: Vec<SimpleOrComplexEntry>,
}

#[derive(Debug)]
pub enum SimpleOrComplexEntry {
    Simple(SimpleValue),
    Complex(ComplexValue),
}

// ─── COMPARAM-REF ─────────────────────────────────────────────────────────

/// A use of a COMPARAM inside a DIAG-LAYER (with an optional value override).
#[derive(Debug)]
pub struct ComParamRef {
    pub id_ref: String,
    pub doc_ref: Option<String>,
    pub simple_value: Option<SimpleValue>,
    pub complex_value: Option<ComplexValue>,
    pub protocol_snref: Option<String>,
    pub prot_stack_snref: Option<String>,
}

// ─── PROT-STACK ───────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ProtStack {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub physical_link_type: Option<String>,
    pub pdu_protocol_type: Option<String>,
    pub comparam_subset_refs: Vec<OdxLink>,
}
