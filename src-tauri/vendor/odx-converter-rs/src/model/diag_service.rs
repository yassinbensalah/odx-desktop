// src/model/diag_service.rs – DIAGSERVICE, SINGLEECUJOB, REQUEST/RESPONSE and all
// PARAM subtypes.  This mirrors the Kotlin schema.odx.* JAXB classes.

use crate::model::odx::{Audience, OdxLink, SnRef, LongName, Text, Sdgs};
use crate::model::dop::DiagCodedType;
use crate::model::state::PreConditionStateRef;

// ─── Addressing and transmission mode enums ───────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Addressing {
    Physical,
    Functional,
    PhysicalOrFunctional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransmissionMode {
    SendOnly,
    ReceiveOnly,
    SendAndReceive,
    SendOrReceive,
}

// ─── DiagClass ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagClassType {
    StartComm,
    StopComm,
    VariantIdentification,
    ReadDynDefMessage,
    DynDefMessage,
    ClearDynDefMessage,
}

// ─── DIAG-COMM (shared fields) ────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct DiagCommCore {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub sdgs: Option<Sdgs>,
    pub semantic: Option<String>,
    pub diag_class: Option<DiagClassType>,
    pub funct_class_refs: Vec<OdxLink>,
    pub precondition_state_refs: Vec<PreConditionStateRef>,
    pub state_transition_refs: Vec<StateTransitionRef>,
    pub protocol_snrefs: Vec<String>,
    pub audience: Option<Audience>,
    pub is_final: bool,
    pub is_mandatory: bool,
    pub is_executable: bool,
}

// ─── STATE-TRANSITION-REF ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StateTransitionRef {
    pub id_ref: Option<String>,
    pub doc_ref: Option<String>,
    pub value: Option<String>,
}

// ─── DIAG-SERVICE ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct DiagService {
    pub comm: DiagCommCore,
    pub request_ref: Option<OdxLink>,
    pub pos_response_refs: Vec<OdxLink>,
    pub neg_response_refs: Vec<OdxLink>,
    pub comparam_refs: Vec<crate::model::comparam::ComParamRef>,
    pub addressing: Option<Addressing>,
    pub transmission_mode: Option<TransmissionMode>,
    pub is_cyclic: bool,
    pub is_multiple: bool,
}

// ─── SINGLE-ECU-JOB ───────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct SingleEcuJob {
    pub comm: DiagCommCore,
    pub prog_codes: Vec<ProgCode>,
    pub input_params: Vec<InputParam>,
    pub output_params: Vec<OutputParam>,
    pub neg_output_params: Vec<NegOutputParam>,
}

/// Reference to an executable code file with entry-point metadata.
#[derive(Debug, Default)]
pub struct ProgCode {
    pub code_file: Option<String>,
    pub encryption: Option<String>,
    pub syntax: Option<String>,
    pub revision: Option<String>,
    pub entrypoint: Option<String>,
    pub library_refs: Vec<OdxLink>,
}

// Job parameter types

#[derive(Debug)]
pub struct InputParam {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub semantic: Option<String>,
    pub physical_default_value: Option<String>,
    pub dop_base_ref: Option<OdxLink>,
}

#[derive(Debug)]
pub struct OutputParam {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub semantic: Option<String>,
    pub dop_base_ref: Option<OdxLink>,
}

#[derive(Debug)]
pub struct NegOutputParam {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub dop_base_ref: Option<OdxLink>,
}

// ─── REQUEST / RESPONSE ───────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct Request {
    pub id: String,
    pub short_name: String,
    pub sdgs: Option<Sdgs>,
    pub params: Vec<Param>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseKind {
    Positive,
    Negative,
    GlobalNegative,
}

#[derive(Debug, Default)]
pub struct Response {
    pub id: String,
    pub short_name: String,
    pub kind: ResponseKind,
    pub sdgs: Option<Sdgs>,
    pub params: Vec<Param>,
}

impl Default for ResponseKind {
    fn default() -> Self {
        Self::Positive
    }
}

// ─── PARAM (all subtypes) ─────────────────────────────────────────────────

/// All ODX param subtypes are represented as variants of this enum.
/// Corresponds to the Kotlin `schema.odx.PARAM` polymorphic hierarchy.
#[derive(Debug)]
pub enum Param {
    Value(ValueParam),
    CodedConst(CodedConstParam),
    Dynamic(DynamicParam),
    LengthKey(LengthKeyParam),
    MatchingRequest(MatchingRequestParam),
    NrcConst(NrcConstParam),
    PhysConst(PhysConstParam),
    Reserved(ReservedParam),
    System(SystemParam),
    TableKey(TableKeyParam),
    TableEntry(TableEntryParam),
    TableStruct(TableStructParam),
}

impl Param {
    pub fn short_name(&self) -> &str {
        match self {
            Param::Value(p) => &p.base.short_name,
            Param::CodedConst(p) => &p.base.short_name,
            Param::Dynamic(p) => &p.base.short_name,
            Param::LengthKey(p) => &p.base.short_name,
            Param::MatchingRequest(p) => &p.base.short_name,
            Param::NrcConst(p) => &p.base.short_name,
            Param::PhysConst(p) => &p.base.short_name,
            Param::Reserved(p) => &p.base.short_name,
            Param::System(p) => &p.base.short_name,
            Param::TableKey(p) => &p.base.short_name,
            Param::TableEntry(p) => &p.base.short_name,
            Param::TableStruct(p) => &p.base.short_name,
        }
    }
}

/// Fields shared by every PARAM subtype.
#[derive(Debug, Default)]
pub struct ParamBase {
    pub id: String,
    pub short_name: String,
    pub semantic: Option<String>,
    pub sdgs: Option<Sdgs>,
    pub byte_position: Option<u32>,
    pub bit_position: Option<u32>,
}

// VALUE: references a DOP and has an optional physical default.
#[derive(Debug)]
pub struct ValueParam {
    pub base: ParamBase,
    pub dop_ref: Option<OdxLink>,
    pub dop_snref: Option<String>,
    pub physical_default_value: Option<String>,
}

// CODED-CONST: carries a static coded value with a DiagCodedType.
#[derive(Debug)]
pub struct CodedConstParam {
    pub base: ParamBase,
    pub diag_coded_type: Option<DiagCodedType>,
    pub coded_value: Option<String>,
}

// DYNAMIC: length determined at runtime.
#[derive(Debug)]
pub struct DynamicParam {
    pub base: ParamBase,
}

// LENGTH-KEY: marks the byte that carries the length.
#[derive(Debug)]
pub struct LengthKeyParam {
    pub base: ParamBase,
    pub dop_ref: Option<OdxLink>,
    pub dop_snref: Option<String>,
}

// MATCHING-REQUEST-PARAM: echoes bytes from the request.
#[derive(Debug)]
pub struct MatchingRequestParam {
    pub base: ParamBase,
    pub request_byte_pos: u32,
    pub byte_length: u32,
}

// NRC-CONST: negative response code constant.
#[derive(Debug)]
pub struct NrcConstParam {
    pub base: ParamBase,
    pub diag_coded_type: Option<DiagCodedType>,
    pub coded_values: Vec<String>,
}

// PHYS-CONST: physical constant (e.g. for overriding a DOP's scaled value).
#[derive(Debug)]
pub struct PhysConstParam {
    pub base: ParamBase,
    pub dop_ref: Option<OdxLink>,
    pub dop_snref: Option<String>,
    pub phys_constant_value: Option<String>,
}

// RESERVED: padding bytes.
#[derive(Debug)]
pub struct ReservedParam {
    pub base: ParamBase,
    pub bit_length: u32,
}

// SYSTEM: system parameter (e.g. UDS_SID, target address).
#[derive(Debug)]
pub struct SystemParam {
    pub base: ParamBase,
    pub sys_param: String,
    pub dop_ref: Option<OdxLink>,
    pub dop_snref: Option<String>,
}

// TABLE-KEY: references a TABLE to select the encoding.
#[derive(Debug)]
pub struct TableKeyParam {
    pub base: ParamBase,
    pub table_ref: Option<TableKeyRef>,
}

#[derive(Debug)]
pub enum TableKeyRef {
    OdxLink(OdxLink),
    TableSnRef(String),
    TableRowSnRef(String),
}

// TABLE-ENTRY: holds a reference to a specific TABLE-ROW.
#[derive(Debug)]
pub struct TableEntryParam {
    pub base: ParamBase,
    pub table_row_ref: Option<OdxLink>,
    pub target: Option<String>,
}

// TABLE-STRUCT: selects the TABLE-KEY that governs this struct param.
#[derive(Debug)]
pub struct TableStructParam {
    pub base: ParamBase,
    pub table_key_ref: Option<OdxLink>,
    pub table_key_snref: Option<String>,
}
