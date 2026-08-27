// src/collection/odx_collection.rs – Single-file index.
//
// Each `.odx` entry in the PDX archive is parsed into one `OdxCollection`.
// It maintains indexed maps (by ID and by short-name) for every element
// category, mirroring the Kotlin `ODXCollection` class.

use std::collections::HashMap;
use crate::model::{
    odx::{AdditionalAudience, FunctClass},
    diag_layer::{DiagLayerContainer, EcuVariant, BaseVariant, Protocol,
                 FunctionalGroup, EcuSharedData, ComParamSubset, ComParamSpec},
    diag_service::{DiagService, SingleEcuJob, Request, Response},
    dop::{DataObjectProp, DtcDop, Structure, StaticField, EndOfPduField,
           DynamicLengthField, DynamicEndMarkerField, Mux, EnvData,
           EnvDataDesc, Table, TableRow, Dtc},
    unit::{Unit, PhysicalDimension},
    state::{StateChart, State, StateTransition},
    comparam::{ComParam, ComplexComParam, ProtStack},
};

/// Indexed view of a single `.odx` file's content.
pub struct OdxCollection {
    pub source_file: String,
    pub container_key: String,

    /// Last DOC-REVISION/REVISION-LABEL found under this file's
    /// ADMIN-DATA/DOC-REVISIONS, if any (mirrors Kotlin's
    /// `ODXCollectionGroup.odxRevision` source).
    pub odx_revision: Option<String>,

    // ── Raw DiagLayerContainer (if present) ──
    pub diag_layer_container: Option<DiagLayerContainer>,
    pub comparam_subset: Option<ComParamSubset>,
    pub comparam_spec: Option<ComParamSpec>,

    // ── Indexed by ID ──
    pub ecu_variants: HashMap<String, usize>,          // id → index in flat vec
    pub base_variants: HashMap<String, usize>,
    pub protocols: HashMap<String, usize>,
    pub functional_groups: HashMap<String, usize>,
    pub ecu_shared_datas: HashMap<String, usize>,

    pub diag_services: HashMap<String, usize>,
    pub single_ecu_jobs: HashMap<String, usize>,
    pub requests: HashMap<String, usize>,
    pub pos_responses: HashMap<String, usize>,
    pub neg_responses: HashMap<String, usize>,
    pub global_neg_responses: HashMap<String, usize>,

    pub data_object_props: HashMap<String, usize>,
    pub dtc_dops: HashMap<String, usize>,
    pub structures: HashMap<String, usize>,
    pub static_fields: HashMap<String, usize>,
    pub end_of_pdu_fields: HashMap<String, usize>,
    pub dynamic_length_fields: HashMap<String, usize>,
    pub muxs: HashMap<String, usize>,
    pub env_datas: HashMap<String, usize>,
    pub env_data_descs: HashMap<String, usize>,
    pub tables: HashMap<String, usize>,
    pub table_rows: HashMap<String, usize>,
    pub dtcs: HashMap<String, usize>,

    pub units: HashMap<String, usize>,
    pub physical_dimensions: HashMap<String, usize>,
    pub state_charts: HashMap<String, usize>,
    pub states: HashMap<String, usize>,
    pub state_transitions: HashMap<String, usize>,

    pub comparams: HashMap<String, usize>,
    pub complex_comparams: HashMap<String, usize>,
    pub prot_stacks: HashMap<String, usize>,

    pub additional_audiences: HashMap<String, usize>,
    pub funct_classes: HashMap<String, usize>,

    // ── Short-name indices ──
    pub data_object_props_by_sn: HashMap<String, usize>,
    pub structures_by_sn: HashMap<String, usize>,
    pub env_data_descs_by_sn: HashMap<String, usize>,
    pub diag_services_by_sn: HashMap<String, usize>,
    pub single_ecu_jobs_by_sn: HashMap<String, usize>,
    pub protocols_by_sn: HashMap<String, usize>,
    pub prot_stacks_by_sn: HashMap<String, usize>,
    pub tables_by_sn: HashMap<String, usize>,
    pub table_rows_by_sn: HashMap<String, usize>,

    // ── Flat storage vecs (indexed by the maps above) ──
    pub ecu_variant_store: Vec<EcuVariant>,
    pub base_variant_store: Vec<BaseVariant>,
    pub protocol_store: Vec<Protocol>,
    pub functional_group_store: Vec<FunctionalGroup>,
    pub ecu_shared_data_store: Vec<EcuSharedData>,

    pub diag_service_store: Vec<DiagService>,
    pub single_ecu_job_store: Vec<SingleEcuJob>,
    pub request_store: Vec<Request>,
    pub response_store: Vec<Response>,

    pub data_object_prop_store: Vec<DataObjectProp>,
    pub dtc_dop_store: Vec<DtcDop>,
    pub structure_store: Vec<Structure>,
    pub static_field_store: Vec<StaticField>,
    pub end_of_pdu_field_store: Vec<EndOfPduField>,
    pub dynamic_length_field_store: Vec<DynamicLengthField>,
    pub mux_store: Vec<Mux>,
    pub env_data_store: Vec<EnvData>,
    pub env_data_desc_store: Vec<EnvDataDesc>,
    pub table_store: Vec<Table>,
    pub table_row_store: Vec<TableRow>,
    pub dtc_store: Vec<Dtc>,

    pub unit_store: Vec<Unit>,
    pub physical_dimension_store: Vec<PhysicalDimension>,
    pub state_chart_store: Vec<StateChart>,
    pub state_store: Vec<State>,
    pub state_transition_store: Vec<StateTransition>,

    pub comparam_store: Vec<ComParam>,
    pub complex_comparam_store: Vec<ComplexComParam>,
    pub prot_stack_store: Vec<ProtStack>,

    pub additional_audience_store: Vec<AdditionalAudience>,
    pub funct_class_store: Vec<FunctClass>,
}

impl OdxCollection {
    pub fn new(source_file: String, container_key: String) -> Self {
        Self {
            source_file,
            container_key,
            odx_revision: None,
            diag_layer_container: None,
            comparam_subset: None,
            comparam_spec: None,
            ecu_variants: HashMap::new(),
            base_variants: HashMap::new(),
            protocols: HashMap::new(),
            functional_groups: HashMap::new(),
            ecu_shared_datas: HashMap::new(),
            diag_services: HashMap::new(),
            single_ecu_jobs: HashMap::new(),
            requests: HashMap::new(),
            pos_responses: HashMap::new(),
            neg_responses: HashMap::new(),
            global_neg_responses: HashMap::new(),
            data_object_props: HashMap::new(),
            dtc_dops: HashMap::new(),
            structures: HashMap::new(),
            static_fields: HashMap::new(),
            end_of_pdu_fields: HashMap::new(),
            dynamic_length_fields: HashMap::new(),
            muxs: HashMap::new(),
            env_datas: HashMap::new(),
            env_data_descs: HashMap::new(),
            tables: HashMap::new(),
            table_rows: HashMap::new(),
            dtcs: HashMap::new(),
            units: HashMap::new(),
            physical_dimensions: HashMap::new(),
            state_charts: HashMap::new(),
            states: HashMap::new(),
            state_transitions: HashMap::new(),
            comparams: HashMap::new(),
            complex_comparams: HashMap::new(),
            prot_stacks: HashMap::new(),
            additional_audiences: HashMap::new(),
            funct_classes: HashMap::new(),
            data_object_props_by_sn: HashMap::new(),
            structures_by_sn: HashMap::new(),
            env_data_descs_by_sn: HashMap::new(),
            diag_services_by_sn: HashMap::new(),
            single_ecu_jobs_by_sn: HashMap::new(),
            protocols_by_sn: HashMap::new(),
            prot_stacks_by_sn: HashMap::new(),
            tables_by_sn: HashMap::new(),
            table_rows_by_sn: HashMap::new(),
            ecu_variant_store: Vec::new(),
            base_variant_store: Vec::new(),
            protocol_store: Vec::new(),
            functional_group_store: Vec::new(),
            ecu_shared_data_store: Vec::new(),
            diag_service_store: Vec::new(),
            single_ecu_job_store: Vec::new(),
            request_store: Vec::new(),
            response_store: Vec::new(),
            data_object_prop_store: Vec::new(),
            dtc_dop_store: Vec::new(),
            structure_store: Vec::new(),
            static_field_store: Vec::new(),
            end_of_pdu_field_store: Vec::new(),
            dynamic_length_field_store: Vec::new(),
            mux_store: Vec::new(),
            env_data_store: Vec::new(),
            env_data_desc_store: Vec::new(),
            table_store: Vec::new(),
            table_row_store: Vec::new(),
            dtc_store: Vec::new(),
            unit_store: Vec::new(),
            physical_dimension_store: Vec::new(),
            state_chart_store: Vec::new(),
            state_store: Vec::new(),
            state_transition_store: Vec::new(),
            comparam_store: Vec::new(),
            complex_comparam_store: Vec::new(),
            prot_stack_store: Vec::new(),
            additional_audience_store: Vec::new(),
            funct_class_store: Vec::new(),
        }
    }

    // ── Accessor helpers ──────────────────────────────────────────────────

    pub fn diag_service_by_id(&self, id: &str) -> Option<&DiagService> {
        self.diag_services.get(id).map(|&i| &self.diag_service_store[i])
    }

    pub fn diag_service_by_sn(&self, sn: &str) -> Option<&DiagService> {
        self.diag_services_by_sn.get(sn).map(|&i| &self.diag_service_store[i])
    }

    pub fn single_ecu_job_by_id(&self, id: &str) -> Option<&SingleEcuJob> {
        self.single_ecu_jobs.get(id).map(|&i| &self.single_ecu_job_store[i])
    }

    pub fn single_ecu_job_by_sn(&self, sn: &str) -> Option<&SingleEcuJob> {
        self.single_ecu_jobs_by_sn.get(sn).map(|&i| &self.single_ecu_job_store[i])
    }

    pub fn dop_by_id(&self, id: &str) -> Option<&DataObjectProp> {
        self.data_object_props.get(id).map(|&i| &self.data_object_prop_store[i])
    }

    pub fn dop_by_sn(&self, sn: &str) -> Option<&DataObjectProp> {
        self.data_object_props_by_sn.get(sn).map(|&i| &self.data_object_prop_store[i])
    }

    pub fn structure_by_sn(&self, sn: &str) -> Option<&Structure> {
        self.structures_by_sn.get(sn).map(|&i| &self.structure_store[i])
    }

    pub fn env_data_desc_by_sn(&self, sn: &str) -> Option<&EnvDataDesc> {
        self.env_data_descs_by_sn.get(sn).map(|&i| &self.env_data_desc_store[i])
    }

    pub fn table_by_sn(&self, sn: &str) -> Option<&Table> {
        self.tables_by_sn.get(sn).map(|&i| &self.table_store[i])
    }

    pub fn table_row_by_sn(&self, sn: &str) -> Option<&TableRow> {
        self.table_rows_by_sn.get(sn).map(|&i| &self.table_row_store[i])
    }

    pub fn protocol_by_sn(&self, sn: &str) -> Option<&Protocol> {
        self.protocols_by_sn.get(sn).map(|&i| &self.protocol_store[i])
    }

    pub fn prot_stack_by_sn(&self, sn: &str) -> Option<&ProtStack> {
        self.prot_stacks_by_sn.get(sn).map(|&i| &self.prot_stack_store[i])
    }

    pub fn all_diag_services(&self) -> &[DiagService] {
        &self.diag_service_store
    }

    pub fn all_single_ecu_jobs(&self) -> &[SingleEcuJob] {
        &self.single_ecu_job_store
    }
}
