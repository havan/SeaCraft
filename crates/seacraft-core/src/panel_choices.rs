//! The choices a user makes for plywood panels, checked once for every front end.
//!
//! The CLI, the GUI, the control channel and MCP each name a choice their own way (`--stock-mm`,
//! `stock_mm`, a field label), so a refusal carries the [`Field`] and the words that follow its
//! name; each front end puts its own name in front.

use crate::panels::{Limits, Stock};

/// Which choice a refusal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    MaxSeamDrift,
    MaxStretch,
    OffsetSpacing,
    Stock,
    Scarf,
}

impl Field {
    /// The choice's name in the control channel and MCP, which [`ChoiceError`]'s `Display` uses.
    pub fn key(self) -> &'static str {
        match self {
            Field::MaxSeamDrift => "max_seam_drift_mm",
            Field::MaxStretch => "max_stretch_pct",
            Field::OffsetSpacing => "offset_spacing_mm",
            Field::Stock => "stock_mm",
            Field::Scarf => "scarf_mm",
        }
    }
}

/// A refused choice: the words follow the front end's name for `field`, after a space.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceError {
    pub field: Field,
    pub message: String,
}

impl std::fmt::Display for ChoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` {}", self.field.key(), self.message)
    }
}

/// The choices as the user gives them, in the CLI's units.
#[derive(Debug, Clone, PartialEq)]
pub struct Choices {
    pub max_seam_drift_mm: f64,
    pub max_stretch_pct: f64,
    pub offset_spacing_mm: f64,
    /// `LENGTHxWIDTH` in millimetres, such as `2440x1220`.
    pub stock_mm: String,
    pub scarf_mm: f64,
}

impl Default for Choices {
    fn default() -> Self {
        Self {
            max_seam_drift_mm: 2.0,
            max_stretch_pct: 0.1,
            offset_spacing_mm: 250.0,
            stock_mm: "2440x1220".into(),
            scarf_mm: 0.0,
        }
    }
}

/// The choices in the units [`crate::panels::PanelOptions`] takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Checked {
    pub limits: Limits,
    pub offset_spacing_m: f64,
    pub stock: Stock,
}

/// Checks every choice, in the order the CLI lists them.
pub fn check(c: &Choices) -> Result<Checked, ChoiceError> {
    let refuse = |field, message: String| ChoiceError { field, message };
    let positive = |field, v: f64| {
        if v.is_finite() && v > 0.0 {
            Ok(v)
        } else {
            Err(refuse(field, format!("must be more than zero, not {v}")))
        }
    };
    let drift = positive(Field::MaxSeamDrift, c.max_seam_drift_mm)?;
    let stretch = positive(Field::MaxStretch, c.max_stretch_pct)?;
    let spacing = positive(Field::OffsetSpacing, c.offset_spacing_mm)?;
    let stock = c
        .stock_mm
        .split_once(['x', 'X'])
        .and_then(|(l, w)| Some((l.trim().parse::<f64>().ok()?, w.trim().parse::<f64>().ok()?)))
        .filter(|(l, w)| [l, w].iter().all(|v| v.is_finite() && **v > 0.0))
        .ok_or_else(|| {
            refuse(
                Field::Stock,
                format!(
                    "takes LENGTHxWIDTH in millimetres, such as 2440x1220, not `{}`",
                    c.stock_mm
                ),
            )
        })?;
    if !(c.scarf_mm.is_finite() && c.scarf_mm >= 0.0 && c.scarf_mm < stock.0) {
        return Err(refuse(
            Field::Scarf,
            format!(
                "must be from 0 to less than the sheet's length, not {}",
                c.scarf_mm
            ),
        ));
    }
    Ok(Checked {
        limits: Limits {
            seam_drift_m: drift / 1000.0,
            rms_stretch: stretch / 100.0,
        },
        offset_spacing_m: spacing / 1000.0,
        stock: Stock {
            length_m: stock.0 / 1000.0,
            width_m: stock.1 / 1000.0,
            scarf_m: c.scarf_mm / 1000.0,
        },
    })
}

/// Refuses paper options given for an SVG or DXF (`extension` without the dot, any case);
/// `given` holds the front end's names for the options the user gave.
pub fn pdf_only(extension: &str, given: &[&str]) -> Result<(), String> {
    if given.is_empty() || !matches!(extension.to_ascii_lowercase().as_str(), "svg" | "dxf") {
        return Ok(());
    }
    Err(format!(
        "{} apply to PDF only: an SVG or a DXF is one drawing at the scale asked for",
        given.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::{Limits, Stock};

    #[test]
    fn the_defaults_are_the_cli_defaults() {
        let c = check(&Choices::default()).expect("defaults pass");
        assert_eq!(c.limits, Limits::default());
        assert_eq!(c.offset_spacing_m, 0.25);
        assert_eq!(c.stock, Stock::default());
    }

    #[test]
    fn each_bad_choice_names_its_field() {
        let refused = |c: Choices| check(&c).expect_err("refused");
        for v in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let e = refused(Choices {
                max_seam_drift_mm: v,
                ..Choices::default()
            });
            assert_eq!(e.field, Field::MaxSeamDrift);
            assert_eq!(e.message, format!("must be more than zero, not {v}"));
            let e = refused(Choices {
                max_stretch_pct: v,
                ..Choices::default()
            });
            assert_eq!(e.field, Field::MaxStretch);
            assert_eq!(e.message, format!("must be more than zero, not {v}"));
            let e = refused(Choices {
                offset_spacing_mm: v,
                ..Choices::default()
            });
            assert_eq!(e.field, Field::OffsetSpacing);
            assert_eq!(e.message, format!("must be more than zero, not {v}"));
        }
        for s in ["2440", "x", "0x10", "2440x-1", "axb", ""] {
            let e = refused(Choices {
                stock_mm: s.into(),
                ..Choices::default()
            });
            assert_eq!(e.field, Field::Stock);
            assert_eq!(
                e.message,
                format!("takes LENGTHxWIDTH in millimetres, such as 2440x1220, not `{s}`")
            );
        }
        for v in [-1.0, 2440.0, f64::NAN] {
            let e = refused(Choices {
                scarf_mm: v,
                ..Choices::default()
            });
            assert_eq!(e.field, Field::Scarf);
            assert_eq!(
                e.message,
                format!("must be from 0 to less than the sheet's length, not {v}")
            );
        }
        let c = check(&Choices {
            stock_mm: " 3000 X 1500 ".into(),
            scarf_mm: 200.0,
            ..Choices::default()
        })
        .unwrap();
        assert_eq!(
            (c.stock.length_m, c.stock.width_m, c.stock.scarf_m),
            (3.0, 1.5, 0.2)
        );
    }

    #[test]
    fn each_field_has_its_control_and_mcp_name() {
        let keys: Vec<&str> = [
            Field::MaxSeamDrift,
            Field::MaxStretch,
            Field::OffsetSpacing,
            Field::Stock,
            Field::Scarf,
        ]
        .into_iter()
        .map(Field::key)
        .collect();
        assert_eq!(
            keys,
            [
                "max_seam_drift_mm",
                "max_stretch_pct",
                "offset_spacing_mm",
                "stock_mm",
                "scarf_mm"
            ]
        );
        let e = check(&Choices {
            stock_mm: "2440".into(),
            ..Choices::default()
        })
        .unwrap_err();
        assert_eq!(
            e.to_string(),
            "`stock_mm` takes LENGTHxWIDTH in millimetres, such as 2440x1220, not `2440`"
        );
    }

    #[test]
    fn paper_options_apply_to_pdf_only() {
        assert!(pdf_only("pdf", &["paper"]).is_ok());
        assert!(pdf_only("svg", &[]).is_ok());
        assert_eq!(
            pdf_only("svg", &["--paper", "--single-page"]),
            Err(
                "--paper, --single-page apply to PDF only: an SVG or a DXF is one drawing at \
                 the scale asked for"
                    .into()
            )
        );
        assert!(pdf_only("DXF", &["margin_mm"]).is_err());
    }
}
