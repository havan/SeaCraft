//! Lengths as text, for labels and tables a builder reads.
//!
//! The program works in metres. Metric is always shown, and first; imperial is a second reading
//! beside it, never a replacement, so turning it on changes no metric number.
//!
//! Imperial readings are to the nearest sixteenth of an inch, with 1 in = 25.4 mm exactly.

/// Metres in one inch, exactly, by the 1959 international definition.
pub const M_PER_IN: f64 = 0.0254;

/// A number with `decimals` places, and no minus sign on a zero.
fn fixed(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    match s.strip_prefix('-') {
        Some(rest) if rest.chars().all(|c| c == '0' || c == '.') => rest.to_owned(),
        _ => s,
    }
}

/// Metres to the millimetre: `2.400 m`.
pub fn metres(m: f64) -> String {
    format!("{} m", fixed(m, 3))
}

/// Whole millimetres with no unit, for a table whose heading names it: `2400`.
pub fn millimetres(m: f64) -> String {
    fixed(m * 1000.0, 0)
}

/// Millimetres to a tenth, for a thickness: `6.0 mm`.
pub fn mm_tenths(m: f64) -> String {
    format!("{} mm", fixed(m * 1000.0, 1))
}

/// A length as a sign, whole feet, whole inches and sixteenths of an inch, rounded to the
/// nearest sixteenth. `None` when the length is not a finite number.
fn sixteenths(m: f64) -> Option<(&'static str, u64, u64, u64)> {
    if !m.is_finite() {
        return None;
    }
    let total = (m.abs() / M_PER_IN * 16.0).round() as u64;
    let sign = if m < 0.0 && total > 0 { "-" } else { "" };
    Some((sign, total / 192, total / 16 % 12, total % 16))
}

/// Feet, inches and a fraction, to the nearest 1/16 in: `7' 10 1/2"`. The fraction is in its
/// lowest terms, and the feet are left out when there are none: `11 13/16"`.
pub fn ft_in(m: f64) -> String {
    let Some((sign, feet, inches, sixteenths)) = sixteenths(m) else {
        return "-".to_owned();
    };
    let fraction = match sixteenths {
        0 => String::new(),
        s => {
            let d = 1 << s.trailing_zeros();
            format!("{}/{}", s / d, 16 / d)
        }
    };
    let inches = match (feet, inches, fraction.is_empty()) {
        (0, 0, false) => fraction,
        (_, _, true) => inches.to_string(),
        (_, _, false) => format!("{inches} {fraction}"),
    };
    if feet == 0 {
        format!("{sign}{inches}\"")
    } else {
        format!("{sign}{feet}' {inches}\"")
    }
}

/// Feet, inches and eighths as boatbuilders' tables write them, to the nearest 1/16 in:
/// `7-10-4` is 7 ft 10 4/8 in, and a trailing `+` is one sixteenth more.
pub fn ft_in_8ths(m: f64) -> String {
    let Some((sign, feet, inches, sixteenths)) = sixteenths(m) else {
        return "-".to_owned();
    };
    let plus = if sixteenths % 2 == 1 { "+" } else { "" };
    format!("{sign}{feet}-{inches}-{}{plus}", sixteenths / 2)
}

/// A length for a label: metres, and with `imperial` the feet and inches after it in brackets,
/// `2.400 m (7' 10 1/2")`.
pub fn length(m: f64, imperial: bool) -> String {
    if imperial {
        format!("{} ({})", metres(m), ft_in(m))
    } else {
        metres(m)
    }
}

/// A number with up to three decimals and no trailing zeros.
fn short(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// A drawing scale as a ratio: `1:1`, `1:10` for 0.1, `2:1` for 2.
pub fn scale_label(scale: f64) -> String {
    if scale >= 1.0 {
        format!("{}:1", short(scale))
    } else {
        format!("1:{}", short(1.0 / scale))
    }
}

/// Read a scale written as a ratio, `1:10`, or as one number, `0.1`. Both sides must be more
/// than zero.
pub fn parse_scale(text: &str) -> Result<f64, String> {
    let bad = || {
        format!(
            "`{}` is not a scale: write it as 1:10, or 1 for full size",
            text.trim()
        )
    };
    let number = |part: &str| -> Result<f64, String> {
        part.trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or_else(bad)
    };
    let scale = match text.split(':').collect::<Vec<_>>()[..] {
        [one] => number(one)?,
        [drawn, real] => number(drawn)? / number(real)?,
        _ => return Err(bad()),
    };
    if scale.is_finite() && scale > 0.0 {
        Ok(scale)
    } else {
        Err(bad())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-computed from 1 in = 25.4 mm exactly; each value rounded to the nearest sixteenth of
    /// an inch.
    #[test]
    fn imperial_readings() {
        for (m, eighths, feet) in [
            (0.3048, "1-0-0", "1' 0\""),    // 12 in
            (1.0, "3-3-3", "3' 3 3/8\""),   // 39.370 in -> 630/16
            (2.4, "7-10-4", "7' 10 1/2\""), // 94.488 in -> 1512/16
            (0.3, "0-11-6+", "11 13/16\""), // 11.811 in -> 189/16
            (0.30475, "1-0-0", "1' 0\""),   // 191.97/16 carries to 12 in
            (0.0, "0-0-0", "0\""),
            (-0.05, "-0-1-7+", "-1 15/16\""), // 1.9685 in -> 31/16
            (0.01, "0-0-3", "3/8\""),         // 0.3937 in -> 6/16
            (0.3148, "1-0-3", "1' 0 3/8\""),  // 12.394 in -> 198/16
            (-0.0001, "0-0-0", "0\""),        // rounds to nothing, so no sign
        ] {
            assert_eq!(ft_in_8ths(m), eighths, "{m}");
            assert_eq!(ft_in(m), feet, "{m}");
        }
    }

    #[test]
    fn metric_comes_first_and_alone_by_default() {
        assert_eq!(length(2.4, false), "2.400 m");
        assert_eq!(length(2.4, true), "2.400 m (7' 10 1/2\")");
        assert_eq!(metres(-0.0001), "0.000 m");
        assert_eq!(millimetres(2.0), "2000");
        assert_eq!(millimetres(-0.0004), "0");
        assert_eq!(millimetres(-0.0126), "-13");
        assert_eq!(mm_tenths(0.006), "6.0 mm");
        assert_eq!(mm_tenths(0.01234), "12.3 mm");
    }

    #[test]
    fn scales() {
        assert_eq!(
            (scale_label(1.0), scale_label(0.1), scale_label(2.0)),
            ("1:1".into(), "1:10".into(), "2:1".into())
        );
        assert_eq!(scale_label(0.4), "1:2.5");
        assert_eq!(parse_scale("1:10"), Ok(0.1));
        assert_eq!(parse_scale("1"), Ok(1.0));
        assert_eq!(parse_scale(" 2:1 "), Ok(2.0));
        assert_eq!(parse_scale(&scale_label(0.05)), Ok(0.05));
        for bad in ["1:0", "0:1", "x", "", "1:-2", "1:2:3", "nan", "inf:1", "0"] {
            assert!(parse_scale(bad).is_err(), "{bad}");
        }
    }
}
