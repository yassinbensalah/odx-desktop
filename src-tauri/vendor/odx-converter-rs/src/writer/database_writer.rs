// src/writer/database_writer.rs – Serialises OdxCollectionGroup to a
// FlatBuffers EcuData binary blob (diagnostic_description.fbs format).
//
// Field orderings (vtable offsets) are taken directly from the upstream
// diagnostic_description.fbs schema.  Rule: field N → vtable offset 4 + N*2.
//
// ── Design note ──────────────────────────────────────────────────────────
// DatabaseWriter holds only a reuse cache (HashMap).  The OdxCollectionGroup
// and ConverterOptions are passed as plain parameters to avoid Rust borrow
// conflicts between the mutable cache and the immutable collection data.
//
// FlatBuffers union fields (ParentRefType) produce TWO vtable slots:
//   slot N   → type discriminator byte  (e.g. fo(0) = 4)
//   slot N+1 → table offset             (e.g. fo(1) = 6)
// Union type bytes for ParentRefType: Variant=1, Protocol=2,
//   FunctionalGroup=3, TableDop=4, EcuSharedData=5

use std::collections::HashMap;
use anyhow::Result;
use log::{info, warn};
use flatbuffers::{FlatBufferBuilder, WIPOffset};
use crate::model::ProtStack;
use crate::model::{ComParam, ComplexComParam, ComParamOrComplex, ComParamSubset, CpType, CpUsage};
use crate::model::ComParamSpec;
use crate::model::{DiagCodedType, Param, ParamBase, Request, Response};
use crate::collection::OdxCollectionGroup;
use crate::model::{SimpleValue, ComplexValue, SimpleOrComplexEntry, ComParamRef};
use crate::model::{
    Limit, IntervalType, ScaleConstr, ConstrValidity, InternalConstr,
    PhysicalType, DisplayRadix, CompuValues, CompuScale, CompuMethod, CompuCategory,
    CompuDefaultValue, CompuRationalCoEffs,
};
use crate::model::{
    diag_layer::{
        DiagLayerCore, EcuVariant, BaseVariant, FunctionalGroup, Protocol,
        ParentRef, ParentRefDocType,
        EcuVariantPattern, MatchingParameter,
        MatchingBaseVariantParameter, BaseVariantPattern,
    },
    diag_service::{
        DiagService, SingleEcuJob, ProgCode, DiagCommCore, StateTransitionRef,
        Addressing, TransmissionMode, DiagClassType,
        TableKeyParam, TableKeyRef, TableEntryParam, TableStructParam,
    },
    dop::{Dtc, Table, TableRow, TableRowOrLink, DataObjectProp},
    state::{StateChart, State, StateTransition, PreConditionStateRef},
    odx::{LongName, Text, AdditionalAudience, OdxLink, Sdgs, SdOrSdg, Sd, Sdg},
    unit::{UnitSpec, UnitGroup, Unit, PhysicalDimension},
};
use crate::options::ConverterOptions;

// ─── VOffset helper ──────────────────────────────────────────────────────
#[inline(always)]
const fn fo(n: u16) -> u16 { 4 + n * 2 }

type TF = flatbuffers::TableFinishedWIPOffset;

// ─── Layer cache entry ────────────────────────────────────────────────────
/// A previously-serialised FlatBuffers table for a DIAG-LAYER subtype.
/// Enables DAG sharing: multiple parent_refs can point to the same table.
#[derive(Copy, Clone)]
struct CachedLayer {
    /// FlatBuffers ParentRefType union byte.
    ///   0 = being built (cycle guard)
    ///   1 = Variant   2 = Protocol   3 = FunctionalGroup   5 = EcuSharedData
    type_byte: u8,
    /// Raw WIPOffset value (absolute position in the FlatBuffers builder buffer).
    offset: u32,
}

// ─── Shared FlatBuffers build cache ──────────────────────────────────────
/// Offsets are valid only for the single `FlatBufferBuilder` used by one
/// `create_ecu_data()` call.  Keeping this cache for the whole traversal
/// prevents shared ODX objects from being serialized repeatedly.
#[derive(Default)]
struct BuildCache {
    layers: HashMap<String, CachedLayer>,
    objects: HashMap<String, u32>,
    pointer_objects: HashMap<(&'static str, usize), u32>,
    cache_hits: usize,
    cache_misses: usize,
    pointer_hits: usize,
    pointer_misses: usize,
}

impl BuildCache {
    fn get(&mut self, kind: &str, id: &str) -> Option<WIPOffset<TF>> {
        // Anonymous inline ODX objects cannot safely be keyed by an empty ID.
        if id.is_empty() {
            self.cache_misses += 1;
            return None;
        }
        let key = format!("{}:{}", kind, id);
        if let Some(offset) = self.objects.get(&key).copied() {
            self.cache_hits += 1;
            Some(WIPOffset::new(offset))
        } else {
            self.cache_misses += 1;
            None
        }
    }

    fn insert(&mut self, kind: &str, id: &str, offset: u32) {
        if !id.is_empty() {
            self.objects.insert(format!("{}:{}", kind, id), offset);
        }
    }

    fn get_ptr<T>(&mut self, kind: &'static str, value: &T) -> Option<WIPOffset<TF>> {
        let key = (kind, value as *const T as usize);
        if let Some(offset) = self.pointer_objects.get(&key).copied() {
            self.pointer_hits += 1;
            Some(WIPOffset::new(offset))
        } else {
            self.pointer_misses += 1;
            None
        }
    }

    fn insert_ptr<T>(&mut self, kind: &'static str, value: &T, offset: u32) {
        let key = (kind, value as *const T as usize);
        self.pointer_objects.insert(key, offset);
    }
}

macro_rules! return_cached {
    ($cache:expr, $kind:expr, $id:expr) => {
        if let Some(offset) = $cache.get($kind, $id) {
            return offset;
        }
    };
}

macro_rules! cache_and_return {
    ($cache:expr, $kind:expr, $id:expr, $offset:expr) => {{
        let offset = $offset;
        $cache.insert($kind, $id, offset.value());
        offset
    }};
}

macro_rules! return_cached_ptr {
    ($cache:expr, $kind:expr, $value:expr) => {
        if let Some(offset) = $cache.get_ptr($kind, $value) {
            return offset;
        }
    };
}

macro_rules! cache_ptr_and_return {
    ($cache:expr, $kind:expr, $value:expr, $offset:expr) => {{
        let offset = $offset;
        $cache.insert_ptr($kind, $value, offset.value());
        offset
    }};
}

// ─── DatabaseWriter ───────────────────────────────────────────────────────
pub struct DatabaseWriter {
    cache: BuildCache,
}

impl DatabaseWriter {
    pub fn new() -> Self {
        Self { cache: BuildCache::default() }
    }

    pub fn create_ecu_data(
        mut self,
        odx: &OdxCollectionGroup,
        _options: &ConverterOptions,
    ) -> Result<Vec<u8>> {
        let mut fbb = FlatBufferBuilder::with_capacity(256 * 1024);
        // Keep FlatBuffers' compact behavior: schema-default scalar values
        // are omitted rather than force-written into every table.
        fbb.force_defaults(false);

        // ── Collect data references upfront (immutable borrows) ───────────
        let dtcs:          Vec<&Dtc>            = odx.all_dtcs().collect();
        let base_variants: Vec<&BaseVariant>    = odx.all_base_variants().collect();
        let ecu_variants:  Vec<&EcuVariant>     = odx.all_ecu_variants().collect();
        let fgs:           Vec<&FunctionalGroup> = odx.all_functional_groups().collect();

        // ── DTCs ──────────────────────────────────────────────────────────
        let dtc_offs: Vec<WIPOffset<TF>> = dtcs.iter()
            .map(|d| build_dtc(&mut fbb, d, &mut self.cache))
            .collect();
        let dtcs_vec = fbb.create_vector(&dtc_offs);

        // ── Variants ─────────────────────────────────────────────────────
        let base_offs: Vec<WIPOffset<TF>> = base_variants.iter()
            .map(|v| {
                let off = build_variant_base(&mut fbb, v, odx, &mut self.cache);
                self.cache.layers.insert(v.core.id.clone(), CachedLayer { type_byte: 1, offset: off.value() });
                off
            })
            .collect();

        let ecu_offs: Vec<WIPOffset<TF>> = ecu_variants.iter()
            .map(|v| {
                let off = build_variant_ecu(&mut fbb, v, odx, &mut self.cache);
                self.cache.layers.insert(v.core.id.clone(), CachedLayer { type_byte: 1, offset: off.value() });
                off
            })
            .collect();

        let all_variant_offs: Vec<WIPOffset<TF>> =
            base_offs.into_iter().chain(ecu_offs).collect();
        let variants_vec = fbb.create_vector(&all_variant_offs);

        // ── Functional groups ─────────────────────────────────────────────
        let fg_offs: Vec<WIPOffset<TF>> = fgs.iter()
            .map(|fg| {
                let off = build_functional_group(&mut fbb, fg, odx, &mut self.cache);
                self.cache.layers.insert(fg.core.id.clone(), CachedLayer { type_byte: 3, offset: off.value() });
                off
            })
            .collect();
        let fg_vec = fbb.create_vector(&fg_offs);

        // ── EcuData root ─────────────────────────────────────────────────
        // table EcuData {
        //   version: string;           // 0 → fo(0) = 4
        //   ecu_name: string;          // 1 → fo(1) = 6
        //   revision: string;          // 2 → fo(2) = 8
        //   metadata: [KeyValue];      // 3 → skipped (in MDD Chunk.metadata)
        //   feature_flags: [Feature];  // 4 → skipped
        //   variants: [Variant];       // 5 → fo(5) = 14
        //   functional_groups: [...];  // 6 → fo(6) = 16
        //   dtcs: [DTC];               // 7 → fo(7) = 18
        // }
        let version  = fbb.create_string("2025-05-10");
        let ecu_name = fbb.create_string(&odx.ecu_name);
        let revision = odx.odx_revision.as_deref().map(|r| fbb.create_string(r));

        let start = fbb.start_table();
        fbb.push_slot_always::<WIPOffset<&str>>(fo(0), version);
        fbb.push_slot_always::<WIPOffset<&str>>(fo(1), ecu_name);
        if let Some(r) = revision { fbb.push_slot_always::<WIPOffset<&str>>(fo(2), r); }
        fbb.push_slot_always(fo(5), variants_vec);
        fbb.push_slot_always(fo(6), fg_vec);
        fbb.push_slot_always(fo(7), dtcs_vec);
        let root = fbb.end_table(start);

        fbb.finish_minimal(root);
        let bytes = fbb.finished_data().to_vec();
        info!("FlatBuffers EcuData: {} bytes for ECU '{}'", bytes.len(), odx.ecu_name);
        info!(
            "FlatBuffers reuse cache: id={} hits/{} misses ({} objects), pointer={} hits/{} misses ({} objects), vtables={}",
            self.cache.cache_hits,
            self.cache.cache_misses,
            self.cache.objects.len(),
            self.cache.pointer_hits,
            self.cache.pointer_misses,
            self.cache.pointer_objects.len(),
            fbb.num_written_vtables()
        );
        Ok(bytes)
    }
}
// table DiagCodedType { type; base_type_encoding; base_data_type; is_high_low_byte_order; specific_data; }
fn data_type_byte(value: &str) -> u8 {
    match value {
        "A_INT32" | "A_INT_32" => 0,
        "A_UINT32" | "A_UINT_32" => 1,
        "A_FLOAT32" | "A_FLOAT_32" => 2,
        "A_ASCIISTRING" => 3,
        "A_UTF8STRING" | "A_UTF_8_STRING" => 4,
        "A_UNICODE2STRING" | "A_UNICODE_2_STRING" => 5,
        "A_BYTEFIELD" => 6,
        "A_FLOAT64" | "A_FLOAT_64" => 7,
        _ => 1,
    }
}

fn build_diag_coded_type(
    fbb: &mut FlatBufferBuilder<'_>,
    dct: &DiagCodedType,
    odx: &OdxCollectionGroup,
    scope: Option<&[Param]>,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    return_cached_ptr!(cache, "diag_coded_type", dct);
    let (type_byte, base, specific): (u8, &crate::model::dop::DiagCodedTypeBase, Option<(u8, WIPOffset<TF>)>) = match dct {
        DiagCodedType::LeadingLengthInfo(t) => {
            let start = fbb.start_table();
            fbb.push_slot::<u32>(fo(0), t.bit_length, 0);
            (0, &t.base, Some((1, fbb.end_table(start))))
        }
        DiagCodedType::MinMaxLength(t) => {
            let termination = match t.termination {
                Some(crate::model::dop::TerminationKind::End) => 0,
                Some(crate::model::dop::TerminationKind::Zero) => 1,
                Some(crate::model::dop::TerminationKind::Hff) => 2,
                None => 0,
            };
            let start = fbb.start_table();
            fbb.push_slot::<u32>(fo(0), t.min_length, 0);
            if let Some(v) = t.max_length { fbb.push_slot::<u32>(fo(1), v, 0); }
            fbb.push_slot::<u8>(fo(2), termination, 0);
            (1, &t.base, Some((2, fbb.end_table(start))))
        }
        DiagCodedType::ParamLengthInfo(t) => {
            let length_key = scope
                .and_then(|params| params.iter().find(|p| param_id(p) == t.length_key_ref.id_ref))
                .or_else(|| odx.resolve_param(&t.length_key_ref.id_ref, t.length_key_ref.doc_ref.as_deref()))
                .map(|p| build_param_in_scope(fbb, p, odx, scope, cache));
            let start = fbb.start_table();
            if let Some(v) = length_key { fbb.push_slot_always(fo(0), v); }
            (2, &t.base, Some((3, fbb.end_table(start))))
        }
        DiagCodedType::StandardLength(t) => {
            let bit_mask = t.bit_mask.as_ref().filter(|v| !v.is_empty()).map(|v| fbb.create_vector(v));
            let start = fbb.start_table();
            fbb.push_slot::<u32>(fo(0), t.bit_length, 0);
            if let Some(v) = bit_mask { fbb.push_slot_always(fo(1), v); }
            fbb.push_slot::<bool>(fo(2), t.is_condensed, false);
            (3, &t.base, Some((4, fbb.end_table(start))))
        }
    };

    let encoding = base.base_type_encoding.as_deref().map(|v| fbb.create_string(v));
    let start = fbb.start_table();
    fbb.push_slot::<u8>(fo(0), type_byte, 0);
    if let Some(v) = encoding { fbb.push_slot_always::<WIPOffset<&str>>(fo(1), v); }
    fbb.push_slot::<u8>(fo(2), data_type_byte(&base.base_data_type), 0);
    fbb.push_slot::<bool>(fo(3), base.is_high_low_byte_order, true);
    if let Some((kind, value)) = specific {
        fbb.push_slot::<u8>(fo(4), kind, 0);
        fbb.push_slot_always(fo(5), value);
    }
    cache_ptr_and_return!(cache, "diag_coded_type", dct, fbb.end_table(start))
}
// table Param { id; param_type; short_name; semantic; sdgs; physical_default_value; byte_position; bit_position; specific_data; }
fn param_id(param: &Param) -> &str {
    match param {
        Param::Value(p) => &p.base.id,
        Param::CodedConst(p) => &p.base.id,
        Param::Dynamic(p) => &p.base.id,
        Param::LengthKey(p) => &p.base.id,
        Param::MatchingRequest(p) => &p.base.id,
        Param::NrcConst(p) => &p.base.id,
        Param::PhysConst(p) => &p.base.id,
        Param::Reserved(p) => &p.base.id,
        Param::System(p) => &p.base.id,
        Param::TableKey(p) => &p.base.id,
        Param::TableEntry(p) => &p.base.id,
        Param::TableStruct(p) => &p.base.id,
    }
}

fn build_dop_ref_or_sn(
    fbb: &mut FlatBufferBuilder<'_>,
    odx: &OdxCollectionGroup,
    link: Option<&OdxLink>,
    short_name: Option<&str>,
    cache: &mut BuildCache,
) -> Option<WIPOffset<TF>> {
    link.and_then(|v| build_any_dop(fbb, v, odx, cache))
        .or_else(|| short_name.and_then(|sn| build_any_dop_by_sn(fbb, sn, odx, cache)))
}

fn build_param(fbb: &mut FlatBufferBuilder<'_>, p: &Param, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    build_param_in_scope(fbb, p, odx, None, cache)
}

fn build_param_in_scope(
    fbb: &mut FlatBufferBuilder<'_>,
    p: &Param,
    odx: &OdxCollectionGroup,
    scope: Option<&[Param]>,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    let stable_param_id = param_id(p);
    if stable_param_id.is_empty() {
        return_cached_ptr!(cache, "param", p);
    } else {
        return_cached!(cache, "param", stable_param_id);
    }
    let (base, param_type, phys_default, specific): (&ParamBase, u8, Option<WIPOffset<&str>>, Option<(u8, WIPOffset<TF>)>) = match p {
        Param::CodedConst(v) => {
            let dct = v.diag_coded_type.as_ref().map(|d| build_diag_coded_type(fbb, d, odx, scope, cache));
            let coded = v.coded_value.as_deref().map(|s| fbb.create_string(s));
            let specific = if dct.is_some() || coded.is_some() {
                let start = fbb.start_table();
                if let Some(x) = coded { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), x); }
                if let Some(x) = dct { fbb.push_slot_always(fo(1), x); }
                Some((1, fbb.end_table(start)))
            } else { None };
            (&v.base, 0, None, specific)
        }
        Param::Dynamic(v) => {
            let start = fbb.start_table();
            (&v.base, 1, None, Some((2, fbb.end_table(start))))
        }
        Param::LengthKey(v) => {
            let dop = build_dop_ref_or_sn(fbb, odx, v.dop_ref.as_ref(), v.dop_snref.as_deref(), cache);
            let specific = dop.map(|x| {
                let start = fbb.start_table();
                fbb.push_slot_always(fo(0), x);
                (12, fbb.end_table(start))
            });
            (&v.base, 2, None, specific)
        }
        Param::MatchingRequest(v) => {
            let start = fbb.start_table();
            fbb.push_slot::<i32>(fo(0), v.request_byte_pos as i32, 0);
            fbb.push_slot::<u32>(fo(1), v.byte_length, 0);
            (&v.base, 3, None, Some((3, fbb.end_table(start))))
        }
        Param::NrcConst(v) => {
            let strings: Vec<WIPOffset<&str>> = v.coded_values.iter().map(|s| fbb.create_string(s)).collect();
            let values = if strings.is_empty() { None } else { Some(fbb.create_vector(&strings)) };
            let dct = v.diag_coded_type.as_ref().map(|d| build_diag_coded_type(fbb, d, odx, scope, cache));
            let start = fbb.start_table();
            if let Some(x) = values { fbb.push_slot_always(fo(0), x); }
            if let Some(x) = dct { fbb.push_slot_always(fo(1), x); }
            (&v.base, 4, None, Some((4, fbb.end_table(start))))
        }
        Param::PhysConst(v) => {
            let physical = v.phys_constant_value.as_deref().map(|s| fbb.create_string(s));
            let dop = build_dop_ref_or_sn(fbb, odx, v.dop_ref.as_ref(), v.dop_snref.as_deref(), cache);
            let start = fbb.start_table();
            if let Some(x) = physical { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), x); }
            if let Some(x) = dop { fbb.push_slot_always(fo(1), x); }
            (&v.base, 5, None, Some((5, fbb.end_table(start))))
        }
        Param::Reserved(v) => {
            let start = fbb.start_table();
            fbb.push_slot::<u32>(fo(0), v.bit_length, 0);
            (&v.base, 6, None, Some((6, fbb.end_table(start))))
        }
        Param::System(v) => {
            let dop = build_dop_ref_or_sn(fbb, odx, v.dop_ref.as_ref(), v.dop_snref.as_deref(), cache);
            let sys_param = fbb.create_string(&v.sys_param);
            let start = fbb.start_table();
            if let Some(x) = dop { fbb.push_slot_always(fo(0), x); }
            fbb.push_slot_always::<WIPOffset<&str>>(fo(1), sys_param);
            (&v.base, 7, None, Some((11, fbb.end_table(start))))
        }
        Param::TableEntry(v) => {
            let row = v.table_row_ref.as_ref().and_then(|link| odx.resolve_table_row(link, None))
                .map(|r| build_table_row(fbb, r, odx, cache));
            let target = match v.target.as_deref() { Some("STRUCT") => 1, _ => 0 };
            let start = fbb.start_table();
            fbb.push_slot::<u8>(fo(1), target, 0);
            if let Some(x) = row { fbb.push_slot_always(fo(2), x); }
            (&v.base, 8, None, Some((8, fbb.end_table(start))))
        }
        Param::TableKey(v) => {
            let reference = v.table_ref.as_ref().and_then(|reference| match reference {
                TableKeyRef::OdxLink(link) => {
                    odx.resolve_table(link, None).map(|t| (1, build_table_dop(fbb, t, odx, cache)))
                        .or_else(|| odx.resolve_table_row(link, None).map(|r| (2, build_table_row(fbb, r, odx, cache))))
                }
                TableKeyRef::TableSnRef(sn) => odx.resolve_table_by_sn(sn, None)
                    .map(|t| (1, build_table_dop(fbb, t, odx, cache))),
                TableKeyRef::TableRowSnRef(sn) => odx.resolve_table_row_by_sn(sn, None)
                    .map(|r| (2, build_table_row(fbb, r, odx, cache))),
            });
            let specific = reference.map(|(kind, value)| {
                let start = fbb.start_table();
                fbb.push_slot::<u8>(fo(0), kind, 0);
                fbb.push_slot_always(fo(1), value);
                (9, fbb.end_table(start))
            });
            (&v.base, 9, None, specific)
        }
        Param::TableStruct(v) => {
            let table_key = scope.and_then(|params| {
                v.table_key_ref.as_ref().and_then(|r| params.iter().find(|p| param_id(p) == r.id_ref))
                    .or_else(|| v.table_key_snref.as_deref().and_then(|sn| params.iter().find(|p| p.short_name() == sn)))
            }).or_else(|| {
                v.table_key_ref.as_ref().and_then(|r| odx.resolve_param(&r.id_ref, r.doc_ref.as_deref()))
                    .or_else(|| v.table_key_snref.as_deref().and_then(|sn| odx.resolve_param_by_sn(sn, None)))
            }).map(|p| build_param_in_scope(fbb, p, odx, scope, cache));
            let start = fbb.start_table();
            if let Some(x) = table_key { fbb.push_slot_always(fo(0), x); }
            (&v.base, 10, None, Some((10, fbb.end_table(start))))
        }
        Param::Value(v) => {
            let phys = v.physical_default_value.as_deref().map(|s| fbb.create_string(s));
            let dop = build_dop_ref_or_sn(fbb, odx, v.dop_ref.as_ref(), v.dop_snref.as_deref(), cache);
            let specific = if phys.is_some() || dop.is_some() {
                let start = fbb.start_table();
                if let Some(x) = phys { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), x); }
                if let Some(x) = dop { fbb.push_slot_always(fo(1), x); }
                Some((7, fbb.end_table(start)))
            } else { None };
            // Kotlin stores PHYSICAL-DEFAULT-VALUE only in the Value union payload.
            // Do not duplicate it in Param.physical_default_value.
            (&v.base, 11, None, specific)
        }
    };

    let short_name = fbb.create_string(&base.short_name);
    let semantic = base.semantic.as_deref().map(|s| fbb.create_string(s));
    let sdgs = base.sdgs.as_ref().map(|v| build_sdgs(fbb, v));
    let start = fbb.start_table();
    fbb.push_slot::<u8>(fo(1), param_type, 0);
    fbb.push_slot_always::<WIPOffset<&str>>(fo(2), short_name);
    if let Some(v) = semantic { fbb.push_slot_always::<WIPOffset<&str>>(fo(3), v); }
    if let Some(v) = sdgs { fbb.push_slot_always(fo(4), v); }
    if let Some(v) = phys_default { fbb.push_slot_always::<WIPOffset<&str>>(fo(5), v); }
    if let Some(v) = base.byte_position { fbb.push_slot::<u32>(fo(6), v, 0); }
    if let Some(v) = base.bit_position { fbb.push_slot::<u32>(fo(7), v, 0); }
    if let Some((kind, value)) = specific {
        fbb.push_slot::<u8>(fo(8), kind, 0);
        fbb.push_slot_always(fo(9), value);
    }
    let offset = fbb.end_table(start);
    if stable_param_id.is_empty() {
        cache.insert_ptr("param", p, offset.value());
    } else {
        cache.insert("param", stable_param_id, offset.value());
    }
    offset
}

fn build_request(fbb: &mut FlatBufferBuilder<'_>, r: &Request, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "request", &r.id);
    let p_offs: Vec<WIPOffset<TF>> = r.params.iter()
        .map(|p| build_param_in_scope(fbb, p, odx, Some(&r.params), cache)).collect();
    let p_vec = if p_offs.is_empty() { None } else { Some(fbb.create_vector(&p_offs)) };
    let sdgs = r.sdgs.as_ref().map(|v| build_sdgs(fbb, v));
    let start = fbb.start_table();
    if let Some(v) = p_vec { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = sdgs { fbb.push_slot_always(fo(1), v); }
    cache_and_return!(cache, "request", &r.id, fbb.end_table(start))
}


fn build_response(fbb: &mut FlatBufferBuilder<'_>, r: &Response, response_type: u8, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    let cache_kind = match response_type { 0 => "pos_response", 1 => "neg_response", _ => "response" };
    return_cached!(cache, cache_kind, &r.id);
    let p_offs: Vec<WIPOffset<TF>> = r.params.iter()
        .map(|p| build_param_in_scope(fbb, p, odx, Some(&r.params), cache)).collect();
    let p_vec = if p_offs.is_empty() { None } else { Some(fbb.create_vector(&p_offs)) };
    let sdgs = r.sdgs.as_ref().map(|v| build_sdgs(fbb, v));
    let start = fbb.start_table();
    fbb.push_slot::<u8>(fo(0), response_type, 0);
    if let Some(v) = p_vec { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = sdgs { fbb.push_slot_always(fo(2), v); }
    cache_and_return!(cache, cache_kind, &r.id, fbb.end_table(start))
}
fn wrap_dop(
    fbb: &mut FlatBufferBuilder<'_>,
    dop_type: u8,
    short_name: &str,
    sdgs: Option<&Sdgs>,
    specific_type: u8,
    specific: WIPOffset<TF>,
) -> WIPOffset<TF> {
    let short_name = fbb.create_string(short_name);
    let sdgs = sdgs.map(|v| build_sdgs(fbb, v));
    let start = fbb.start_table();
    // The Kotlin reference writer leaves DOP.dop_type unset and relies on
    // SpecificDOPData as the discriminator. Keep the field omitted for parity.
    let _ = dop_type;
    fbb.push_slot_always::<WIPOffset<&str>>(fo(1), short_name);
    if let Some(v) = sdgs { fbb.push_slot_always(fo(2), v); }
    fbb.push_slot::<u8>(fo(3), specific_type, 0);
    fbb.push_slot_always(fo(4), specific);
    fbb.end_table(start)
}

fn build_structure(fbb: &mut FlatBufferBuilder<'_>, s: &crate::model::dop::Structure, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "dop", &s.id);
    let params: Vec<WIPOffset<TF>> = s.params.iter()
        .map(|p| build_param_in_scope(fbb, p, odx, Some(&s.params), cache)).collect();
    let params = if params.is_empty() { None } else { Some(fbb.create_vector(&params)) };
    let start = fbb.start_table();
    if let Some(v) = params { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = s.byte_size { fbb.push_slot::<u32>(fo(1), v, 0); }
    fbb.push_slot::<bool>(fo(2), s.is_visible, true);
    let specific = fbb.end_table(start);
    let offset = wrap_dop(fbb, 8, &s.short_name, s.sdgs.as_ref(), 7, specific);
    cache_and_return!(cache, "dop", &s.id, offset)
}

fn build_dtc_dop(fbb: &mut FlatBufferBuilder<'_>, d: &crate::model::dop::DtcDop, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "dop", &d.id);
    let dct = build_diag_coded_type(fbb, &d.diag_coded_type, odx, None, cache);
    let physical_type = build_physical_type(fbb, &d.physical_type);
    let compu_method = build_compu_method(fbb, &d.compu_method);
    let dtcs: Vec<WIPOffset<TF>> = d.dtcs.iter().filter_map(|value| match value {
        crate::model::dop::DtcOrRef::Dtc(v) => Some(build_dtc(fbb, v, cache)),
        crate::model::dop::DtcOrRef::OdxLink(link) => odx.resolve_dtc(link, None).map(|v| build_dtc(fbb, v, cache)),
    }).collect();
    let dtcs = if dtcs.is_empty() { None } else { Some(fbb.create_vector(&dtcs)) };
    let start = fbb.start_table();
    fbb.push_slot_always(fo(0), dct);
    fbb.push_slot_always(fo(1), physical_type);
    fbb.push_slot_always(fo(2), compu_method);
    if let Some(v) = dtcs { fbb.push_slot_always(fo(3), v); }
    fbb.push_slot::<bool>(fo(4), d.is_visible, false);
    let specific = fbb.end_table(start);
    let offset = wrap_dop(fbb, 9, &d.short_name, d.sdgs.as_ref(), 6, specific);
    cache_and_return!(cache, "dop", &d.id, offset)
}

fn build_field(fbb: &mut FlatBufferBuilder<'_>, field: &crate::model::dop::Field, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    let basic = build_dop_ref_or_sn(fbb, odx, field.basic_structure_ref.as_ref(), field.basic_structure_snref.as_deref(), cache);
    let env_desc = build_dop_ref_or_sn(fbb, odx, field.env_data_desc_ref.as_ref(), field.env_data_desc_snref.as_deref(), cache);
    let start = fbb.start_table();
    if let Some(v) = basic { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = env_desc { fbb.push_slot_always(fo(1), v); }
    fbb.push_slot::<bool>(fo(2), field.is_visible, true);
    fbb.end_table(start)
}

fn build_static_field(fbb: &mut FlatBufferBuilder<'_>, value: &crate::model::dop::StaticField, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "dop", &value.id);
    let field = build_field(fbb, &value.field, odx, cache);
    let start = fbb.start_table();
    fbb.push_slot::<u32>(fo(0), value.fixed_number_of_items, 0);
    fbb.push_slot::<u32>(fo(1), value.item_byte_size, 0);
    fbb.push_slot_always(fo(2), field);
    let specific = fbb.end_table(start);
    let offset = wrap_dop(fbb, 6, &value.short_name, value.sdgs.as_ref(), 3, specific);
    cache_and_return!(cache, "dop", &value.id, offset)
}

fn build_end_of_pdu_field(fbb: &mut FlatBufferBuilder<'_>, value: &crate::model::dop::EndOfPduField, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "dop", &value.id);
    let field = build_field(fbb, &value.field, odx, cache);
    let start = fbb.start_table();
    if let Some(v) = value.max_number_of_items { fbb.push_slot::<u32>(fo(0), v, 0); }
    if let Some(v) = value.min_number_of_items { fbb.push_slot::<u32>(fo(1), v, 0); }
    fbb.push_slot_always(fo(2), field);
    let specific = fbb.end_table(start);
    let offset = wrap_dop(fbb, 5, &value.short_name, value.sdgs.as_ref(), 2, specific);
    cache_and_return!(cache, "dop", &value.id, offset)
}

fn build_env_data(fbb: &mut FlatBufferBuilder<'_>, value: &crate::model::dop::EnvData, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "dop", &value.id);
    let dtcs = if value.dtc_values.is_empty() { None } else { Some(fbb.create_vector(&value.dtc_values)) };
    let params: Vec<WIPOffset<TF>> = value.params.iter()
        .map(|p| build_param_in_scope(fbb, p, odx, Some(&value.params), cache)).collect();
    let params = if params.is_empty() { None } else { Some(fbb.create_vector(&params)) };
    let start = fbb.start_table();
    if let Some(v) = dtcs { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = params { fbb.push_slot_always(fo(1), v); }
    let specific = fbb.end_table(start);
    let offset = wrap_dop(fbb, 7, &value.short_name, None, 5, specific);
    cache_and_return!(cache, "dop", &value.id, offset)
}

fn build_env_data_desc(fbb: &mut FlatBufferBuilder<'_>, value: &crate::model::dop::EnvDataDesc, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "dop", &value.id);
    let param_sn = value.param_snref.as_deref().map(|v| fbb.create_string(v));
    let param_path = value.param_sn_pathref.as_deref().map(|v| fbb.create_string(v));
    let env_datas: Vec<WIPOffset<TF>> = value.env_data_refs.iter()
        .filter_map(|link| odx.resolve_env_data(link, None))
        .map(|v| build_env_data(fbb, v, odx, cache)).collect();
    let env_datas = if env_datas.is_empty() { None } else { Some(fbb.create_vector(&env_datas)) };
    let start = fbb.start_table();
    if let Some(v) = param_sn { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), v); }
    if let Some(v) = param_path { fbb.push_slot_always::<WIPOffset<&str>>(fo(1), v); }
    if let Some(v) = env_datas { fbb.push_slot_always(fo(2), v); }
    let specific = fbb.end_table(start);
    let offset = wrap_dop(fbb, 1, &value.short_name, None, 4, specific);
    cache_and_return!(cache, "dop", &value.id, offset)
}

fn build_dynamic_length_field(fbb: &mut FlatBufferBuilder<'_>, value: &crate::model::dop::DynamicLengthField, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "dop", &value.id);
    let field = build_field(fbb, &value.field, odx, cache);
    let determine_dop = value.determine_number_of_items.data_object_prop_ref.as_ref()
        .and_then(|link| build_any_dop(fbb, link, odx, cache));
    let determine_start = fbb.start_table();
    fbb.push_slot::<u32>(fo(0), value.determine_number_of_items.byte_position, 0);
    if let Some(v) = value.determine_number_of_items.bit_position { fbb.push_slot::<u32>(fo(1), v, 0); }
    if let Some(v) = determine_dop { fbb.push_slot_always(fo(2), v); }
    let determine = fbb.end_table(determine_start);
    let start = fbb.start_table();
    fbb.push_slot::<u32>(fo(0), value.offset, 0);
    fbb.push_slot_always(fo(1), field);
    fbb.push_slot_always(fo(2), determine);
    let specific = fbb.end_table(start);
    let offset = wrap_dop(fbb, 4, &value.short_name, None, 9, specific);
    cache_and_return!(cache, "dop", &value.id, offset)
}

fn build_mux(fbb: &mut FlatBufferBuilder<'_>, value: &crate::model::dop::Mux, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "dop", &value.id);
    let switch_dop = build_any_dop(fbb, &value.switch_key.data_object_prop_ref, odx, cache);
    let switch_start = fbb.start_table();
    fbb.push_slot::<u32>(fo(0), value.switch_key.byte_position, 0);
    if let Some(v) = value.switch_key.bit_position { fbb.push_slot::<u32>(fo(1), v, 0); }
    if let Some(v) = switch_dop { fbb.push_slot_always(fo(2), v); }
    let switch_key = fbb.end_table(switch_start);

    let default_case = value.default_case.as_ref().map(|v| {
        let sn = fbb.create_string(&v.short_name);
        let ln = v.long_name.as_ref().map(|x| build_long_name(fbb, x));
        let structure = v.structure_ref.as_ref().and_then(|x| build_any_dop(fbb, x, odx, cache));
        let start = fbb.start_table();
        fbb.push_slot_always::<WIPOffset<&str>>(fo(0), sn);
        if let Some(x) = ln { fbb.push_slot_always(fo(1), x); }
        if let Some(x) = structure { fbb.push_slot_always(fo(2), x); }
        fbb.end_table(start)
    });
    let cases: Vec<WIPOffset<TF>> = value.cases.iter().map(|v| {
        let sn = fbb.create_string(&v.short_name);
        let ln = v.long_name.as_ref().map(|x| build_long_name(fbb, x));
        let structure = v.structure_ref.as_ref().and_then(|x| build_any_dop(fbb, x, odx, cache))
            .or_else(|| v.structure_snref.as_deref().and_then(|sn| build_any_dop_by_sn(fbb, sn, odx, cache)));
        let lower = build_limit(fbb, &v.lower_limit);
        let upper = build_limit(fbb, &v.upper_limit);
        let start = fbb.start_table();
        fbb.push_slot_always::<WIPOffset<&str>>(fo(0), sn);
        if let Some(x) = ln { fbb.push_slot_always(fo(1), x); }
        if let Some(x) = structure { fbb.push_slot_always(fo(2), x); }
        fbb.push_slot_always(fo(3), lower);
        fbb.push_slot_always(fo(4), upper);
        fbb.end_table(start)
    }).collect();
    let cases = if cases.is_empty() { None } else { Some(fbb.create_vector(&cases)) };
    let start = fbb.start_table();
    fbb.push_slot::<u32>(fo(0), value.byte_position, 0);
    fbb.push_slot_always(fo(1), switch_key);
    if let Some(v) = default_case { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = cases { fbb.push_slot_always(fo(3), v); }
    fbb.push_slot::<bool>(fo(4), value.is_visible, false);
    let specific = fbb.end_table(start);
    let offset = wrap_dop(fbb, 2, &value.short_name, None, 8, specific);
    cache_and_return!(cache, "dop", &value.id, offset)
}

fn build_any_dop(fbb: &mut FlatBufferBuilder<'_>, link: &OdxLink, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> Option<WIPOffset<TF>> {
    if let Some(v) = odx.resolve_dop(link, None) { return Some(build_dop(fbb, v, odx, cache)); }
    if let Some(v) = odx.resolve_structure(link, None) { return Some(build_structure(fbb, v, odx, cache)); }
    if let Some(v) = odx.resolve_dtc_dop(link, None) { return Some(build_dtc_dop(fbb, v, odx, cache)); }
    if let Some(v) = odx.resolve_static_field(link, None) { return Some(build_static_field(fbb, v, odx, cache)); }
    if let Some(v) = odx.resolve_end_of_pdu_field(link, None) { return Some(build_end_of_pdu_field(fbb, v, odx, cache)); }
    if let Some(v) = odx.resolve_dynamic_length_field(link, None) { return Some(build_dynamic_length_field(fbb, v, odx, cache)); }
    if let Some(v) = odx.resolve_mux(link, None) { return Some(build_mux(fbb, v, odx, cache)); }
    if let Some(v) = odx.resolve_env_data(link, None) { return Some(build_env_data(fbb, v, odx, cache)); }
    if let Some(v) = odx.resolve_env_data_desc(link, None) { return Some(build_env_data_desc(fbb, v, odx, cache)); }
    None
}

fn build_any_dop_by_sn(fbb: &mut FlatBufferBuilder<'_>, short_name: &str, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> Option<WIPOffset<TF>> {
    if let Some(v) = odx.resolve_dop_by_sn(short_name, None) { return Some(build_dop(fbb, v, odx, cache)); }
    if let Some(v) = odx.resolve_structure_by_sn(short_name, None) { return Some(build_structure(fbb, v, odx, cache)); }
    if let Some(v) = odx.resolve_env_data_desc_by_sn(short_name, None) { return Some(build_env_data_desc(fbb, v, odx, cache)); }
    None
}
// ─── DTC ──────────────────────────────────────────────────────────────────
// table DTC {
//   short_name: string;            // 0 → 4
//   trouble_code: uint32;          // 1 → 6
//   display_trouble_code: string;  // 2 → 8
//   text: Text;                    // 3 → 10
//   level: uint32 = null;          // 4 → 12
//   sdgs: SDGS;                    // 5 → 14
//   is_temporary: bool = false;    // 6 → 16
// }
fn build_dtc(fbb: &mut FlatBufferBuilder<'_>, dtc: &Dtc, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "dtc", &dtc.id);
    let sn   = fbb.create_string(&dtc.short_name);
    let disp = dtc.display_trouble_code.as_deref().map(|s| fbb.create_string(s));
    let text = dtc.text.as_ref().map(|t| build_text(fbb, t));
    let sdgs = dtc.sdgs.as_ref().map(|v| build_sdgs(fbb, v));

    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), sn);
    fbb.push_slot::<u32>(fo(1), dtc.trouble_code, 0);
    if let Some(d) = disp { fbb.push_slot_always::<WIPOffset<&str>>(fo(2), d); }
    if let Some(t) = text { fbb.push_slot_always(fo(3), t); }
    if let Some(lvl) = dtc.level { fbb.push_slot::<u32>(fo(4), lvl, 0); }
    if let Some(v) = sdgs { fbb.push_slot_always(fo(5), v); }
    fbb.push_slot::<bool>(fo(6), dtc.is_temporary, false);
    cache_and_return!(cache, "dtc", &dtc.id, fbb.end_table(start))
}

fn build_sd(fbb: &mut FlatBufferBuilder<'_>, value: &Sd) -> WIPOffset<TF> {
    let text = value.value.as_deref().map(|v| fbb.create_string(v));
    let si = value.si.as_deref().map(|v| fbb.create_string(v));
    let ti = value.ti.as_deref().map(|v| fbb.create_string(v));
    let start = fbb.start_table();
    if let Some(v) = text { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), v); }
    if let Some(v) = si { fbb.push_slot_always::<WIPOffset<&str>>(fo(1), v); }
    if let Some(v) = ti { fbb.push_slot_always::<WIPOffset<&str>>(fo(2), v); }
    fbb.end_table(start)
}

fn build_sdg(fbb: &mut FlatBufferBuilder<'_>, value: &Sdg) -> WIPOffset<TF> {
    let caption = value.caption_sn.as_deref().map(|v| fbb.create_string(v));
    let si = value.si.as_deref().map(|v| fbb.create_string(v));
    let items: Vec<WIPOffset<TF>> = value.items.iter().map(|item| {
        let (kind, offset) = match item {
            SdOrSdg::Sd(v) => (1, build_sd(fbb, v)),
            SdOrSdg::Sdg(v) => (2, build_sdg(fbb, v)),
        };
        let start = fbb.start_table();
        fbb.push_slot::<u8>(fo(0), kind, 0);
        fbb.push_slot_always(fo(1), offset);
        fbb.end_table(start)
    }).collect();
    let items = if items.is_empty() { None } else { Some(fbb.create_vector(&items)) };
    let start = fbb.start_table();
    if let Some(v) = caption { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), v); }
    if let Some(v) = items { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = si { fbb.push_slot_always::<WIPOffset<&str>>(fo(2), v); }
    fbb.end_table(start)
}

fn build_sdgs(fbb: &mut FlatBufferBuilder<'_>, value: &Sdgs) -> WIPOffset<TF> {
    let entries: Vec<WIPOffset<TF>> = value.sdg.iter().map(|v| build_sdg(fbb, v)).collect();
    let entries = if entries.is_empty() { None } else { Some(fbb.create_vector(&entries)) };
    let start = fbb.start_table();
    if let Some(v) = entries { fbb.push_slot_always(fo(0), v); }
    fbb.end_table(start)
}

// ─── Text / LongName ──────────────────────────────────────────────────────
// table Text { value: string; ti: string; }
fn build_text(fbb: &mut FlatBufferBuilder<'_>, text: &Text) -> WIPOffset<TF> {
    let value = text.value.as_deref().map(|s| fbb.create_string(s));
    let ti = text.ti.as_deref().map(|s| fbb.create_string(s));
    let start = fbb.start_table();
    if let Some(v) = value { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), v); }
    if let Some(v) = ti { fbb.push_slot_always::<WIPOffset<&str>>(fo(1), v); }
    fbb.end_table(start)
}

// table LongName { value: string; ti: string; }
fn build_long_name(fbb: &mut FlatBufferBuilder<'_>, ln: &LongName) -> WIPOffset<TF> {
    let value = ln.value.as_deref().map(|s| fbb.create_string(s));
    let ti    = ln.ti.as_deref().map(|s| fbb.create_string(s));

    let start = fbb.start_table();
    if let Some(v) = value { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), v); }
    if let Some(t) = ti    { fbb.push_slot_always::<WIPOffset<&str>>(fo(1), t); }
    fbb.end_table(start)
}

// ─── FunctClass ───────────────────────────────────────────────────────────
// table FunctClass { short_name: string; }
fn build_funct_class(
    fbb: &mut FlatBufferBuilder<'_>,
    sn: &str,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    return_cached!(cache, "funct_class", sn);
    let sn_off = fbb.create_string(sn);
    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), sn_off);
    cache_and_return!(cache, "funct_class", sn, fbb.end_table(start))
}
fn build_audience(
    fbb: &mut FlatBufferBuilder<'_>,
    a: &crate::model::odx::Audience,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    return_cached_ptr!(cache, "audience", a);
    let enabled: Vec<WIPOffset<TF>> = a.enabled_audience_refs.iter()
        .filter_map(|link| odx.resolve_additional_audience(link, None))
        .map(|v| build_additional_audience(fbb, v, cache)).collect();
    let disabled: Vec<WIPOffset<TF>> = a.disabled_audience_refs.iter()
        .filter_map(|link| odx.resolve_additional_audience(link, None))
        .map(|v| build_additional_audience(fbb, v, cache)).collect();
    let enabled = if enabled.is_empty() { None } else { Some(fbb.create_vector(&enabled)) };
    let disabled = if disabled.is_empty() { None } else { Some(fbb.create_vector(&disabled)) };
    let start = fbb.start_table();
    if let Some(v) = enabled { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = disabled { fbb.push_slot_always(fo(1), v); }
    fbb.push_slot::<bool>(fo(2), a.is_supplier, false);
    fbb.push_slot::<bool>(fo(3), a.is_development, false);
    fbb.push_slot::<bool>(fo(4), a.is_manufacturing, false);
    fbb.push_slot::<bool>(fo(5), a.is_after_sales, false);
    fbb.push_slot::<bool>(fo(6), a.is_after_market, false);
    cache_ptr_and_return!(cache, "audience", a, fbb.end_table(start))
}

// ─── AdditionalAudience ───────────────────────────────────────────────────
// table AdditionalAudience { short_name: string; long_name: LongName; }
fn build_additional_audience(
    fbb: &mut FlatBufferBuilder<'_>,
    aa: &AdditionalAudience,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    return_cached!(cache, "additional_audience", &aa.id);
    let sn = fbb.create_string(&aa.short_name);
    let ln = aa.long_name.as_ref().map(|l| build_long_name(fbb, l));

    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), sn);
    if let Some(l) = ln { fbb.push_slot_always(fo(1), l); }
    cache_and_return!(cache, "additional_audience", &aa.id, fbb.end_table(start))
}

// ─── State / StateTransition / StateChart ────────────────────────────────
// table State { short_name: string; long_name: LongName; }
fn build_state(fbb: &mut FlatBufferBuilder<'_>, s: &State) -> WIPOffset<TF> {
    let sn = fbb.create_string(&s.short_name);
    let ln = s.long_name.as_ref().map(|l| build_long_name(fbb, l));

    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), sn);
    if let Some(l) = ln { fbb.push_slot_always(fo(1), l); }
    fbb.end_table(start)
}

// table StateTransition {
//   short_name: string;              // 0 → 4
//   source_short_name_ref: string;   // 1 → 6
//   target_short_name_ref: string;   // 2 → 8
// }
fn build_state_transition(fbb: &mut FlatBufferBuilder<'_>, t: &StateTransition) -> WIPOffset<TF> {
    let sn  = fbb.create_string(&t.short_name);
    let src = fbb.create_string(&t.source_snref);
    let tgt = fbb.create_string(&t.target_snref);

    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), sn);
    fbb.push_slot_always::<WIPOffset<&str>>(fo(1), src);
    fbb.push_slot_always::<WIPOffset<&str>>(fo(2), tgt);
    fbb.end_table(start)
}

// table StateChart {
//   short_name: string;                  // 0 → 4
//   semantic: string;                    // 1 → 6
//   state_transitions: [StateTransition];// 2 → 8
//   start_state_short_name_ref: string;  // 3 → 10
//   states: [State];                     // 4 → 12
// }
fn build_state_chart(fbb: &mut FlatBufferBuilder<'_>, sc: &StateChart) -> WIPOffset<TF> {
    let trans_offs: Vec<WIPOffset<TF>> = sc.state_transitions.iter()
        .map(|t| build_state_transition(fbb, t))
        .collect();
    let trans_vec = fbb.create_vector(&trans_offs);

    let state_offs: Vec<WIPOffset<TF>> = sc.states.iter()
        .map(|s| build_state(fbb, s))
        .collect();
    let states_vec = fbb.create_vector(&state_offs);

    let sn       = fbb.create_string(&sc.short_name);
    let semantic = fbb.create_string(&sc.semantic);
    let start_sn = fbb.create_string(&sc.start_state_snref);

    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), sn);
    fbb.push_slot_always::<WIPOffset<&str>>(fo(1), semantic);
    fbb.push_slot_always(fo(2), trans_vec);
    fbb.push_slot_always::<WIPOffset<&str>>(fo(3), start_sn);
    fbb.push_slot_always(fo(4), states_vec);
    fbb.end_table(start)
}

// ─── PreConditionStateRef ─────────────────────────────────────────────────
// table PreConditionStateRef {
//   value: string;                  // 0 → 4
//   in_param_if_short_name: string; // 1 → 6
//   in_param_path_short_name: string; // 2 → 8
//   state: State;                   // 3 → 10
// }
fn build_precondition_state_ref(
    fbb: &mut FlatBufferBuilder<'_>,
    pcsr: &PreConditionStateRef,
    odx: &OdxCollectionGroup,
) -> WIPOffset<TF> {
    let value = pcsr.value.as_deref().map(|s| fbb.create_string(s));
    let in_sn = pcsr.in_param_if_snref.as_deref().map(|s| fbb.create_string(s));
    let in_path = pcsr.in_param_if_snpathref.as_deref().map(|s| fbb.create_string(s));

    // Resolve the referenced State
    let state_off = odx.resolve_state(&pcsr.id_ref, None)
        .map(|s| build_state(fbb, s));

    let start = fbb.start_table();
    if let Some(v) = value  { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), v); }
    if let Some(s) = in_sn  { fbb.push_slot_always::<WIPOffset<&str>>(fo(1), s); }
    if let Some(p) = in_path{ fbb.push_slot_always::<WIPOffset<&str>>(fo(2), p); }
    if let Some(s) = state_off { fbb.push_slot_always(fo(3), s); }
    fbb.end_table(start)
}

// ─── StateTransitionRef ───────────────────────────────────────────────────
// table StateTransitionRef {
//   value: string;                        // 0 → 4
//   state_transition: StateTransition;    // 1 → 6
// }
fn build_state_transition_ref(
    fbb: &mut FlatBufferBuilder<'_>,
    str_ref: &StateTransitionRef,
    odx: &OdxCollectionGroup,
) -> WIPOffset<TF> {
    let value = str_ref.value.as_deref().map(|s| fbb.create_string(s));

    // Resolve the referenced StateTransition
    let st_off = str_ref.id_ref.as_deref().and_then(|id| {
        odx.resolve_state_transition(id, None).map(|t| build_state_transition(fbb, t))
    });

    let start = fbb.start_table();
    if let Some(v) = value { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), v); }
    if let Some(t) = st_off { fbb.push_slot_always(fo(1), t); }
    fbb.end_table(start)
}

// ─── ProgCode ─────────────────────────────────────────────────────────────
// table ProgCode {
//   code_file: string;  // 0 → 4
//   encryption: string; // 1 → 6
//   syntax: string;     // 2 → 8
//   revision: string;   // 3 → 10
//   entrypoint: string; // 4 → 12
//   library: [Library]; // 5 → 14 (omitted)
// }
fn build_prog_code(fbb: &mut FlatBufferBuilder<'_>, pc: &ProgCode) -> WIPOffset<TF> {
    let cf  = pc.code_file.as_deref().map(|s| fbb.create_string(s));
    let enc = pc.encryption.as_deref().map(|s| fbb.create_string(s));
    let syn = pc.syntax.as_deref().map(|s| fbb.create_string(s));
    let rev = pc.revision.as_deref().map(|s| fbb.create_string(s));
    let ep  = pc.entrypoint.as_deref().map(|s| fbb.create_string(s));

    let start = fbb.start_table();
    if let Some(v) = cf  { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), v); }
    if let Some(v) = enc { fbb.push_slot_always::<WIPOffset<&str>>(fo(1), v); }
    if let Some(v) = syn { fbb.push_slot_always::<WIPOffset<&str>>(fo(2), v); }
    if let Some(v) = rev { fbb.push_slot_always::<WIPOffset<&str>>(fo(3), v); }
    if let Some(v) = ep  { fbb.push_slot_always::<WIPOffset<&str>>(fo(4), v); }
    fbb.end_table(start)
}

// ─── DiagComm ─────────────────────────────────────────────────────────────
// table DiagComm {
//   short_name: string;             // 0 → 4
//   long_name: LongName;            // 1 → 6
//   semantic: string;               // 2 → 8
//   funct_class: [FunctClass];      // 3 → 10
//   sdgs: SDGS;                     // 4 → 12
//   diag_class_type: byte;          // 5 → 14  (omitted = default 0)
//   pre_condition_state_refs: [...];// 6 → 16
//   state_transition_refs: [...];   // 7 → 18
//   protocols: [Protocol];          // 8 → 20  (omitted – complex resolution)
//   audience: Audience;             // 9 → 22
//   is_mandatory: bool = false;     // 10 → 24
//   is_executable: bool = true;     // 11 → 26
//   is_final: bool = false;         // 12 → 28
// }
fn build_diag_comm(
    fbb: &mut FlatBufferBuilder<'_>,
    comm: &DiagCommCore,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    let sn       = fbb.create_string(&comm.short_name);
    let ln       = comm.long_name.as_ref().map(|l| build_long_name(fbb, l));
    let semantic = comm.semantic.as_deref().map(|s| fbb.create_string(s));
    let sdgs = comm.sdgs.as_ref().map(|v| build_sdgs(fbb, v));

    // Funct classes (from the funct_class_refs OdxLinks)
    let fc_offs: Vec<WIPOffset<TF>> = comm.funct_class_refs.iter()
        .filter_map(|link| {
            odx.resolve_funct_class(link, None)
                .map(|fc| build_funct_class(fbb, &fc.short_name, cache))
        })
        .collect();
    let fc_vec = if fc_offs.is_empty() { None } else { Some(fbb.create_vector(&fc_offs)) };

    // PreConditionStateRefs
    let pcsr_offs: Vec<WIPOffset<TF>> = comm.precondition_state_refs.iter()
        .map(|pcsr| build_precondition_state_ref(fbb, pcsr, odx))
        .collect();
    let pcsr_vec = if pcsr_offs.is_empty() { None } else { Some(fbb.create_vector(&pcsr_offs)) };

    // StateTransitionRefs
    let str_offs: Vec<WIPOffset<TF>> = comm.state_transition_refs.iter()
        .map(|str_ref| build_state_transition_ref(fbb, str_ref, odx))
        .collect();
    let str_vec = if str_offs.is_empty() { None } else { Some(fbb.create_vector(&str_offs)) };
    let audience = comm.audience.as_ref().map(|a| build_audience(fbb, a, odx, cache));
    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), sn);
    if let Some(l) = ln       { fbb.push_slot_always(fo(1), l); }
    if let Some(s) = semantic { fbb.push_slot_always::<WIPOffset<&str>>(fo(2), s); }
    if let Some(v) = fc_vec   { fbb.push_slot_always(fo(3), v); }
    if let Some(v) = sdgs     { fbb.push_slot_always(fo(4), v); }
    if let Some(value) = comm.diag_class {
        let value = match value {
            DiagClassType::StartComm => 0,
            DiagClassType::StopComm => 1,
            DiagClassType::VariantIdentification => 2,
            DiagClassType::ReadDynDefMessage => 3,
            DiagClassType::DynDefMessage => 4,
            DiagClassType::ClearDynDefMessage => 5,
        };
        fbb.push_slot::<u8>(fo(5), value, 0);
    }
    if let Some(v) = pcsr_vec { fbb.push_slot_always(fo(6), v); }
    if let Some(v) = str_vec  { fbb.push_slot_always(fo(7), v); }
    if let Some(a) = audience { fbb.push_slot_always(fo(9), a); }
    fbb.push_slot::<bool>(fo(10), comm.is_mandatory, false);
    fbb.push_slot::<bool>(fo(11), comm.is_executable, true);
    fbb.push_slot::<bool>(fo(12), comm.is_final, false);
    fbb.end_table(start)
}

// ─── DiagService ──────────────────────────────────────────────────────────
// table DiagService {
//   diag_comm: DiagComm;        // 0 → 4
//   request: Request;           // 1 → 6
//   pos_responses: [Response];  // 2 → 8
//   neg_responses: [Response];  // 3 → 10
//   is_cyclic: bool = false;    // 4 → 12
//   is_multiple: bool = false;  // 5 → 14
//   addressing: Addressing;     // 6 → 16
//   transmission_mode: ..;      // 7 → 18
//   com_param_refs: [...];      // 8 → 20
// }
fn build_simple_value(fbb: &mut FlatBufferBuilder<'_>, sv: &SimpleValue) -> WIPOffset<TF> {
    let v = sv.value.as_deref().map(|s| fbb.create_string(s));
    let start = fbb.start_table();
    if let Some(val) = v { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), val); }
    fbb.end_table(start)
}

fn build_complex_value(fbb: &mut FlatBufferBuilder<'_>, cv: &ComplexValue) -> WIPOffset<TF> {
    // ComplexValue has entries: [SimpleOrComplexValueEntry] (union vector)
    // In FlatBuffers union vectors, we need type bytes + offset vectors
    let mut type_bytes: Vec<u8> = Vec::new();
    let mut entry_offs: Vec<WIPOffset<TF>> = Vec::new();

    for entry in &cv.entries {
        match entry {
            SimpleOrComplexEntry::Simple(sv) => {
                type_bytes.push(1); // SimpleValue = union member #1
                entry_offs.push(build_simple_value(fbb, sv));
            }
            SimpleOrComplexEntry::Complex(nested) => {
                type_bytes.push(2); // ComplexValue = union member #2
                entry_offs.push(build_complex_value(fbb, nested));
            }
        }
    }

    let entries_vec = if entry_offs.is_empty() { None } else { Some(fbb.create_vector(&entry_offs)) };
    let types_vec = if type_bytes.is_empty() { None } else { Some(fbb.create_vector(&type_bytes)) };

    let start = fbb.start_table();
    // table ComplexValue { entries: [SimpleOrComplexValueEntry]; }
    // Union vector: types at fo(0), offsets at fo(1)
    if let Some(t) = types_vec  { fbb.push_slot_always(fo(0), t); }
    if let Some(v) = entries_vec { fbb.push_slot_always(fo(1), v); }
    fbb.end_table(start)
}

fn build_com_param_ref(fbb: &mut FlatBufferBuilder<'_>, cpr: &ComParamRef, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    // table ComParamRef {
    //   simple_value: SimpleValue;    // 0 → fo(0) = 4
    //   complex_value: ComplexValue;  // 1 → fo(1) = 6
    //   com_param: ComParam;          // 2 → fo(2) = 8
    //   protocol: Protocol;           // 3 → fo(3) = 10
    //   prot_stack: ProtStack;        // 4 → fo(4) = 12
    // }
    let sv_off = cpr.simple_value.as_ref().map(|sv| build_simple_value(fbb, sv));
    let cv_off = cpr.complex_value.as_ref().map(|cv| build_complex_value(fbb, cv));
    let link = OdxLink {
        id_ref: cpr.id_ref.clone(),
        doc_ref: cpr.doc_ref.clone(),
        doc_type: None,
    };
    let cp_off = odx
        .resolve_comparam(&link, None)
        .map(|cp| build_com_param(fbb, cp, odx, cache))
        .or_else(|| {
            odx.resolve_complex_comparam(&link, None)
                .map(|cp| build_complex_com_param_entry(fbb, cp, odx, cache))
        });

    // Resolve protocol by short-name reference
    let proto_off = cpr.protocol_snref.as_deref()
        .and_then(|sn| odx.resolve_protocol_by_sn(sn, None))
        .map(|p| build_protocol(fbb, p, odx, cache));

    // Resolve prot_stack by short-name reference
    let ps_off = cpr.prot_stack_snref.as_deref()
        .and_then(|sn| odx.resolve_prot_stack_by_sn(sn, None))
        .map(|ps| build_prot_stack(fbb, ps, odx, cache));

    let start = fbb.start_table();
    if let Some(v) = sv_off    { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = cv_off    { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = cp_off    { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = proto_off { fbb.push_slot_always(fo(3), v); }
    if let Some(v) = ps_off    { fbb.push_slot_always(fo(4), v); }
    fbb.end_table(start)
}
fn build_diag_service(fbb: &mut FlatBufferBuilder<'_>, svc: &DiagService, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "diag_service", &svc.comm.id);
    let diag_comm = build_diag_comm(fbb, &svc.comm, odx, cache);

    let req_off = svc.request_ref.as_ref()
        .and_then(|link| odx.resolve_request(link, None))
        .map(|r| build_request(fbb, r, odx, cache));

    let pos_offs: Vec<WIPOffset<TF>> = svc.pos_response_refs.iter()
        .filter_map(|link| odx.resolve_pos_response(link, None))
        .map(|r| build_response(fbb, r, 0, odx, cache))
        .collect();
    let pos_vec = if pos_offs.is_empty() { None } else { Some(fbb.create_vector(&pos_offs)) };

    let neg_offs: Vec<WIPOffset<TF>> = svc.neg_response_refs.iter()
        .filter_map(|link| odx.resolve_neg_response(link, None))
        .map(|r| build_response(fbb, r, 1, odx, cache))
        .collect();
    let neg_vec = if neg_offs.is_empty() { None } else { Some(fbb.create_vector(&neg_offs)) };

    let cpr_offs: Vec<WIPOffset<TF>> = svc.comparam_refs.iter()
        .map(|cpr| build_com_param_ref(fbb, cpr, odx, cache))
        .collect();
    let cpr_vec = if cpr_offs.is_empty() { None } else { Some(fbb.create_vector(&cpr_offs)) };

    let start = fbb.start_table();
    fbb.push_slot_always(fo(0), diag_comm);
    if let Some(r) = req_off  { fbb.push_slot_always(fo(1), r); }
    if let Some(v) = pos_vec  { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = neg_vec  { fbb.push_slot_always(fo(3), v); }
    fbb.push_slot::<bool>(fo(4), svc.is_cyclic, false);
    fbb.push_slot::<bool>(fo(5), svc.is_multiple, false);
    if let Some(value) = svc.addressing {
        let value = match value {
            Addressing::Functional => 0,
            Addressing::Physical => 1,
            Addressing::PhysicalOrFunctional => 2,
        };
        fbb.push_slot::<u8>(fo(6), value, 0);
    }
    if let Some(value) = svc.transmission_mode {
        let value = match value {
            TransmissionMode::SendOnly => 0,
            TransmissionMode::ReceiveOnly => 1,
            TransmissionMode::SendAndReceive => 2,
            TransmissionMode::SendOrReceive => 3,
        };
        fbb.push_slot::<u8>(fo(7), value, 0);
    }
    if let Some(v) = cpr_vec  { fbb.push_slot_always(fo(8), v); }
    cache_and_return!(cache, "diag_service", &svc.comm.id, fbb.end_table(start))
}

// ─── SingleEcuJob ─────────────────────────────────────────────────────────
// table SingleEcuJob {
//   diag_comm: DiagComm;       // 0 → 4
//   prog_codes: [ProgCode];    // 1 → 6
//   input_params: [JobParam];  // 2 → 8  (omitted)
//   output_params: [JobParam]; // 3 → 10 (omitted)
//   neg_output_params: [...];  // 4 → 12 (omitted)
// }
fn build_single_ecu_job(
    fbb: &mut FlatBufferBuilder<'_>,
    job: &SingleEcuJob,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    let pc_offs: Vec<WIPOffset<TF>> = job.prog_codes.iter()
        .map(|pc| build_prog_code(fbb, pc))
        .collect();
    let pc_vec = if pc_offs.is_empty() { None } else { Some(fbb.create_vector(&pc_offs)) };

    let diag_comm = build_diag_comm(fbb, &job.comm, odx, cache);

    let start = fbb.start_table();
    fbb.push_slot_always(fo(0), diag_comm);
    if let Some(v) = pc_vec { fbb.push_slot_always(fo(1), v); }
    fbb.end_table(start)
}

// ─── MatchingParameter ────────────────────────────────────────────────────
// table MatchingParameter {
//   expected_value: string;             // 0 → 4
//   diag_service: DiagService;          // 1 → 6
//   out_param: Param;                   // 2 → 8  (omitted)
//   use_physical_addressing: bool = null; // 3 → 10
// }
fn build_matching_parameter(
    fbb: &mut FlatBufferBuilder<'_>,
    mp: &MatchingParameter,
    layer_id: &str,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    let expected = mp.expected_value.as_deref().map(|s| fbb.create_string(s));

    // Resolve DiagService by short-name within the owning layer's collection
    let svc_off = odx.collections.values()
        .find(|c| {
            c.ecu_variants.contains_key(layer_id) ||
            c.base_variants.contains_key(layer_id)
        })
        .and_then(|c| c.diag_service_by_sn(&mp.diag_comm_snref))
        .map(|svc| build_diag_service(fbb, svc, odx, cache));

    let start = fbb.start_table();
    if let Some(e) = expected { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), e); }
    if let Some(s) = svc_off  { fbb.push_slot_always(fo(1), s); }
    if mp.use_physical_addressing {
        fbb.push_slot::<bool>(fo(3), true, false);
    }
    fbb.end_table(start)
}

// ─── VariantPattern ───────────────────────────────────────────────────────
// table VariantPattern {
//   matching_parameter: [MatchingParameter]; // 0 → 4
// }
fn build_variant_pattern(
    fbb: &mut FlatBufferBuilder<'_>,
    vp: &EcuVariantPattern,
    layer_id: &str,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    let mp_offs: Vec<WIPOffset<TF>> = vp.matching_parameters.iter()
        .map(|mp| build_matching_parameter(fbb, mp, layer_id, odx, cache))
        .collect();
    let mp_vec = fbb.create_vector(&mp_offs);

    let start = fbb.start_table();
    fbb.push_slot_always(fo(0), mp_vec);
    fbb.end_table(start)
}

// ─── ParentRef ────────────────────────────────────────────────────────────
// table ParentRef {
//   ref: ParentRefType (union);              // 0 → ref_type (fo(0)=4, byte)
//                                            // 1 → ref offset (fo(1)=6)
//   not_inherited_diag_comm_short_names: [string];  // 2 → fo(2) = 8
//   not_inherited_variables_short_names: [string];  // 3 → fo(3) = 10
//   not_inherited_dops_short_names: [string];       // 4 → fo(4) = 12
//   not_inherited_tables_short_names: [string];     // 5 → fo(5) = 14
//   not_inherited_global_neg_responses_short_names: [string]; // 6 → fo(6) = 16
// }
//
// ParentRefType union bytes:
//   1 = Variant   2 = Protocol   3 = FunctionalGroup
//   4 = TableDop  5 = EcuSharedData

fn build_parent_ref(
    fbb: &mut FlatBufferBuilder<'_>,
    pr: &ParentRef,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> Option<WIPOffset<TF>> {
    // ── Resolve or build the referenced layer ─────────────────────────────

    // Cycle guard: if already being built (type_byte = 0), skip
    if let Some(cached) = cache.layers.get(&pr.id_ref) {
        if cached.type_byte == 0 { return None; }
        // Return cached offset for already-built layers (DAG sharing)
        let (type_byte, offset) = (cached.type_byte, cached.offset);
        return build_parent_ref_table(fbb, pr, type_byte, WIPOffset::new(offset));
    }

    // Mark as being built to prevent cycles
    cache.layers.insert(pr.id_ref.clone(), CachedLayer { type_byte: 0, offset: 0 });

    // Determine type from doc_type hint, then scan collections
    let resolved: Option<(u8, WIPOffset<TF>)> = resolve_parent_type(fbb, pr, odx, cache);

    if let Some((type_byte, ref_off)) = resolved {
        cache.layers.insert(pr.id_ref.clone(), CachedLayer { type_byte, offset: ref_off.value() });
        build_parent_ref_table(fbb, pr, type_byte, ref_off)
    } else {
        warn!("Could not resolve PARENT-REF id_ref='{}' doc_ref='{:?}'", pr.id_ref, pr.doc_ref);
        cache.layers.remove(&pr.id_ref);
        None
    }
}

/// Resolve the FlatBuffers offset for the layer referenced by `pr`.
fn resolve_parent_type(
    fbb: &mut FlatBufferBuilder<'_>,
    pr: &ParentRef,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> Option<(u8, WIPOffset<TF>)> {
    let id = &pr.id_ref;

    // Use doc_type hint first for faster lookup
    match pr.doc_type {
        Some(ParentRefDocType::EcuVariant) | Some(ParentRefDocType::BaseVariant) => {
            try_build_variant(fbb, id, odx, cache)
        }
        Some(ParentRefDocType::Protocol) => {
            try_build_protocol(fbb, id, odx, cache)
        }
        Some(ParentRefDocType::FunctionalGroup) => {
            try_build_fg(fbb, id, odx, cache)
        }
        Some(ParentRefDocType::EcuSharedData) => {
            try_build_ecu_shared_data(fbb, id, odx, cache)
        }
        _ => {
            // No hint: scan all stores
            try_build_variant(fbb, id, odx, cache)
                .or_else(|| try_build_protocol(fbb, id, odx, cache))
                .or_else(|| try_build_fg(fbb, id, odx, cache))
                .or_else(|| try_build_ecu_shared_data(fbb, id, odx, cache))
        }
    }
}

fn try_build_variant(
    fbb: &mut FlatBufferBuilder<'_>,
    id: &str,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> Option<(u8, WIPOffset<TF>)> {
    // Try ECU-VARIANT first
    for coll in odx.collections.values() {
        if let Some(&idx) = coll.ecu_variants.get(id) {
            let v = &coll.ecu_variant_store[idx];
            let off = build_variant_ecu(fbb, v, odx, cache);
            return Some((1u8, off));
        }
        if let Some(&idx) = coll.base_variants.get(id) {
            let v = &coll.base_variant_store[idx];
            let off = build_variant_base(fbb, v, odx, cache);
            return Some((1u8, off));
        }
    }
    None
}

fn try_build_protocol(
    fbb: &mut FlatBufferBuilder<'_>,
    id: &str,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> Option<(u8, WIPOffset<TF>)> {
    for coll in odx.collections.values() {
        if let Some(&idx) = coll.protocols.get(id) {
            let p = &coll.protocol_store[idx];
            let off = build_protocol(fbb, p, odx, cache);
            return Some((2u8, off));
        }
    }
    None
}

fn try_build_fg(
    fbb: &mut FlatBufferBuilder<'_>,
    id: &str,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> Option<(u8, WIPOffset<TF>)> {
    for coll in odx.collections.values() {
        if let Some(&idx) = coll.functional_groups.get(id) {
            let fg = &coll.functional_group_store[idx];
            let off = build_functional_group(fbb, fg, odx, cache);
            return Some((3u8, off));
        }
    }
    None
}

fn try_build_ecu_shared_data(
    fbb: &mut FlatBufferBuilder<'_>,
    id: &str,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> Option<(u8, WIPOffset<TF>)> {
    for coll in odx.collections.values() {
        if let Some(&idx) = coll.ecu_shared_datas.get(id) {
            let es = &coll.ecu_shared_data_store[idx];
            let layer = build_diag_layer_core(fbb, &es.core, &[], odx, cache);
            // EcuSharedData table: { diag_layer: DiagLayer }  → fo(0) = 4
            let start = fbb.start_table();
            fbb.push_slot_always(fo(0), layer);
            let off = fbb.end_table(start);
            return Some((5u8, off));
        }
    }
    None
}

/// Write the ParentRef table given a resolved union type+offset.
fn build_parent_ref_table<'a>(
    fbb: &mut FlatBufferBuilder<'a>,
    pr: &ParentRef,
    type_byte: u8,
    ref_off: WIPOffset<TF>,
) -> Option<WIPOffset<TF>> {
    // Helper: create a [string] vector
   fn string_vec<'a>(
    fbb: &mut FlatBufferBuilder<'a>,
    items: &[String],
) -> Option<WIPOffset<flatbuffers::Vector<'a, flatbuffers::ForwardsUOffset<&'a str>>>> {
    if items.is_empty() {
        return None;
    }

    let offs: Vec<WIPOffset<&str>> = items
        .iter()
        .map(|s| fbb.create_string(s))
        .collect();

    Some(fbb.create_vector(&offs))
}

    let ni_dc   = string_vec(fbb, &pr.not_inherited_diag_comm_short_names);
    let ni_var  = string_vec(fbb, &pr.not_inherited_variables_short_names);
    let ni_dop  = string_vec(fbb, &pr.not_inherited_dop_short_names);
    let ni_tbl  = string_vec(fbb, &pr.not_inherited_table_short_names);
    let ni_gnr  = string_vec(fbb, &pr.not_inherited_global_neg_response_short_names);

    let start = fbb.start_table();
    // Union type byte at fo(0), union offset at fo(1)
    fbb.push_slot::<u8>(fo(0), type_byte, 0);
    fbb.push_slot_always(fo(1), ref_off);
    if let Some(v) = ni_dc  { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = ni_var { fbb.push_slot_always(fo(3), v); }
    if let Some(v) = ni_dop { fbb.push_slot_always(fo(4), v); }
    if let Some(v) = ni_tbl { fbb.push_slot_always(fo(5), v); }
    if let Some(v) = ni_gnr { fbb.push_slot_always(fo(6), v); }
    Some(fbb.end_table(start))
}

// ─── DiagLayer ────────────────────────────────────────────────────────────
// table DiagLayer {
//   short_name: string;                    // 0 → 4
//   long_name: LongName;                   // 1 → 6
//   funct_classes: [FunctClass];           // 2 → 8
//   com_param_refs: [ComParamRef];         // 3 → 10
//   diag_services: [DiagService];          // 4 → 12
//   single_ecu_jobs: [SingleEcuJob];       // 5 → 14
//   state_charts: [StateChart];            // 6 → 16
//   additional_audiences: [AdditionalAudience]; // 7 → 18
//   sdgs: SDGS;                            // 8 → 20
// }
fn build_diag_layer_core(
    fbb: &mut FlatBufferBuilder<'_>,
    core: &DiagLayerCore,
    comparam_refs: &[ComParamRef],
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    // Diag services (look up by ID)
    let svc_offs: Vec<WIPOffset<TF>> = core.diag_comms.diag_service_ids.iter()
        .filter_map(|id| {
            odx.resolve_diag_service(&OdxLink::new(id.as_str()), Some(&core.id))
                .map(|s| build_diag_service(fbb, s, odx, cache))
        })
        .collect();
    // Kotlin always writes both vectors on a DiagLayer, including empty ones.
    // Preserve that presence because FlatBuffers distinguishes an absent vector
    // from a present zero-length vector.
    let svcs_vec = Some(fbb.create_vector(&svc_offs));

    // Single-ECU jobs
    let job_offs: Vec<WIPOffset<TF>> = core.diag_comms.single_ecu_job_ids.iter()
        .filter_map(|id| {
            odx.resolve_single_ecu_job(&OdxLink::new(id.as_str()), Some(&core.id))
                .map(|j| build_single_ecu_job(fbb, j, odx, cache))
        })
        .collect();
    let jobs_vec = Some(fbb.create_vector(&job_offs));

    // ComParamRefs
    let cpr_offs: Vec<WIPOffset<TF>> = comparam_refs.iter()
        .map(|cpr| build_com_param_ref(fbb, cpr, odx, cache))
        .collect();
    let cpr_vec = if cpr_offs.is_empty() { None } else { Some(fbb.create_vector(&cpr_offs)) };

    // State charts
    let sc_offs: Vec<WIPOffset<TF>> = core.state_chart_ids.iter()
        .filter_map(|id| {
            odx.collections.values()
                .find_map(|c| c.state_charts.get(id.as_str()).map(|&i| &c.state_chart_store[i]))
                .map(|sc| build_state_chart(fbb, sc))
        })
        .collect();
    let sc_vec = if sc_offs.is_empty() { None } else { Some(fbb.create_vector(&sc_offs)) };

    // Funct classes (short-names)
    let fc_offs: Vec<WIPOffset<TF>> = core.funct_classes.iter()
        .map(|sn| build_funct_class(fbb, sn, cache))
        .collect();
    let fc_vec = if fc_offs.is_empty() { None } else { Some(fbb.create_vector(&fc_offs)) };

    // Additional audiences (IDs)
    let aa_offs: Vec<WIPOffset<TF>> = core.additional_audiences.iter()
        .filter_map(|id| {
            odx.collections.values()
                .find_map(|c| c.additional_audiences.get(id.as_str()).map(|&i| &c.additional_audience_store[i]))
                .map(|aa| build_additional_audience(fbb, aa, cache))
        })
        .collect();
    let aa_vec = if aa_offs.is_empty() { None } else { Some(fbb.create_vector(&aa_offs)) };

    let sn = fbb.create_string(&core.short_name);
    let ln = core.long_name.as_ref().map(|l| build_long_name(fbb, l));
    let sdgs = core.sdgs.as_ref().map(|value| build_sdgs(fbb, value));

    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), sn);
    if let Some(l) = ln       { fbb.push_slot_always(fo(1), l); }
    if let Some(v) = fc_vec   { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = cpr_vec  { fbb.push_slot_always(fo(3), v); }
    if let Some(v) = svcs_vec { fbb.push_slot_always(fo(4), v); }
    if let Some(v) = jobs_vec { fbb.push_slot_always(fo(5), v); }
    if let Some(v) = sc_vec   { fbb.push_slot_always(fo(6), v); }
    if let Some(v) = aa_vec   { fbb.push_slot_always(fo(7), v); }
    if let Some(v) = sdgs     { fbb.push_slot_always(fo(8), v); }
    fbb.end_table(start)
}

// ─── Variant ──────────────────────────────────────────────────────────────
// table Variant {
//   diag_layer: DiagLayer;          // 0 → 4
//   is_base_variant: bool = false;  // 1 → 6
//   variant_pattern: [VariantPattern]; // 2 → 8
//   parent_refs: [ParentRef];       // 3 → 10
// }
fn build_variant_ecu(
    fbb: &mut FlatBufferBuilder<'_>,
    v: &EcuVariant,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    // Variant patterns (ECU-VARIANT-PATTERN for identification)
    let vp_offs: Vec<WIPOffset<TF>> = v.ecu_variant_patterns.iter()
        .map(|vp| build_variant_pattern(fbb, vp, &v.core.id, odx, cache))
        .collect();
    let vp_vec = if vp_offs.is_empty() { None } else { Some(fbb.create_vector(&vp_offs)) };

    // Parent refs (only from core, which covers all PARENT-REF elements in the layer XML)
    let pr_offs: Vec<WIPOffset<TF>> = v.core.parent_refs.iter()
        .filter_map(|pr| build_parent_ref(fbb, pr, odx, cache))
        .collect();
    let pr_vec = if pr_offs.is_empty() { None } else { Some(fbb.create_vector(&pr_offs)) };

    let layer = build_diag_layer_core(fbb, &v.core, &v.comparam_refs, odx, cache);

    let start = fbb.start_table();
    fbb.push_slot_always(fo(0), layer);
    fbb.push_slot::<bool>(fo(1), false, false);  // is_base_variant = false
    if let Some(v) = vp_vec { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = pr_vec { fbb.push_slot_always(fo(3), v); }
    fbb.end_table(start)
}

fn build_variant_base(
    fbb: &mut FlatBufferBuilder<'_>,
    v: &BaseVariant,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    // Base variant patterns
    let vp_offs: Vec<WIPOffset<TF>> = v.base_variant_pattern.iter()
        .flat_map(|bvp| &bvp.matching_parameters)
        .map(|mmp| build_matching_parameter_base(fbb, mmp, &v.core.id, odx, cache))
        .collect();
    let vp_vec = if vp_offs.is_empty() { None } else {
        // Wrap in a single VariantPattern containing all matching params
        let mp_vec = fbb.create_vector(&vp_offs);
        let pat_start = fbb.start_table();
        fbb.push_slot_always(fo(0), mp_vec);
        let pat = fbb.end_table(pat_start);
        Some(fbb.create_vector(&[pat]))
    };

    // Parent refs from core.parent_refs (parsed from the same XML node by DiagLayerCore)
    let pr_offs: Vec<WIPOffset<TF>> = v.core.parent_refs.iter()
        .filter_map(|pr| build_parent_ref(fbb, pr, odx, cache))
        .collect();
    let pr_vec = if pr_offs.is_empty() { None } else { Some(fbb.create_vector(&pr_offs)) };

    let layer = build_diag_layer_core(fbb, &v.core, &v.comparam_refs, odx, cache);

    let start = fbb.start_table();
    fbb.push_slot_always(fo(0), layer);
    fbb.push_slot::<bool>(fo(1), true, false);  // is_base_variant = true
    if let Some(v) = vp_vec { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = pr_vec { fbb.push_slot_always(fo(3), v); }
    fbb.end_table(start)
}

/// Build a MatchingParameter from a BaseVariant matching param (same FBS table).
fn build_matching_parameter_base(
    fbb: &mut FlatBufferBuilder<'_>,
    mmp: &MatchingBaseVariantParameter,
    layer_id: &str,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    let expected = fbb.create_string(&mmp.expected_value);

    let svc_off = odx.collections.values()
        .find(|c| c.base_variants.contains_key(layer_id))
        .and_then(|c| c.diag_service_by_sn(&mmp.diag_comm_snref))
        .map(|svc| build_diag_service(fbb, svc, odx, cache));

    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), expected);
    if let Some(s) = svc_off { fbb.push_slot_always(fo(1), s); }
    fbb.push_slot::<bool>(fo(3), mmp.use_physical_addressing, false);
    fbb.end_table(start)
}

// ─── FunctionalGroup ──────────────────────────────────────────────────────
// table FunctionalGroup {
//   diag_layer: DiagLayer;    // 0 → 4
//   parent_refs: [ParentRef]; // 1 → 6
// }
fn build_functional_group(
    fbb: &mut FlatBufferBuilder<'_>,
    fg: &FunctionalGroup,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    let pr_offs: Vec<WIPOffset<TF>> = fg.core.parent_refs.iter()
        .filter_map(|pr| build_parent_ref(fbb, pr, odx, cache))
        .collect();
    let pr_vec = if pr_offs.is_empty() { None } else { Some(fbb.create_vector(&pr_offs)) };

    let layer = build_diag_layer_core(fbb, &fg.core, &fg.comparam_refs, odx, cache);

    let start = fbb.start_table();
    fbb.push_slot_always(fo(0), layer);
    if let Some(v) = pr_vec { fbb.push_slot_always(fo(1), v); }
    fbb.end_table(start)
}

// ─── ProtStack ──────────────────────────────────────────────────────────────
// table ProtStack {
//   short_name: string;                     // 0 → 4
//   long_name: LongName;                    // 1 → 6
//   pdu_protocol_type: string;               // 2 → 8
//   physical_link_type: string;              // 3 → 10
//   comparam_subset_refs: [ComParamSubSet]; // 4 → 12
// }
// table RegularComParam { physical_default_value; dop; }
fn build_regular_com_param(fbb: &mut FlatBufferBuilder<'_>, cp: &ComParam, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    let pdv = cp.physical_default_value.as_deref().map(|s| fbb.create_string(s));
    let dop_off = cp.data_object_prop_ref.as_ref().and_then(|link| {
        match odx.resolve_dop(link, None) {
            Some(dop) => Some(build_dop(fbb, dop, odx, cache)),
            None => {
                warn!(
                    "Could not resolve DATA-OBJECT-PROP-REF '{}' (DOCREF={:?}) for COMPARAM '{}'",
                    link.id_ref,
                    link.doc_ref,
                    cp.short_name
                );
                None
            }
        }
    });

    let start = fbb.start_table();
    if let Some(v) = pdv { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), v); }
    if let Some(v) = dop_off { fbb.push_slot_always(fo(1), v); }
    fbb.end_table(start)
}
// table Limit { value: string; interval_type: IntervalType; }
fn build_limit(fbb: &mut FlatBufferBuilder<'_>, l: &Limit) -> WIPOffset<TF> {
    let value = l.value.as_deref().map(|s| fbb.create_string(s));
    let it = match l.interval_type {
        Some(IntervalType::Open) => Some(0),
        Some(IntervalType::Closed) => Some(1),
        Some(IntervalType::Infinite) => Some(2),
        None => None,
    };
    let start = fbb.start_table();
    if let Some(v) = value { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), v); }
    if let Some(v) = it { fbb.push_slot::<u8>(fo(1), v, 0); }
    fbb.end_table(start)
}

// table ScaleConstr { short_label: Text; lower_limit; upper_limit; validity: ValidType; }
fn build_scale_constr(fbb: &mut FlatBufferBuilder<'_>, sc: &ScaleConstr) -> WIPOffset<TF> {
    let short_label = sc.short_label.as_ref().map(|v| build_text(fbb, v));
    let lower = sc.lower_limit.as_ref().map(|v| build_limit(fbb, v));
    let upper = sc.upper_limit.as_ref().map(|v| build_limit(fbb, v));
    let validity = match sc.validity {
        ConstrValidity::Valid => 0,
        ConstrValidity::Invalid => 1,
        ConstrValidity::NotDefined => 2,
        ConstrValidity::NotAvailable => 3,
    };
    let start = fbb.start_table();
    if let Some(v) = short_label { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = lower { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = upper { fbb.push_slot_always(fo(2), v); }
    fbb.push_slot::<u8>(fo(3), validity, 0);
    fbb.end_table(start)
}

fn build_internal_constr(fbb: &mut FlatBufferBuilder<'_>, value: &InternalConstr) -> WIPOffset<TF> {
    let lower = value.lower_limit.as_ref().map(|v| build_limit(fbb, v));
    let upper = value.upper_limit.as_ref().map(|v| build_limit(fbb, v));
    let scales: Vec<WIPOffset<TF>> = value.scale_constrs.iter().map(|v| build_scale_constr(fbb, v)).collect();
    let scales = if scales.is_empty() { None } else { Some(fbb.create_vector(&scales)) };
    let start = fbb.start_table();
    if let Some(v) = lower { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = upper { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = scales { fbb.push_slot_always(fo(2), v); }
    fbb.end_table(start)
}

fn build_physical_type(fbb: &mut FlatBufferBuilder<'_>, value: &PhysicalType) -> WIPOffset<TF> {
    let radix = match value.display_radix {
        Some(DisplayRadix::Hex) => Some(0),
        Some(DisplayRadix::Decimal) => Some(1),
        Some(DisplayRadix::Binary) => Some(2),
        Some(DisplayRadix::Oct) => Some(3),
        None => None,
    };
    let start = fbb.start_table();
    if let Some(v) = value.precision { fbb.push_slot_always::<u32>(fo(0), v); }
    fbb.push_slot::<u8>(fo(1), data_type_byte(&value.base_data_type), 0);
    if let Some(v) = radix { fbb.push_slot::<u8>(fo(2), v, 0); }
    fbb.end_table(start)
}

fn build_physical_dimension(fbb: &mut FlatBufferBuilder<'_>, value: &PhysicalDimension, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "physical_dimension", &value.id);
    let short_name = fbb.create_string(&value.short_name);
    let long_name = value.long_name.as_ref().map(|v| build_long_name(fbb, v));
    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), short_name);
    if let Some(v) = long_name { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = value.length_exp { fbb.push_slot_always::<i32>(fo(2), v); }
    if let Some(v) = value.mass_exp { fbb.push_slot_always::<i32>(fo(3), v); }
    if let Some(v) = value.time_exp { fbb.push_slot_always::<i32>(fo(4), v); }
    if let Some(v) = value.current_exp { fbb.push_slot_always::<i32>(fo(5), v); }
    if let Some(v) = value.temperature_exp { fbb.push_slot_always::<i32>(fo(6), v); }
    if let Some(v) = value.molar_amount_exp { fbb.push_slot_always::<i32>(fo(7), v); }
    if let Some(v) = value.luminous_intensity_exp { fbb.push_slot_always::<i32>(fo(8), v); }
    cache_and_return!(cache, "physical_dimension", &value.id, fbb.end_table(start))
}

fn build_unit(fbb: &mut FlatBufferBuilder<'_>, value: &Unit, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "unit", &value.id);
    let short_name = fbb.create_string(&value.short_name);
    let display_name = fbb.create_string(&value.display_name);
    let dimension = value.physical_dimension_ref.as_ref()
        .and_then(|link| odx.resolve_phys_dimension(link, None))
        .map(|v| build_physical_dimension(fbb, v, cache));
    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), short_name);
    fbb.push_slot_always::<WIPOffset<&str>>(fo(1), display_name);
    if let Some(v) = value.factor_si_to_unit { fbb.push_slot::<f64>(fo(2), v, 0.0); }
    if let Some(v) = value.offset_si_to_unit { fbb.push_slot::<f64>(fo(3), v, 0.0); }
    if let Some(v) = dimension { fbb.push_slot_always(fo(4), v); }
    cache_and_return!(cache, "unit", &value.id, fbb.end_table(start))
}

fn build_unit_group(fbb: &mut FlatBufferBuilder<'_>, value: &UnitGroup, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    let short_name = fbb.create_string(&value.short_name);
    let long_name = value.long_name.as_ref().map(|v| build_long_name(fbb, v));
    let units: Vec<WIPOffset<TF>> = value.unit_refs.iter().filter_map(|link| odx.resolve_unit(link, None))
        .map(|v| build_unit(fbb, v, odx, cache)).collect();
    let units = if units.is_empty() { None } else { Some(fbb.create_vector(&units)) };
    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), short_name);
    if let Some(v) = long_name { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = units { fbb.push_slot_always(fo(2), v); }
    fbb.end_table(start)
}

fn build_unit_spec(fbb: &mut FlatBufferBuilder<'_>, value: &UnitSpec, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    let groups: Vec<WIPOffset<TF>> = value.unit_groups.iter().map(|v| build_unit_group(fbb, v, odx, cache)).collect();
    let units: Vec<WIPOffset<TF>> = value.units.iter().map(|v| build_unit(fbb, v, odx, cache)).collect();
    let dimensions: Vec<WIPOffset<TF>> = value.physical_dimensions.iter().map(|v| build_physical_dimension(fbb, v, cache)).collect();
    let groups = if groups.is_empty() { None } else { Some(fbb.create_vector(&groups)) };
    let units = if units.is_empty() { None } else { Some(fbb.create_vector(&units)) };
    let dimensions = if dimensions.is_empty() { None } else { Some(fbb.create_vector(&dimensions)) };
    let sdgs = value.sdgs.as_ref().map(|v| build_sdgs(fbb, v));
    let start = fbb.start_table();
    if let Some(v) = groups { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = units { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = dimensions { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = sdgs { fbb.push_slot_always(fo(3), v); }
    fbb.end_table(start)
}

fn build_dop(fbb: &mut FlatBufferBuilder<'_>, dop: &DataObjectProp, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "dop", &dop.id);
    let compu_method = dop.compu_method.as_ref().map(|v| build_compu_method(fbb, v));
    let coded_type = dop.diag_coded_type.as_ref().map(|v| build_diag_coded_type(fbb, v, odx, None, cache));
    let physical_type = dop.physical_type.as_ref().map(|v| build_physical_type(fbb, v));
    let internal_constr = dop.internal_constr.as_ref().map(|v| build_internal_constr(fbb, v));
    let unit = dop.unit_ref.as_ref().and_then(|link| odx.resolve_unit(link, None)).map(|v| build_unit(fbb, v, odx, cache));
    let phys_constr = dop.phys_constr.as_ref().map(|v| build_internal_constr(fbb, v));
    let start = fbb.start_table();
    if let Some(v) = compu_method { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = coded_type { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = physical_type { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = internal_constr { fbb.push_slot_always(fo(3), v); }
    if let Some(v) = unit { fbb.push_slot_always(fo(4), v); }
    if let Some(v) = phys_constr { fbb.push_slot_always(fo(5), v); }
    let specific = fbb.end_table(start);
    let offset = wrap_dop(fbb, 0, &dop.short_name, dop.sdgs.as_ref(), 1, specific);
    cache_and_return!(cache, "dop", &dop.id, offset)
}

fn build_compu_values(fbb: &mut FlatBufferBuilder<'_>, value: &CompuValues) -> WIPOffset<TF> {
    let vt = value.vt.as_deref().map(|v| fbb.create_string(v));
    let ti = value.vt_ti.as_deref().map(|v| fbb.create_string(v));
    let start = fbb.start_table();
    if let Some(v) = value.v { fbb.push_slot::<f64>(fo(0), v, 0.0); }
    if let Some(v) = vt { fbb.push_slot_always::<WIPOffset<&str>>(fo(1), v); }
    if let Some(v) = ti { fbb.push_slot_always::<WIPOffset<&str>>(fo(2), v); }
    fbb.end_table(start)
}

fn build_compu_rational_coeffs(fbb: &mut FlatBufferBuilder<'_>, value: &CompuRationalCoEffs) -> WIPOffset<TF> {
    let numerator = if value.numerator.is_empty() { None } else { Some(fbb.create_vector(&value.numerator)) };
    let denominator = if value.denominator.is_empty() { None } else { Some(fbb.create_vector(&value.denominator)) };
    let start = fbb.start_table();
    if let Some(v) = numerator { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = denominator { fbb.push_slot_always(fo(1), v); }
    fbb.end_table(start)
}

fn build_compu_scale(fbb: &mut FlatBufferBuilder<'_>, value: &CompuScale) -> WIPOffset<TF> {
    let short_label = value.short_label.as_ref().map(|v| build_text(fbb, v));
    let lower = value.lower_limit.as_ref().map(|v| build_limit(fbb, v));
    let upper = value.upper_limit.as_ref().map(|v| build_limit(fbb, v));
    let inverse = value.inverse_value.as_ref().map(|v| build_compu_values(fbb, v));
    let constants = value.compu_const.as_ref().map(|v| build_compu_values(fbb, v));
    let rational = value.rational_coeffs.as_ref().map(|v| build_compu_rational_coeffs(fbb, v));
    let start = fbb.start_table();
    if let Some(v) = short_label { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = lower { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = upper { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = inverse { fbb.push_slot_always(fo(3), v); }
    if let Some(v) = constants { fbb.push_slot_always(fo(4), v); }
    if let Some(v) = rational { fbb.push_slot_always(fo(5), v); }
    fbb.end_table(start)
}

fn build_compu_default_value(fbb: &mut FlatBufferBuilder<'_>, value: &CompuDefaultValue) -> WIPOffset<TF> {
    let values = value.values.as_ref().map(|v| build_compu_values(fbb, v));
    let inverse = value.inverse_values.as_ref().map(|v| build_compu_values(fbb, v));
    let start = fbb.start_table();
    if let Some(v) = values { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = inverse { fbb.push_slot_always(fo(1), v); }
    fbb.end_table(start)
}

fn build_compu_internal_to_phys(fbb: &mut FlatBufferBuilder<'_>, value: &crate::model::compu_method::CompuInternalToPhys) -> WIPOffset<TF> {
    let scales: Vec<WIPOffset<TF>> = value.compu_scales.iter().map(|v| build_compu_scale(fbb, v)).collect();
    let scales = if scales.is_empty() { None } else { Some(fbb.create_vector(&scales)) };
    let default_value = value.compu_default_value.as_ref().map(|v| build_compu_default_value(fbb, v));
    let start = fbb.start_table();
    if let Some(v) = scales { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = default_value { fbb.push_slot_always(fo(2), v); }
    fbb.end_table(start)
}

fn build_compu_phys_to_internal(fbb: &mut FlatBufferBuilder<'_>, value: &crate::model::compu_method::CompuPhysToInternal) -> WIPOffset<TF> {
    let scales: Vec<WIPOffset<TF>> = value.compu_scales.iter().map(|v| build_compu_scale(fbb, v)).collect();
    let scales = if scales.is_empty() { None } else { Some(fbb.create_vector(&scales)) };
    let default_value = value.compu_default_value.as_ref().map(|v| build_compu_default_value(fbb, v));
    let start = fbb.start_table();
    if let Some(v) = scales { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = default_value { fbb.push_slot_always(fo(2), v); }
    fbb.end_table(start)
}

fn build_compu_method(fbb: &mut FlatBufferBuilder<'_>, value: &CompuMethod) -> WIPOffset<TF> {
    let category = match value.category {
        Some(CompuCategory::Identical) => Some(0),
        Some(CompuCategory::Linear) => Some(1),
        Some(CompuCategory::ScaleLinear) => Some(2),
        Some(CompuCategory::Texttable) => Some(3),
        Some(CompuCategory::CompuCode) => Some(4),
        Some(CompuCategory::TabNoInterpol) => Some(5),
        Some(CompuCategory::RatFunc) => Some(6),
        Some(CompuCategory::ScaleRatFunc) => Some(7),
        None => None,
    };
    let internal = value.internal_to_phys.as_ref().map(|v| build_compu_internal_to_phys(fbb, v));
    let physical = value.phys_to_internal.as_ref().map(|v| build_compu_phys_to_internal(fbb, v));
    let start = fbb.start_table();
    if let Some(v) = category { fbb.push_slot::<u8>(fo(0), v, 0); }
    if let Some(v) = internal { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = physical { fbb.push_slot_always(fo(2), v); }
    fbb.end_table(start)
}

// table ComplexComParam { com_params: [ComParam]; complex_physical_default_values; allow_multiple_values; }
fn build_complex_com_param(
    fbb: &mut FlatBufferBuilder<'_>,
    ccp: &ComplexComParam,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    let sub_offs: Vec<WIPOffset<TF>> = ccp
        .sub_params
        .iter()
        .map(|sub_param| match sub_param {
            ComParamOrComplex::Simple(cp) => build_com_param(fbb, cp, odx, cache),
            ComParamOrComplex::Complex(cp) => build_complex_com_param_entry(fbb, cp, odx, cache),
        })
        .collect();
    let sub_vec = if sub_offs.is_empty() {
        None
    } else {
        Some(fbb.create_vector(&sub_offs))
    };

    let default_value_offs: Vec<WIPOffset<TF>> = ccp
        .complex_physical_default_values
        .iter()
        .map(|value| build_complex_value(fbb, value))
        .collect();
    let default_values_vec = if default_value_offs.is_empty() {
        None
    } else {
        Some(fbb.create_vector(&default_value_offs))
    };

    let start = fbb.start_table();
    if let Some(value) = sub_vec {
        fbb.push_slot_always(fo(0), value);
    }
    if let Some(value) = default_values_vec {
        fbb.push_slot_always(fo(1), value);
    }
    fbb.push_slot::<bool>(fo(2), ccp.allow_multiple_values, false);
    fbb.end_table(start)
}

fn build_complex_com_param_entry(
    fbb: &mut FlatBufferBuilder<'_>,
    ccp: &ComplexComParam,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    return_cached!(cache, "complex_comparam", &ccp.id);
    let short_name = fbb.create_string(&ccp.short_name);
    let long_name = ccp.long_name.as_ref().map(|value| build_long_name(fbb, value));
    let param_class = ccp
        .param_class
        .as_deref()
        .map(|value| fbb.create_string(value));
    let complex_data = build_complex_com_param(fbb, ccp, odx, cache);

    let start = fbb.start_table();
    fbb.push_slot::<u8>(fo(0), 1, 0); // COMPLEX
    fbb.push_slot_always::<WIPOffset<&str>>(fo(1), short_name);
    if let Some(value) = long_name {
        fbb.push_slot_always(fo(2), value);
    }
    if let Some(value) = param_class {
        fbb.push_slot_always::<WIPOffset<&str>>(fo(3), value);
    }
    if let Some(value) = ccp.cp_type {
        fbb.push_slot::<u8>(fo(4), cp_type_byte(value), 0);
    }
    if let Some(value) = ccp.display_level {
        fbb.push_slot::<u32>(fo(5), value, u32::MAX);
    }
    if let Some(value) = ccp.cp_usage {
        fbb.push_slot::<u8>(fo(6), cp_usage_byte(value), 0);
    }
    fbb.push_slot::<u8>(fo(7), 2, 0); // ComplexComParam union member
    fbb.push_slot_always(fo(8), complex_data);
    cache_and_return!(cache, "complex_comparam", &ccp.id, fbb.end_table(start))
}

// table ComParam { com_param_type; short_name; long_name; param_class; cp_type; display_level; cp_usage; specific_data; }
fn cp_type_byte(t: CpType) -> u8 {
    match t {
        CpType::Standard => 0,
        CpType::OemSpecific => 1,
        CpType::Optional => 2,
        CpType::OemOptional => 3,
    }
}

fn cp_usage_byte(u: CpUsage) -> u8 {
    match u {
        CpUsage::EcuSoftware => 0,
        CpUsage::EcuComm => 1,
        CpUsage::Application => 2,
        CpUsage::Tester => 3,
    }
}

fn build_com_param(fbb: &mut FlatBufferBuilder<'_>, cp: &ComParam, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "comparam", &cp.id);
    let sn = fbb.create_string(&cp.short_name);
    let ln = cp.long_name.as_ref().map(|l| build_long_name(fbb, l));
    let pc = cp.param_class.as_deref().map(|s| fbb.create_string(s));
    let regular = build_regular_com_param(fbb, cp, odx, cache) ;  // ComParamType::REGULAR = 0
    let start = fbb.start_table();
    fbb.push_slot::<u8>(fo(0), 0, 0);  // com_param_type = REGULAR
    fbb.push_slot_always::<WIPOffset<&str>>(fo(1), sn);
    if let Some(l) = ln { fbb.push_slot_always(fo(2), l); }
    if let Some(p) = pc { fbb.push_slot_always::<WIPOffset<&str>>(fo(3), p); }
    if let Some(t) = cp.cp_type { fbb.push_slot::<u8>(fo(4), cp_type_byte(t), 0); }
    if let Some(dl) = cp.display_level { fbb.push_slot::<u32>(fo(5), dl, u32::MAX); }
    if let Some(u) = cp.cp_usage { fbb.push_slot::<u8>(fo(6), cp_usage_byte(u), 0); }
    fbb.push_slot::<u8>(fo(7), 1, 0);   // specific_data type = RegularComParam (union member #1)
    fbb.push_slot_always(fo(8), regular);   // fo(8), not fo(7) + 1
    cache_and_return!(cache, "comparam", &cp.id, fbb.end_table(start))
}
// table ComParamSubSet { com_params: [ComParam]; complex_com_params: [ComParam]; data_object_props: [DOP]; unit_spec: UnitSpec; }
fn build_comparam_subset(fbb: &mut FlatBufferBuilder<'_>, cs: &ComParamSubset, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "comparam_subset", &cs.id);
    let cp_offs: Vec<WIPOffset<TF>> = cs.comparams.iter().map(|cp| build_com_param(fbb, cp, odx, cache)).collect();
    let cp_vec = if cp_offs.is_empty() { None } else { Some(fbb.create_vector(&cp_offs)) };

    let ccp_offs: Vec<WIPOffset<TF>> = cs
        .complex_comparams
        .iter()
        .map(|ccp| build_complex_com_param_entry(fbb, ccp, odx, cache))
        .collect();
    let ccp_vec = if ccp_offs.is_empty() { None } else { Some(fbb.create_vector(&ccp_offs)) };

    let dop_offs: Vec<WIPOffset<TF>> = cs.data_object_props.iter()
        .map(|dop| build_dop(fbb, dop, odx, cache))
        .collect();
    let dop_vec = if dop_offs.is_empty() { None } else { Some(fbb.create_vector(&dop_offs)) };
    let unit_spec = cs.unit_spec.as_ref().map(|value| build_unit_spec(fbb, value, odx, cache));

    let start = fbb.start_table();
    if let Some(v) = cp_vec  { fbb.push_slot_always(fo(0), v); }
    if let Some(v) = ccp_vec { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = dop_vec { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = unit_spec { fbb.push_slot_always(fo(3), v); }
    cache_and_return!(cache, "comparam_subset", &cs.id, fbb.end_table(start))
}
fn build_prot_stack(fbb: &mut FlatBufferBuilder<'_>, ps: &ProtStack, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "prot_stack", &ps.id);
    let short_name = fbb.create_string(&ps.short_name);
    let long_name = ps.long_name.as_ref().map(|ln| build_long_name(fbb, ln));
    let pdu_protocol_type = ps.pdu_protocol_type.as_deref().map(|s| fbb.create_string(s));
    let physical_link_type = ps.physical_link_type.as_deref().map(|s| fbb.create_string(s));

    let cs_offs: Vec<WIPOffset<TF>> = ps.comparam_subset_refs.iter()
        .filter_map(|link| odx.resolve_comparam_subset(link, None))
        .map(|cs| build_comparam_subset(fbb, cs, odx, cache))
        .collect();
    let cs_vec = if cs_offs.is_empty() { None } else { Some(fbb.create_vector(&cs_offs)) };

    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), short_name);
    if let Some(ln) = long_name { fbb.push_slot_always(fo(1), ln); }
    if let Some(v) = pdu_protocol_type { fbb.push_slot_always::<WIPOffset<&str>>(fo(2), v); }
    if let Some(v) = physical_link_type { fbb.push_slot_always::<WIPOffset<&str>>(fo(3), v); }
    if let Some(v) = cs_vec { fbb.push_slot_always(fo(4), v); }
    cache_and_return!(cache, "prot_stack", &ps.id, fbb.end_table(start))
}

// ─── ComParamSpec ─────────────────────────────────────────────────────────
// table ComParamSpec {
//   prot_stacks: [ProtStack];  // 0 → 4
// }
fn build_com_param_spec(fbb: &mut FlatBufferBuilder<'_>, cps: &ComParamSpec, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "comparam_spec", &cps.id);
    let ps_offs: Vec<WIPOffset<TF>> = cps.prot_stacks.iter()
        .map(|ps| build_prot_stack(fbb, ps, odx, cache))
        .collect();
    let ps_vec = if ps_offs.is_empty() { None } else { Some(fbb.create_vector(&ps_offs)) };
    let start = fbb.start_table();
    if let Some(v) = ps_vec { fbb.push_slot_always(fo(0), v); }
    cache_and_return!(cache, "comparam_spec", &cps.id, fbb.end_table(start))
}

/// Resolve the protocol's direct PROT-STACK reference in the same order as
/// the Kotlin converter: first inside the referenced COMPARAM-SPEC, then by a
/// global short-name lookup. Scoping to the spec avoids selecting a same-named
/// stack from another document.
fn resolve_protocol_prot_stack<'a>(
    protocol: &Protocol,
    odx: &'a OdxCollectionGroup,
) -> Option<&'a ProtStack> {
    let short_name = protocol.prot_stack_snref.as_deref()?;

    if let Some(spec_ref) = protocol.comparam_spec_ref.as_ref() {
        if let Some(spec) = odx.resolve_comparam_spec(spec_ref, None) {
            if let Some(stack) = spec
                .prot_stacks
                .iter()
                .find(|stack| stack.short_name == short_name)
            {
                return Some(stack);
            }
        }
    }

    odx.resolve_prot_stack_by_sn(short_name, None)
}

// ─── Protocol ─────────────────────────────────────────────────────────────
// table Protocol {
//   diag_layer: DiagLayer;    // 0 → 4
//   com_param_spec: ComParamSpec; // 1 → 6
//   prot_stack: ProtStack;    // 2 → 8
//   parent_refs: [ParentRef]; // 3 → 10
// }
fn build_protocol(
    fbb: &mut FlatBufferBuilder<'_>,
    p: &Protocol,
    odx: &OdxCollectionGroup,
    cache: &mut BuildCache,
) -> WIPOffset<TF> {
    if let Some(cached) = cache.layers.get(&p.core.id) {
        if cached.type_byte == 2 {
            return WIPOffset::new(cached.offset);
        }
    }
    let pr_offs: Vec<WIPOffset<TF>> = p.core.parent_refs.iter()
        .filter_map(|pr| build_parent_ref(fbb, pr, odx, cache))
        .collect();
    let pr_vec = if pr_offs.is_empty() { None } else { Some(fbb.create_vector(&pr_offs)) };

    let cps_off = p.comparam_spec_ref.as_ref()
        .and_then(|link| odx.resolve_comparam_spec(link, None))
        .map(|cps| build_com_param_spec(fbb, cps, odx, cache));

    let prot_stack_off = match resolve_protocol_prot_stack(p, odx) {
        Some(prot_stack) => Some(build_prot_stack(fbb, prot_stack, odx, cache)),
        None => {
            if let Some(short_name) = p.prot_stack_snref.as_deref() {
                warn!(
                    "Could not resolve PROT-STACK-SNREF '{}' for protocol '{}'",
                    short_name,
                    p.core.short_name
                );
            }
            None
        }
    };

    // Kotlin serializes PROTOCOL through DIAGLAYER.offsetType(), which does
    // not include hierarchy-element COMPARAM-REFS in Protocol.diag_layer.
    let layer = build_diag_layer_core(fbb, &p.core, &[], odx, cache);

    let start = fbb.start_table();
    fbb.push_slot_always(fo(0), layer);
    if let Some(v) = cps_off { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = prot_stack_off { fbb.push_slot_always(fo(2), v); }
    if let Some(v) = pr_vec { fbb.push_slot_always(fo(3), v); }
    let offset = fbb.end_table(start);
    cache.layers.insert(
        p.core.id.clone(),
        CachedLayer { type_byte: 2, offset: offset.value() },
    );
    offset
}

// ─── TableRow ─────────────────────────────────────────────────────────────
// table TableRow {
//   short_name: string;                    // 0
//   long_name: LongName;                   // 1
//   key: string;                           // 2
//   dop: DOP;                              // 3
//   structure: DOP;                        // 4
//   sdgs: SDGS;                            // 5
//   audience: Audience;                    // 6
//   funct_class_refs: [FunctClass];        // 7
//   state_transition_refs: [...];          // 8
//   pre_condition_state_refs: [...];       // 9
//   is_executable: bool = true;            // 10
//   semantic: string;                      // 11
//   is_mandatory: bool = false;            // 12
//   is_final: bool = false;               // 13
// }
fn build_table_row(fbb: &mut FlatBufferBuilder<'_>, row: &TableRow, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "table_row", &row.id);
    let short_name = fbb.create_string(&row.short_name);
    let long_name = row.long_name.as_ref().map(|v| build_long_name(fbb, v));
    let key = row.key.as_deref().map(|v| fbb.create_string(v));
    let semantic = row.semantic.as_deref().map(|v| fbb.create_string(v));
    let dop = build_dop_ref_or_sn(fbb, odx, row.dop_ref.as_ref(), row.dop_snref.as_deref(), cache);
    let structure = build_dop_ref_or_sn(fbb, odx, row.structure_ref.as_ref(), row.structure_snref.as_deref(), cache);
    let sdgs = row.sdgs.as_ref().map(|v| build_sdgs(fbb, v));
    let audience = row.audience.as_ref().map(|v| build_audience(fbb, v, odx, cache));
    let funct_classes: Vec<WIPOffset<TF>> = row.funct_class_refs.iter()
        .filter_map(|link| odx.resolve_funct_class(link, None))
        .map(|v| build_funct_class(fbb, &v.short_name, cache)).collect();
    let funct_classes = if funct_classes.is_empty() { None } else { Some(fbb.create_vector(&funct_classes)) };
    let transitions: Vec<WIPOffset<TF>> = row.state_transition_refs.iter()
        .map(|v| build_state_transition_ref(fbb, v, odx)).collect();
    let transitions = if transitions.is_empty() { None } else { Some(fbb.create_vector(&transitions)) };
    let preconditions: Vec<WIPOffset<TF>> = row.precondition_state_refs.iter()
        .map(|v| build_precondition_state_ref(fbb, v, odx)).collect();
    let preconditions = if preconditions.is_empty() { None } else { Some(fbb.create_vector(&preconditions)) };

    let start = fbb.start_table();
    fbb.push_slot_always::<WIPOffset<&str>>(fo(0), short_name);
    if let Some(v) = long_name { fbb.push_slot_always(fo(1), v); }
    if let Some(v) = key { fbb.push_slot_always::<WIPOffset<&str>>(fo(2), v); }
    if let Some(v) = dop { fbb.push_slot_always(fo(3), v); }
    if let Some(v) = structure { fbb.push_slot_always(fo(4), v); }
    if let Some(v) = sdgs { fbb.push_slot_always(fo(5), v); }
    if let Some(v) = audience { fbb.push_slot_always(fo(6), v); }
    if let Some(v) = funct_classes { fbb.push_slot_always(fo(7), v); }
    if let Some(v) = transitions { fbb.push_slot_always(fo(8), v); }
    if let Some(v) = preconditions { fbb.push_slot_always(fo(9), v); }
    fbb.push_slot::<bool>(fo(10), row.is_executable, true);
    if let Some(v) = semantic { fbb.push_slot_always::<WIPOffset<&str>>(fo(11), v); }
    fbb.push_slot::<bool>(fo(12), row.is_mandatory, false);
    fbb.push_slot::<bool>(fo(13), row.is_final, false);
    cache_and_return!(cache, "table_row", &row.id, fbb.end_table(start))
}

// ─── TableDop ─────────────────────────────────────────────────────────────
// table TableDop {
//   semantic: string;                      // 0
//   short_name: string;                    // 1
//   long_name: LongName;                   // 2
//   key_label: string;                     // 3
//   struct_label: string;                  // 4
//   key_dop: DOP;                          // 5
//   rows: [TableRow];                      // 6
//   diag_comm_connector: [...];            // 7  (omitted)
//   sdgs: SDGS;                            // 8
// }
fn build_table_dop(fbb: &mut FlatBufferBuilder<'_>, table: &Table, odx: &OdxCollectionGroup, cache: &mut BuildCache) -> WIPOffset<TF> {
    return_cached!(cache, "table", &table.id);
    let sn = fbb.create_string(&table.short_name);
    let ln = table.long_name.as_ref().map(|l| build_long_name(fbb, l));
    let semantic = table.semantic.as_deref().map(|s| fbb.create_string(s));
    let key_label = table.key_label.as_deref().map(|s| fbb.create_string(s));
    let struct_label = table.struct_label.as_deref().map(|s| fbb.create_string(s));
    let sdgs = table.sdgs.as_ref().map(|v| build_sdgs(fbb, v));

    // key_dop
    let key_dop_off = table.key_dop_ref.as_ref()
        .and_then(|link| build_any_dop(fbb, link, odx, cache));
    // rows
    let row_offs: Vec<WIPOffset<TF>> = table.rows.iter()
        .filter_map(|row_or_link| match row_or_link {
            TableRowOrLink::Row(row) => Some(build_table_row(fbb, row, odx, cache)),
            TableRowOrLink::OdxLink(link) => {
                odx.resolve_table_row(link, None)
                    .map(|row| build_table_row(fbb, row, odx, cache))
            }
        })
        .collect();
    let rows_vec = if row_offs.is_empty() { None } else { Some(fbb.create_vector(&row_offs)) };

    let start = fbb.start_table();
    if let Some(s) = semantic    { fbb.push_slot_always::<WIPOffset<&str>>(fo(0), s); }
    fbb.push_slot_always::<WIPOffset<&str>>(fo(1), sn);
    if let Some(l) = ln          { fbb.push_slot_always(fo(2), l); }
    if let Some(k) = key_label   { fbb.push_slot_always::<WIPOffset<&str>>(fo(3), k); }
    if let Some(s) = struct_label { fbb.push_slot_always::<WIPOffset<&str>>(fo(4), s); }
    if let Some(d) = key_dop_off { fbb.push_slot_always(fo(5), d); }
    if let Some(v) = rows_vec    { fbb.push_slot_always(fo(6), v); }
    if let Some(v) = sdgs        { fbb.push_slot_always(fo(8), v); }
    cache_and_return!(cache, "table", &table.id, fbb.end_table(start))
}
