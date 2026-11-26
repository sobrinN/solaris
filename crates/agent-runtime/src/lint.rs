use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GfxLintReport {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    #[serde(skip)]
    pub descendant_svg_trap: bool,
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

/// Finds the matching closing `}` for a `{` at `open_brace`, tracking nested braces.
fn find_matching_brace(s: &str, open_brace: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0usize;
    let mut i = open_brace;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn strip_important(val: &str) -> &str {
    let trimmed = val.trim();
    if let Some(idx) = trimmed.rfind("!important")
        && idx + "!important".len() == trimmed.len()
    {
        return trimmed[..idx].trim_end();
    }
    trimmed
}

/// Detects descendant CSS selectors like `.funnel svg { position: absolute; inset: 0; ... }`
/// or bare `svg { position: absolute; ... }` (without a direct-child `>` combinator) that
/// inadvertently capture nested icon `<svg>` elements when a scene contains multiple `<svg>` tags.
fn css_has_descendant_svg_absolute_trap(css: &str) -> bool {
    let lower = css.to_ascii_lowercase();
    let mut cursor = 0usize;
    while cursor < lower.len() {
        let Some(open_rel) = lower[cursor..].find('{') else {
            break;
        };
        let open = cursor + open_rel;
        let prelude_segment = &lower[cursor..open];
        if let Some(semi_rel) = prelude_segment.find(';') {
            let semi = cursor + semi_rel;
            if lower[cursor..semi].trim_start().starts_with('@') {
                cursor = semi + 1;
                continue;
            }
        }
        let Some(close) = find_matching_brace(&lower, open) else {
            break;
        };
        let selector_block = prelude_segment.trim();
        let body = &lower[open + 1..close];
        cursor = close + 1;

        if selector_block.starts_with('@') {
            if (selector_block.starts_with("@media")
                || selector_block.starts_with("@supports")
                || selector_block.starts_with("@layer")
                || selector_block.starts_with("@container")
                || selector_block.starts_with("@scope"))
                && css_has_descendant_svg_absolute_trap(body)
            {
                return true;
            }
            continue;
        }

        let mut positions_absolutely = false;
        let mut has_inset = false;
        let mut has_full_dim = false;
        let mut has_top = false;
        let mut has_left = false;
        for decl in body.split(';') {
            let mut parts = decl.splitn(2, ':');
            let prop = parts.next().map(str::trim).unwrap_or("");
            let val = parts.next().map(strip_important).unwrap_or("");
            match prop {
                "position" if val == "absolute" || val == "fixed" => {
                    positions_absolutely = true;
                }
                "inset" => has_inset = true,
                "width" | "height" if matches!(val, "100%" | "100vw" | "100vh" | "100dvh") => {
                    has_full_dim = true;
                }
                "top" => has_top = true,
                "left" => has_left = true,
                _ => {}
            }
        }
        if !positions_absolutely {
            continue;
        }
        let is_backdrop_sized = has_inset || has_full_dim || (has_top && has_left);

        for raw_sel in selector_block.split(',') {
            let sel = raw_sel.trim();
            if sel == "svg" {
                return true;
            }
            if is_backdrop_sized && sel.len() >= 4 && sel.ends_with("svg") {
                let before_svg = &sel[..sel.len() - 3];
                if before_svg.ends_with(char::is_whitespace) {
                    let trimmed_parent = before_svg.trim_end();
                    if !trimmed_parent.is_empty()
                        && !trimmed_parent.ends_with('>')
                        && !trimmed_parent.ends_with('+')
                        && !trimmed_parent.ends_with('~')
                    {
                        return true;
                    }
                }
            }
        }
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
    let mut has_descendant_svg_abs = false;
    for raw_css in css_snippets {
        let css = strip_css_comments(raw_css);
        if css_has_at_import(&css) {
            has_css_import = true;
        }
        if css_has_external_url(&css) {
            has_css_ext_url = true;
        }
        if css_has_descendant_svg_absolute_trap(&css) {
            has_descendant_svg_abs = true;
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

    let svg_tag_count = scanned.tags.iter().filter(|t| t.name == "svg").count();
    let has_svg_tag = svg_tag_count > 0;
    let descendant_svg_trap = svg_tag_count >= 2 && has_descendant_svg_abs;
    if descendant_svg_trap {
        warnings.push(
            "Descendant CSS selector targeting 'svg' with position:absolute/fixed can capture nested icon <svg> elements; use a direct-child selector (e.g. '.funnel > svg') or an explicit class on the background <svg>."
                .to_string(),
        );
    }
    let js_rotates_without_origin = has_svg_tag
        && js_snippets.iter().any(|js| {
            has_word_token(js, "rotation")
                && !js.contains("svgOrigin")
                && !js.contains("transformOrigin")
        });
    if js_rotates_without_origin {
        warnings.push(
            "SVG rotation animation without svgOrigin or transformOrigin may orbit off-center; set svgOrigin: \"<cx> <cy>\" matching the viewBox center."
                .to_string(),
        );
    }

    GfxLintReport {
        valid: errors.is_empty(),
        errors,
        warnings,
        descendant_svg_trap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_on_descendant_svg_absolute_trap_and_allows_direct_child_selector() {
        let trapped = r##"<style>.funnel svg { position: absolute; inset: 0; width: 100%; height: 100%; }</style><div class="clip funnel" data-start="0" data-duration="4" style="color:var(--brand-ink)"><svg viewBox="0 0 100 100"></svg><div class="ic"><svg viewBox="0 0 24 24"></svg></div></div>"##;
        let rep = lint_gfx_html(trapped);
        assert!(rep.valid);
        assert!(rep.descendant_svg_trap);
        assert!(
            rep.warnings
                .iter()
                .any(|w| w.contains("Descendant CSS selector")),
            "expected descendant svg warning, got: {:?}",
            rep.warnings
        );

        let bare_svg = r##"<style>svg { position: absolute; top: 0; }</style><div class="clip" data-start="0" data-duration="4" style="color:var(--brand-ink)"><svg viewBox="0 0 100 100"></svg><svg viewBox="0 0 24 24"></svg></div>"##;
        let rep_bare = lint_gfx_html(bare_svg);
        assert!(rep_bare.descendant_svg_trap);

        let media_important = r##"<style>@media (min-width: 1px) { .funnel svg { position: absolute !important; inset: 0; } } .other { color: #fff; }</style><div class="clip funnel" data-start="0" data-duration="4" style="color:var(--brand-ink)"><svg viewBox="0 0 100 100"></svg><div class="ic"><svg viewBox="0 0 24 24"></svg></div></div>"##;
        let rep_media = lint_gfx_html(media_important);
        assert!(rep_media.descendant_svg_trap);

        let small_icon = r##"<style>.badge svg { position: absolute; width: 14px; height: 14px; }</style><div class="clip" data-start="0" data-duration="4" style="color:var(--brand-ink)"><svg viewBox="0 0 100 100"></svg><div class="badge"><svg viewBox="0 0 24 24"></svg></div></div>"##;
        let rep_small = lint_gfx_html(small_icon);
        assert!(!rep_small.descendant_svg_trap);
        assert!(
            rep_small.warnings.is_empty(),
            "got: {:?}",
            rep_small.warnings
        );

        let scoped = r##"<style>.funnel > svg { position: absolute; inset: 0; width: 100%; height: 100%; }</style><div class="clip funnel" data-start="0" data-duration="4" style="color:var(--brand-ink)"><svg viewBox="0 0 100 100"></svg><div class="ic"><svg viewBox="0 0 24 24"></svg></div></div>"##;
        let rep_ok = lint_gfx_html(scoped);
        assert!(rep_ok.valid);
        assert!(!rep_ok.descendant_svg_trap);
        assert!(rep_ok.warnings.is_empty(), "got: {:?}", rep_ok.warnings);
    }

    #[test]
    fn warns_on_svg_rotation_without_origin_and_accepts_svg_origin() {
        let missing_origin = r##"<div class="clip" data-start="0" data-duration="4" style="color:var(--brand-ink)"><svg viewBox="0 0 540 540"><g id="globe"></g></svg><script>gsap.to("#globe", { rotation: 360, duration: 4 });</script></div>"##;
        let rep = lint_gfx_html(missing_origin);
        assert!(rep.valid);
        assert!(
            rep.warnings.iter().any(|w| w.contains("svgOrigin")),
            "expected svgOrigin warning, got: {:?}",
            rep.warnings
        );

        let with_origin = r##"<div class="clip" data-start="0" data-duration="4" style="color:var(--brand-ink)"><svg viewBox="0 0 540 540"><g id="globe"></g></svg><script>gsap.to("#globe", { rotation: 360, svgOrigin: "270 270", duration: 4 });</script></div>"##;
        let rep_ok = lint_gfx_html(with_origin);
        assert!(rep_ok.valid);
        assert!(rep_ok.warnings.is_empty());
    }

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
