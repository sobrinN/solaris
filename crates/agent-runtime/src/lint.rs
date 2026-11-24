use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GfxLintReport {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Matches JS regex `\w` (`[A-Za-z0-9_]`), intentionally excluding `$` so `$fetch()` matches `\bfetch\s*\(`.
fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Checks for `<word><optional whitespace>(` with a non-word boundary before `<word>`.
fn has_call_pattern(source: &str, name: &str) -> bool {
    let bytes = source.as_bytes();
    let needle = name.as_bytes();
    let mut start = 0;
    while start + needle.len() <= bytes.len() {
        let Some(rel) = source[start..].find(name) else {
            break;
        };
        let pos = start + rel;
        let prev_ok = pos == 0 || !is_word_byte(bytes[pos - 1]);
        let mut after = pos + needle.len();
        while after < bytes.len() && bytes[after].is_ascii_whitespace() {
            after += 1;
        }
        if prev_ok && after < bytes.len() && bytes[after] == b'(' {
            return true;
        }
        start = pos + needle.len();
    }
    false
}

/// Checks for `new <whitespace> Date <optional whitespace>(`.
fn has_new_date_call(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut start = 0;
    while start + 3 <= bytes.len() {
        let Some(rel) = source[start..].find("new") else {
            break;
        };
        let pos = start + rel;
        let prev_ok = pos == 0 || !is_word_byte(bytes[pos - 1]);
        let mut i = pos + 3;
        let ws_start = i;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if prev_ok && i > ws_start && source[i..].starts_with("Date") {
            let mut after = i + 4;
            while after < bytes.len() && bytes[after].is_ascii_whitespace() {
                after += 1;
            }
            if after < bytes.len() && bytes[after] == b'(' {
                return true;
            }
        }
        start = pos + 3;
    }
    false
}

/// Checks for an exact identifier token (JS `\b` word boundary before and after).
fn has_word_token(source: &str, name: &str) -> bool {
    let bytes = source.as_bytes();
    let needle = name.as_bytes();
    let mut start = 0;
    while start + needle.len() <= bytes.len() {
        let Some(rel) = source[start..].find(name) else {
            break;
        };
        let pos = start + rel;
        let prev_ok = pos == 0 || !is_word_byte(bytes[pos - 1]);
        let after = pos + needle.len();
        let next_ok = after >= bytes.len() || !is_word_byte(bytes[after]);
        if prev_ok && next_ok {
            return true;
        }
        start = pos + needle.len();
    }
    false
}

/// Checks for `<obj>.<prop>` (with optional whitespace around `.`) bounded by word boundaries.
fn has_dotted_token(source: &str, obj: &str, prop: &str) -> bool {
    let bytes = source.as_bytes();
    let mut start = 0;
    while start + obj.len() <= bytes.len() {
        let Some(rel) = source[start..].find(obj) else {
            break;
        };
        let pos = start + rel;
        let prev_ok = pos == 0 || !is_word_byte(bytes[pos - 1]);
        let mut i = pos + obj.len();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if prev_ok && i < bytes.len() && bytes[i] == b'.' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if source[i..].starts_with(prop) {
                let after = i + prop.len();
                if after >= bytes.len() || !is_word_byte(bytes[after]) {
                    return true;
                }
            }
        }
        start = pos + obj.len();
    }
    false
}

fn is_external_url(raw: &str) -> bool {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() || trimmed.starts_with("data:") || trimmed.starts_with('#') {
        return false;
    }
    true
}

/// Parses HTML `srcset` candidates without splitting on the payload comma inside `data:` URIs.
fn srcset_has_external_url(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        while i < len && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if i >= len {
            break;
        }
        let url_start = i;
        let is_data_uri = raw[i..].len() >= 5 && raw[i..i + 5].eq_ignore_ascii_case("data:");
        let mut seen_data_comma = false;
        while i < len && !bytes[i].is_ascii_whitespace() {
            if bytes[i] == b',' {
                if is_data_uri && !seen_data_comma {
                    seen_data_comma = true;
                } else {
                    break;
                }
            }
            i += 1;
        }
        let url = &raw[url_start..i];
        if is_external_url(url) {
            return true;
        }
        // Skip optional descriptor (e.g. `2x` or `400w`) up to the next comma.
        while i < len && bytes[i] != b',' {
            i += 1;
        }
        if i < len && bytes[i] == b',' {
            i += 1;
        }
    }
    false
}

fn strip_css_comments(css: &str) -> String {
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;
    while i < len {
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
        } else {
            let ch = css[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn css_has_at_import(css: &str) -> bool {
    let lower = css.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut start = 0;
    while let Some(rel) = lower[start..].find("@import") {
        let pos = start + rel;
        let after = pos + "@import".len();
        let next_ok = after >= bytes.len()
            || (!bytes[after].is_ascii_alphanumeric()
                && bytes[after] != b'-'
                && bytes[after] != b'_');
        if next_ok {
            return true;
        }
        start = after;
    }
    false
}

fn css_has_external_url(css: &str) -> bool {
    let lower = css.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while let Some(rel) = lower[i..].find("url") {
        let pos = i + rel;
        let prev_ok = pos == 0 || (!is_word_byte(bytes[pos - 1]) && bytes[pos - 1] != b'-');
        let mut cursor = pos + 3;
        while cursor < len && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if !prev_ok || cursor >= len || bytes[cursor] != b'(' {
            i = pos + 3;
            continue;
        }
        cursor += 1;
        while cursor < len && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let val = if cursor < len && (bytes[cursor] == b'"' || bytes[cursor] == b'\'') {
            let quote = bytes[cursor];
            cursor += 1;
            let val_start = cursor;
            while cursor < len && bytes[cursor] != quote {
                if bytes[cursor] == b'\\' && cursor + 1 < len {
                    cursor += 1 + css[cursor + 1..].chars().next().map_or(0, char::len_utf8);
                } else {
                    cursor += 1;
                }
            }
            let slice = &css[val_start..cursor];
            if cursor < len {
                cursor += 1;
            }
            slice
        } else {
            let val_start = cursor;
            while cursor < len && bytes[cursor] != b')' {
                cursor += 1;
            }
            &css[val_start..cursor]
        };
        if is_external_url(val) {
            return true;
        }
        i = cursor.min(len);
    }
    false
}

#[derive(Debug)]
struct HtmlTag {
    name: String,
    attrs: Vec<(String, String)>,
}

struct ScannedHtml<'a> {
    tags: Vec<HtmlTag>,
    script_blocks: Vec<&'a str>,
    style_blocks: Vec<&'a str>,
}

fn find_closing_tag(html: &str, from: usize, tag_name: &str) -> Option<(usize, usize)> {
    let bytes = html.as_bytes();
    let len = bytes.len();
    let tag_bytes = tag_name.as_bytes();
    let mut i = from;
    while i + 2 + tag_bytes.len() <= len {
        if bytes[i] == b'<'
            && bytes[i + 1] == b'/'
            && bytes[i + 2..i + 2 + tag_bytes.len()].eq_ignore_ascii_case(tag_bytes)
        {
            let mut end = i + 2 + tag_bytes.len();
            if end < len && (bytes[end].is_ascii_whitespace() || bytes[end] == b'>') {
                while end < len && bytes[end] != b'>' {
                    end += 1;
                }
                let after = if end < len { end + 1 } else { len };
                return Some((i, after));
            }
        }
        i += 1;
    }
    None
}

/// Quote-aware HTML scanner that extracts opening tags plus raw `<script>` and `<style>` blocks.
fn scan_html(html: &str) -> ScannedHtml<'_> {
    let bytes = html.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut tags = Vec::new();
    let mut script_blocks = Vec::new();
    let mut style_blocks = Vec::new();

    while i < len {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // Skip HTML comments <!-- ... -->
        if i + 4 <= len && &bytes[i..i + 4] == b"<!--" {
            if let Some(end) = html[i + 4..].find("-->") {
                i = i + 4 + end + 3;
                continue;
            }
            break;
        }
        i += 1;
        if i >= len || bytes[i] == b'/' || bytes[i] == b'!' || bytes[i] == b'?' {
            continue;
        }
        let name_start = i;
        while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
            i += 1;
        }
        if i == name_start {
            continue;
        }
        let tag_name = html[name_start..i].to_ascii_lowercase();
        let mut attrs = Vec::new();
        let mut self_closing = false;

        while i < len {
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= len || bytes[i] == b'>' {
                i = (i + 1).min(len);
                break;
            }
            if bytes[i] == b'/' {
                i += 1;
                if i < len && bytes[i] == b'>' {
                    self_closing = true;
                    i += 1;
                    break;
                }
                continue;
            }
            let attr_start = i;
            while i < len
                && !bytes[i].is_ascii_whitespace()
                && bytes[i] != b'='
                && bytes[i] != b'>'
                && bytes[i] != b'/'
            {
                i += 1;
            }
            let attr_name = html[attr_start..i].to_ascii_lowercase();
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let mut attr_val = String::new();
            if i < len && bytes[i] == b'=' {
                i += 1;
                while i < len && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i < len && (bytes[i] == b'"' || bytes[i] == b'\'') {
                    let quote = bytes[i];
                    i += 1;
                    let val_start = i;
                    while i < len && bytes[i] != quote {
                        i += 1;
                    }
                    attr_val = html[val_start..i].to_string();
                    if i < len {
                        i += 1;
                    }
                } else {
                    let val_start = i;
                    while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                        i += 1;
                    }
                    attr_val = html[val_start..i].to_string();
                }
            }
            if !attr_name.is_empty() {
                attrs.push((attr_name, attr_val));
            }
        }

        if !self_closing && (tag_name == "script" || tag_name == "style") {
            let body_start = i;
            let (body_end, next_pos) =
                find_closing_tag(html, body_start, &tag_name).unwrap_or((len, len));
            let body = &html[body_start..body_end];
            if tag_name == "script" {
                script_blocks.push(body);
            } else {
                style_blocks.push(body);
            }
            i = next_pos;
        }

        tags.push(HtmlTag {
            name: tag_name,
            attrs,
        });
    }

    ScannedHtml {
        tags,
        script_blocks,
        style_blocks,
    }
}

pub fn lint_gfx_html(html: &str) -> GfxLintReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let scanned = scan_html(html);
    let mut has_ext_script = false;
    let mut has_ext_resource = false;
    let mut has_forbidden_media_tag = false;
    let mut has_meta_refresh = false;

    let mut js_snippets: Vec<&str> = scanned.script_blocks;
    let mut css_snippets: Vec<&str> = scanned.style_blocks;

    for tag in &scanned.tags {
        for (k, v) in &tag.attrs {
            if k.starts_with("on") && k.len() > 2 && !v.trim().is_empty() {
                js_snippets.push(v.as_str());
            } else if k == "style" && !v.trim().is_empty() {
                css_snippets.push(v.as_str());
            }
        }

        match tag.name.as_str() {
            "script" if tag.attrs.iter().any(|(k, _)| k == "src") => {
                has_ext_script = true;
            }
            "img"
                if tag.attrs.iter().any(|(k, v)| {
                    (k == "src" && is_external_url(v))
                        || (k == "srcset" && srcset_has_external_url(v))
                }) =>
            {
                has_ext_resource = true;
            }
            "iframe"
                if tag
                    .attrs
                    .iter()
                    .any(|(k, v)| k == "src" && is_external_url(v)) =>
            {
                has_ext_resource = true;
            }
            "link"
                if tag
                    .attrs
                    .iter()
                    .any(|(k, v)| k == "href" && is_external_url(v)) =>
            {
                has_ext_resource = true;
            }
            "image"
                if tag
                    .attrs
                    .iter()
                    .any(|(k, v)| (k == "href" || k == "xlink:href") && is_external_url(v)) =>
            {
                has_ext_resource = true;
            }
            "video" | "audio" | "source" | "object" | "embed" => {
                has_forbidden_media_tag = true;
            }
            "meta"
                if tag.attrs.iter().any(|(k, v)| {
                    k == "http-equiv" && v.trim().eq_ignore_ascii_case("refresh")
                }) =>
            {
                has_meta_refresh = true;
            }
            _ => {}
        }
    }

    if has_ext_script {
        errors.push(
            "External <script src=...> tags are forbidden in Solaris GFX; use inline deterministic CSS or runtime bindings.".to_string(),
        );
    }
    if has_ext_resource {
        errors.push(
            "External URLs in <img>, <image>, <link>, or <iframe> tags are forbidden in Solaris GFX; inline data: URIs or CSS tokens instead.".to_string(),
        );
    }
    if has_forbidden_media_tag {
        errors.push(
            "Embedded media/plugin tags (<video>, <audio>, <source>, <object>, <embed>) are forbidden in Solaris GFX; compose media via timeline tracks.".to_string(),
        );
    }
    if has_meta_refresh {
        errors.push(
            "<meta http-equiv=\"refresh\"> redirects are forbidden in Solaris GFX.".to_string(),
        );
    }

    let mut has_css_import = false;
    let mut has_css_ext_url = false;
    for raw_css in css_snippets {
        let css = strip_css_comments(raw_css);
        if css_has_at_import(&css) {
            has_css_import = true;
        }
        if css_has_external_url(&css) {
            has_css_ext_url = true;
        }
    }
    if has_css_import {
        errors.push(
            "CSS @import rules are forbidden in Solaris GFX; inline deterministic styles only."
                .to_string(),
        );
    }
    if has_css_ext_url {
        errors.push(
            "External CSS url(...) resources are forbidden in Solaris GFX; use data: URIs or #fragment refs.".to_string(),
        );
    }

    for (api, msg) in [
        (
            "Date.now",
            "Non-deterministic API 'Date.now()' is forbidden in Solaris GFX; bind animations to timeline time.",
        ),
        (
            "performance.now",
            "Non-deterministic API 'performance.now()' is forbidden in Solaris GFX; bind animations to timeline time.",
        ),
        (
            "Math.random",
            "Non-deterministic API 'Math.random()' is forbidden in Solaris GFX; use a seeded PRNG.",
        ),
        (
            "setInterval",
            "Non-deterministic API 'setInterval()' is forbidden in Solaris GFX; drive motion via __solarisOnSeek or GSAP timelines.",
        ),
        (
            "setTimeout",
            "Non-deterministic API 'setTimeout()' is forbidden in Solaris GFX; drive motion via __solarisOnSeek or GSAP timelines.",
        ),
        (
            "requestAnimationFrame",
            "Non-deterministic API 'requestAnimationFrame()' is forbidden in Solaris GFX; drive motion via __solarisOnSeek.",
        ),
        (
            "fetch",
            "Network API 'fetch()' is forbidden in Solaris GFX; overlays must render offline deterministically.",
        ),
        (
            "import",
            "Dynamic 'import()' is forbidden in Solaris GFX; inline deterministic code only.",
        ),
    ] {
        if js_snippets.iter().any(|js| has_call_pattern(js, api)) {
            errors.push(msg.to_string());
        }
    }

    if js_snippets.iter().any(|js| has_new_date_call(js)) {
        errors.push(
            "Non-deterministic API 'new Date()' is forbidden in Solaris GFX; bind animations to timeline time.".to_string(),
        );
    }
    if js_snippets
        .iter()
        .any(|js| has_word_token(js, "XMLHttpRequest"))
    {
        errors.push(
            "Network API 'XMLHttpRequest' is forbidden in Solaris GFX; overlays must render offline deterministically.".to_string(),
        );
    }
    if js_snippets.iter().any(|js| has_word_token(js, "WebSocket")) {
        errors.push(
            "Network API 'WebSocket' is forbidden in Solaris GFX; overlays must render offline deterministically.".to_string(),
        );
    }
    if js_snippets
        .iter()
        .any(|js| has_word_token(js, "EventSource"))
    {
        errors.push(
            "Network API 'EventSource' is forbidden in Solaris GFX; overlays must render offline deterministically.".to_string(),
        );
    }
    if js_snippets
        .iter()
        .any(|js| has_dotted_token(js, "navigator", "sendBeacon"))
    {
        errors.push(
            "Network API 'navigator.sendBeacon' is forbidden in Solaris GFX; overlays must render offline deterministically.".to_string(),
        );
    }
    if js_snippets
        .iter()
        .any(|js| has_dotted_token(js, "window", "location"))
    {
        errors.push("Navigation API 'window.location' is forbidden in Solaris GFX.".to_string());
    }

    if !html.contains("data-start") || !html.contains("data-duration") {
        warnings.push(
            "Root element should include data-start and data-duration attributes for deterministic seeking."
                .to_string(),
        );
    }
    if !html.contains("--brand-") {
        warnings.push(
            "Consider using Brand Playbook CSS variables (e.g. var(--brand-accent), var(--brand-ink))."
                .to_string(),
        );
    }

    GfxLintReport {
        valid: errors.is_empty(),
        errors,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_clean_brand_gfx_and_inline_style_and_data_uri_img() {
        let clean = r#"<style>@keyframes fade { from { opacity: 0; } to { opacity: 1; } } .bg { background: url("data:image/png;base64,iVBORw0KGgo="); fill: url(#grad); }</style><div class="clip" data-start="0" data-duration="2.5" style="color:var(--brand-ink);"><img src="data:image/png;base64,iVBORw0KGgo=" srcset="data:image/png;base64,iVBORw0KGgo= 1x" /><svg><image href="data:image/png;base64,iVBORw0KGgo=" /></svg><script>window.__solarisOnSeek = (t) => t;</script></div>"#;
        let ok = lint_gfx_html(clean);
        assert!(ok.valid, "expected valid, got errors: {:?}", ok.errors);
        assert!(ok.warnings.is_empty());
    }

    #[test]
    fn ignores_api_words_in_plain_html_text_and_comments() {
        let prose = r#"<!-- fetch('https://example.com') Date.now() --><div class="clip" data-start="0" data-duration="3.0" style="color:var(--brand-ink);">Learn fetch() and Date.now() vs new Date() and WebSocket in 60s!</div>"#;
        let ok = lint_gfx_html(prose);
        assert!(ok.valid, "expected valid, got errors: {:?}", ok.errors);
    }

    #[test]
    fn rejects_external_scripts_resources_timers_and_network_apis() {
        let bad = r#"<script src="https://cdn.example.com/a.js"></script><img src="https://evil.test/x.png"><script>Date.now (); performance.now(); Math.random(); setTimeout(() => {}, 10); setInterval(() => {}, 10); requestAnimationFrame(() => {}); fetch('https://a.b'); new XMLHttpRequest(); import('https://a.b/m.js');</script>"#;
        let fail = lint_gfx_html(bad);
        assert!(!fail.valid);
        assert_eq!(fail.errors.len(), 11);
    }

    #[test]
    fn rejects_new_date_dollar_bypass_inline_handlers_and_extra_network_apis() {
        let bad_script = r#"<div data-start="0" data-duration="2" style="color:var(--brand-ink)"><script>const d = new Date(); $fetch('https://x'); new WebSocket('wss://x'); new EventSource('/e'); navigator.sendBeacon('/b'); window.location.href = 'https://x';</script></div>"#;
        let fail = lint_gfx_html(bad_script);
        assert!(!fail.valid);
        assert_eq!(fail.errors.len(), 6, "errors: {:?}", fail.errors);

        let inline_handler = r#"<div data-start="0" data-duration="2" style="color:var(--brand-ink)" onclick="fetch('https://evil.test')">Click</div>"#;
        assert!(!lint_gfx_html(inline_handler).valid);
    }

    #[test]
    fn rejects_css_import_external_urls_srcset_media_tags_svg_image_and_meta_refresh() {
        let css_import = r#"<style>@import url("https://fonts.googleapis.com/css2?family=Inter");</style><div data-start="0" data-duration="1" style="color:var(--brand-ink)">Hi</div>"#;
        assert!(!lint_gfx_html(css_import).valid);

        let inline_style_url = r#"<div data-start="0" data-duration="1" style="color:var(--brand-ink);background:url(https://evil.test/bg.png)">Hi</div>"#;
        assert!(!lint_gfx_html(inline_style_url).valid);

        let srcset_bad = r#"<div data-start="0" data-duration="1" style="color:var(--brand-ink)"><img srcset="data:image/png;base64,AAA 1x, https://evil.test/hi.png 2x" /></div>"#;
        assert!(!lint_gfx_html(srcset_bad).valid);

        for tag in [
            r#"<video src="a.mp4"></video>"#,
            r#"<audio src="a.mp3"></audio>"#,
            r#"<source src="a.mp4" />"#,
            r#"<object data="a.swf"></object>"#,
            r#"<embed src="a.swf" />"#,
            r#"<svg><image href="https://evil.test/a.png" /></svg>"#,
            r#"<svg><image xlink:href="https://evil.test/a.png" /></svg>"#,
            r#"<meta http-equiv="refresh" content="0;url=https://evil.test" />"#,
            r#"<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter" />"#,
            r#"<iframe src="https://evil.example/frame"></iframe>"#,
        ] {
            let html = format!(
                r#"{tag}<div data-start="0" data-duration="1" style="color:var(--brand-ink)">Hi</div>"#
            );
            assert!(
                !lint_gfx_html(&html).valid,
                "expected lint error for tag: {tag}"
            );
        }
    }
}
