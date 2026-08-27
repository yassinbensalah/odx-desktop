// src/model/diag_layer.rs – DIAGLAYERCONTAINER and all DIAGLAYER subtypes.

use crate::model::odx::{OdxLink, LongName, Sdgs};
use crate::model::dop::DiagDataDictionarySpec;
use crate::model::comparam::{ComParamRef, ProtStack};
use crate::model::ComParam;
use crate::model::ComplexComParam;
use crate::model::DataObjectProp;
use crate::model::UnitSpec;
// ─── DIAG-LAYER-CONTAINER ─────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct DiagLayerContainer {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub sdgs: Option<Sdgs>,
    /// IDs of contained variants/layers (data lives in `OdxCollection`).
    pub ecu_variant_ids: Vec<String>,
    pub base_variant_ids: Vec<String>,
    pub protocol_ids: Vec<String>,
    pub functional_group_ids: Vec<String>,
    pub ecu_shared_data_ids: Vec<String>,
}

// ─── Shared DIAGLAYER fields ──────────────────────────────────────────────

/// Fields common to all DIAG-LAYER subtypes.
#[derive(Debug, Default)]
pub struct DiagLayerCore {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub sdgs: Option<Sdgs>,

    /// IDs of diagnostic services / jobs owned by this layer.
    /// Actual data lives in `OdxCollection.*_store`.
    pub diag_comms: DiagComms,
    /// Whether the DIAG-COMMS wrapper exists, even when it contains no jobs or services.
    pub diag_comms_present: bool,

    pub diag_data_dictionary_spec: Option<DiagDataDictionarySpec>,

    pub funct_classes: Vec<String>, // short-names
    pub additional_audiences: Vec<String>, // IDs
    pub state_chart_ids: Vec<String>, // IDs
    pub parent_refs: Vec<ParentRef>,
}

/// The mixed DIAG-COMMS element: stores IDs of services; full data is in
/// `OdxCollection` to avoid cloning / double-ownership.
#[derive(Debug, Default)]
pub struct DiagComms {
    /// IDs of inline DIAG-SERVICE elements.
    pub diag_service_ids: Vec<String>,
    /// IDs of inline SINGLE-ECU-JOB elements.
    pub single_ecu_job_ids: Vec<String>,
    /// ODXLINK references to services defined in parent layers.
    pub odx_links: Vec<OdxLink>,
}

// ─── PARENT-REF ───────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ParentRef {
    pub id_ref: String,
    pub doc_ref: Option<String>,
    pub doc_type: Option<ParentRefDocType>,

    /// Short names of services excluded from inheritance.
    pub not_inherited_diag_comm_short_names: Vec<String>,
    pub not_inherited_dop_short_names: Vec<String>,
    pub not_inherited_table_short_names: Vec<String>,
    pub not_inherited_variables_short_names: Vec<String>,
    pub not_inherited_global_neg_response_short_names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentRefDocType {
    EcuVariant,
    BaseVariant,
    Protocol,
    FunctionalGroup,
    EcuSharedData,
    ComParamSubset,
}

// ─── ECU-VARIANT ──────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct EcuVariant {
    pub core: DiagLayerCore,
    pub comparam_refs: Vec<ComParamRef>,
    pub ecu_variant_patterns: Vec<EcuVariantPattern>,
}

/// Pattern that uniquely identifies an ECU variant.
#[derive(Debug, Default)]
pub struct EcuVariantPattern {
    pub matching_parameters: Vec<MatchingParameter>,
}

#[derive(Debug)]
pub struct MatchingParameter {
    pub diag_comm_snref: String,
    pub expected_value: Option<String>,
    pub out_param_if_snref: Option<String>,
    pub use_physical_addressing: bool,
}

// ─── BASE-VARIANT ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct BaseVariant {
    pub core: DiagLayerCore,
    pub comparam_refs: Vec<ComParamRef>,
    pub base_variant_pattern: Option<BaseVariantPattern>,
}

#[derive(Debug, Default)]
pub struct BaseVariantPattern {
    pub matching_parameters: Vec<MatchingBaseVariantParameter>,
}

#[derive(Debug)]
pub struct MatchingBaseVariantParameter {
    pub diag_comm_snref: String,
    pub expected_value: String,
    pub out_param_if_snref: Option<String>,
    pub use_physical_addressing: bool,
}

// ─── PROTOCOL ─────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct Protocol {
    pub core: DiagLayerCore,
    pub comparam_refs: Vec<ComParamRef>,
    pub comparam_spec_ref: Option<OdxLink>,
    pub prot_stack_snref: Option<String>,
}

// ─── FUNCTIONAL-GROUP ─────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct FunctionalGroup {
    pub core: DiagLayerCore,
    pub comparam_refs: Vec<ComParamRef>,
}

// ─── ECU-SHARED-DATA ──────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct EcuSharedData {
    pub core: DiagLayerCore,
}

// ─── COMPARAM-SUBSET (top-level container) ────────────────────────────────

#[derive(Debug, Default)]
pub struct ComParamSubset {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,

    pub comparams: Vec<ComParam>,
    pub complex_comparams: Vec<ComplexComParam>,
    pub data_object_props: Vec<DataObjectProp>,
    pub unit_spec: Option<UnitSpec>,
}

// ─── COMPARAM-SPEC (top-level container) ──────────────────────────────────

#[derive(Debug, Default)]
pub struct ComParamSpec {
    pub id: String,
    pub short_name: String,
    pub prot_stacks: Vec<ProtStack>,
}
