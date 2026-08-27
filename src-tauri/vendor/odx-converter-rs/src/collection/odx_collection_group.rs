// src/collection/odx_collection_group.rs – Multi-file merged index.
//
// `OdxCollectionGroup` owns all per-file `OdxCollection`s and provides
// scoped ODXLINK resolution that walks from the requesting file outward
// to the appropriate container.  Mirrors the Kotlin `ODXCollectionGroup`.

use std::collections::HashMap;
use std::path::Path;
use anyhow::{anyhow, Result};
use log::warn;

use crate::collection::OdxCollection;
use crate::error::OdxError;
use crate::model::{
    odx::OdxLink,
    diag_layer::{EcuVariant, BaseVariant, Protocol, FunctionalGroup, EcuSharedData},
    diag_service::{DiagService, SingleEcuJob, Request, Response},
    dop::{DataObjectProp, DtcDop, DtcOrRef, Structure, Table, TableRow, Dtc},
    unit::{Unit, PhysicalDimension},
    state::{State, StateTransition, StateChart, PreConditionStateRef},
    comparam::{ComParam, ComplexComParam, ProtStack, ComParamRef},
};
use crate::options::ConverterOptions;

/// Merged index across all ODX files inside one PDX.
pub struct OdxCollectionGroup {
    /// All per-file collections, keyed by their `container_key`.
    pub collections: HashMap<String, OdxCollection>,

    /// Mapping from element ID to the container key of the owning file.
    /// Built during indexing.
    pub link_to_file: HashMap<String, String>,

    /// Maps ODX DOCREF aliases to the file/container that owns them.
    ///
    /// DOCREF commonly names a DIAG-LAYER (for example an ECU-SHARED-DATA
    /// short-name), not the outer DIAG-LAYER-CONTAINER.  Keeping these aliases
    /// avoids silently dropping valid cross-document references.
    pub docref_to_file: HashMap<String, String>,

    /// ECU name (short-name of the first ECU-VARIANT found, or the
    /// container short-name).
    pub ecu_name: String,

    /// Optional revision string from the ODX file.
    pub odx_revision: Option<String>,

    /// Total raw ODX byte size before conversion.
    pub raw_size: u64,
}

impl OdxCollectionGroup {
    /// Build a group from a map of already-parsed collections.
    pub fn new(
        collections: Vec<OdxCollection>,
        raw_size: u64,
    ) -> Self {
        let mut link_to_file: HashMap<String, String> = HashMap::new();
        let mut docref_to_file: HashMap<String, String> = HashMap::new();
        let mut col_map: HashMap<String, OdxCollection> = HashMap::new();

        for coll in collections {
            let container_key = coll.container_key.clone();

            // Register all indexed IDs, not only the most common object types.
            // This provides a reliable fallback when a producer uses a DOCREF
            // spelling that differs from the outer container short-name.
            let id_maps: &[&HashMap<String, usize>] = &[
                &coll.ecu_variants, &coll.base_variants, &coll.protocols,
                &coll.functional_groups, &coll.ecu_shared_datas,
                &coll.diag_services, &coll.single_ecu_jobs, &coll.requests,
                &coll.pos_responses, &coll.neg_responses, &coll.global_neg_responses,
                &coll.data_object_props, &coll.dtc_dops, &coll.structures,
                &coll.static_fields, &coll.end_of_pdu_fields,
                &coll.dynamic_length_fields, &coll.muxs, &coll.env_datas,
                &coll.env_data_descs, &coll.tables, &coll.table_rows, &coll.dtcs,
                &coll.units, &coll.physical_dimensions, &coll.state_charts,
                &coll.states, &coll.state_transitions, &coll.comparams,
                &coll.complex_comparams, &coll.prot_stacks,
                &coll.additional_audiences, &coll.funct_classes,
            ];
            for map in id_maps {
                for id in map.keys() {
                    link_to_file.insert(id.clone(), container_key.clone());
                }
            }

            // Register every useful spelling that may occur in DOCREF.  ODX
            // files often reference a contained layer short-name (e.g.
            // `Datas_Format`) rather than the outer DLC (`Alliance_Datas_Format`).
            let mut register_alias = |alias: &str| {
                let alias = alias.trim();
                if alias.is_empty() {
                    return;
                }
                docref_to_file
                    .entry(alias.to_string())
                    .and_modify(|current| {
                        // Deterministic tie-break if a malformed PDX reuses an alias.
                        if container_key < *current {
                            *current = container_key.clone();
                        }
                    })
                    .or_insert_with(|| container_key.clone());
            };

            register_alias(&container_key);
            register_alias(&coll.source_file);
            if let Some(file_name) = Path::new(&coll.source_file).file_name().and_then(|v| v.to_str()) {
                register_alias(file_name);
            }
            if let Some(file_stem) = Path::new(&coll.source_file).file_stem().and_then(|v| v.to_str()) {
                register_alias(file_stem);
            }
            if let Some(container) = &coll.diag_layer_container {
                register_alias(&container.short_name);
            }
            for value in &coll.ecu_variant_store { register_alias(&value.core.short_name); }
            for value in &coll.base_variant_store { register_alias(&value.core.short_name); }
            for value in &coll.protocol_store { register_alias(&value.core.short_name); }
            for value in &coll.functional_group_store { register_alias(&value.core.short_name); }
            for value in &coll.ecu_shared_data_store { register_alias(&value.core.short_name); }
            if let Some(value) = &coll.comparam_subset { register_alias(&value.short_name); }
            if let Some(value) = &coll.comparam_spec { register_alias(&value.short_name); }

            col_map.insert(container_key, coll);
        }

        // Mirrors Kotlin's `ODXCollectionGroup.ecuName` exactly:
        //   1. The short_name of the first BASE-VARIANT found (not
        //      ECU-VARIANT — Kotlin checks BASE-VARIANT specifically), in
        //      the collection whose container key sorts first alphabetically
        //      (deterministic, unlike raw HashMap iteration order).
        //   2. Otherwise, if any FUNCTIONAL-GROUP is present anywhere, the
        //      literal string "functional_groups" (a deliberate fallback
        //      name in Kotlin, not a computed value).
        //   3. Otherwise, "Unknown" (Kotlin instead errors with
        //      `error("No base variant")`; this stays non-fatal here).
        let mut sorted_keys: Vec<&String> = col_map.keys().collect();
        sorted_keys.sort();

        let base_variant_collection = sorted_keys
            .iter()
            .find(|k| !col_map[**k].base_variant_store.is_empty());

        let ecu_name = match base_variant_collection {
            Some(key) => col_map[*key].base_variant_store[0].core.short_name.clone(),
            None => {
                let has_functional_group = col_map.values().any(|c| !c.functional_group_store.is_empty());
                if has_functional_group {
                    "functional_groups".to_string()
                } else {
                    "Unknown".to_string()
                }
            }
        };

        // Mirrors Kotlin's `ODXCollectionGroup.odxRevision`: prefer the
        // base-variant file's revision; else the functional-group file's.
        let odx_revision = base_variant_collection
            .and_then(|key| col_map[*key].odx_revision.clone())
            .or_else(|| {
                let functional_group_collection = sorted_keys
                    .iter()
                    .find(|k| !col_map[**k].functional_group_store.is_empty());
                functional_group_collection.and_then(|key| col_map[*key].odx_revision.clone())
            });

        Self {
            collections: col_map,
            link_to_file,
            docref_to_file,
            ecu_name,
            odx_revision,
            raw_size,
        }
    }

    // ── ODXLINK resolution ────────────────────────────────────────────────

    /// Resolve an ODXLINK scoped to the container that holds the requesting
    /// element.  Falls back to a global search when `doc_ref` is absent.
    fn resolve_in<T>(
        &self,
        link: &OdxLink,
        requesting_container: Option<&str>,
        extractor: impl Fn(&OdxCollection) -> Option<&T>,
    ) -> Option<&T> {
        // 1. A DOCREF may name either the outer container, the source file,
        //    or a contained DIAG-LAYER short-name.  Try all registered aliases.
        if let Some(doc_ref) = &link.doc_ref {
            if let Some(collection) = self.collections.get(doc_ref.as_str()) {
                if let Some(value) = extractor(collection) {
                    return Some(value);
                }
            }
            if let Some(file_key) = self.docref_to_file.get(doc_ref.as_str()) {
                if let Some(collection) = self.collections.get(file_key.as_str()) {
                    if let Some(value) = extractor(collection) {
                        return Some(value);
                    }
                }
            }

            // Some real-world PDX files contain a stale/abbreviated DOCREF but
            // still use a globally unique ID-REF.  Resolve by ID before failing.
            if let Some(file_key) = self.link_to_file.get(&link.id_ref) {
                if let Some(collection) = self.collections.get(file_key.as_str()) {
                    if let Some(value) = extractor(collection) {
                        return Some(value);
                    }
                }
            }
            return None;
        }

        // 2. Try the owning container first (most hits are local).
        if let Some(key) = requesting_container {
            if let Some(c) = self.collections.get(key) {
                if let Some(v) = extractor(c) {
                    return Some(v);
                }
            }
        }

        // 3. Try the file that registered this ID.
        if let Some(file_key) = self.link_to_file.get(&link.id_ref) {
            if let Some(c) = self.collections.get(file_key.as_str()) {
                return extractor(c);
            }
        }

        // 4. Global scan (last resort).
        for c in self.collections.values() {
            if let Some(v) = extractor(c) {
                return Some(v);
            }
        }

        None
    }

    /// Report a failed ODXLINK resolution (strict) or warn (lenient).
    pub fn resolution_error(
        &self,
        expected_type: &str,
        link: &OdxLink,
        options: &ConverterOptions,
    ) -> Result<(), OdxError> {
        let msg = format!(
            "Could not resolve {} ODXLINK: id_ref='{}' doc_ref='{}'",
            expected_type,
            link.id_ref,
            link.doc_ref.as_deref().unwrap_or("<none>")
        );
        if options.lenient {
            warn!("{}", msg);
            Ok(())
        } else {
            Err(OdxError::ResolutionError(msg))
        }
    }

    // ── Typed resolution helpers ──────────────────────────────────────────

    pub fn resolve_diag_service(
        &self,
        link: &OdxLink,
        from: Option<&str>,
    ) -> Option<&DiagService> {
        self.resolve_in(link, from, |c| c.diag_service_by_id(&link.id_ref))
    }

    pub fn resolve_single_ecu_job(
        &self,
        link: &OdxLink,
        from: Option<&str>,
    ) -> Option<&SingleEcuJob> {
        self.resolve_in(link, from, |c| c.single_ecu_job_by_id(&link.id_ref))
    }

    pub fn resolve_request(&self, link: &OdxLink, from: Option<&str>) -> Option<&Request> {
        self.resolve_in(link, from, |c| {
            c.requests.get(&link.id_ref).map(|&i| &c.request_store[i])
        })
    }

    pub fn resolve_pos_response(&self, link: &OdxLink, from: Option<&str>) -> Option<&Response> {
       self.resolve_in(link, from, |c| {
        c.pos_responses.get(&link.id_ref).map(|&i| &c.response_store[i])
    })
    }

    pub fn resolve_neg_response(&self, link: &OdxLink, from: Option<&str>) -> Option<&Response> {
        self.resolve_in(link, from, |c| {
            c.neg_responses.get(&link.id_ref).map(|&i| &c.response_store[i])
        })
    }

    pub fn resolve_dop(&self, link: &OdxLink, from: Option<&str>) -> Option<&DataObjectProp> {
        self.resolve_in(link, from, |c| {
            c.dop_by_id(&link.id_ref).or_else(|| {
                c.comparam_subset.as_ref().and_then(|subset| {
                    subset
                        .data_object_props
                        .iter()
                        .find(|dop| dop.id == link.id_ref)
                })
            })
        })
    }

    pub fn resolve_dtc_dop(&self, link: &OdxLink, from: Option<&str>) -> Option<&DtcDop> {
        self.resolve_in(link, from, |c| {
            c.dtc_dops.get(&link.id_ref).map(|&i| &c.dtc_dop_store[i])
        })
    }

    pub fn resolve_dtc(&self, link: &OdxLink, from: Option<&str>) -> Option<&crate::model::dop::Dtc> {
        if let Some(key) = from {
            if let Some(c) = self.collections.get(key) {
                if let Some(v) = find_dtc_by_id(c, &link.id_ref) { return Some(v); }
            }
        }
        self.collections.values().find_map(|c| find_dtc_by_id(c, &link.id_ref))
    }

    pub fn resolve_structure(&self, link: &OdxLink, from: Option<&str>) -> Option<&Structure> {
        self.resolve_in(link, from, |c| {
            c.structures.get(&link.id_ref).map(|&i| &c.structure_store[i])
        })
    }

    pub fn resolve_static_field(&self, link: &OdxLink, from: Option<&str>) -> Option<&crate::model::dop::StaticField> {
        self.resolve_in(link, from, |c| {
            c.static_fields.get(&link.id_ref).map(|&i| &c.static_field_store[i])
        })
    }

    pub fn resolve_end_of_pdu_field(&self, link: &OdxLink, from: Option<&str>) -> Option<&crate::model::dop::EndOfPduField> {
        self.resolve_in(link, from, |c| {
            c.end_of_pdu_fields.get(&link.id_ref).map(|&i| &c.end_of_pdu_field_store[i])
        })
    }

    pub fn resolve_dynamic_length_field(&self, link: &OdxLink, from: Option<&str>) -> Option<&crate::model::dop::DynamicLengthField> {
        self.resolve_in(link, from, |c| {
            c.dynamic_length_fields.get(&link.id_ref).map(|&i| &c.dynamic_length_field_store[i])
        })
    }

    pub fn resolve_mux(&self, link: &OdxLink, from: Option<&str>) -> Option<&crate::model::dop::Mux> {
        self.resolve_in(link, from, |c| {
            c.muxs.get(&link.id_ref).map(|&i| &c.mux_store[i])
        })
    }

    pub fn resolve_env_data(&self, link: &OdxLink, from: Option<&str>) -> Option<&crate::model::dop::EnvData> {
        self.resolve_in(link, from, |c| {
            c.env_datas.get(&link.id_ref).map(|&i| &c.env_data_store[i])
        })
    }

    pub fn resolve_env_data_desc(&self, link: &OdxLink, from: Option<&str>) -> Option<&crate::model::dop::EnvDataDesc> {
        self.resolve_in(link, from, |c| {
            c.env_data_descs.get(&link.id_ref).map(|&i| &c.env_data_desc_store[i])
        })
    }

    pub fn resolve_table(&self, link: &OdxLink, from: Option<&str>) -> Option<&Table> {
        self.resolve_in(link, from, |c| {
            c.tables.get(&link.id_ref).map(|&i| &c.table_store[i])
        })
    }

    pub fn resolve_table_row(&self, link: &OdxLink, from: Option<&str>) -> Option<&TableRow> {
        self.resolve_in(link, from, |c| {
            c.table_rows.get(&link.id_ref).map(|&i| &c.table_row_store[i])
        })
    }

    pub fn resolve_dop_by_sn(&self, short_name: &str, from: Option<&str>) -> Option<&DataObjectProp> {
        if let Some(key) = from {
            if let Some(c) = self.collections.get(key) {
                if let Some(v) = c.dop_by_sn(short_name) { return Some(v); }
            }
        }
        self.collections.values().find_map(|c| c.dop_by_sn(short_name))
    }

    pub fn resolve_structure_by_sn(&self, short_name: &str, from: Option<&str>) -> Option<&Structure> {
        if let Some(key) = from {
            if let Some(c) = self.collections.get(key) {
                if let Some(v) = c.structure_by_sn(short_name) { return Some(v); }
            }
        }
        self.collections.values().find_map(|c| c.structure_by_sn(short_name))
    }

    pub fn resolve_env_data_desc_by_sn(&self, short_name: &str, from: Option<&str>) -> Option<&crate::model::dop::EnvDataDesc> {
        if let Some(key) = from {
            if let Some(c) = self.collections.get(key) {
                if let Some(v) = c.env_data_desc_by_sn(short_name) { return Some(v); }
            }
        }
        self.collections.values().find_map(|c| c.env_data_desc_by_sn(short_name))
    }

    pub fn resolve_table_by_sn(&self, short_name: &str, from: Option<&str>) -> Option<&Table> {
        if let Some(key) = from {
            if let Some(c) = self.collections.get(key) {
                if let Some(&i) = c.tables_by_sn.get(short_name) { return Some(&c.table_store[i]); }
            }
        }
        self.collections.values().find_map(|c| c.tables_by_sn.get(short_name).map(|&i| &c.table_store[i]))
    }

    pub fn resolve_table_row_by_sn(&self, short_name: &str, from: Option<&str>) -> Option<&TableRow> {
        if let Some(key) = from {
            if let Some(c) = self.collections.get(key) {
                if let Some(&i) = c.table_rows_by_sn.get(short_name) { return Some(&c.table_row_store[i]); }
            }
        }
        self.collections.values().find_map(|c| c.table_rows_by_sn.get(short_name).map(|&i| &c.table_row_store[i]))
    }

    pub fn resolve_unit(&self, link: &OdxLink, from: Option<&str>) -> Option<&Unit> {
        self.resolve_in(link, from, |c| {
            c.units.get(&link.id_ref).map(|&i| &c.unit_store[i])
        })
    }

    pub fn resolve_phys_dimension(
        &self,
        link: &OdxLink,
        from: Option<&str>,
    ) -> Option<&PhysicalDimension> {
        self.resolve_in(link, from, |c| {
            c.physical_dimensions.get(&link.id_ref).map(|&i| &c.physical_dimension_store[i])
        })
    }

    pub fn resolve_param(&self, id_ref: &str, from: Option<&str>) -> Option<&crate::model::diag_service::Param> {
        if let Some(key) = from {
            if let Some(c) = self.collections.get(key) {
                if let Some(p) = find_param_by_id(c, id_ref) { return Some(p); }
            }
        }
        if let Some(file_key) = self.link_to_file.get(id_ref) {
            if let Some(c) = self.collections.get(file_key) {
                if let Some(p) = find_param_by_id(c, id_ref) { return Some(p); }
            }
        }
        self.collections.values().find_map(|c| find_param_by_id(c, id_ref))
    }

    pub fn resolve_param_by_sn(&self, short_name: &str, from: Option<&str>) -> Option<&crate::model::diag_service::Param> {
        if let Some(key) = from {
            if let Some(c) = self.collections.get(key) {
                if let Some(p) = find_param_by_sn(c, short_name) { return Some(p); }
            }
        }
        self.collections.values().find_map(|c| find_param_by_sn(c, short_name))
    }

    pub fn resolve_state(&self, id_ref: &str, from: Option<&str>) -> Option<&crate::model::state::State> {
        let link = OdxLink::new(id_ref);
        self.resolve_in(&link, from, |c| {
            c.states.get(id_ref).map(|&i| &c.state_store[i])
        })
    }

    pub fn resolve_state_transition(
        &self,
        id_ref: &str,
        from: Option<&str>,
    ) -> Option<&StateTransition> {
        let link = OdxLink::new(id_ref);
        self.resolve_in(&link, from, |c| {
            c.state_transitions.get(id_ref).map(|&i| &c.state_transition_store[i])
        })
    }

    pub fn resolve_comparam(
        &self,
        link: &OdxLink,
        from: Option<&str>,
    ) -> Option<&ComParam> {
        self.resolve_in(link, from, |c| {
            c.comparams
                .get(&link.id_ref)
                .map(|&i| &c.comparam_store[i])
                .or_else(|| {
                    c.comparam_subset.as_ref().and_then(|subset| {
                        subset.comparams.iter().find(|cp| cp.id == link.id_ref)
                    })
                })
        })
    }

    pub fn resolve_complex_comparam(
        &self,
        link: &OdxLink,
        from: Option<&str>,
    ) -> Option<&ComplexComParam> {
        self.resolve_in(link, from, |c| {
            c.complex_comparams
                .get(&link.id_ref)
                .map(|&i| &c.complex_comparam_store[i])
                .or_else(|| {
                    c.comparam_subset.as_ref().and_then(|subset| {
                        subset
                            .complex_comparams
                            .iter()
                            .find(|cp| cp.id == link.id_ref)
                    })
                })
        })
    }

    pub fn resolve_comparam_subset(
        &self,
        link: &OdxLink,
        from: Option<&str>,
    ) -> Option<&crate::model::diag_layer::ComParamSubset> {
        self.resolve_in(link, from, |c| {
            c.comparam_subset.as_ref().filter(|s| s.id == link.id_ref)
        })
    }

    pub fn resolve_comparam_spec(
        &self,
        link: &OdxLink,
        from: Option<&str>,
    ) -> Option<&crate::model::diag_layer::ComParamSpec> {
        self.resolve_in(link, from, |c| {
            c.comparam_spec.as_ref().filter(|s| s.id == link.id_ref)
        })
    }

    pub fn resolve_prot_stack_by_sn(
        &self,
        sn: &str,
        from: Option<&str>,
    ) -> Option<&ProtStack> {
        // Local first.
        if let Some(key) = from {
            if let Some(c) = self.collections.get(key) {
                if let Some(ps) = c.prot_stack_by_sn(sn) {
                    return Some(ps);
                }
            }
        }

        // Use a deterministic global order. HashMap iteration order is random,
        // and vendor PDX files may contain same-named stacks in several specs.
        let mut keys: Vec<&String> = self.collections.keys().collect();
        keys.sort();
        keys.into_iter()
            .find_map(|key| self.collections.get(key).and_then(|c| c.prot_stack_by_sn(sn)))
    }

    pub fn resolve_protocol_by_sn(
        &self,
        sn: &str,
        from: Option<&str>,
    ) -> Option<&crate::model::diag_layer::Protocol> {
        if let Some(key) = from {
            if let Some(c) = self.collections.get(key) {
                if let Some(p) = c.protocol_by_sn(sn) {
                    return Some(p);
                }
            }
        }
        self.collections.values().find_map(|c| c.protocol_by_sn(sn))
    }

    pub fn resolve_additional_audience(
        &self,
        link: &OdxLink,
        from: Option<&str>,
    ) -> Option<&crate::model::odx::AdditionalAudience> {
        self.resolve_in(link, from, |c| {
            c.additional_audiences.get(&link.id_ref).map(|&i| &c.additional_audience_store[i])
        })
    }

    pub fn resolve_funct_class(
        &self,
        link: &OdxLink,
        from: Option<&str>,
    ) -> Option<&crate::model::odx::FunctClass> {
        self.resolve_in(link, from, |c| {
            c.funct_classes.get(&link.id_ref).map(|&i| &c.funct_class_store[i])
        })
    }

    // ── Aggregated iterators over all files ────────────────────────────────

    pub fn all_ecu_variants(&self) -> impl Iterator<Item = &EcuVariant> {
        self.collections.values().flat_map(|c| c.ecu_variant_store.iter())
    }

    pub fn all_base_variants(&self) -> impl Iterator<Item = &BaseVariant> {
        self.collections.values().flat_map(|c| c.base_variant_store.iter())
    }

    pub fn all_protocols(&self) -> impl Iterator<Item = &Protocol> {
        self.collections.values().flat_map(|c| c.protocol_store.iter())
    }

    pub fn all_functional_groups(&self) -> impl Iterator<Item = &FunctionalGroup> {
        self.collections.values().flat_map(|c| c.functional_group_store.iter())
    }

    pub fn all_dtcs(&self) -> impl Iterator<Item = &crate::model::dop::Dtc> {
        // The Kotlin converter exposes every concrete DTC from every collection
        // in the root EcuData.dtcs vector, including shared-data containers.
        self.collections.values()
            .flat_map(|c| c.dtc_dop_store.iter())
            .flat_map(|dop| dop.dtcs.iter())
            .filter_map(|dtc_or_ref| match dtc_or_ref {
                DtcOrRef::Dtc(dtc) => Some(dtc),
                DtcOrRef::OdxLink(_) => None,
            })
    }

    pub fn all_additional_audiences(
        &self,
    ) -> impl Iterator<Item = &crate::model::odx::AdditionalAudience> {
        self.collections
            .values()
            .flat_map(|c| c.additional_audience_store.iter())
    }
}


fn find_dtc_by_id<'a>(collection: &'a OdxCollection, id_ref: &str) -> Option<&'a crate::model::dop::Dtc> {
    collection.dtc_dop_store.iter().flat_map(|dop| dop.dtcs.iter()).find_map(|value| match value {
        crate::model::dop::DtcOrRef::Dtc(dtc) if dtc.id == id_ref => Some(dtc),
        _ => None,
    })
}

fn find_param_by_id<'a>(collection: &'a OdxCollection, id_ref: &str) -> Option<&'a crate::model::diag_service::Param> {
    collection.request_store.iter().flat_map(|r| r.params.iter())
        .chain(collection.response_store.iter().flat_map(|r| r.params.iter()))
        .chain(collection.structure_store.iter().flat_map(|r| r.params.iter()))
        .chain(collection.env_data_store.iter().flat_map(|r| r.params.iter()))
        .find(|p| param_id(p) == id_ref)
}

fn find_param_by_sn<'a>(collection: &'a OdxCollection, short_name: &str) -> Option<&'a crate::model::diag_service::Param> {
    collection.request_store.iter().flat_map(|r| r.params.iter())
        .chain(collection.response_store.iter().flat_map(|r| r.params.iter()))
        .chain(collection.structure_store.iter().flat_map(|r| r.params.iter()))
        .chain(collection.env_data_store.iter().flat_map(|r| r.params.iter()))
        .find(|p| p.short_name() == short_name)
}

fn param_id(param: &crate::model::diag_service::Param) -> &str {
    use crate::model::diag_service::Param::*;
    match param {
        Value(p) => &p.base.id,
        CodedConst(p) => &p.base.id,
        Dynamic(p) => &p.base.id,
        LengthKey(p) => &p.base.id,
        MatchingRequest(p) => &p.base.id,
        NrcConst(p) => &p.base.id,
        PhysConst(p) => &p.base.id,
        Reserved(p) => &p.base.id,
        System(p) => &p.base.id,
        TableKey(p) => &p.base.id,
        TableEntry(p) => &p.base.id,
        TableStruct(p) => &p.base.id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::diag_layer::EcuSharedData;
    use crate::model::dop::DataObjectProp;

    #[test]
    fn resolves_docref_that_names_a_contained_diag_layer() {
        let dop_id = "DLC.Alliance_Datas_Format.ESD.Datas_Format.DOP.IDE_2B_ByteField_None";
        let mut collection = OdxCollection::new(
            "Alliance_DatasFormat.odx-d".to_string(),
            "Alliance_Datas_Format".to_string(),
        );

        let mut shared_data = EcuSharedData::default();
        shared_data.core.short_name = "Datas_Format".to_string();
        collection.ecu_shared_data_store.push(shared_data);

        collection.data_object_props.insert(dop_id.to_string(), 0);
        collection.data_object_prop_store.push(DataObjectProp {
            id: dop_id.to_string(),
            short_name: "IDE_2B_ByteField_None".to_string(),
            long_name: None,
            sdgs: None,
            diag_coded_type: None,
            physical_type: None,
            compu_method: None,
            unit_ref: None,
            internal_constr: None,
            phys_constr: None,
        });

        let group = OdxCollectionGroup::new(vec![collection], 0);
        let link = OdxLink {
            id_ref: dop_id.to_string(),
            doc_ref: Some("Datas_Format".to_string()),
            doc_type: Some("LAYER".to_string()),
        };

        assert!(group.resolve_dop(&link, None).is_some());
    }
}
