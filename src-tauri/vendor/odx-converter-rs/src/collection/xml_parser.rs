// src/collection/xml_parser.rs – Parses a single ODX XML file into `OdxCollection`.
//
// Uses `roxmltree` for simple, namespace-aware XML traversal without the
// serde/xsi:type complexity of quick-xml.
//
// Each `parse_*` helper corresponds to one ODX element type; they mirror
// the JAXB-generated `schema.odx.*` classes from the Kotlin project.

use anyhow::{Context, Result};
use roxmltree::Node;
use log::{warn, debug};

use crate::collection::OdxCollection;
use crate::error::OdxError;
use crate::model::{
    odx::{AdditionalAudience, Audience, FunctClass, LongName, OdxLink, Sd, Sdg, Sdgs,
          SdOrSdg, Text},
    diag_layer::*,
    diag_service::*,
    dop::*,
    compu_method::*,
    unit::*,
    state::*,
    comparam::*,
};

// ─── Public entry point ──────────────────────────────────────────────────

/// Parse the XML content of a single `.odx` file into an `OdxCollection`.
pub fn parse_odx_file(xml: &str, source_file: &str) -> Result<OdxCollection> {
    let doc = roxmltree::Document::parse(xml)
        .with_context(|| format!("Failed to parse XML in '{}'", source_file))?;

    let root = doc.root_element(); // Should be <ODX>
    if root.tag_name().name() != "ODX" {
        return Err(OdxError::ParseError(format!(
            "'{}' root element is '{}', expected 'ODX'",
            source_file,
            root.tag_name().name()
        ))
        .into());
    }

    // Determine container key from first recognised top-level child.
    let container_key = root
        .children()
        .filter(|n| n.is_element())
        .find_map(|n| {
            child_text(&n, "SHORT-NAME").map(|sn| sn.to_owned())
        })
        .unwrap_or_else(|| source_file.to_string());

    let mut coll = OdxCollection::new(source_file.to_string(), container_key);

    // Parse each top-level container.
    for node in root.children().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "DIAG-LAYER-CONTAINER" => {
                let dlc = parse_diag_layer_container(node, &mut coll)?;
                coll.diag_layer_container = Some(dlc);
            }
            "COMPARAM-SUBSET" => {
                let cps = parse_comparam_subset(node, &mut coll)?;
                coll.comparam_subset = Some(cps);
            }
            "COMPARAM-SPEC" => {
                let spec = parse_comparam_spec(node, &mut coll)?;
                coll.comparam_spec = Some(spec);
            }
            other => {
                debug!("Skipping unsupported top-level element '{}' in '{}'", other, source_file);
            }
        }
    }

    Ok(coll)
}

// ─── DIAG-LAYER-CONTAINER ─────────────────────────────────────────────────

fn parse_diag_layer_container(
    node: Node,
    coll: &mut OdxCollection,
) -> Result<DiagLayerContainer> {
    let id = attr(&node, "ID").unwrap_or_default();
    let short_name = child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned();
    let long_name = parse_long_name_opt(node);
    let sdgs = parse_sdgs_opt(node);

    // Mirrors Kotlin's `ODXCollectionGroup.odxRevision`:
    //   ADMIN-DATA/DOC-REVISIONS/DOC-REVISION (the LAST one)/REVISION-LABEL
    coll.odx_revision = find_child(node, "ADMIN-DATA")
        .and_then(|admin| find_child(admin, "DOC-REVISIONS"))
        .and_then(|revs| {
            revs.children()
                .filter(|n| n.tag_name().name() == "DOC-REVISION")
                .last()
        })
        .and_then(|last_rev| child_text(&last_rev, "REVISION-LABEL"))
        .map(|s| s.to_owned());

    let mut ecu_variant_ids = Vec::new();
    let mut base_variant_ids = Vec::new();
    let mut protocol_ids = Vec::new();
    let mut functional_group_ids = Vec::new();
    let mut ecu_shared_data_ids = Vec::new();

    for child in node.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "ECU-VARIANTS" => {
                for ev in child.children().filter(|n| n.tag_name().name() == "ECU-VARIANT") {
                    let variant = parse_ecu_variant(ev, coll)?;
                    let variant_id = variant.core.id.clone();
                    let idx = coll.ecu_variant_store.len();
                    coll.ecu_variants.insert(variant_id.clone(), idx);
                    ecu_variant_ids.push(variant_id);
                    coll.ecu_variant_store.push(variant);
                }
            }
            "BASE-VARIANTS" => {
                for bv in child.children().filter(|n| n.tag_name().name() == "BASE-VARIANT") {
                    let variant = parse_base_variant(bv, coll)?;
                    let variant_id = variant.core.id.clone();
                    let idx = coll.base_variant_store.len();
                    coll.base_variants.insert(variant_id.clone(), idx);
                    base_variant_ids.push(variant_id);
                    coll.base_variant_store.push(variant);
                }
            }
            
            "PROTOCOLS" => {
                for p in child.children().filter(|n| n.tag_name().name() == "PROTOCOL") {
                    let proto = parse_protocol(p, coll)?;
                    let proto_id = proto.core.id.clone();
                    let sn = proto.core.short_name.clone();
                    let idx = coll.protocol_store.len();
                    coll.protocols.insert(proto_id.clone(), idx);
                    coll.protocols_by_sn.insert(sn, idx);
                    protocol_ids.push(proto_id);
                    coll.protocol_store.push(proto);
                }
            }
            "FUNCTIONAL-GROUPS" => {
                for fg in child.children().filter(|n| n.tag_name().name() == "FUNCTIONAL-GROUP") {
                    let group = parse_functional_group(fg, coll)?;
                    let group_id = group.core.id.clone();
                    let idx = coll.functional_group_store.len();
                    coll.functional_groups.insert(group_id.clone(), idx);
                    functional_group_ids.push(group_id);
                    coll.functional_group_store.push(group);
                }
            }
            "ECU-SHARED-DATAS" => {
                for es in child.children().filter(|n| n.tag_name().name() == "ECU-SHARED-DATA") {
                    let shared = parse_ecu_shared_data(es, coll)?;
                    let shared_id = shared.core.id.clone();
                    let idx = coll.ecu_shared_data_store.len();
                    coll.ecu_shared_datas.insert(shared_id.clone(), idx);
                    ecu_shared_data_ids.push(shared_id);
                    coll.ecu_shared_data_store.push(shared);
                }
            }
            _ => {}
        }
    }

    Ok(DiagLayerContainer {
        id,
        short_name,
        long_name,
        sdgs,
        ecu_variant_ids,
        base_variant_ids,
        protocol_ids,
        functional_group_ids,
        ecu_shared_data_ids,
    })
}

// ─── DIAG-LAYER core ──────────────────────────────────────────────────────

fn parse_diag_layer_core(node: Node, coll: &mut OdxCollection) -> Result<DiagLayerCore> {
    let id = attr(&node, "ID").unwrap_or_default();
    let short_name = child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned();
    let long_name = parse_long_name_opt(node);
    let sdgs = parse_sdgs_opt(node);

    let mut diag_comms = DiagComms::default();
    let mut diag_comms_present = false;
    let mut diag_data_dictionary_spec = None;
    let mut funct_class_sn = Vec::new();
    let mut additional_audience_ids = Vec::new();
    let mut state_chart_ids = Vec::new();
    let mut parent_refs = Vec::new();

    for child in node.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "DIAG-COMMS" => {
                diag_comms_present = true;
                parse_diag_comms_into(child, &mut diag_comms, coll)?;
            }
            "DIAG-DATA-DICTIONARY-SPEC" => {
                diag_data_dictionary_spec = Some(parse_diag_data_dictionary_spec(child, coll)?);
            }
            "FUNCT-CLASSS" => {
                for fc in child.children().filter(|n| n.tag_name().name() == "FUNCT-CLASS") {
                    let sn = child_text(&fc, "SHORT-NAME").unwrap_or_default().to_owned();
                    let funct_class = FunctClass {
                        id: attr(&fc, "ID").unwrap_or_default(),
                        short_name: sn.clone(),
                    };
                    let idx = coll.funct_class_store.len();
                    coll.funct_classes.insert(funct_class.id.clone(), idx);
                    funct_class_sn.push(sn);
                    coll.funct_class_store.push(funct_class);
                }
            }
            // inside parse_diag_layer_core's match child.tag_name().name() { ... }
            "REQUESTS" => {
                for req_node in child.children().filter(|n| n.tag_name().name() == "REQUEST") {
                    let req = parse_request(req_node);
                    let idx = coll.request_store.len();
                    coll.requests.insert(req.id.clone(), idx);
                    coll.request_store.push(req);
                }
            }
            "POS-RESPONSES" => {
                for resp_node in child.children().filter(|n| n.tag_name().name() == "POS-RESPONSE") {
                    let resp = parse_response(resp_node, ResponseKind::Positive); // also fix the kind, see below
                    let idx = coll.response_store.len();
                    coll.pos_responses.insert(resp.id.clone(), idx);
                    coll.response_store.push(resp);
                }
            }
            "NEG-RESPONSES" => {
                for resp_node in child.children().filter(|n| n.tag_name().name() == "NEG-RESPONSE") {
                    let resp = parse_response(resp_node, ResponseKind::Negative);
                    let idx = coll.response_store.len();
                    coll.neg_responses.insert(resp.id.clone(), idx);
                    coll.response_store.push(resp);
                }
            }
            "ADDITIONAL-AUDIENCES" => {
                for aa in child.children().filter(|n| n.tag_name().name() == "ADDITIONAL-AUDIENCE") {
                    let aud = AdditionalAudience {
                        id: attr(&aa, "ID").unwrap_or_default(),
                        short_name: child_text(&aa, "SHORT-NAME").unwrap_or_default().to_owned(),
                        long_name: parse_long_name_opt(aa),
                    };
                    let id_aa = aud.id.clone();
                    let idx = coll.additional_audience_store.len();
                    coll.additional_audiences.insert(aud.id.clone(), idx);
                    additional_audience_ids.push(id_aa);
                    coll.additional_audience_store.push(aud);
                }
            }
            "STATE-CHARTS" => {
                for sc in child.children().filter(|n| n.tag_name().name() == "STATE-CHART") {
                    let state_chart = parse_state_chart(sc, coll)?;
                    let sc_id = state_chart.id.clone();
                    let idx = coll.state_chart_store.len();
                    coll.state_charts.insert(state_chart.id.clone(), idx);
                    state_chart_ids.push(sc_id);
                    coll.state_chart_store.push(state_chart);
                }
            }
            "PARENT-REFS" => {
                for pr in child.children().filter(|n| n.tag_name().name() == "PARENT-REF") {
                    parent_refs.push(parse_parent_ref(pr));
                }
            }
            _ => {}
        }
    }

    Ok(DiagLayerCore {
        id,
        short_name,
        long_name,
        sdgs,
        diag_comms,
        diag_comms_present,
        diag_data_dictionary_spec,
        funct_classes: funct_class_sn,
        additional_audiences: additional_audience_ids,
        state_chart_ids,
        parent_refs,
    })
}

fn parse_diag_comms_into(
    node: Node,
    out: &mut DiagComms,
    coll: &mut OdxCollection,
) -> Result<()> {
    for child in node.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "DIAG-SERVICE" => {
                let svc = parse_diag_service(child, coll)?;
                let id = svc.comm.id.clone();
                let sn = svc.comm.short_name.clone();
                let idx = coll.diag_service_store.len();
                coll.diag_services.insert(id.clone(), idx);
                coll.diag_services_by_sn.insert(sn, idx);
                out.diag_service_ids.push(id);
                coll.diag_service_store.push(svc);
            }
            "SINGLE-ECU-JOB" => {
                let job = parse_single_ecu_job(child, coll)?;
                let id = job.comm.id.clone();
                let sn = job.comm.short_name.clone();
                let idx = coll.single_ecu_job_store.len();
                coll.single_ecu_jobs.insert(id.clone(), idx);
                coll.single_ecu_jobs_by_sn.insert(sn, idx);
                out.single_ecu_job_ids.push(id);
                coll.single_ecu_job_store.push(job);
            }
            "ODXLINK" => {
                out.odx_links.push(parse_odx_link(child));
            }
            _ => {}
        }
    }
    Ok(())
}

// ─── ECU-VARIANT / BASE-VARIANT / PROTOCOL / FUNCTIONAL-GROUP ─────────────

fn parse_ecu_variant(node: Node, coll: &mut OdxCollection) -> Result<EcuVariant> {
    let core = parse_diag_layer_core(node, coll)?;
    let comparam_refs = parse_comparam_refs(node);
    let ecu_variant_patterns = parse_ecu_variant_patterns(node);
    Ok(EcuVariant { core, comparam_refs, ecu_variant_patterns })
}

fn parse_base_variant(node: Node, coll: &mut OdxCollection) -> Result<BaseVariant> {
    let core = parse_diag_layer_core(node, coll)?;
    let comparam_refs = parse_comparam_refs(node);
    Ok(BaseVariant { core, comparam_refs, base_variant_pattern: None })
}

fn parse_protocol(node: Node, coll: &mut OdxCollection) -> Result<Protocol> {
    let core = parse_diag_layer_core(node, coll)?;
    let comparam_refs = parse_comparam_refs(node);
    let comparam_spec_ref = find_child(node, "COMPARAM-SPEC-REF").map(parse_odx_link);
    let prot_stack_snref = parse_short_name_ref(node, "PROT-STACK-SNREF");
    Ok(Protocol { core, comparam_refs, comparam_spec_ref, prot_stack_snref })
}

fn parse_functional_group(node: Node, coll: &mut OdxCollection) -> Result<FunctionalGroup> {
    let core = parse_diag_layer_core(node, coll)?;
    let comparam_refs = parse_comparam_refs(node);
    Ok(FunctionalGroup { core, comparam_refs })
}

fn parse_ecu_shared_data(node: Node, coll: &mut OdxCollection) -> Result<EcuSharedData> {
    let core = parse_diag_layer_core(node, coll)?;
    Ok(EcuSharedData { core })
}

fn parse_parent_refs(node: Node) -> Vec<ParentRef> {
    find_child(node, "PARENT-REFS")
        .map(|pr_node| {
            pr_node
                .children()
                .filter(|n| n.tag_name().name() == "PARENT-REF")
                .map(parse_parent_ref)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_parent_ref(node: Node) -> ParentRef {
    let doc_type = attr(&node, "DOCTYPE").and_then(|s| match s.to_uppercase().as_str() {
        "ECU-VARIANT"      | "ECUVARIANT"      => Some(ParentRefDocType::EcuVariant),
        "BASE-VARIANT"     | "BASEVARIANT"      => Some(ParentRefDocType::BaseVariant),
        "PROTOCOL"                              => Some(ParentRefDocType::Protocol),
        "FUNCTIONAL-GROUP" | "FUNCTIONALGROUP"  => Some(ParentRefDocType::FunctionalGroup),
        "ECU-SHARED-DATA"  | "ECUSHAREDDATA"    => Some(ParentRefDocType::EcuSharedData),
        "COMPARAM-SUBSET"  | "COMPARAMSUBSET"   => Some(ParentRefDocType::ComParamSubset),
        _ => None,
    });

    ParentRef {
        id_ref: attr(&node, "ID-REF").unwrap_or_default(),
        doc_ref: attr(&node, "DOCREF"),
        doc_type,
        not_inherited_diag_comm_short_names: parse_not_inherited(
            node, "NOT-INHERITED-DIAG-COMMS",
            "NOT-INHERITED-DIAG-COMM", "DIAG-COMM-SNREF",
        ),
        not_inherited_dop_short_names: parse_not_inherited(
            node, "NOT-INHERITED-DOPS",
            "NOT-INHERITED-DOP", "DOP-BASE-SNREF",
        ),
        not_inherited_table_short_names: parse_not_inherited(
            node, "NOT-INHERITED-TABLES",
            "NOT-INHERITED-TABLE", "TABLE-SNREF",
        ),
        not_inherited_variables_short_names: parse_not_inherited(
            node, "NOT-INHERITED-VARIABLES",
            "NOT-INHERITED-VARIABLE", "DIAG-VARIABLE-SNREF",
        ),
        not_inherited_global_neg_response_short_names: parse_not_inherited(
            node, "NOT-INHERITED-GLOBAL-NEG-RESPONSES",
            "NOT-INHERITED-GLOBAL-NEG-RESPONSE", "GLOBAL-NEG-RESPONSE-SNREF",
        ),
    }
}

/// Extract short-names from `PARENT-REF > <container> > <item> > <snref_elem SHORT-NAME="...">`.
fn parse_not_inherited(
    parent_ref_node: Node,
    container: &str,
    item: &str,
    snref_elem: &str,
) -> Vec<String> {
    find_child(parent_ref_node, container)
        .map(|c_node| {
            c_node
                .children()
                .filter(|n| n.tag_name().name() == item)
                .filter_map(|item_node| {
                    find_child(item_node, snref_elem)
                        .and_then(|snref| snref.attribute("SHORT-NAME").map(|s| s.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_ecu_variant_patterns(node: Node) -> Vec<EcuVariantPattern> {
    find_child(node, "ECU-VARIANT-PATTERNS")
        .map(|evp| {
            evp.children()
                .filter(|n| n.tag_name().name() == "ECU-VARIANT-PATTERN")
                .map(|pat| EcuVariantPattern {
                    matching_parameters: parse_matching_parameters(pat),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_matching_parameters(node: Node) -> Vec<MatchingParameter> {
    find_child(node, "MATCHING-PARAMETERS")
        .map(|mp| {
            mp.children()
                .filter(|n| n.tag_name().name() == "MATCHING-PARAMETER")
                .map(|p| MatchingParameter {
                    diag_comm_snref: find_child(p, "DIAG-COMM-SNREF")
                        .and_then(|n| n.attribute("SHORT-NAME").map(|s| s.to_owned()))
                        .unwrap_or_default(),
                    expected_value: child_text(&p, "EXPECTED-VALUE").map(|s| s.to_owned()),
                    out_param_if_snref: find_child(p, "OUT-PARAM-IF-SNREF")
                        .and_then(|n| n.attribute("SHORT-NAME").map(|s| s.to_owned())),
                    use_physical_addressing: bool_attr(&p, "USE-PHYSICAL-ADDRESSING"),
                })
                .collect()
        })
        .unwrap_or_default()
}

// ─── DIAG-SERVICE ─────────────────────────────────────────────────────────

fn parse_diag_service(node: Node, coll: &mut OdxCollection) -> Result<DiagService> {
    let comm = parse_diag_comm_core(node, coll)?;

    let request_ref = find_child(node, "REQUEST-REF").map(parse_odx_link);
    let pos_response_refs = find_child(node, "POS-RESPONSE-REFS")
        .map(|n| {
            n.children()
                .filter(|c| c.tag_name().name() == "POS-RESPONSE-REF")
                .map(parse_odx_link)
                .collect()
        })
        .unwrap_or_default();
    let neg_response_refs = find_child(node, "NEG-RESPONSE-REFS")
        .map(|n| {
            n.children()
                .filter(|c| c.tag_name().name() == "NEG-RESPONSE-REF")
                .map(parse_odx_link)
                .collect()
        })
        .unwrap_or_default();

    Ok(DiagService {
        comm,
        request_ref,
        pos_response_refs,
        neg_response_refs,
        comparam_refs: parse_comparam_refs(node),
        // ODX 2.2 defaults ADDRESSING to PHYSICAL and TRANSMISSION-MODE to
        // SEND-AND-RECEIVE when the attributes are absent. JAXB applies these
        // defaults in the Kotlin converter, so Rust must do the same explicitly.
        addressing: Some(match attr(&node, "ADDRESSING").as_deref() {
            Some("FUNCTIONAL") => Addressing::Functional,
            Some("FUNCTIONAL-OR-PHYSICAL") | Some("PHYSICAL-OR-FUNCTIONAL") => {
                Addressing::PhysicalOrFunctional
            }
            Some("PHYSICAL") | None => Addressing::Physical,
            Some(_) => Addressing::Physical,
        }),
        transmission_mode: Some(match attr(&node, "TRANSMISSION-MODE").as_deref() {
            Some("SEND-ONLY") => TransmissionMode::SendOnly,
            Some("RECEIVE-ONLY") => TransmissionMode::ReceiveOnly,
            Some("SEND-OR-RECEIVE") => TransmissionMode::SendOrReceive,
            Some("SEND-AND-RECEIVE") | None => TransmissionMode::SendAndReceive,
            Some(_) => TransmissionMode::SendAndReceive,
        }),
        is_cyclic: bool_attr(&node, "IS-CYCLIC"),
        is_multiple: bool_attr(&node, "IS-MULTIPLE"),
    })
}
fn parse_request(node: Node) -> Request {
    Request {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        sdgs: parse_sdgs_opt(node),
        params: parse_params(node),
    }
}

fn parse_response(node: Node, kind: ResponseKind) -> Response {
    Response {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        kind,
        sdgs: parse_sdgs_opt(node),
        params: parse_params(node),
    }
}

fn parse_diag_comm_core(node: Node, _coll: &mut OdxCollection) -> Result<DiagCommCore> {
    Ok(DiagCommCore {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        long_name: parse_long_name_opt(node),
        sdgs: parse_sdgs_opt(node),
        semantic: attr(&node, "SEMANTIC"),
        diag_class: attr(&node, "DIAGNOSTIC-CLASS").and_then(|value| match value.as_str() {
            "STARTCOMM" => Some(DiagClassType::StartComm),
            "STOPCOMM" => Some(DiagClassType::StopComm),
            "VARIANTIDENTIFICATION" => Some(DiagClassType::VariantIdentification),
            "READ-DYN-DEF-MESSAGE" => Some(DiagClassType::ReadDynDefMessage),
            "DYN-DEF-MESSAGE" => Some(DiagClassType::DynDefMessage),
            "CLEAR-DYN-DEF-MESSAGE" => Some(DiagClassType::ClearDynDefMessage),
            _ => None,
        }),
        funct_class_refs: parse_odx_link_list(node, "FUNCT-CLASS-REFS", "FUNCT-CLASS-REF"),
        precondition_state_refs: parse_precondition_state_refs(node),
        state_transition_refs: parse_state_transition_refs(node),
        protocol_snrefs: find_child(node, "PROTOCOL-SNREFS")
            .map(|wrapper| {
                wrapper.children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "PROTOCOL-SNREF")
                    .filter_map(|n| n.attribute("SHORT-NAME").map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        audience: parse_audience_opt(node),
        is_final: bool_attr(&node, "IS-FINAL"),
        is_mandatory: bool_attr(&node, "IS-MANDATORY"),
        // ODX defaults IS-EXECUTABLE to true when the attribute is absent.
        is_executable: bool_attr_default(&node, "IS-EXECUTABLE", true),
    })
}

fn parse_single_ecu_job(node: Node, coll: &mut OdxCollection) -> Result<SingleEcuJob> {
    let comm = parse_diag_comm_core(node, coll)?;
    let prog_codes = find_child(node, "PROG-CODES")
        .map(|n| {
            n.children()
                .filter(|c| c.tag_name().name() == "PROG-CODE")
                .map(parse_prog_code)
                .collect()
        })
        .unwrap_or_default();
    Ok(SingleEcuJob {
        comm,
        prog_codes,
        input_params: Vec::new(),
        output_params: Vec::new(),
        neg_output_params: Vec::new(),
    })
}

fn parse_prog_code(node: Node) -> ProgCode {
    ProgCode {
        code_file: child_text(&node, "CODE-FILE").map(|s| s.to_owned()),
        encryption: child_text(&node, "ENCRYPTION").map(|s| s.to_owned()),
        syntax: child_text(&node, "SYNTAX").map(|s| s.to_owned()),
        revision: child_text(&node, "REVISION").map(|s| s.to_owned()),
        entrypoint: child_text(&node, "ENTRYPOINT").map(|s| s.to_owned()),
        library_refs: find_child(node, "LIBRARY-REFS")
            .map(|n| {
                n.children()
                    .filter(|c| c.tag_name().name() == "LIBRARY-REF")
                    .map(parse_odx_link)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

// ─── DIAG-DATA-DICTIONARY-SPEC ────────────────────────────────────────────

fn parse_diag_data_dictionary_spec(
    node: Node,
    coll: &mut OdxCollection,
) -> Result<DiagDataDictionarySpec> {
    let mut spec = DiagDataDictionarySpec::default();

    for child in node.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "DATA-OBJECT-PROPS" => {
                for dop_node in child.children().filter(|n| n.tag_name().name() == "DATA-OBJECT-PROP") {
                    let dop = parse_data_object_prop(dop_node)?;
                    let id = dop.id.clone();
                    let sn = dop.short_name.clone();
                    let idx = coll.data_object_prop_store.len();
                    coll.data_object_props.insert(id.clone(), idx);
                    coll.data_object_props_by_sn.insert(sn, idx);
                    spec.data_object_prop_ids.push(id);
                    coll.data_object_prop_store.push(dop);
                }
            }
            "DTC-DOPS" => {
                for n in child.children().filter(|n| n.tag_name().name() == "DTC-DOP") {
                    let dtc_dop = parse_dtc_dop(n)?;
                    let id = dtc_dop.id.clone();
                    let idx = coll.dtc_dop_store.len();
                    coll.dtc_dops.insert(id.clone(), idx);
                    spec.dtc_dop_ids.push(id);
                    coll.dtc_dop_store.push(dtc_dop);
                }
            }
            "STRUCTURES" => {
                for n in child.children().filter(|n| n.tag_name().name() == "STRUCTURE") {
                    let value = parse_structure(n, coll)?;
                    let id = value.id.clone();
                    let sn = value.short_name.clone();
                    let idx = coll.structure_store.len();
                    coll.structures.insert(id.clone(), idx);
                    coll.structures_by_sn.insert(sn, idx);
                    spec.structure_ids.push(id);
                    coll.structure_store.push(value);
                }
            }
            "STATIC-FIELDS" => {
                for n in child.children().filter(|n| n.tag_name().name() == "STATIC-FIELD") {
                    let value = parse_static_field(n);
                    let id = value.id.clone();
                    let idx = coll.static_field_store.len();
                    coll.static_fields.insert(id.clone(), idx);
                    spec.static_field_ids.push(id);
                    coll.static_field_store.push(value);
                }
            }
            "END-OF-PDU-FIELDS" => {
                for n in child.children().filter(|n| n.tag_name().name() == "END-OF-PDU-FIELD") {
                    let value = parse_end_of_pdu_field(n);
                    let id = value.id.clone();
                    let idx = coll.end_of_pdu_field_store.len();
                    coll.end_of_pdu_fields.insert(id.clone(), idx);
                    spec.end_of_pdu_field_ids.push(id);
                    coll.end_of_pdu_field_store.push(value);
                }
            }
            "DYNAMIC-LENGTH-FIELDS" => {
                for n in child.children().filter(|n| n.tag_name().name() == "DYNAMIC-LENGTH-FIELD") {
                    let value = parse_dynamic_length_field(n);
                    let id = value.id.clone();
                    let idx = coll.dynamic_length_field_store.len();
                    coll.dynamic_length_fields.insert(id.clone(), idx);
                    spec.dynamic_length_field_ids.push(id);
                    coll.dynamic_length_field_store.push(value);
                }
            }
            "MUXS" => {
                for n in child.children().filter(|n| n.tag_name().name() == "MUX") {
                    let value = parse_mux(n);
                    let id = value.id.clone();
                    let idx = coll.mux_store.len();
                    coll.muxs.insert(id.clone(), idx);
                    spec.mux_ids.push(id);
                    coll.mux_store.push(value);
                }
            }
            "ENV-DATAS" => {
                for n in child.children().filter(|n| n.tag_name().name() == "ENV-DATA") {
                    let value = parse_env_data(n);
                    let id = value.id.clone();
                    let idx = coll.env_data_store.len();
                    coll.env_datas.insert(id.clone(), idx);
                    spec.env_data_ids.push(id);
                    coll.env_data_store.push(value);
                }
            }
            "ENV-DATA-DESCS" => {
                for n in child.children().filter(|n| n.tag_name().name() == "ENV-DATA-DESC") {
                    let value = parse_env_data_desc(n);
                    let id = value.id.clone();
                    let sn = value.short_name.clone();
                    let idx = coll.env_data_desc_store.len();
                    coll.env_data_descs.insert(id.clone(), idx);
                    coll.env_data_descs_by_sn.insert(sn, idx);
                    spec.env_data_desc_ids.push(id);
                    coll.env_data_desc_store.push(value);
                }
            }
            "TABLES" => {
                for n in child.children().filter(|n| n.tag_name().name() == "TABLE") {
                    let value = parse_table(n, coll)?;
                    let id = value.id.clone();
                    let sn = value.short_name.clone();
                    let idx = coll.table_store.len();
                    coll.tables.insert(id.clone(), idx);
                    coll.tables_by_sn.insert(sn, idx);
                    spec.table_ids.push(id);
                    coll.table_store.push(value);
                }
            }
            "UNIT-SPEC" => {
                spec.unit_spec = Some(parse_unit_spec(child, coll));
            }
            _ => {}
        }
    }

    Ok(spec)
}

// ─── DOP types ────────────────────────────────────────────────────────────

fn parse_data_object_prop(node: Node) -> Result<DataObjectProp> {
    Ok(DataObjectProp {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        long_name: parse_long_name_opt(node),
        sdgs: parse_sdgs_opt(node),
        diag_coded_type: find_child(node, "DIAG-CODED-TYPE").map(parse_diag_coded_type),
        physical_type: find_child(node, "PHYSICAL-TYPE").map(parse_physical_type),
        compu_method: find_child(node, "COMPU-METHOD").map(parse_compu_method),
        unit_ref: find_child(node, "UNIT-REF").map(parse_odx_link),
        internal_constr: find_child(node, "INTERNAL-CONSTR").map(parse_internal_constr),
        phys_constr: find_child(node, "PHYS-CONSTR").map(parse_internal_constr),
    })
}

fn parse_dtc_dop(node: Node) -> Result<DtcDop> {
    let diag_coded_type = find_child(node, "DIAG-CODED-TYPE")
        .map(parse_diag_coded_type)
        .ok_or_else(|| OdxError::ParseError("DTC-DOP missing DIAG-CODED-TYPE".into()))?;
    let physical_type = find_child(node, "PHYSICAL-TYPE")
        .map(parse_physical_type)
        .ok_or_else(|| OdxError::ParseError("DTC-DOP missing PHYSICAL-TYPE".into()))?;
    let compu_method = find_child(node, "COMPU-METHOD")
        .map(parse_compu_method)
        .ok_or_else(|| OdxError::ParseError("DTC-DOP missing COMPU-METHOD".into()))?;

    let dtcs = find_child(node, "DTCS")
        .map(|n| {
            n.children()
                .filter(|c| c.is_element())
                .map(|c| match c.tag_name().name() {
                    "DTC" => DtcOrRef::Dtc(parse_dtc(c)),
                    _ => DtcOrRef::OdxLink(parse_odx_link(c)),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(DtcDop {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        long_name: parse_long_name_opt(node),
        sdgs: parse_sdgs_opt(node),
        diag_coded_type,
        physical_type,
        compu_method,
        dtcs,
        is_visible: bool_attr(&node, "IS-VISIBLE"),
    })
}

fn parse_dtc(node: Node) -> Dtc {
    Dtc {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        sdgs: parse_sdgs_opt(node),
        trouble_code: child_text(&node, "TROUBLE-CODE")
            .and_then(parse_u32_auto)
            .unwrap_or(0),
        display_trouble_code: child_text(&node, "DISPLAY-TROUBLE-CODE").map(|s| s.to_owned()),
        text: find_child(node, "TEXT").map(parse_text),
        level: child_text(&node, "LEVEL").and_then(|s| s.parse().ok()),
        is_temporary: bool_attr(&node, "IS-TEMPORARY"),
    }
}

fn parse_structure(node: Node, coll: &mut OdxCollection) -> Result<Structure> {
    let params = parse_params(node);
    Ok(Structure {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        long_name: parse_long_name_opt(node),
        sdgs: parse_sdgs_opt(node),
        byte_size: child_text(&node, "BYTE-SIZE").and_then(|s| s.parse().ok()),
        params,
        is_visible: bool_attr_default(&node, "IS-VISIBLE", true),
    })
}

fn parse_field(node: Node) -> Field {
    Field {
        basic_structure_ref: find_child(node, "BASIC-STRUCTURE-REF").map(parse_odx_link),
        basic_structure_snref: find_child(node, "BASIC-STRUCTURE-SNREF")
            .and_then(|n| n.attribute("SHORT-NAME").map(str::to_owned)),
        env_data_desc_ref: find_child(node, "ENV-DATA-DESC-REF").map(parse_odx_link),
        env_data_desc_snref: find_child(node, "ENV-DATA-DESC-SNREF")
            .and_then(|n| n.attribute("SHORT-NAME").map(str::to_owned)),
        is_visible: bool_attr_default(&node, "IS-VISIBLE", true),
    }
}

fn parse_static_field(node: Node) -> StaticField {
    StaticField {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        sdgs: parse_sdgs_opt(node),
        field: parse_field(node),
        fixed_number_of_items: child_text(&node, "FIXED-NUMBER-OF-ITEMS")
            .and_then(|s| s.parse().ok()).unwrap_or(0),
        item_byte_size: child_text(&node, "ITEM-BYTE-SIZE")
            .and_then(|s| s.parse().ok()).unwrap_or(0),
    }
}

fn parse_end_of_pdu_field(node: Node) -> EndOfPduField {
    EndOfPduField {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        sdgs: parse_sdgs_opt(node),
        field: parse_field(node),
        max_number_of_items: child_text(&node, "MAX-NUMBER-OF-ITEMS").and_then(|s| s.parse().ok()),
        min_number_of_items: child_text(&node, "MIN-NUMBER-OF-ITEMS").and_then(|s| s.parse().ok()),
    }
}

fn parse_env_data(node: Node) -> EnvData {
    let dtc_values = find_child(node, "DTC-VALUES")
        .map(|wrapper| wrapper.children()
            .filter(|n| n.is_element() && n.tag_name().name() == "DTC-VALUE")
            .filter_map(|n| n.text().and_then(|s| parse_u32_auto(s.trim())))
            .collect())
        .unwrap_or_default();
    EnvData {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        dtc_values,
        params: parse_params(node),
    }
}

fn parse_env_data_desc(node: Node) -> EnvDataDesc {
    EnvDataDesc {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        env_data_refs: parse_odx_link_list(node, "ENV-DATA-REFS", "ENV-DATA-REF"),
        param_snref: find_child(node, "PARAM-SNREF")
            .and_then(|n| n.attribute("SHORT-NAME").map(str::to_owned)),
        param_sn_pathref: find_child(node, "PARAM-SNPATHREF")
            .and_then(|n| n.attribute("SHORT-NAME-PATH").or_else(|| n.attribute("SHORT-NAME")))
            .map(str::to_owned),
    }
}

fn parse_dynamic_length_field(node: Node) -> DynamicLengthField {
    let determine_node = find_child(node, "DETERMINE-NUMBER-OF-ITEMS");
    let determine_number_of_items = DetermineNumberOfItems {
        byte_position: determine_node.and_then(|n| child_text(&n, "BYTE-POSITION"))
            .and_then(|s| s.parse().ok()).unwrap_or(0),
        bit_position: determine_node.and_then(|n| child_text(&n, "BIT-POSITION"))
            .and_then(|s| s.parse().ok()),
        data_object_prop_ref: determine_node
            .and_then(|n| find_child(n, "DATA-OBJECT-PROP-REF"))
            .map(parse_odx_link),
    };
    DynamicLengthField {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        field: parse_field(node),
        offset: child_text(&node, "OFFSET").and_then(|s| s.parse().ok()).unwrap_or(0),
        determine_number_of_items,
    }
}

fn parse_mux(node: Node) -> Mux {
    let switch_node = find_child(node, "SWITCH-KEY");
    let switch_key = SwitchKey {
        byte_position: switch_node.and_then(|n| child_text(&n, "BYTE-POSITION"))
            .and_then(|s| s.parse().ok()).unwrap_or(0),
        bit_position: switch_node.and_then(|n| child_text(&n, "BIT-POSITION"))
            .and_then(|s| s.parse().ok()),
        data_object_prop_ref: switch_node
            .and_then(|n| find_child(n, "DATA-OBJECT-PROP-REF"))
            .map(parse_odx_link).unwrap_or_else(|| OdxLink::new("")),
    };
    let default_case = find_child(node, "DEFAULT-CASE").map(|n| DefaultCase {
        short_name: child_text(&n, "SHORT-NAME").unwrap_or_default().to_owned(),
        long_name: parse_long_name_opt(n),
        structure_ref: find_child(n, "STRUCTURE-REF").map(parse_odx_link),
    });
    let cases = find_child(node, "CASES").map(|wrapper| {
        wrapper.children().filter(|n| n.is_element() && n.tag_name().name() == "CASE")
            .map(|n| Case {
                short_name: child_text(&n, "SHORT-NAME").unwrap_or_default().to_owned(),
                long_name: parse_long_name_opt(n),
                lower_limit: find_child(n, "LOWER-LIMIT").map(parse_limit).unwrap_or(Limit { value: None, interval_type: None }),
                upper_limit: find_child(n, "UPPER-LIMIT").map(parse_limit).unwrap_or(Limit { value: None, interval_type: None }),
                structure_ref: find_child(n, "STRUCTURE-REF").map(parse_odx_link),
                structure_snref: find_child(n, "STRUCTURE-SNREF")
                    .and_then(|v| v.attribute("SHORT-NAME").map(str::to_owned)),
            }).collect()
    }).unwrap_or_default();
    Mux {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        is_visible: bool_attr_default(&node, "IS-VISIBLE", false),
        byte_position: child_text(&node, "BYTE-POSITION").and_then(|s| s.parse().ok()).unwrap_or(0),
        switch_key,
        default_case,
        cases,
    }
}

fn parse_unit_spec(node: Node, coll: &mut OdxCollection) -> UnitSpec {
    let mut result = UnitSpec::default();
    if let Some(wrapper) = find_child(node, "PHYSICAL-DIMENSIONS") {
        for n in wrapper.children().filter(|n| n.is_element() && n.tag_name().name() == "PHYSICAL-DIMENSION") {
            let pd = PhysicalDimension {
                id: attr(&n, "ID").unwrap_or_default(),
                short_name: child_text(&n, "SHORT-NAME").unwrap_or_default().to_owned(),
                long_name: parse_long_name_opt(n),
                length_exp: child_text(&n, "LENGTH-EXP").and_then(|s| s.parse().ok()),
                mass_exp: child_text(&n, "MASS-EXP").and_then(|s| s.parse().ok()),
                time_exp: child_text(&n, "TIME-EXP").and_then(|s| s.parse().ok()),
                current_exp: child_text(&n, "CURRENT-EXP").and_then(|s| s.parse().ok()),
                temperature_exp: child_text(&n, "TEMPERATURE-EXP").and_then(|s| s.parse().ok()),
                molar_amount_exp: child_text(&n, "MOLAR-AMOUNT-EXP").and_then(|s| s.parse().ok()),
                luminous_intensity_exp: child_text(&n, "LUMINOUS-INTENSITY-EXP").and_then(|s| s.parse().ok()),
            };
            let idx = coll.physical_dimension_store.len();
            coll.physical_dimensions.insert(pd.id.clone(), idx);
            coll.physical_dimension_store.push(pd);
        }
    }
    if let Some(wrapper) = find_child(node, "UNITS") {
        for n in wrapper.children().filter(|n| n.is_element() && n.tag_name().name() == "UNIT") {
            let unit = Unit {
                id: attr(&n, "ID").unwrap_or_default(),
                short_name: child_text(&n, "SHORT-NAME").unwrap_or_default().to_owned(),
                display_name: child_text(&n, "DISPLAY-NAME").unwrap_or_default().to_owned(),
                factor_si_to_unit: child_text(&n, "FACTOR-SI-TO-UNIT").and_then(|s| s.parse().ok()),
                offset_si_to_unit: child_text(&n, "OFFSET-SI-TO-UNIT").and_then(|s| s.parse().ok()),
                physical_dimension_ref: find_child(n, "PHYSICAL-DIMENSION-REF").map(parse_odx_link),
            };
            let idx = coll.unit_store.len();
            coll.units.insert(unit.id.clone(), idx);
            coll.unit_store.push(unit);
        }
    }
    if let Some(wrapper) = find_child(node, "UNIT-GROUPS") {
        result.unit_groups = wrapper.children()
            .filter(|n| n.is_element() && n.tag_name().name() == "UNIT-GROUP")
            .map(|n| UnitGroup {
                short_name: child_text(&n, "SHORT-NAME").unwrap_or_default().to_owned(),
                long_name: parse_long_name_opt(n),
                category: attr(&n, "CATEGORY").or_else(|| child_text(&n, "CATEGORY").map(str::to_owned)),
                unit_refs: parse_odx_link_list(n, "UNIT-REFS", "UNIT-REF"),
            }).collect();
    }
    result.units = coll.unit_store.iter().map(|u| Unit {
        id: u.id.clone(), short_name: u.short_name.clone(), display_name: u.display_name.clone(),
        factor_si_to_unit: u.factor_si_to_unit, offset_si_to_unit: u.offset_si_to_unit,
        physical_dimension_ref: u.physical_dimension_ref.clone(),
    }).collect();
    result.physical_dimensions = coll.physical_dimension_store.iter().map(|pd| PhysicalDimension {
        id: pd.id.clone(), short_name: pd.short_name.clone(), long_name: pd.long_name.clone(),
        length_exp: pd.length_exp, mass_exp: pd.mass_exp, time_exp: pd.time_exp,
        current_exp: pd.current_exp, temperature_exp: pd.temperature_exp,
        molar_amount_exp: pd.molar_amount_exp, luminous_intensity_exp: pd.luminous_intensity_exp,
    }).collect();
    result.sdgs = parse_sdgs_opt(node);
    result
}

fn parse_table(node: Node, coll: &mut OdxCollection) -> Result<Table> {
    let mut rows = Vec::new();
    let rows_parent = find_child(node, "TABLE-ROWS")
        .or_else(|| find_child(node, "ROW-WRAPPER"))
        .unwrap_or(node);
    for r in rows_parent.children().filter(|n| n.is_element()) {
        let row = match r.tag_name().name() {
            "TABLE-ROW" => {
                let tr = parse_table_row(r);
                let idx = coll.table_row_store.len();
                let sn = tr.short_name.clone();
                coll.table_rows.insert(tr.id.clone(), idx);
                coll.table_rows_by_sn.insert(sn, idx);
                coll.table_row_store.push(tr.clone());
                TableRowOrLink::Row(tr)
            }
            "TABLE-ROW-REF" => TableRowOrLink::OdxLink(parse_odx_link(r)),
            _ => continue,
        };
        rows.push(row);
    }

    let diag_comm_connectors = find_child(node, "DIAG-COMM-CONNECTORS")
        .map(|wrapper| wrapper.children()
            .filter(|n| n.is_element() && n.tag_name().name() == "DIAG-COMM-CONNECTOR")
            .map(|n| TableDiagCommConnector {
                semantic: attr(&n, "SEMANTIC").or_else(|| child_text(&n, "SEMANTIC").map(str::to_owned)),
                diag_comm_ref: find_child(n, "DIAG-COMM-REF").map(parse_odx_link),
                diag_comm_snref: find_child(n, "DIAG-COMM-SNREF")
                    .and_then(|v| v.attribute("SHORT-NAME").map(str::to_owned)),
            }).collect())
        .unwrap_or_default();

    Ok(Table {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        long_name: parse_long_name_opt(node),
        sdgs: parse_sdgs_opt(node),
        semantic: attr(&node, "SEMANTIC").or_else(|| child_text(&node, "SEMANTIC").map(str::to_owned)),
        key_label: child_text(&node, "KEY-LABEL").map(str::to_owned),
        struct_label: child_text(&node, "STRUCT-LABEL").map(str::to_owned),
        key_dop_ref: find_child(node, "KEY-DOP-REF").map(parse_odx_link),
        rows,
        diag_comm_connectors,
    })
}

fn parse_table_row(node: Node) -> TableRow {
    TableRow {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        long_name: parse_long_name_opt(node),
        sdgs: parse_sdgs_opt(node),
        semantic: attr(&node, "SEMANTIC").or_else(|| child_text(&node, "SEMANTIC").map(str::to_owned)),
        audience: parse_audience_opt(node),
        key: child_text(&node, "KEY").map(str::to_owned),
        dop_ref: find_child(node, "DATA-OBJECT-PROP-REF").map(parse_odx_link),
        dop_snref: find_child(node, "DATA-OBJECT-PROP-SNREF")
            .and_then(|n| n.attribute("SHORT-NAME").map(str::to_owned)),
        structure_ref: find_child(node, "STRUCTURE-REF").map(parse_odx_link),
        structure_snref: find_child(node, "STRUCTURE-SNREF")
            .and_then(|n| n.attribute("SHORT-NAME").map(str::to_owned)),
        funct_class_refs: parse_odx_link_list(node, "FUNCT-CLASS-REFS", "FUNCT-CLASS-REF"),
        state_transition_refs: parse_state_transition_refs(node),
        precondition_state_refs: parse_precondition_state_refs(node),
        is_executable: bool_attr_default(&node, "IS-EXECUTABLE", true),
        is_mandatory: bool_attr(&node, "IS-MANDATORY"),
        is_final: bool_attr(&node, "IS-FINAL"),
        numeric_id: 0,
        cells: Vec::new(),
        row_type: IntervalType::default(),
    }
}

// ─── PARAMs ───────────────────────────────────────────────────────────────

fn parse_params(node: Node) -> Vec<Param> {
    let mut params = Vec::new();
    let params_node = match find_child(node, "PARAMS") {
        Some(n) => n,
        None => return params,
    };

    for p in params_node.children().filter(|n| n.is_element()) {
        let xsi_type = p
            .attribute(("http://www.w3.org/2001/XMLSchema-instance", "type"))
            .or_else(|| p.attribute("xsi:type"))
            .unwrap_or(p.tag_name().name());

        let base = ParamBase {
            id: attr(&p, "ID").unwrap_or_default(),
            short_name: child_text(&p, "SHORT-NAME").unwrap_or_default().to_owned(),
            semantic: attr(&p, "SEMANTIC"),
            sdgs: parse_sdgs_opt(p),
            byte_position: child_text(&p, "BYTE-POSITION").and_then(|s| s.parse().ok()),
            bit_position: child_text(&p, "BIT-POSITION").and_then(|s| s.parse().ok()),
        };

        let param = match xsi_type {
            "VALUE" | "PARAM" => Param::Value(ValueParam {
                dop_ref: find_child(p, "DOP-REF").map(parse_odx_link),
                dop_snref: find_child(p, "DOP-SNREF")
                    .and_then(|n| n.attribute("SHORT-NAME").map(|s| s.to_owned())),
                physical_default_value: child_text_preserve_empty(p, "PHYSICAL-DEFAULT-VALUE"),
                base,
            }),
            "CODED-CONST" => Param::CodedConst(CodedConstParam {
                diag_coded_type: find_child(p, "DIAG-CODED-TYPE").map(parse_diag_coded_type),
                coded_value: child_text(&p, "CODED-VALUE").map(|s| s.to_owned()),
                base,
            }),
            "DYNAMIC" => Param::Dynamic(DynamicParam { base }),
            "LENGTH-KEY" => Param::LengthKey(LengthKeyParam {
                dop_ref: find_child(p, "DOP-REF").map(parse_odx_link),
                dop_snref: find_child(p, "DOP-SNREF")
                    .and_then(|n| n.attribute("SHORT-NAME").map(|s| s.to_owned())),
                base,
            }),
            "MATCHING-REQUEST-PARAM" => Param::MatchingRequest(MatchingRequestParam {
                request_byte_pos: child_text(&p, "REQUEST-BYTE-POS")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                byte_length: child_text(&p, "BYTE-LENGTH")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                base,
            }),
            "NRC-CONST" => Param::NrcConst(NrcConstParam {
                diag_coded_type: find_child(p, "DIAG-CODED-TYPE").map(parse_diag_coded_type),
                coded_values: find_child(p, "CODED-VALUES")
                    .map(|n| {
                        n.children()
                            .filter(|c| c.is_element() && c.tag_name().name() == "CODED-VALUE")
                            .filter_map(|c| c.text().map(|s| s.trim().to_owned()))
                            .collect()
                    })
                    .unwrap_or_default(),
                base,
            }),
            "PHYS-CONST" => Param::PhysConst(PhysConstParam {
                dop_ref: find_child(p, "DOP-REF").map(parse_odx_link),
                dop_snref: find_child(p, "DOP-SNREF")
                    .and_then(|n| n.attribute("SHORT-NAME").map(|s| s.to_owned())),
                phys_constant_value: child_text(&p, "PHYS-CONSTANT-VALUE").map(|s| s.to_owned()),
                base,
            }),
            "RESERVED" => Param::Reserved(ReservedParam {
                bit_length: child_text(&p, "BIT-LENGTH")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                base,
            }),
            "SYSTEM" => Param::System(SystemParam {
                sys_param: child_text(&p, "SYSPARAM").unwrap_or_default().to_owned(),
                dop_ref: find_child(p, "DOP-REF").map(parse_odx_link),
                dop_snref: find_child(p, "DOP-SNREF")
                    .and_then(|n| n.attribute("SHORT-NAME").map(|s| s.to_owned())),
                base,
            }),
            "TABLE-KEY" => {
                let table_ref = find_child(p, "TABLE-REF")
                    .map(parse_odx_link)
                    .map(TableKeyRef::OdxLink)
                    .or_else(|| {
                        find_child(p, "TABLE-ROW-REF")
                            .map(parse_odx_link)
                            .map(TableKeyRef::OdxLink)
                    })
                    .or_else(|| {
                        find_child(p, "TABLE-SNREF")
                            .and_then(|n| n.attribute("SHORT-NAME"))
                            .map(|sn| TableKeyRef::TableSnRef(sn.to_owned()))
                    })
                    .or_else(|| {
                        find_child(p, "TABLE-ROW-SNREF")
                            .and_then(|n| n.attribute("SHORT-NAME"))
                            .map(|sn| TableKeyRef::TableRowSnRef(sn.to_owned()))
                    });
                Param::TableKey(TableKeyParam { base, table_ref })
            },
            "TABLE-ENTRY" => Param::TableEntry(TableEntryParam {
                table_row_ref: find_child(p, "TABLE-ROW-REF").map(parse_odx_link),
                target: child_text(&p, "TARGET").map(|s| s.to_owned()),
                base,
            }),
            "TABLE-STRUCT" => Param::TableStruct(TableStructParam {
                table_key_ref: find_child(p, "TABLE-KEY-REF").map(parse_odx_link),
                table_key_snref: find_child(p, "TABLE-KEY-SNREF")
                    .and_then(|n| n.attribute("SHORT-NAME").map(|s| s.to_owned())),
                base,
            }),
            other => {
                warn!("Unknown PARAM type '{}', defaulting to VALUE", other);
                Param::Value(ValueParam {
                    dop_ref: None,
                    dop_snref: None,
                    physical_default_value: None,
                    base,
                })
            }
        };

        params.push(param);
    }

    params
}

// ─── DIAG-CODED-TYPE ──────────────────────────────────────────────────────

fn parse_diag_coded_type(node: Node) -> DiagCodedType {
    let base = DiagCodedTypeBase {
        base_data_type: attr(&node, "BASE-DATA-TYPE").unwrap_or_default(),
        base_type_encoding: attr(&node, "BASE-TYPE-ENCODING"),
        is_high_low_byte_order: bool_attr_default(&node, "IS-HIGHLOW-BYTE-ORDER", true),
    };

    let xsi_type = node
        .attribute(("http://www.w3.org/2001/XMLSchema-instance", "type"))
        .or_else(|| node.attribute("xsi:type"))
        .unwrap_or("STANDARD-LENGTH-TYPE");

    match xsi_type {
        "STANDARD-LENGTH-TYPE" => DiagCodedType::StandardLength(StandardLengthType {
            base,
            bit_length: child_text(&node, "BIT-LENGTH").and_then(|s| s.parse().ok()).unwrap_or(0),
            bit_mask: child_text(&node, "BIT-MASK").map(parse_hex_bytes).filter(|v| !v.is_empty()),
            is_condensed: bool_attr(&node, "CONDENSED") || bool_child(&node, "CONDENSED"),
        }),
        "MIN-MAX-LENGTH-TYPE" => DiagCodedType::MinMaxLength(MinMaxLengthType {
            base,
            min_length: child_text(&node, "MIN-LENGTH").and_then(|s| s.parse().ok()).unwrap_or(0),
            max_length: child_text(&node, "MAX-LENGTH").and_then(|s| s.parse().ok()),
            termination: attr(&node, "TERMINATION").as_deref().and_then(|s| match s {
                "END-OF-PDU" => Some(TerminationKind::End),
                "ZERO" => Some(TerminationKind::Zero),
                "HEX-FF" => Some(TerminationKind::Hff),
                _ => None,
            }),
        }),
        "LEADING-LENGTH-INFO-TYPE" => DiagCodedType::LeadingLengthInfo(LeadingLengthInfoType {
            base,
            bit_length: child_text(&node, "BIT-LENGTH").and_then(|s| s.parse().ok()).unwrap_or(0),
        }),
        "PARAM-LENGTH-INFO-TYPE" => {
            let length_key_ref = find_child(node, "LENGTH-KEY-REF")
                .map(parse_odx_link)
                .unwrap_or_else(|| OdxLink::new(""));
            DiagCodedType::ParamLengthInfo(ParamLengthInfoType { base, length_key_ref })
        }
        other => {
            warn!("Unknown DIAG-CODED-TYPE '{}', using STANDARD-LENGTH-TYPE", other);
            DiagCodedType::StandardLength(StandardLengthType {
                base, bit_length: 0, bit_mask: None, is_condensed: false,
            })
        }
    }
}

// ─── PHYSICAL-TYPE ────────────────────────────────────────────────────────

fn parse_physical_type(node: Node) -> PhysicalType {
    PhysicalType {
        base_data_type: attr(&node, "BASE-DATA-TYPE").unwrap_or_default(),
        precision: attr(&node, "PRECISION").and_then(|s| s.parse().ok()),
        display_radix: attr(&node, "DISPLAY-RADIX").and_then(|r| match r.as_str() {
            "HEX" => Some(DisplayRadix::Hex),
            "DEC" => Some(DisplayRadix::Decimal),
            "BIN" => Some(DisplayRadix::Binary),
            "OCT" => Some(DisplayRadix::Oct),
            _ => None,
        }),
    }
}

// ─── COMPU-METHOD ─────────────────────────────────────────────────────────

fn parse_compu_method(node: Node) -> CompuMethod {
    CompuMethod {
        category: attr(&node, "CATEGORY")
            .or_else(|| child_text(&node, "CATEGORY").map(str::to_owned))
            .as_deref()
            .and_then(parse_compu_category),
        internal_to_phys: find_child(node, "COMPU-INTERNAL-TO-PHYS")
            .map(parse_compu_internal_to_phys),
        phys_to_internal: find_child(node, "COMPU-PHYS-TO-INTERNAL")
            .map(parse_compu_phys_to_internal),
    }
}

fn parse_compu_category(s: &str) -> Option<CompuCategory> {
    match s {
        "IDENTICAL" => Some(CompuCategory::Identical),
        "LINEAR" => Some(CompuCategory::Linear),
        "SCALE-LINEAR" => Some(CompuCategory::ScaleLinear),
        "TEXTTABLE" => Some(CompuCategory::Texttable),
        "COMPUCODE" => Some(CompuCategory::CompuCode),
        "RAT-FUNC" => Some(CompuCategory::RatFunc),
        "SCALE-RAT-FUNC" => Some(CompuCategory::ScaleRatFunc),
        "TAB-NOINTP" => Some(CompuCategory::TabNoInterpol),
        _ => None,
    }
}

fn parse_compu_internal_to_phys(node: Node) -> CompuInternalToPhys {
    CompuInternalToPhys {
        prog_code: None,
        compu_scales: parse_compu_scales(node),
        compu_default_value: find_child(node, "COMPU-DEFAULT-VALUE")
            .map(parse_compu_default_value),
    }
}

fn parse_compu_phys_to_internal(node: Node) -> CompuPhysToInternal {
    CompuPhysToInternal {
        prog_code: None,
        compu_scales: parse_compu_scales(node),
        compu_default_value: find_child(node, "COMPU-DEFAULT-VALUE")
            .map(parse_compu_default_value),
    }
}

fn parse_compu_scales(node: Node) -> Vec<CompuScale> {
    find_child(node, "COMPU-SCALES")
        .map(|n| {
            n.children()
                .filter(|c| c.tag_name().name() == "COMPU-SCALE")
                .map(|s| CompuScale {
                    short_label: find_child(s, "SHORT-LABEL").map(parse_text),
                    lower_limit: find_child(s, "LOWER-LIMIT").map(parse_limit),
                    upper_limit: find_child(s, "UPPER-LIMIT").map(parse_limit),
                    inverse_value: find_child(s, "COMPU-INVERSE-VALUE")
                        .map(parse_compu_values),
                    compu_const: find_child(s, "COMPU-CONST").map(parse_compu_values),
                    rational_coeffs: find_child(s, "COMPU-RATIONAL-COEFFS")
                        .map(parse_rational_coeffs),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_compu_values(node: Node) -> CompuValues {
    let vt_node = find_child(node, "VT");
    CompuValues {
        v: find_child(node, "V").and_then(|n| n.text().and_then(|s| s.trim().parse().ok())),
        vt: vt_node.and_then(|n| {
            find_child(n, "VALUE").and_then(|v| v.text())
                .or_else(|| n.text())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        }),
        vt_ti: vt_node.and_then(|n| {
            attr(&n, "TI").or_else(|| find_child(n, "TI").and_then(|v| v.text().map(str::to_owned)))
        }),
    }
}

fn parse_compu_default_value(node: Node) -> CompuDefaultValue {
    CompuDefaultValue {
        values: Some(parse_compu_values(node)),
        inverse_values: find_child(node, "COMPU-INVERSE-VALUE").map(parse_compu_values),
    }
}

fn parse_rational_coeffs(node: Node) -> CompuRationalCoEffs {
    let numerator = find_child(node, "COMPU-NUMERATOR")
        .map(|n| {
            n.children()
                .filter_map(|c| c.text().and_then(|s| s.parse::<f64>().ok()))
                .collect()
        })
        .unwrap_or_default();
    let denominator = find_child(node, "COMPU-DENOMINATOR")
        .map(|n| {
            n.children()
                .filter_map(|c| c.text().and_then(|s| s.parse::<f64>().ok()))
                .collect()
        })
        .unwrap_or_default();
    CompuRationalCoEffs { numerator, denominator }
}

fn parse_internal_constr(node: Node) -> InternalConstr {
    InternalConstr {
        lower_limit: find_child(node, "LOWER-LIMIT").map(parse_limit),
        upper_limit: find_child(node, "UPPER-LIMIT").map(parse_limit),
        scale_constrs: find_child(node, "SCALE-CONSTRS")
            .map(|n| {
                n.children()
                    .filter(|c| c.tag_name().name() == "SCALE-CONSTR")
                    .map(|sc| ScaleConstr {
                        short_label: find_child(sc, "SHORT-LABEL").map(parse_text),
                        lower_limit: find_child(sc, "LOWER-LIMIT").map(parse_limit),
                        upper_limit: find_child(sc, "UPPER-LIMIT").map(parse_limit),
                        validity: parse_constr_validity(attr(&sc, "VALIDITY").as_deref()),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Parse ODX SCALE-CONSTR/VALIDITY.
///
/// ODX XML uses hyphenated values such as `NOT-VALID`, while FlatBuffers
/// renders the corresponding enum as `NOT_VALID`.  Some producer tools also
/// emit underscore or legacy `INVALID` spellings, so normalize all supported
/// forms before mapping them to the model enum.
fn parse_constr_validity(raw: Option<&str>) -> ConstrValidity {
    let Some(raw) = raw else {
        // VALIDITY is required by ODX for SCALE-CONSTR, but preserve the
        // schema-compatible default if a non-conforming file omits it.
        return ConstrValidity::Valid;
    };

    let normalized = raw
        .trim()
        .to_ascii_uppercase()
        .replace('-', "_")
        .replace(' ', "_");

    match normalized.as_str() {
        "VALID" => ConstrValidity::Valid,
        "NOT_VALID" | "INVALID" => ConstrValidity::Invalid,
        "NOT_DEFINED" => ConstrValidity::NotDefined,
        "NOT_AVAILABLE" => ConstrValidity::NotAvailable,
        other => {
            warn!(
                "Unknown SCALE-CONSTR VALIDITY '{}'; defaulting to VALID",
                other
            );
            ConstrValidity::Valid
        }
    }
}

fn parse_limit(node: Node) -> Limit {
    Limit {
        value: node.text().map(|s| s.to_owned()),
        // ODX 2.2 defines CLOSED as the default when INTERVAL-TYPE is absent.
        interval_type: Some(match attr(&node, "INTERVAL-TYPE").as_deref() {
            Some("OPEN") => IntervalType::Open,
            Some("INFINITE") => IntervalType::Infinite,
            Some("CLOSED") | None => IntervalType::Closed,
            Some(_) => IntervalType::Closed,
        }),
    }
}

// ─── STATE-CHART ──────────────────────────────────────────────────────────

fn parse_state_chart(node: Node, coll: &mut OdxCollection) -> Result<StateChart> {
    let mut states = Vec::new();
    let mut state_transitions = Vec::new();

    if let Some(states_node) = find_child(node, "STATES") {
        for s in states_node.children().filter(|n| n.tag_name().name() == "STATE") {
            let state = State {
                id: attr(&s, "ID").unwrap_or_default(),
                short_name: child_text(&s, "SHORT-NAME").unwrap_or_default().to_owned(),
                long_name: parse_long_name_opt(s),
            };
            let idx = coll.state_store.len();
            coll.states.insert(state.id.clone(), idx);
            states.push(state.clone());
            coll.state_store.push(state);
        }
    }

    if let Some(trans_node) = find_child(node, "STATE-TRANSITIONS") {
        for t in trans_node.children().filter(|n| n.tag_name().name() == "STATE-TRANSITION") {
            let st = StateTransition {
                id: attr(&t, "ID").unwrap_or_default(),
                short_name: child_text(&t, "SHORT-NAME").unwrap_or_default().to_owned(),
                source_snref: find_child(t, "SOURCE-SNREF")
                    .and_then(|n| n.attribute("SHORT-NAME").map(|s| s.to_owned()))
                    .unwrap_or_default(),
                target_snref: find_child(t, "TARGET-SNREF")
                    .and_then(|n| n.attribute("SHORT-NAME").map(|s| s.to_owned()))
                    .unwrap_or_default(),
            };
            let idx = coll.state_transition_store.len();
            coll.state_transitions.insert(st.id.clone(), idx);
            state_transitions.push(st.clone());
            coll.state_transition_store.push(st);
        }
    }

    Ok(StateChart {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        // STATE-CHART/SEMANTIC is an XML child element, not an attribute.
        semantic: child_text(&node, "SEMANTIC").unwrap_or_default().to_owned(),
        start_state_snref: find_child(node, "START-STATE-SNREF")
            .and_then(|n| n.attribute("SHORT-NAME").map(|s| s.to_owned()))
            .unwrap_or_default(),
        states,
        state_transitions,
    })
}

// ─── COMPARAM types ───────────────────────────────────────────────────────

fn parse_comparam_refs(node: Node) -> Vec<ComParamRef> {
    find_child(node, "COMPARAM-REFS")
        .map(|n| {
            n.children()
                .filter(|c| c.tag_name().name() == "COMPARAM-REF")
                .map(|c| {
                    let simple_value = find_child(c, "SIMPLE-VALUE")
                        .map(|sv| SimpleValue {
                            value: sv.text().map(|s| s.trim().to_owned()),
                        });

                    let complex_value = find_child(c, "COMPLEX-VALUE")
                        .map(|cv| parse_complex_value(cv));

                    let protocol_snref = parse_short_name_ref(c, "PROTOCOL-SNREF");
                    let prot_stack_snref = parse_short_name_ref(c, "PROT-STACK-SNREF");

                    ComParamRef {
                        id_ref: attr(&c, "ID-REF").unwrap_or_default(),
                        doc_ref: attr(&c, "DOCREF"),
                        simple_value,
                        complex_value,
                        protocol_snref,
                        prot_stack_snref,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_simple_value(node: Node) -> SimpleValue {
    SimpleValue {
        value: node.text().map(|s| s.trim().to_owned()),
    }
}

fn parse_complex_value(node: Node) -> ComplexValue {
    let entries = node
        .children()
        .filter(|n| n.is_element())
        .filter_map(|child| match child.tag_name().name() {
            "SIMPLE-VALUE" => Some(SimpleOrComplexEntry::Simple(parse_simple_value(child))),
            "COMPLEX-VALUE" => Some(SimpleOrComplexEntry::Complex(parse_complex_value(child))),
            _ => None,
        })
        .collect();
    ComplexValue { entries }
}

fn parse_comparam_subset(node: Node, coll: &mut OdxCollection) -> Result<ComParamSubset> {
    let comparams = find_child(node, "COMPARAMS")
        .map(|wrapper| {
            wrapper
                .children()
                .filter(|n| n.tag_name().name() == "COMPARAM")
                .map(parse_comparam)
                .collect()
        })
        .unwrap_or_default();

    let complex_comparams = find_child(node, "COMPLEX-COMPARAMS")
        .map(|wrapper| {
            wrapper
                .children()
                .filter(|n| n.tag_name().name() == "COMPLEX-COMPARAM")
                .map(parse_complex_comparam)
                .collect()
        })
        .unwrap_or_default();

    // DOPs declared in a COMPARAM-SUBSET are local to that subset. They must
    // remain available so referenced communication parameters can serialize
    // their complete DOP and coded-type payload.
    let data_object_props = find_child(node, "DATA-OBJECT-PROPS")
        .map(|wrapper| {
            wrapper
                .children()
                .filter(|n| n.tag_name().name() == "DATA-OBJECT-PROP")
                .map(parse_data_object_prop)
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();

    Ok(ComParamSubset {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        long_name: parse_long_name_opt(node),
        comparams,
        complex_comparams,
        data_object_props,
        unit_spec: find_child(node, "UNIT-SPEC").map(|n| parse_unit_spec(n, coll)),
    })
}
fn parse_cp_type(s: &str) -> Option<CpType> {
    match s {
        "STANDARD" => Some(CpType::Standard),
        "OEM-SPECIFIC" => Some(CpType::OemSpecific),
        "OPTIONAL" => Some(CpType::Optional),
        "OEM-OPTIONAL" => Some(CpType::OemOptional),
        _ => None,
    }
}

fn parse_cp_usage(s: &str) -> Option<CpUsage> {
    match s {
        "ECU-SOFTWARE" => Some(CpUsage::EcuSoftware),
        "ECU-COMM" => Some(CpUsage::EcuComm),
        "APPLICATION" => Some(CpUsage::Application),
        "TESTER" => Some(CpUsage::Tester),
        _ => None,
    }
}

fn parse_comparam(node: Node) -> ComParam {
    ComParam {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        long_name: parse_long_name_opt(node),
        param_class: attr(&node, "PARAM-CLASS"),
        cp_type: attr(&node, "CPTYPE").as_deref().and_then(parse_cp_type),
        cp_usage: attr(&node, "CPUSAGE").as_deref().and_then(parse_cp_usage),
        display_level: attr(&node, "DISPLAY-LEVEL").and_then(|s| s.parse().ok()),
        physical_default_value: child_text_preserve_empty(node, "PHYSICAL-DEFAULT-VALUE"),
        data_object_prop_ref: find_child(node, "DATA-OBJECT-PROP-REF").map(parse_odx_link),
    }
}

fn parse_complex_comparam(node: Node) -> ComplexComParam {
    let sub_params = node
        .children()
        .filter(|n| n.is_element())
        .filter_map(|child| match child.tag_name().name() {
            "COMPARAM" => Some(ComParamOrComplex::Simple(parse_comparam(child))),
            "COMPLEX-COMPARAM" => {
                Some(ComParamOrComplex::Complex(parse_complex_comparam(child)))
            }
            _ => None,
        })
        .collect();

    let complex_physical_default_values = node
        .children()
        .filter(|n| n.tag_name().name() == "COMPLEX-PHYSICAL-DEFAULT-VALUE")
        .map(parse_complex_value)
        .collect();

    ComplexComParam {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        long_name: parse_long_name_opt(node),
        param_class: attr(&node, "PARAM-CLASS"),
        cp_type: attr(&node, "CPTYPE").as_deref().and_then(parse_cp_type),
        cp_usage: attr(&node, "CPUSAGE").as_deref().and_then(parse_cp_usage),
        display_level: attr(&node, "DISPLAY-LEVEL").and_then(|s| s.parse().ok()),
        allow_multiple_values: bool_attr(&node, "ALLOW-MULTIPLE-VALUES"),
        sub_params,
        complex_physical_default_values,
    }
}

fn parse_comparam_spec(node: Node, coll: &mut OdxCollection) -> Result<ComParamSpec> {
    Ok(ComParamSpec {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: child_text(&node, "SHORT-NAME").unwrap_or_default().to_owned(),
        prot_stacks: find_child(node, "PROT-STACKS")
          .map(|wrapper| {
             wrapper.children()
              .filter(|n| n.tag_name().name() == "PROT-STACK")
              .map(parse_prot_stack)
              .collect()
    })
    .unwrap_or_default(),
    })
}
fn parse_prot_stack(node: Node) -> ProtStack {
    ProtStack {
        id: attr(&node, "ID").unwrap_or_default(),
        short_name: find_child(node, "SHORT-NAME")
            .and_then(|n| n.text())
            .unwrap_or_default()
            .to_string(),
        long_name: parse_long_name_opt(node),
        physical_link_type: find_child(node, "PHYSICAL-LINK-TYPE")
            .and_then(|n| n.text())
            .map(String::from),
        pdu_protocol_type: find_child(node, "PDU-PROTOCOL-TYPE")
            .and_then(|n| n.text())
            .map(String::from),
        comparam_subset_refs: find_child(node, "COMPARAM-SUBSET-REFS")
            .map(|wrapper| wrapper.children()
                .filter(|n| n.tag_name().name() == "COMPARAM-SUBSET-REF")
                .map(parse_odx_link)
                .collect())
            .unwrap_or_default(),
    }
}

// ─── Audience ─────────────────────────────────────────────────────────────

fn parse_audience_opt(node: Node) -> Option<Audience> {
    find_child(node, "AUDIENCE").map(|n| Audience {
        enabled_audience_refs: find_child(n, "ENABLED-AUDIENCE-REFS")
            .map(|r| {
                r.children()
                    .filter_map(|c| {
                        if c.is_element() {
                            Some(parse_odx_link(c))
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        disabled_audience_refs: find_child(n, "DISABLED-AUDIENCE-REFS")
            .map(|r| {
                r.children()
                    .filter(|c| c.is_element())
                    .map(parse_odx_link)
                    .collect()
            })
            .unwrap_or_default(),

        // ODX 2.2 defines these as AUDIENCE attributes with default=true.
        // Some producer dialects use hyphenated AFTER-SALES / AFTER-MARKET
        // spellings, so accept both forms while preferring the schema names.
        is_supplier: bool_attr_default(&n, "IS-SUPPLIER", true),
        is_development: bool_attr_default(&n, "IS-DEVELOPMENT", true),
        is_manufacturing: bool_attr_default(&n, "IS-MANUFACTURING", true),
        is_after_sales: bool_attr_alias_default(
            &n,
            &["IS-AFTERSALES", "IS-AFTER-SALES"],
            true,
        ),
        is_after_market: bool_attr_alias_default(
            &n,
            &["IS-AFTERMARKET", "IS-AFTER-MARKET"],
            true,
        ),
    })
}

// ─── SD / SDG ─────────────────────────────────────────────────────────────

fn parse_sdgs_opt(node: Node) -> Option<Sdgs> {
    find_child(node, "SDGS").map(|n| Sdgs {
        sdg: n
            .children()
            .filter(|c| c.tag_name().name() == "SDG")
            .map(parse_sdg)
            .collect(),
    })
}

fn parse_sdg(node: Node) -> Sdg {
    Sdg {
        caption_sn: find_child(node, "SDG-CAPTION")
            .and_then(|caption| child_text(&caption, "SHORT-NAME").map(str::to_owned)),
        si: attr(&node, "SI"),
        items: node
            .children()
            .filter(|c| c.is_element())
            .filter_map(|c| match c.tag_name().name() {
                "SD" => Some(SdOrSdg::Sd(Sd {
                    value: c.text().map(|s| s.to_owned()),
                    si: attr(&c, "SI"),
                    ti: attr(&c, "TI"),
                })),
                "SDG" => Some(SdOrSdg::Sdg(parse_sdg(c))),
                _ => None,
            })
            .collect(),
    }
}

// ─── LONG-NAME / TEXT ─────────────────────────────────────────────────────

fn parse_long_name_opt(node: Node) -> Option<LongName> {
    find_child(node, "LONG-NAME").map(|n| LongName {
        value: n.text().map(|s| s.to_owned()),
        ti: attr(&n, "TI"),
    })
}

fn parse_text(node: Node) -> Text {
    Text {
        value: node.text().map(|s| s.to_owned()),
        ti: attr(&node, "TI"),
    }
}

fn parse_odx_link_list(node: Node, wrapper_name: &str, item_name: &str) -> Vec<OdxLink> {
    find_child(node, wrapper_name)
        .map(|wrapper| wrapper.children()
            .filter(|n| n.is_element() && n.tag_name().name() == item_name)
            .map(parse_odx_link)
            .collect())
        .unwrap_or_default()
}

fn parse_precondition_state_refs(node: Node) -> Vec<PreConditionStateRef> {
    find_child(node, "PRE-CONDITION-STATE-REFS")
        .map(|wrapper| wrapper.children()
            .filter(|n| n.is_element() && n.tag_name().name() == "PRE-CONDITION-STATE-REF")
            .map(|n| PreConditionStateRef {
                id_ref: attr(&n, "ID-REF").unwrap_or_default(),
                doc_ref: attr(&n, "DOCREF"),
                value: n.text().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
                in_param_if_snref: find_child(n, "IN-PARAM-IF-SNREF")
                    .and_then(|v| v.attribute("SHORT-NAME").map(str::to_owned)),
                in_param_if_snpathref: find_child(n, "IN-PARAM-IF-SNPATHREF")
                    .and_then(|v| v.attribute("SHORT-NAME-PATH").or_else(|| v.attribute("SHORT-NAME")))
                    .map(str::to_owned),
            }).collect())
        .unwrap_or_default()
}

fn parse_state_transition_refs(node: Node) -> Vec<StateTransitionRef> {
    find_child(node, "STATE-TRANSITION-REFS")
        .map(|wrapper| wrapper.children()
            .filter(|n| n.is_element() && n.tag_name().name() == "STATE-TRANSITION-REF")
            .map(|n| StateTransitionRef {
                id_ref: attr(&n, "ID-REF"),
                doc_ref: attr(&n, "DOCREF"),
                value: n.text().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
            }).collect())
        .unwrap_or_default()
}

fn parse_u32_auto(value: &str) -> Option<u32> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        value.parse().ok()
    }
}

fn parse_hex_bytes(value: &str) -> Vec<u8> {
    let compact: String = value.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let padded = if compact.len() % 2 == 0 { compact } else { format!("0{}", compact) };
    (0..padded.len()).step_by(2)
        .filter_map(|i| u8::from_str_radix(&padded[i..i + 2], 16).ok())
        .collect()
}

// ─── ODXLINK ──────────────────────────────────────────────────────────────

fn parse_odx_link(node: Node) -> OdxLink {
    OdxLink {
        id_ref: attr(&node, "ID-REF").unwrap_or_default(),
        doc_ref: attr(&node, "DOCREF"),
        doc_type: attr(&node, "DOCTYPE"),
    }
}

// ─── XML helpers ──────────────────────────────────────────────────────────

/// Find the first direct child element with the given local name.
fn find_child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children().find(|n| n.is_element() && n.tag_name().name() == name)
}

/// Get the trimmed text content of the first child element with the given name.
fn child_text<'a>(node: &Node<'a, 'a>, name: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
        .and_then(|n| n.text())
        .map(|s| s.trim())
}

/// Get child text while preserving an explicitly present empty element.
///
/// JAXB represents `<PHYSICAL-DEFAULT-VALUE/>` as an empty string, not as
/// `null`. Keeping that distinction is required for Kotlin-compatible MDDs.
fn child_text_preserve_empty(node: Node<'_, '_>, name: &str) -> Option<String> {
    find_child(node, name).map(|child| child.text().unwrap_or("").trim().to_owned())
}

/// Parse an ODX short-name reference from the common schema spellings.
///
/// Most files use `SHORT-NAME`, but vendor files may use `SHORT-NAME-PATH`
/// or element text. Empty references are treated as absent.
fn parse_short_name_ref(node: Node<'_, '_>, child_name: &str) -> Option<String> {
    find_child(node, child_name)
        .and_then(|child| {
            child
                .attribute("SHORT-NAME")
                .or_else(|| child.attribute("SHORT-NAME-PATH"))
                .or_else(|| child.text())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
}

/// Get an attribute value as an owned String.
fn attr(node: &Node, name: &str) -> Option<String> {
    node.attribute(name).map(|s| s.to_owned())
}

/// Get a boolean attribute (default false).
fn bool_attr(node: &Node, name: &str) -> bool {
    node.attribute(name)
        .map(|s| matches!(s.to_uppercase().as_str(), "TRUE" | "1" | "YES"))
        .unwrap_or(false)
}

fn bool_attr_default(node: &Node, name: &str, default: bool) -> bool {
    node.attribute(name)
        .map(|s| matches!(s.to_uppercase().as_str(), "TRUE" | "1" | "YES"))
        .unwrap_or(default)
}

fn bool_attr_alias_default(node: &Node, names: &[&str], default: bool) -> bool {
    names
        .iter()
        .find_map(|name| node.attribute(*name))
        .map(|s| matches!(s.to_uppercase().as_str(), "TRUE" | "1" | "YES"))
        .unwrap_or(default)
}

/// Get a boolean from a child element's text (default false).
fn bool_child(node: &Node, name: &str) -> bool {
    child_text(node, name)
        .map(|s| matches!(s.to_uppercase().trim(), "TRUE" | "1" | "YES"))
        .unwrap_or(false)
}

#[cfg(test)]
mod xml_value_tests {
    use super::*;

    #[test]
    fn parses_odx_not_valid_spelling() {
        assert_eq!(
            parse_constr_validity(Some("NOT-VALID")),
            ConstrValidity::Invalid
        );
    }

    #[test]
    fn parses_flatbuffers_and_legacy_not_valid_spellings() {
        assert_eq!(
            parse_constr_validity(Some("NOT_VALID")),
            ConstrValidity::Invalid
        );
        assert_eq!(
            parse_constr_validity(Some("INVALID")),
            ConstrValidity::Invalid
        );
    }

    #[test]
    fn parses_all_validity_values() {
        assert_eq!(parse_constr_validity(Some("VALID")), ConstrValidity::Valid);
        assert_eq!(
            parse_constr_validity(Some("NOT-DEFINED")),
            ConstrValidity::NotDefined
        );
        assert_eq!(
            parse_constr_validity(Some("NOT_AVAILABLE")),
            ConstrValidity::NotAvailable
        );
    }


    #[test]
    fn preserves_empty_physical_default_value() {
        let doc = roxmltree::Document::parse(
            r#"<PARAM><PHYSICAL-DEFAULT-VALUE/></PARAM>"#,
        )
        .unwrap();
        assert_eq!(
            child_text_preserve_empty(doc.root_element(), "PHYSICAL-DEFAULT-VALUE"),
            Some(String::new())
        );
    }

    #[test]
    fn parses_protocol_stack_short_name_reference() {
        for xml in [
            r#"<PROTOCOL><PROT-STACK-SNREF SHORT-NAME="StackA"/></PROTOCOL>"#,
            r#"<PROTOCOL><PROT-STACK-SNREF SHORT-NAME-PATH="StackA"/></PROTOCOL>"#,
            r#"<PROTOCOL><PROT-STACK-SNREF>StackA</PROT-STACK-SNREF></PROTOCOL>"#,
        ] {
            let doc = roxmltree::Document::parse(xml).unwrap();
            assert_eq!(
                parse_short_name_ref(doc.root_element(), "PROT-STACK-SNREF"),
                Some("StackA".to_owned())
            );
        }
    }

    #[test]
    fn applies_closed_interval_default() {
        let doc = roxmltree::Document::parse(r#"<LOWER-LIMIT>0</LOWER-LIMIT>"#).unwrap();
        let limit = parse_limit(doc.root_element());
        assert_eq!(limit.interval_type, Some(IntervalType::Closed));
    }

    #[test]
    fn applies_odx_audience_true_defaults() {
        let doc = roxmltree::Document::parse(
            r#"<DIAG-SERVICE><AUDIENCE/></DIAG-SERVICE>"#,
        )
        .unwrap();
        let audience = parse_audience_opt(doc.root_element()).unwrap();
        assert!(audience.is_supplier);
        assert!(audience.is_development);
        assert!(audience.is_manufacturing);
        assert!(audience.is_after_sales);
        assert!(audience.is_after_market);
    }

    #[test]
    fn parses_audience_attributes_and_aliases() {
        let doc = roxmltree::Document::parse(
            r#"<DIAG-SERVICE><AUDIENCE IS-SUPPLIER="false" IS-DEVELOPMENT="false" IS-MANUFACTURING="true" IS-AFTER-SALES="true" IS-AFTERMARKET="false"/></DIAG-SERVICE>"#,
        )
        .unwrap();
        let audience = parse_audience_opt(doc.root_element()).unwrap();
        assert!(!audience.is_supplier);
        assert!(!audience.is_development);
        assert!(audience.is_manufacturing);
        assert!(audience.is_after_sales);
        assert!(!audience.is_after_market);
    }
}

