use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::model::SavedCodeAsset;

pub const SOLARIS_ASSET_PACK_SCHEMA_VERSION: &str = "solaris.asset-pack/v1";
pub const MAX_GFX_HTML_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeAssetPackManifest {
    pub schema_version: String,
    pub pack_id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub description: String,
    pub assets: Vec<SavedCodeAsset>,
}

fn is_safe_ascii_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn validate_single_asset(asset: &SavedCodeAsset) -> Result<(), String> {
    if !is_safe_ascii_id(&asset.id) {
        return Err(format!(
            "asset id '{}' is invalid: expected 1..=64 ASCII [a-zA-Z0-9_-]",
            asset.id
        ));
    }
    if asset.name.trim().is_empty() {
        return Err("asset name must not be empty".to_string());
    }
    if asset.html_source.trim().is_empty() {
        return Err("asset html_source must not be empty".to_string());
    }
    if asset.html_source.len() > MAX_GFX_HTML_BYTES {
        return Err(format!(
            "asset html_source exceeds maximum size of {} bytes",
            MAX_GFX_HTML_BYTES
        ));
    }
    if !asset.default_duration_sec.is_finite() || asset.default_duration_sec <= 0.0 {
        return Err("asset default_duration_sec must be finite and > 0".to_string());
    }
    Ok(())
}

pub fn validate_asset_pack_manifest(manifest: &CodeAssetPackManifest) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if manifest.schema_version != SOLARIS_ASSET_PACK_SCHEMA_VERSION {
        errors.push(format!(
            "unsupported schema_version '{}'; expected '{}'",
            manifest.schema_version, SOLARIS_ASSET_PACK_SCHEMA_VERSION
        ));
    }
    if !is_safe_ascii_id(&manifest.pack_id) {
        errors.push(format!(
            "invalid pack_id '{}': expected 1..=64 ASCII [a-zA-Z0-9_-]",
            manifest.pack_id
        ));
    }
    if manifest.name.trim().is_empty() {
        errors.push("pack name must not be empty".to_string());
    }
    if manifest.assets.is_empty() {
        errors.push("asset pack must contain at least 1 asset".to_string());
    } else if manifest.assets.len() > 50 {
        errors.push("asset pack must not contain more than 50 assets".to_string());
    }

    let mut seen_ids = HashSet::new();
    for (idx, asset) in manifest.assets.iter().enumerate() {
        if let Err(err) = validate_single_asset(asset) {
            errors.push(format!("assets[{idx}]: {err}"));
        } else if !seen_ids.insert(asset.id.as_str()) {
            errors.push(format!("duplicate asset id '{}' in pack", asset.id));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn build_asset_pack_manifest(
    pack_id: impl Into<String>,
    name: impl Into<String>,
    assets: Vec<SavedCodeAsset>,
) -> CodeAssetPackManifest {
    CodeAssetPackManifest {
        schema_version: SOLARIS_ASSET_PACK_SCHEMA_VERSION.to_string(),
        pack_id: pack_id.into(),
        name: name.into(),
        version: "1.0.0".to_string(),
        author: "Solaris Community".to_string(),
        license: "Apache-2.0".to_string(),
        description: "Reusable Solaris HTML/GSAP motion graphics asset pack".to_string(),
        assets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn sample_asset(id: &str) -> SavedCodeAsset {
        SavedCodeAsset {
            id: id.to_string(),
            name: "Sample Card".to_string(),
            category: "cards".to_string(),
            tags: vec!["sample".to_string()],
            props_schema: json!({ "type": "object", "properties": {} }),
            default_props: json!({}),
            html_source: r#"<div class="clip" data-start="0" data-duration="2.0" style="color:var(--brand-ink);">Hi</div>"#.to_string(),
            default_duration_sec: 2.0,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn validates_valid_and_rejects_malformed_packs() {
        let ok =
            build_asset_pack_manifest("solar-pack", "Solar Pack", vec![sample_asset("card-1")]);
        assert!(validate_asset_pack_manifest(&ok).is_ok());

        let mut bad = ok.clone();
        bad.schema_version = "wrong/v0".to_string();
        bad.pack_id = "../evil".to_string();
        bad.assets.push(sample_asset("card-1")); // duplicate
        let errs = validate_asset_pack_manifest(&bad).unwrap_err();
        assert!(errs.len() >= 3);
    }
}
