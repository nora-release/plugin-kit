use crate::error::{ensure, Result};
use regex::Regex;
use std::sync::LazyLock;

pub const MAXIMUM_ICON_BYTES: usize = 512 * 1024;

const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
const SVG_ELEMENTS: [&str; 17] = [
    "svg",
    "g",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "defs",
    "title",
    "desc",
    "clipPath",
    "mask",
    "linearGradient",
    "radialGradient",
    "stop",
];

static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<!--[\s\S]*?-->").unwrap());
static DECLARATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\x{FEFF}?\s*<\?xml\s[^?]*\?>").unwrap());
static ROOT_START: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^<svg(?:\s|>)").unwrap());
static ROOT_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:</svg\s*>|/>$)").unwrap());
static ACTIVE_MARKUP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<[!?]|</?(?:script|foreignObject|image|use|style)\b").unwrap()
});
static ACTIVE_ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:\bhref|\bstyle|\bon[a-z]+)\s*=|url\s*\(").unwrap());
static ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^&(?:amp|lt|gt|quot|apos|#\d+|#x[0-9a-f]+);").unwrap());
static CHARACTER_REFERENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)&#(x[0-9a-f]+|\d+);").unwrap());
static LINKED_RESOURCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)url\s*\(").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<(/?)([A-Za-z][A-Za-z0-9:.-]*)((?:\s+[A-Za-z_:][A-Za-z0-9_:.-]*\s*=\s*(?:"[^"<]*"|'[^'<]*'))*\s*)(/?)>"#,
    )
    .unwrap()
});

/// Accepts a small PNG, JPEG, or static SVG that cannot run code or load
/// anything when Nora draws it.
pub fn validate_icon_data(data: &[u8], path: &str) -> Result<()> {
    ensure(
        !data.is_empty() && data.len() <= MAXIMUM_ICON_BYTES,
        "Plugin icon must be a nonempty file of at most 512 KiB",
    )?;
    if path.ends_with(".png") {
        return ensure(
            data.starts_with(&PNG_SIGNATURE),
            "Plugin PNG icon has an invalid signature",
        );
    }
    if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        return ensure(
            data.len() >= 4 && data.starts_with(&[0xff, 0xd8]) && data.ends_with(&[0xff, 0xd9]),
            "Plugin JPEG icon has an invalid signature",
        );
    }
    ensure(
        path.ends_with(".svg"),
        "Plugin icon must be an SVG, PNG, or JPEG file",
    )?;
    let svg = std::str::from_utf8(data)
        .map_err(|_| crate::Error::new("Plugin SVG icon must be UTF-8"))?;
    validate_svg(svg)
}

fn validate_svg(svg: &str) -> Result<()> {
    let markup = COMMENT.replace_all(svg, "");
    let markup = DECLARATION.replace(&markup, "");
    let markup = markup.trim();
    ensure(
        ROOT_START.is_match(markup)
            && ROOT_END.is_match(markup)
            && !ACTIVE_MARKUP.is_match(markup)
            && !ACTIVE_ATTRIBUTE.is_match(markup)
            && markup.match_indices('&').all(|(index, _)| ENTITY.is_match(&markup[index..])),
        "Plugin SVG icon must be static vector markup without entities, scripts, or linked resources",
    )?;
    let mut valid_references = true;
    let decoded = CHARACTER_REFERENCE.replace_all(markup, |captures: &regex::Captures<'_>| {
        let reference = &captures[1];
        let code = match reference.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => reference.parse().ok(),
        };
        let character = code.and_then(char::from_u32);
        valid_references &= character.is_some();
        character.map(String::from).unwrap_or_default()
    });
    ensure(
        valid_references,
        "Plugin SVG icon has an invalid character reference",
    )?;
    ensure(
        !LINKED_RESOURCE.is_match(&decoded),
        "Plugin SVG icon cannot reference linked resources",
    )?;
    let mut stack: Vec<&str> = Vec::new();
    let mut end = 0;
    let mut roots = 0;
    for tag in TAG.captures_iter(markup) {
        let whole = tag.get(0).unwrap();
        let between = &markup[end..whole.start()];
        ensure(
            !between.contains('<') && (!stack.is_empty() || between.trim().is_empty()),
            "Malformed SVG icon XML",
        )?;
        let closing = !tag[1].is_empty();
        let name = tag.get(2).unwrap().as_str();
        let attributes = &tag[3];
        let self_closing = !tag[4].is_empty();
        ensure(
            SVG_ELEMENTS.contains(&name),
            "Plugin SVG icon contains an unsupported vector element",
        )?;
        if closing {
            ensure(
                !self_closing && attributes.trim().is_empty() && stack.pop() == Some(name),
                "Malformed SVG icon XML",
            )?;
        } else {
            if stack.is_empty() {
                roots += 1;
                ensure(
                    roots == 1 && name == "svg",
                    "Plugin icon must contain one SVG root",
                )?;
            }
            if !self_closing {
                stack.push(name);
            }
        }
        end = whole.end();
    }
    ensure(
        roots == 1 && stack.is_empty() && markup[end..].trim().is_empty(),
        "Malformed SVG icon XML",
    )
}
