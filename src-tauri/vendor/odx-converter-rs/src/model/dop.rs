// src/model/dop.rs – Data Object Prop hierarchy, DiagDataDictionarySpec, TABLE, DTC, etc.
//
// Corresponds to: DATAOBJECTPROP, DTCDOP, STRUCTURE, STATICFIELD, ENDOFPDUFIELD,
// DYNAMICLENGTHFIELD, DYNAMICENDMARKERFIELD, MUX, ENVDATA, ENVDATADESC, TABLE, TABLEROW.

use crate::model::odx::{Audience, OdxLink, LongName, Sdgs};
use crate::model::compu_method::CompuMethod;
use crate::model::unit::Unit;
use crate::model::diag_service::Param;
use crate::model::state::{PreConditionStateRef, StateTransitionRef};

// ─── DOP base trait / enum ────────────────────────────────────────────────

/// Unified DOP type that covers all `DOPBASE` subclasses from the ODX schema.
#[derive(Debug)]
pub enum DopBase {
    DataObjectProp(DataObjectProp),
    DtcDop(DtcDop),
    Structure(Structure),
    StaticField(StaticField),
    EndOfPduField(EndOfPduField),
    DynamicLengthField(DynamicLengthField),
    DynamicEndMarkerField(DynamicEndMarkerField),
    Mux(Mux),
    EnvData(EnvData),
    EnvDataDesc(EnvDataDesc),
}

impl DopBase {
    pub fn id(&self) -> &str {
        match self {
            DopBase::DataObjectProp(d) => &d.id,
            DopBase::DtcDop(d) => &d.id,
            DopBase::Structure(d) => &d.id,
            DopBase::StaticField(d) => &d.id,
            DopBase::EndOfPduField(d) => &d.id,
            DopBase::DynamicLengthField(d) => &d.id,
            DopBase::DynamicEndMarkerField(d) => &d.id,
            DopBase::Mux(d) => &d.id,
            DopBase::EnvData(d) => &d.id,
            DopBase::EnvDataDesc(d) => &d.id,
        }
    }

    pub fn short_name(&self) -> &str {
        match self {
            DopBase::DataObjectProp(d) => &d.short_name,
            DopBase::DtcDop(d) => &d.short_name,
            DopBase::Structure(d) => &d.short_name,
            DopBase::StaticField(d) => &d.short_name,
            DopBase::EndOfPduField(d) => &d.short_name,
            DopBase::DynamicLengthField(d) => &d.short_name,
            DopBase::DynamicEndMarkerField(d) => &d.short_name,
            DopBase::Mux(d) => &d.short_name,
            DopBase::EnvData(d) => &d.short_name,
            DopBase::EnvDataDesc(d) => &d.short_name,
        }
    }
}

// ─── DIAG-DATA-DICTIONARY-SPEC ────────────────────────────────────────────

/// Holds IDs of its children; actual data lives in `OdxCollection.*_store`.
#[derive(Debug, Default)]
pub struct DiagDataDictionarySpec {
    pub data_object_prop_ids: Vec<String>,
    pub dtc_dop_ids: Vec<String>,
    pub structure_ids: Vec<String>,
    pub static_field_ids: Vec<String>,
    pub end_of_pdu_field_ids: Vec<String>,
    pub dynamic_length_field_ids: Vec<String>,
    pub mux_ids: Vec<String>,
    pub env_data_ids: Vec<String>,
    pub env_data_desc_ids: Vec<String>,
    pub table_ids: Vec<String>,
    pub unit_spec: Option<crate::model::unit::UnitSpec>,
    pub sdgs: Option<Sdgs>,
}

// ─── DATAOBJECTPROP ───────────────────────────────────────────────────────

#[derive(Debug)]
pub struct DataObjectProp {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub sdgs: Option<Sdgs>,
    pub diag_coded_type: Option<DiagCodedType>,
    pub physical_type: Option<PhysicalType>,
    pub compu_method: Option<CompuMethod>,
    pub unit_ref: Option<OdxLink>,
    pub internal_constr: Option<InternalConstr>,
    pub phys_constr: Option<InternalConstr>,
}

// ─── DTC-DOP ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct DtcDop {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub sdgs: Option<Sdgs>,
    pub diag_coded_type: DiagCodedType,
    pub physical_type: PhysicalType,
    pub compu_method: CompuMethod,
    pub dtcs: Vec<DtcOrRef>,
    pub is_visible: bool,
}

#[derive(Debug)]
pub enum DtcOrRef {
    Dtc(Dtc),
    OdxLink(OdxLink),
}

#[derive(Debug)]
pub struct Dtc {
    pub id: String,
    pub short_name: String,
    pub sdgs: Option<Sdgs>,
    pub trouble_code: u32,
    pub display_trouble_code: Option<String>,
    pub text: Option<crate::model::odx::Text>,
    pub level: Option<u32>,
    pub is_temporary: bool,
}

// ─── STRUCTURE ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Structure {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub sdgs: Option<Sdgs>,
    pub byte_size: Option<u32>,
    pub params: Vec<Param>,
    pub is_visible: bool,
}

// ─── FIELD (shared by StaticField, EndOfPduField) ─────────────────────────

#[derive(Debug, Default)]
pub struct Field {
    pub basic_structure_ref: Option<OdxLink>,
    pub basic_structure_snref: Option<String>,
    pub env_data_desc_ref: Option<OdxLink>,
    pub env_data_desc_snref: Option<String>,
    pub is_visible: bool,
}

// ─── STATIC-FIELD ─────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct StaticField {
    pub id: String,
    pub short_name: String,
    pub sdgs: Option<Sdgs>,
    pub field: Field,
    pub fixed_number_of_items: u32,
    pub item_byte_size: u32,
}

// ─── END-OF-PDU-FIELD ─────────────────────────────────────────────────────

#[derive(Debug)]
pub struct EndOfPduField {
    pub id: String,
    pub short_name: String,
    pub sdgs: Option<Sdgs>,
    pub field: Field,
    pub max_number_of_items: Option<u32>,
    pub min_number_of_items: Option<u32>,
}

// ─── ENV-DATA ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct EnvData {
    pub id: String,
    pub short_name: String,
    pub dtc_values: Vec<u32>,
    pub params: Vec<Param>,
}

// ─── ENV-DATA-DESC ────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct EnvDataDesc {
    pub id: String,
    pub short_name: String,
    pub env_data_refs: Vec<OdxLink>,
    pub param_snref: Option<String>,
    pub param_sn_pathref: Option<String>,
}

// ─── DYNAMIC-LENGTH-FIELD ─────────────────────────────────────────────────

#[derive(Debug)]
pub struct DynamicLengthField {
    pub id: String,
    pub short_name: String,
    pub field: Field,
    pub offset: u32,
    pub determine_number_of_items: DetermineNumberOfItems,
}

#[derive(Debug)]
pub struct DetermineNumberOfItems {
    pub byte_position: u32,
    pub bit_position: Option<u32>,
    pub data_object_prop_ref: Option<OdxLink>,
}

// ─── DYNAMIC-END-MARKER-FIELD ─────────────────────────────────────────────

#[derive(Debug)]
pub struct DynamicEndMarkerField {
    pub id: String,
    pub short_name: String,
    pub field: Field,
}

// ─── MUX ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Mux {
    pub id: String,
    pub short_name: String,
    pub is_visible: bool,
    pub byte_position: u32,
    pub switch_key: SwitchKey,
    pub default_case: Option<DefaultCase>,
    pub cases: Vec<Case>,
}

#[derive(Debug)]
pub struct SwitchKey {
    pub byte_position: u32,
    pub bit_position: Option<u32>,
    pub data_object_prop_ref: OdxLink,
}

#[derive(Debug)]
pub struct DefaultCase {
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub structure_ref: Option<OdxLink>,
}

#[derive(Debug)]
pub struct Case {
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub lower_limit: Limit,
    pub upper_limit: Limit,
    pub structure_ref: Option<OdxLink>,
    pub structure_snref: Option<String>,
}

// ─── TABLE ────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct Table {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub sdgs: Option<Sdgs>,
    pub semantic: Option<String>,
    pub key_label: Option<String>,
    pub struct_label: Option<String>,
    pub key_dop_ref: Option<OdxLink>,
    pub rows: Vec<TableRowOrLink>,
    pub diag_comm_connectors: Vec<TableDiagCommConnector>,
}

#[derive(Debug)]
pub enum TableRowOrLink {
    Row(TableRow),
    OdxLink(OdxLink),
}

#[derive(Debug, Clone)]
pub struct TableRow {

    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub sdgs: Option<Sdgs>,
    pub semantic: Option<String>,
    pub audience: Option<Audience>,
    pub key: Option<String>,
    pub dop_ref: Option<OdxLink>,
    pub dop_snref: Option<String>,
    pub structure_ref: Option<OdxLink>,
    pub structure_snref: Option<String>,
    pub funct_class_refs: Vec<OdxLink>,
    pub state_transition_refs: Vec<StateTransitionRef>,
    pub precondition_state_refs: Vec<PreConditionStateRef>,
    pub is_executable: bool,
    pub is_mandatory: bool,
    pub is_final: bool,
    pub numeric_id: i32,
    pub cells: Vec<String>,
    pub row_type: IntervalType,
}

#[derive(Debug)]
pub struct TableDiagCommConnector {
    pub semantic: Option<String>,
    pub diag_comm_ref: Option<OdxLink>,
    pub diag_comm_snref: Option<String>,
}

// ─── DIAG-CODED-TYPE hierarchy ────────────────────────────────────────────

#[derive(Debug)]
pub enum DiagCodedType {
    StandardLength(StandardLengthType),
    MinMaxLength(MinMaxLengthType),
    LeadingLengthInfo(LeadingLengthInfoType),
    ParamLengthInfo(ParamLengthInfoType),
}

impl DiagCodedType {
    pub fn base_data_type(&self) -> &str {
        match self {
            DiagCodedType::StandardLength(t) => &t.base.base_data_type,
            DiagCodedType::MinMaxLength(t) => &t.base.base_data_type,
            DiagCodedType::LeadingLengthInfo(t) => &t.base.base_data_type,
            DiagCodedType::ParamLengthInfo(t) => &t.base.base_data_type,
        }
    }
}

#[derive(Debug, Default)]
pub struct DiagCodedTypeBase {
    pub base_data_type: String,
    pub base_type_encoding: Option<String>,
    pub is_high_low_byte_order: bool,
}

#[derive(Debug)]
pub struct StandardLengthType {
    pub base: DiagCodedTypeBase,
    pub bit_length: u32,
    pub bit_mask: Option<Vec<u8>>,
    pub is_condensed: bool,
}

#[derive(Debug)]
pub struct MinMaxLengthType {
    pub base: DiagCodedTypeBase,
    pub min_length: u32,
    pub max_length: Option<u32>,
    pub termination: Option<TerminationKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationKind {
    End,
    Zero,
    Hff,
}

#[derive(Debug)]
pub struct LeadingLengthInfoType {
    pub base: DiagCodedTypeBase,
    pub bit_length: u32,
}

#[derive(Debug)]
pub struct ParamLengthInfoType {
    pub base: DiagCodedTypeBase,
    pub length_key_ref: OdxLink,
}

// ─── PHYSICAL-TYPE ────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct PhysicalType {
    pub base_data_type: String,
    pub precision: Option<u32>,
    pub display_radix: Option<DisplayRadix>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayRadix {
    Decimal,
    Hex,
    Binary,
    Oct,
}

// ─── Constraints ──────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct InternalConstr {
    pub lower_limit: Option<Limit>,
    pub upper_limit: Option<Limit>,
    pub scale_constrs: Vec<ScaleConstr>,
}

#[derive(Debug)]
pub struct Limit {
    pub value: Option<String>,
    pub interval_type: Option<IntervalType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq,Default)]
pub enum IntervalType {
    #[default]
    Closed,
    Open,
    Infinite,
    
}

#[derive(Debug)]
pub struct ScaleConstr {
    pub short_label: Option<crate::model::odx::Text>,
    pub lower_limit: Option<Limit>,
    pub upper_limit: Option<Limit>,
    pub validity: ConstrValidity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstrValidity {
    Valid,
    Invalid,
    NotDefined,
    NotAvailable,
}
