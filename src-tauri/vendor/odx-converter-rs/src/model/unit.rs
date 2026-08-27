// src/model/unit.rs – UNIT, UNIT-SPEC, UNIT-GROUP, PHYSICAL-DIMENSION.

use crate::model::odx::{OdxLink, LongName, Sdgs};

#[derive(Debug)]
pub struct Unit {
    pub id: String,
    pub short_name: String,
    pub display_name: String,
    pub factor_si_to_unit: Option<f64>,
    pub offset_si_to_unit: Option<f64>,
    pub physical_dimension_ref: Option<OdxLink>,
}

#[derive(Debug, Default)]
pub struct UnitSpec {
    pub units: Vec<Unit>,
    pub unit_groups: Vec<UnitGroup>,
    pub physical_dimensions: Vec<PhysicalDimension>,
    pub sdgs: Option<Sdgs>,
}

#[derive(Debug)]
pub struct UnitGroup {
    pub short_name: String,
    pub long_name: Option<LongName>,
    pub category: Option<String>,
    pub unit_refs: Vec<OdxLink>,
}

#[derive(Debug)]
pub struct PhysicalDimension {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
    // SI base dimension exponents
    pub length_exp: Option<i32>,
    pub mass_exp: Option<i32>,
    pub time_exp: Option<i32>,
    pub current_exp: Option<i32>,
    pub temperature_exp: Option<i32>,
    pub molar_amount_exp: Option<i32>,
    pub luminous_intensity_exp: Option<i32>,
}
