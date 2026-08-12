use crate::mf4::blocks::conversion::base::ConversionBlock;
use crate::mf4::error::MdfError;
use crate::mf4::parsing::decoder::DecodedValue;
use meval::{Context, eval_str_with_context};

/// Attempts to extract a numeric value from a [`DecodedValue`].
/// Returns `Some(f64)` if the input is numeric, or `None` otherwise.
pub fn extract_numeric(value: &DecodedValue) -> Option<f64> {
    match value {
        DecodedValue::Float(n) => Some(*n),
        DecodedValue::UnsignedInteger(n) => Some(*n as f64),
        DecodedValue::SignedInteger(n) => Some(*n as f64),
        _ => None,
    }
}

/// Apply a linear conversion.
pub fn apply_linear(block: &ConversionBlock, value: DecodedValue) -> Result<DecodedValue, MdfError> {
    if let Some(raw) = extract_numeric(&value) {
        if block.cc_val.len() >= 2 {
            let result = block.cc_val[0] + block.cc_val[1] * raw;
            Ok(DecodedValue::Float(result))
        } else {
            Ok(DecodedValue::Float(raw))
        }
    } else {
        Ok(value)
    }
}

/// Apply a rational conversion.
pub fn apply_rational(block: &ConversionBlock, value: DecodedValue) -> Result<DecodedValue, MdfError> {
    if let Some(raw) = extract_numeric(&value) {
        if block.cc_val.len() >= 6 {
            let p1 = block.cc_val[0];
            let p2 = block.cc_val[1];
            let p3 = block.cc_val[2];
            let p4 = block.cc_val[3];
            let p5 = block.cc_val[4];
            let p6 = block.cc_val[5];

            let num = p1 * raw * raw + p2 * raw + p3;
            let den = p4 * raw * raw + p5 * raw + p6;
            // Always divide: a true zero denominator yields IEEE 754 ±inf/NaN,
            // matching asammdf. Guarding with an epsilon falsified results for
            // legitimate tiny denominators (e.g. 1/x at x = 1e-20).
            Ok(DecodedValue::Float(num / den))
        } else {
            Ok(DecodedValue::Float(raw))
        }
    } else {
        Ok(value)
    }
}

/// Apply an algebraic conversion using a stored formula.
pub fn apply_algebraic(block: &ConversionBlock, value: DecodedValue) -> Result<DecodedValue, MdfError> {
    if let (Some(raw), Some(expr_str)) = (extract_numeric(&value), block.formula.as_ref()) {
        // asammdf compatibility: files converted from MDF3 commonly use `X1`
        // as the variable name; normalize it to `X` before evaluating.
        let expr = expr_str.replace("X1", "X");
        let mut ctx = Context::new();
        ctx.var("X", raw);
        ctx.var("X1", raw); // bind both spellings so either form evaluates
        match eval_str_with_context(&expr, ctx) {
            Ok(res) => Ok(DecodedValue::Float(res)),
            Err(_) => Ok(DecodedValue::Float(raw)),
        }
    } else {
        Ok(value)
    }
}
