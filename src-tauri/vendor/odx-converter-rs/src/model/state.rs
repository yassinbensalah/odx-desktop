// src/model/state.rs – STATE-CHART, STATE, STATE-TRANSITION and related refs.

use crate::model::odx::{OdxLink, LongName};

// ─── STATE-CHART ──────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct StateChart {
    pub id: String,
    pub short_name: String,
    pub semantic: String,
    pub start_state_snref: String,
    pub states: Vec<State>,
    pub state_transitions: Vec<StateTransition>,
}

// ─── STATE ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct State {
    pub id: String,
    pub short_name: String,
    pub long_name: Option<LongName>,
}

// ─── STATE-TRANSITION ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct StateTransition {
    pub id: String,
    pub short_name: String,
    pub source_snref: String,
    pub target_snref: String,
}

// ─── PRECONDITION-STATE-REF ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PreConditionStateRef {
    pub id_ref: String,
    pub doc_ref: Option<String>,
    pub value: Option<String>,
    pub in_param_if_snref: Option<String>,
    pub in_param_if_snpathref: Option<String>,
}

// ─── STATE-TRANSITION-REF (re-exported from diag_service) ─────────────────

pub use crate::model::diag_service::StateTransitionRef;
