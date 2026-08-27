// src/model/compu_method.rs – COMPUMETHOD and all scale/coefficient types.

/// Computation method: maps internal (raw) coded values to physical values.
#[derive(Debug)]
pub struct CompuMethod {
    pub category: Option<CompuCategory>,
    pub internal_to_phys: Option<CompuInternalToPhys>,
    pub phys_to_internal: Option<CompuPhysToInternal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompuCategory {
    Identical,
    Linear,
    ScaleLinear,
    Texttable,
    CompuCode,
    RatFunc,
    ScaleRatFunc,
    TabNoInterpol,
}

#[derive(Debug, Default)]
pub struct CompuInternalToPhys {
    pub prog_code: Option<ProgCodeRef>,
    pub compu_scales: Vec<CompuScale>,
    pub compu_default_value: Option<CompuDefaultValue>,
}

#[derive(Debug, Default)]
pub struct CompuPhysToInternal {
    pub prog_code: Option<ProgCodeRef>,
    pub compu_scales: Vec<CompuScale>,
    pub compu_default_value: Option<CompuDefaultValue>,
}

/// Reference to a computation program code block.
#[derive(Debug)]
pub struct ProgCodeRef {
    pub code_file: Option<String>,
    pub encryption: Option<String>,
    pub syntax: Option<String>,
    pub revision: Option<String>,
    pub entrypoint: Option<String>,
    pub library_refs: Vec<crate::model::odx::OdxLink>,
}

#[derive(Debug, Default)]
pub struct CompuScale {
    pub short_label: Option<crate::model::odx::Text>,
    pub lower_limit: Option<crate::model::dop::Limit>,
    pub upper_limit: Option<crate::model::dop::Limit>,
    pub inverse_value: Option<CompuValues>,
    pub compu_const: Option<CompuValues>,
    pub rational_coeffs: Option<CompuRationalCoEffs>,
}

/// Numerical or text constant used in a scale or default value.
#[derive(Debug, Default)]
pub struct CompuValues {
    pub v: Option<f64>,
    pub vt: Option<String>,
    pub vt_ti: Option<String>,
}

#[derive(Debug, Default)]
pub struct CompuDefaultValue {
    pub values: Option<CompuValues>,
    pub inverse_values: Option<CompuValues>,
}

/// Coefficients for a rational function: phys = (a0 + a1*raw + ...) / (b0 + b1*raw + ...).
#[derive(Debug, Default)]
pub struct CompuRationalCoEffs {
    pub numerator: Vec<f64>,
    pub denominator: Vec<f64>,
}
