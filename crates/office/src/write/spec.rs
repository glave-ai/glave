//! The description of a workbook to write, as JSON. Unknown fields are refused, so a typo is
//! said rather than left out of the file.

use std::collections::BTreeMap;

use rust_xlsxwriter::ChartType;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Spec {
    pub(super) sheets: Vec<SheetSpec>,
    /// Defined names and what each refers to: `{"TaxRate": "Summary!$B$4"}`.
    #[serde(default)]
    pub(super) names: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SheetSpec {
    pub(super) name: String,
    /// Rows of values from `start`, each a list of cells.
    #[serde(default)]
    pub(super) rows: Vec<Vec<Value>>,
    /// The cell the first row starts at: `A1` when absent.
    #[serde(default)]
    pub(super) start: Option<String>,
    /// The first row is a header: bold and shaded, frozen, with a filter over the rows.
    #[serde(default)]
    pub(super) header: bool,
    /// Number formats by range: `{"C2:D13": "$#,##0.00", "E2:E13": "0.0%"}`.
    #[serde(default)]
    pub(super) formats: BTreeMap<String, String>,
    /// Ranges in bold: `["A14:E14"]`.
    #[serde(default)]
    pub(super) bold: Vec<String>,
    /// Column widths in characters, by column: `{"A": 14}`. Others fit what they hold.
    #[serde(default)]
    pub(super) widths: BTreeMap<String, f64>,
    /// The first cell that scrolls: `B2` keeps row 1 and column A in view.
    #[serde(default)]
    pub(super) freeze: Option<String>,
    #[serde(default)]
    pub(super) charts: Vec<ChartSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChartSpec {
    #[serde(rename = "type")]
    pub(super) kind: ChartKind,
    /// The cell its top left corner sits on.
    pub(super) at: String,
    #[serde(default)]
    pub(super) title: Option<String>,
    /// The range naming each point: months, regions.
    #[serde(default)]
    pub(super) categories: Option<String>,
    /// One range per series.
    pub(super) values: Vec<String>,
    /// Each series' name, in the order of `values`.
    #[serde(default)]
    pub(super) names: Vec<String>,
    #[serde(default)]
    pub(super) x_title: Option<String>,
    #[serde(default)]
    pub(super) y_title: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ChartKind {
    Column,
    Bar,
    Line,
    Pie,
    Doughnut,
    Area,
    Scatter,
}

impl ChartKind {
    pub(super) fn chart_type(self) -> ChartType {
        match self {
            Self::Column => ChartType::Column,
            Self::Bar => ChartType::Bar,
            Self::Line => ChartType::Line,
            Self::Pie => ChartType::Pie,
            Self::Doughnut => ChartType::Doughnut,
            Self::Area => ChartType::Area,
            Self::Scatter => ChartType::Scatter,
        }
    }
}
