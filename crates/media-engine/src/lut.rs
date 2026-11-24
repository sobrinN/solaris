use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::MediaError;

const MAX_CUBE_FILE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedCubeLut {
    #[serde(default)]
    pub title: Option<String>,
    pub size: usize,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// Flattened RGB triplets (`size * size * size * 3` floats, B-major/R-minor standard `.cube` order)
    pub data: Vec<f32>,
}

pub fn parse_cube_file(path: &Path) -> Result<ParsedCubeLut, MediaError> {
    let meta = fs::metadata(path)?;
    if meta.len() > MAX_CUBE_FILE_BYTES {
        return Err(MediaError::ParseError(format!(
            ".cube file exceeds {} MiB limit ({} bytes)",
            MAX_CUBE_FILE_BYTES / (1024 * 1024),
            meta.len()
        )));
    }
    let raw = fs::read_to_string(path)?;
    parse_cube_str(&raw)
}

/// Strips inline `# ...` comments while ignoring `#` inside double-quoted strings (e.g. `TITLE "Look #1"`).
fn strip_cube_comment(line: &str) -> &str {
    let mut in_quotes = false;
    for (idx, ch) in line.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            '#' if !in_quotes => return line[..idx].trim(),
            _ => {}
        }
    }
    line.trim()
}

pub fn parse_cube_str(raw: &str) -> Result<ParsedCubeLut, MediaError> {
    let raw = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
    let mut title: Option<String> = None;
    let mut size: Option<usize> = None;
    let mut domain_min = [0.0_f32, 0.0, 0.0];
    let mut domain_max = [1.0_f32, 1.0, 1.0];
    let mut data: Vec<f32> = Vec::new();

    for line in raw.lines() {
        let code = strip_cube_comment(line);
        if code.is_empty() {
            continue;
        }

        let (keyword, rest) = match code.split_once(char::is_whitespace) {
            Some((kw, r)) => (kw, r.trim()),
            None => (code, ""),
        };

        match keyword {
            "TITLE" => {
                title = Some(rest.trim_matches('"').to_string());
                continue;
            }
            "LUT_3D_SIZE" => {
                if size.is_some() {
                    return Err(MediaError::ParseError(
                        "duplicate LUT_3D_SIZE directive".to_string(),
                    ));
                }
                if !data.is_empty() {
                    return Err(MediaError::ParseError(
                        "LUT_3D_SIZE must appear before LUT data rows".to_string(),
                    ));
                }
                let parsed = rest
                    .parse::<usize>()
                    .map_err(|e| MediaError::ParseError(format!("invalid LUT_3D_SIZE: {e}")))?;
                if !(2..=65).contains(&parsed) {
                    return Err(MediaError::ParseError(format!(
                        "LUT_3D_SIZE {parsed} out of supported range 2..=65"
                    )));
                }
                let expected = parsed * parsed * parsed * 3;
                data.reserve_exact(expected);
                size = Some(parsed);
                continue;
            }
            "DOMAIN_MIN" => {
                domain_min = parse_triplet(rest)?;
                continue;
            }
            "DOMAIN_MAX" => {
                domain_max = parse_triplet(rest)?;
                continue;
            }
            "LUT_3D_INPUT_RANGE" => {
                let mut parts = rest.split_whitespace();
                let min_v = parts
                    .next()
                    .and_then(|s| s.parse::<f32>().ok())
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| {
                        MediaError::ParseError(format!("invalid LUT_3D_INPUT_RANGE: '{rest}'"))
                    })?;
                let max_v = parts
                    .next()
                    .and_then(|s| s.parse::<f32>().ok())
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| {
                        MediaError::ParseError(format!("invalid LUT_3D_INPUT_RANGE: '{rest}'"))
                    })?;
                if parts.next().is_some() {
                    return Err(MediaError::ParseError(format!(
                        "unexpected extra token in LUT_3D_INPUT_RANGE: '{rest}'"
                    )));
                }
                domain_min = [min_v; 3];
                domain_max = [max_v; 3];
                continue;
            }
            "LUT_1D_SIZE" | "LUT_1D_INPUT_RANGE" => {
                return Err(MediaError::ParseError(
                    "1D .cube LUTs are not supported; expected LUT_3D_SIZE".to_string(),
                ));
            }
            _ => {}
        }

        if let Some(sz) = size {
            let max_floats = sz * sz * sz * 3;
            if data.len() + 3 > max_floats {
                return Err(MediaError::ParseError(format!(
                    "LUT data exceeds expected {max_floats} floats for size {sz}"
                )));
            }
        }
        let triplet = parse_triplet(code)?;
        data.extend_from_slice(&triplet);
    }

    let size = size.ok_or_else(|| MediaError::ParseError("missing LUT_3D_SIZE".to_string()))?;
    for c in 0..3 {
        if domain_min[c] >= domain_max[c] {
            return Err(MediaError::ParseError(format!(
                "DOMAIN_MIN ({}) must be strictly less than DOMAIN_MAX ({})",
                domain_min[c], domain_max[c]
            )));
        }
    }
    let expected = size * size * size * 3;
    if data.len() != expected {
        return Err(MediaError::ParseError(format!(
            "LUT data length {} does not match expected {} for size {size}",
            data.len(),
            expected
        )));
    }

    Ok(ParsedCubeLut {
        title,
        size,
        domain_min,
        domain_max,
        data,
    })
}

fn parse_triplet(raw: &str) -> Result<[f32; 3], MediaError> {
    let mut parts = raw.split_whitespace();
    let r = parts
        .next()
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .ok_or_else(|| MediaError::ParseError(format!("invalid RGB triplet: '{raw}'")))?;
    let g = parts
        .next()
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .ok_or_else(|| MediaError::ParseError(format!("invalid RGB triplet: '{raw}'")))?;
    let b = parts
        .next()
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .ok_or_else(|| MediaError::ParseError(format!("invalid RGB triplet: '{raw}'")))?;
    if parts.next().is_some() {
        return Err(MediaError::ParseError(format!(
            "unexpected extra token in RGB triplet: '{raw}'"
        )));
    }
    Ok([r, g, b])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_2x2x2_cube_lut_with_bom_hash_title_and_input_range() {
        let cube = "\u{FEFF}TITLE \"Solar Warm #1\" # header comment\n\
LUT_3D_SIZE 2\n\
LUT_3D_INPUT_RANGE 0.0 1.0\n\
0.0 0.0 0.0 # black\n\
1.0 0.0 0.0\n\
0.0 1.0 0.0\n\
1.0 1.0 0.0\n\
0.0 0.0 1.0\n\
1.0 0.0 1.0\n\
0.0 1.0 1.0\n\
1.0 1.0 1.0\n";
        let parsed = parse_cube_str(cube).unwrap();
        assert_eq!(parsed.title.as_deref(), Some("Solar Warm #1"));
        assert_eq!(parsed.size, 2);
        assert_eq!(parsed.data.len(), 24);
    }

    #[test]
    fn rejects_invalid_lut_size_domain_and_duplicate_size() {
        assert!(parse_cube_str("LUT_3D_SIZE 1\n0 0 0\n").is_err());

        let bad_domain = "LUT_3D_SIZE 2\nDOMAIN_MIN 1 0 0\nDOMAIN_MAX 1 1 1\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        assert!(parse_cube_str(bad_domain).is_err());

        let dup_size = "LUT_3D_SIZE 2\nLUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        assert!(parse_cube_str(dup_size).is_err());

        let misplaced = "0 0 0\nLUT_3D_SIZE 2\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        assert!(parse_cube_str(misplaced).is_err());
    }

    #[test]
    fn rejects_non_finite_and_extra_tokens_and_bad_keywords() {
        let nan_cube = "LUT_3D_SIZE 2\nNaN 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n";
        assert!(parse_cube_str(nan_cube).is_err());

        let extra_cube =
            "LUT_3D_SIZE 2\n0 0 0 1\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n0 0 0\n";
        assert!(parse_cube_str(extra_cube).is_err());

        let bad_kw = "LUT_3D_SIZE_X 2\n0 0 0\n";
        assert!(parse_cube_str(bad_kw).is_err());

        let too_many =
            "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n0.5 0.5 0.5\n";
        assert!(parse_cube_str(too_many).is_err());
    }
}
